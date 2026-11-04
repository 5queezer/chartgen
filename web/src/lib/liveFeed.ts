// Keeps the loaded series live (ADR-0009): polls the tail of the chart
// and merges it in, and loads older history pages on request. The merged
// series it emits is what the chart renders.
//
// Lifecycle: `reset(base, request)` adopts a full load (from the TanStack
// chart query) and starts a new generation; `suspend()` stops one while
// the next full load is in flight. Every request carries the generation
// it was made for and is aborted, or its response dropped, once that
// generation is over, so a symbol/timeframe/indicator change can never
// merge bars of the old chart into the new one.
//
// No Solid and no klinecharts: App wires the callbacks to signals, and
// liveFeed.test.ts drives it with fake timers and a fake document.

import type { SeriesPayload } from "../types/responses";
import type { ActiveIndicator } from "../stores/indicators";
import { barCloseMs, isContinuousMarket } from "./countdown";
import {
  mergeOlder,
  mergeTail,
  missingBars,
  sortChannels,
} from "./seriesMerge";

/** Bars of the initial full load. */
export const INITIAL_BARS = 500;
/** Bars per history page. With WARMUP that is ≤ 1000 klines: one Binance call. */
export const PAGE_BARS = 500;
/**
 * Extra bars each request computes indicators over and then trims
 * (`warmup`), so values at a page seam or on a tail are warmed up. Covers
 * the longest common lookbacks (SMA/EMA 200) with room for EMA-style
 * recursions to converge.
 */
export const WARMUP = 300;
/** A tail poll: the last closed bar and the forming one. */
export const TAIL_BARS = 2;
/** Poll period for 24/7 markets, where a bar can move every second. */
export const FAST_POLL_MS = 2_000;
/** Poll period for session markets (Yahoo data is delayed anyway). */
export const SLOW_POLL_MS = 15_000;
/** Ceiling for the error backoff. */
export const MAX_BACKOFF_MS = 60_000;
/**
 * Largest catch-up tail after missed bars (sleep, hidden tab); with
 * WARMUP still one Binance call. A longer gap reloads the chart.
 */
export const MAX_CATCHUP_BARS = 700;
/**
 * Most bars kept loaded. Paging stops there ("history limit reached"):
 * every tick recalculates every indicator over all loaded bars.
 */
export const MAX_HISTORY_BARS = 20_000;

/** True if `series` may page in older bars: the source has more, under the cap. */
export function canPageBefore(series: SeriesPayload): boolean {
  return series.has_more && series.bars.length < MAX_HISTORY_BARS;
}

const historyStatusOf = (series: SeriesPayload): HistoryStatus =>
  !series.has_more
    ? { kind: "end" }
    : series.bars.length >= MAX_HISTORY_BARS
      ? { kind: "limit" }
      : { kind: "idle" };

export interface LiveRequest {
  ticker: string;
  timeframe: string;
  indicators: ActiveIndicator[];
}

export interface FetchArgs {
  bars: number;
  warmup: number;
  end_time?: number;
  signal: AbortSignal;
}

/**
 * What changed in the latest snapshot. `version` increases by one per
 * snapshot, so a consumer can tell whether it saw the previous one and
 * may apply this change incrementally.
 */
export type SeriesUpdate =
  | { version: number; kind: "reset" }
  /** `count` bars were prepended. */
  | { version: number; kind: "older"; count: number }
  /** Bars from `from` on changed; there were `prevLength` bars before. */
  | { version: number; kind: "tail"; from: number; prevLength: number };

export interface LiveSnapshot {
  series: SeriesPayload;
  update: SeriesUpdate;
}

export type PollStatus =
  | { kind: "off" }
  | { kind: "live"; everyMs: number }
  | { kind: "paused" }
  | { kind: "retrying"; inMs: number; message: string }
  | { kind: "reloading" };

export type HistoryStatus =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "error"; message: string }
  /** The source has no older bars. */
  | { kind: "end" }
  /** MAX_HISTORY_BARS are loaded; paging stopped. */
  | { kind: "limit" };

type VisibilityDoc = Pick<
  Document,
  "visibilityState" | "addEventListener" | "removeEventListener"
>;

export interface LiveFeedOptions {
  fetch: (request: LiveRequest, args: FetchArgs) => Promise<SeriesPayload>;
  /** Full reload through the chart query; ends in `reset` on success. */
  reload: () => Promise<unknown>;
  onSnapshot: (snapshot: LiveSnapshot | null) => void;
  onPollStatus: (status: PollStatus) => void;
  onHistoryStatus: (status: HistoryStatus) => void;
  doc?: VisibilityDoc;
}

export interface LiveFeed {
  /** Adopt a full load (ownership passes to the feed) or clear with null. */
  reset: (base: SeriesPayload | null, request: LiveRequest | null) => void;
  /** End the current generation but keep showing its last snapshot. */
  suspend: () => void;
  /**
   * Load the page before the oldest bar. True if a load is now in flight;
   * false if there is nothing to load (no series, no more history).
   */
  loadOlder: () => boolean;
  dispose: () => void;
}

const message = (err: unknown) =>
  err instanceof Error ? err.message : String(err);

export function createLiveFeed(opts: LiveFeedOptions): LiveFeed {
  const doc = opts.doc ?? document;
  let gen = 0;
  let version = 0;
  let series: SeriesPayload | null = null;
  let request: LiveRequest | null = null;
  let abort = new AbortController();
  let timer: ReturnType<typeof setTimeout> | null = null;
  let polling = false;
  // A full reload is in flight: no polls until it lands (reset) or fails.
  let reloading = false;
  let pageLoading = false;
  let failures = 0;
  // Bars for the next poll: TAIL_BARS, or more to catch up after a gap.
  let nextBars = TAIL_BARS;

  const hidden = () => doc.visibilityState === "hidden";
  const period = () =>
    request && isContinuousMarket(request.ticker) ? FAST_POLL_MS : SLOW_POLL_MS;

  const emit = (update: SeriesUpdate) => {
    if (series) opts.onSnapshot({ series, update });
  };

  const clearTimer = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };

  /** End the generation: abort its requests and drop their responses. */
  const endGeneration = () => {
    gen++;
    abort.abort();
    abort = new AbortController();
    clearTimer();
    polling = false;
    reloading = false;
    pageLoading = false;
    failures = 0;
    nextBars = TAIL_BARS;
  };

  const schedule = (ms: number) => {
    clearTimer();
    timer = setTimeout(poll, ms);
  };

  const backoff = (err: unknown) => {
    failures++;
    const inMs = Math.min(period() * 2 ** failures, MAX_BACKOFF_MS);
    opts.onPollStatus({ kind: "retrying", inMs, message: message(err) });
    schedule(inMs);
  };

  const reload = (my: number) => {
    clearTimer();
    reloading = true;
    opts.onPollStatus({ kind: "reloading" });
    // A successful reload lands in reset() and starts a new generation.
    // The query publishes its result to App a few ticks after its
    // refetch() promise settles (TanStack notifies in a later task), so
    // the check is deferred by a task. If this generation is still current
    // then, the reload failed: keep polling, backed off.
    const resume = () =>
      setTimeout(() => {
        if (my !== gen) return;
        reloading = false;
        backoff(new Error("reload did not replace the chart"));
      }, 0);
    opts.reload().then(resume, resume);
  };

  async function poll(): Promise<void> {
    timer = null;
    if (!series || !request || polling || reloading) return;
    if (hidden()) {
      opts.onPollStatus({ kind: "paused" });
      return;
    }
    const my = gen;
    const bars = nextBars;
    polling = true;
    let tail: SeriesPayload;
    try {
      tail = await opts.fetch(request, {
        bars,
        warmup: WARMUP,
        signal: abort.signal,
      });
    } catch (err) {
      if (my !== gen) return;
      polling = false;
      backoff(err);
      return;
    }
    if (my !== gen || !series) return;
    polling = false;

    const prevLength = series.bars.length;
    const newest = series.bars[prevLength - 1]?.t ?? null;
    const r = mergeTail(series, tail);
    if (r.ok) {
      series = r.series;
      failures = 0;
      nextBars = TAIL_BARS;
      emit({ version: ++version, kind: "tail", from: r.from, prevLength });
    } else if (r.reason === "gap" && bars === TAIL_BARS && newest !== null) {
      // Missed bars (sleep, hidden tab): fetch enough to overlap again,
      // with margin for a rollover while that request runs.
      const head = tail.bars[0]!.t!;
      const close = barCloseMs(newest, series.interval ?? request.timeframe);
      const step = close === null ? null : close / 1000 - newest;
      const need = step ? missingBars(newest, head, step) + TAIL_BARS + 3 : Infinity;
      if (need <= MAX_CATCHUP_BARS) {
        nextBars = need;
        return poll();
      }
      // The estimate counts every interval of the gap. A session market
      // has no bars while closed (nights, weekends), so the real gap can
      // be far smaller: try the largest catch-up before reloading.
      if (!isContinuousMarket(request.ticker)) {
        nextBars = MAX_CATCHUP_BARS;
        return poll();
      }
      return reload(my);
    } else if (r.reason !== "stale") {
      // A catch-up that still leaves a gap, or data that does not line
      // up with ours: start over from a full load.
      return reload(my);
    }
    opts.onPollStatus({ kind: "live", everyMs: period() });
    schedule(period());
  }

  const onVisibility = () => {
    if (hidden() || !series || reloading) return;
    // Catch up right away; an in-flight poll reschedules itself.
    if (!polling) {
      clearTimer();
      void poll();
    }
  };
  doc.addEventListener("visibilitychange", onVisibility);

  return {
    reset(base, req) {
      endGeneration();
      series = base && sortChannels(base);
      request = base ? req : null;
      opts.onHistoryStatus(base ? historyStatusOf(base) : { kind: "idle" });
      if (!base) {
        opts.onPollStatus({ kind: "off" });
        opts.onSnapshot(null);
        return;
      }
      emit({ version: ++version, kind: "reset" });
      if (hidden()) {
        opts.onPollStatus({ kind: "paused" });
      } else {
        opts.onPollStatus({ kind: "live", everyMs: period() });
        schedule(period());
      }
    },

    suspend() {
      endGeneration();
      opts.onPollStatus({ kind: "off" });
      opts.onHistoryStatus({ kind: "idle" });
    },

    loadOlder() {
      if (!series || !request) return false;
      if (pageLoading) return true;
      const first = series.bars[0]?.t;
      if (!canPageBefore(series) || first == null) {
        opts.onHistoryStatus(historyStatusOf(series));
        return false;
      }
      const my = gen;
      pageLoading = true;
      opts.onHistoryStatus({ kind: "loading" });
      opts
        .fetch(request, {
          bars: PAGE_BARS,
          warmup: WARMUP,
          end_time: first * 1000,
          signal: abort.signal,
        })
        .then(
          (page) => {
            if (my !== gen || !series) return;
            pageLoading = false;
            const r = mergeOlder(series, page);
            if (!r.ok) {
              opts.onHistoryStatus({
                kind: "error",
                message: `history page rejected (${r.reason})`,
              });
              return;
            }
            series = r.series;
            opts.onHistoryStatus(historyStatusOf(series));
            emit({ version: ++version, kind: "older", count: r.count });
          },
          (err) => {
            if (my !== gen) return;
            pageLoading = false;
            opts.onHistoryStatus({ kind: "error", message: message(err) });
          },
        );
      return true;
    },

    dispose() {
      endGeneration();
      doc.removeEventListener("visibilitychange", onVisibility);
    },
  };
}
