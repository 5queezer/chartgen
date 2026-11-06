// Keeps the loaded series live (ADR-0009): polls the tail of the chart
// and merges it in, and loads older history pages on request. The merged
// series it emits is what the chart renders.
//
// Push mode (ADR-0021): after a full load the feed also subscribes to
// server-pushed tails (`subscribe_chart`). While a subscription is live,
// interval polling stops and each pushed tail goes through the same
// `mergeTail` path a poll uses. A tail that does not line up (a gap after
// a stalled stream) triggers one poll, which catches up or reloads as
// usual. No push for PUSH_WATCHDOG_MS (or a dropped SSE stream, which ends
// the server-side subscription) falls back to polling and retries the
// subscription with backoff. A hidden tab unsubscribes, like it stops
// polling, and subscribes again when visible. Charts the server cannot
// push (non-Binance symbols) just keep polling.
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
import type { ChartParamsWire } from "./priceBars";
import type { ChartStyle } from "../stores/layout";
import { barCloseMs, isContinuousMarket } from "./countdown";
import {
  mergeOlder,
  mergeTail,
  missingBars,
  sortChannels,
  withoutMisfits,
} from "./seriesMerge";

/** Bars of the initial full load. */
export const INITIAL_BARS = 500;
/** Bars per history page. With WARMUP that is ≤ 1000 klines: one Binance call. */
export const PAGE_BARS = 500;
/**
 * Extra bars each request computes indicators over and then trims
 * (`warmup`), so values at a page seam or on a tail are warmed up. Covers
 * the longest common lookbacks (SMA 200); the server raises it for
 * indicators whose EMAs need more to converge, such as ema_stack's EMA 200
 * (ADR-0032).
 */
export const WARMUP = 300;
/** A tail poll: the last closed bar and the forming one. */
export const TAIL_BARS = 2;
/**
 * One closed source bar can confirm several price-based bars (ADR-0047), and how
 * many is not known beforehand, so their tails are longer. It matches the
 * server's `PRICE_TAIL_BARS`.
 */
export const PRICE_TAIL_BARS = 64;
/** Poll period for 24/7 markets, where a bar can move every second. */
export const FAST_POLL_MS = 2_000;
/** Poll period for session markets (Yahoo data is delayed anyway). */
export const SLOW_POLL_MS = 15_000;
/** Ceiling for the error backoff. */
export const MAX_BACKOFF_MS = 60_000;
/**
 * Most full reloads the feed asks for per chart in `RELOAD_WINDOW_MS`. A reload
 * starts a new generation, so only this outlives it: a chart whose tails never
 * fit its full loads would otherwise reload for as long as it is open.
 */
export const MAX_RELOADS = 3;
export const RELOAD_WINDOW_MS = 5 * 60_000;
/**
 * Longest expected gap between pushes: Binance sends a kline update at
 * least every 2 s, trades in between.
 */
export const PUSH_CADENCE_MS = 2_000;
/** No push for this long: presume the subscription dead and poll again. */
export const PUSH_WATCHDOG_MS = 3 * PUSH_CADENCE_MS;
/** First retry of a failed subscription; doubles up to MAX_BACKOFF_MS. */
export const PUSH_RETRY_MS = 2_000;
/** How long a chart refused for a cap or rate limit polls before trying push again. */
export const PUSH_LIMIT_RETRY_MS = 5 * 60_000;
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
  /** Sent with every poll and page, so merged bars share one style. */
  chart_style: ChartStyle;
  /** Inputs of a price-based style (ADR-0047). */
  chart_params?: ChartParamsWire;
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
  /** Polling the tail every `everyMs`. */
  | { kind: "live"; everyMs: number }
  /** Applying server-pushed tails. */
  | { kind: "push" }
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

/** The page visibility the feed follows (`document`, or bar replay's stand-in). */
export type VisibilityDoc = Pick<
  Document,
  "visibilityState" | "addEventListener" | "removeEventListener"
>;

/** Server push (`subscribe_chart`); without it the feed only polls. */
export interface PushApi {
  /**
   * Subscribe to pushed tails of `request` computed with `warmup`; resolves
   * to the subscription id. Rejects with a message containing "not
   * pushable" for charts the server cannot push, or "push limit reached"
   * when a cap or rate limit refuses it. Anything else, such as "feeds busy"
   * when too many feeds are starting, is retried with a backoff.
   */
  subscribe: (request: LiveRequest, warmup: number) => Promise<string>;
  unsubscribe: (subscriptionId: string) => Promise<unknown>;
}

/** True when `err` says this chart can never be pushed: poll it for this generation. */
export const isNotPushable = (err: unknown) => /not pushable/i.test(message(err));

/** True when a cap or rate limit refused the chart for now: poll, and try again later. */
export const isPushLimit = (err: unknown) => /push limit reached/i.test(message(err));

export interface LiveFeedOptions {
  fetch: (request: LiveRequest, args: FetchArgs) => Promise<SeriesPayload>;
  push?: PushApi;
  /** Full reload through the chart query; ends in `reset` on success. */
  reload: () => Promise<unknown>;
  onSnapshot: (snapshot: LiveSnapshot | null) => void;
  onPollStatus: (status: PollStatus) => void;
  onHistoryStatus: (status: HistoryStatus) => void;
  doc?: VisibilityDoc;
}

export type LoadUntilResult = "covered" | "end" | "limit" | "error" | "aborted";

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
  /**
   * Page back until the oldest loaded bar opens at or before `ts` (seconds).
   * "covered": it does. "end": the source has no older bars. "limit":
   * MAX_HISTORY_BARS are loaded. "error": a page failed (see the history
   * status). "aborted": the chart was replaced meanwhile, or `isCurrent`
   * (checked before each page) turned false.
   */
  loadUntil: (ts: number, isCurrent?: () => boolean) => Promise<LoadUntilResult>;
  /** A pushed tail arrived for `subscriptionId`. */
  applyPush: (subscriptionId: string, tail: SeriesPayload) => void;
  dispose: () => void;
}

const message = (err: unknown) =>
  err instanceof Error ? err.message : String(err);

/** `request` with the box its load was built on. */
function withLoadedBox(request: LiveRequest | null, base: SeriesPayload): LiveRequest | null {
  const box_size = base.chart_params?.box_size;
  return request?.chart_params && box_size !== undefined
    ? { ...request, chart_params: { ...request.chart_params, box_size } }
    : request;
}

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
  // Bars for the next poll: the tail's, or more to catch up after a gap. A bar
  // shorter than the poll needs enough to overlap, or every poll would be a gap.
  const tailBars = () => {
    if (!request) return TAIL_BARS;
    if (request.chart_params) return PRICE_TAIL_BARS;
    const barMs = barCloseMs(0, request.timeframe) ?? Infinity;
    return barMs < period() ? Math.ceil(period() / barMs) + TAIL_BARS : TAIL_BARS;
  };
  let nextBars = TAIL_BARS;
  // Push: the live subscription, a subscribe in flight, failed subscribes
  // since the last push, whether this generation's chart is pushable, and
  // the watchdog or subscribe-retry timer.
  let sub: string | null = null;
  let subscribing = false;
  let pushFailures = 0;
  let pushable = true;
  let pushTimer: ReturnType<typeof setTimeout> | null = null;
  // A pushed tail did not line up and asked for this poll (and its
  // catch-up). Only such a poll is merged while subscribed; any other may
  // carry bars older than the pushes already applied.
  let catchUp = false;
  // Full reloads asked for, and the chart they were for (see MAX_RELOADS).
  let reloads: number[] = [];
  let reloadsOf = "";

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

  const clearPushTimer = () => {
    if (pushTimer !== null) clearTimeout(pushTimer);
    pushTimer = null;
  };

  // Subscriptions still being ended on the server. The next subscribe waits for
  // them: at a per-token cap it could otherwise overtake them and be refused.
  let unsubscribing: Promise<void> | null = null;
  const endSubscription = (id: string) => {
    const done: Promise<void> = Promise.all([
      unsubscribing,
      opts.push?.unsubscribe(id).catch(() => {}),
    ]).then(() => {
      if (unsubscribing === done) unsubscribing = null;
    });
    unsubscribing = done;
  };

  /** Forget the subscription and end it on the server (best effort). */
  const dropSubscription = () => {
    clearPushTimer();
    if (sub !== null) endSubscription(sub);
    sub = null;
  };

  /** End the generation: abort its requests and drop their responses. */
  const endGeneration = () => {
    gen++;
    abort.abort();
    abort = new AbortController();
    clearTimer();
    dropSubscription();
    polling = false;
    reloading = false;
    pageLoading = false;
    failures = 0;
    nextBars = TAIL_BARS;
    subscribing = false;
    pushFailures = 0;
    pushable = true;
    catchUp = false;
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

  /** Whether this chart may still ask for a full reload (MAX_RELOADS). */
  const canReload = () => {
    const key = JSON.stringify(request);
    const now = Date.now();
    if (key !== reloadsOf) {
      reloadsOf = key;
      reloads = [];
    }
    reloads = reloads.filter((at) => now - at < RELOAD_WINDOW_MS);
    return reloads.length < MAX_RELOADS;
  };

  /** Merge a tail. Out of reloads, what does not fit fails alone: the chart stays live. */
  const merge = (cur: SeriesPayload, tail: SeriesPayload) => {
    const r = mergeTail(cur, tail);
    return !r.ok && r.reason === "shape" && !canReload()
      ? mergeTail(cur, withoutMisfits(cur, tail))
      : r;
  };

  const reload = (my: number) => {
    if (!canReload()) {
      backoff(new Error("the chart's data keeps not lining up; reload the page to start over"));
      return;
    }
    reloads.push(Date.now());
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

  const retrySubscribe = () => {
    const inMs = Math.min(PUSH_RETRY_MS * 2 ** pushFailures, MAX_BACKOFF_MS);
    pushFailures++;
    clearPushTimer();
    pushTimer = setTimeout(() => void subscribe(), inMs);
  };

  const armWatchdog = () => {
    clearPushTimer();
    pushTimer = setTimeout(pushLost, PUSH_WATCHDOG_MS);
  };

  /** Pushes stopped: poll (catching up at once) and subscribe again. */
  const pushLost = () => {
    dropSubscription();
    if (!polling && !reloading) {
      clearTimer();
      void poll();
    }
    retrySubscribe();
  };

  async function subscribe(): Promise<void> {
    pushTimer = null;
    const push = opts.push;
    if (!push || !pushable || sub !== null || subscribing) return;
    if (!series || !request || hidden()) return;
    const my = gen;
    subscribing = true;
    if (unsubscribing) {
      await unsubscribing;
      if (my !== gen || hidden()) {
        if (my === gen) subscribing = false;
        return;
      }
    }
    let id: string;
    try {
      id = await push.subscribe(request, WARMUP);
    } catch (err) {
      if (my !== gen) return;
      subscribing = false;
      if (isNotPushable(err)) {
        pushable = false;
      } else if (isPushLimit(err)) {
        clearPushTimer();
        pushTimer = setTimeout(() => void subscribe(), PUSH_LIMIT_RETRY_MS);
      } else {
        retrySubscribe();
      }
      return;
    }
    if (my !== gen || hidden()) {
      // Too late: the chart changed or the tab was hidden meanwhile.
      endSubscription(id);
      if (my === gen) subscribing = false;
      return;
    }
    subscribing = false;
    sub = id;
    // Pushes replace polling; an in-flight poll lands and stops there.
    clearTimer();
    armWatchdog();
    if (!polling && !reloading) opts.onPollStatus({ kind: "push" });
  }

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
      catchUp = false;
      // Pushes keep the chart live; the next misfit tail polls again.
      if (sub === null) backoff(err);
      return;
    }
    if (my !== gen || !series) return;
    polling = false;
    if (sub !== null && !catchUp) {
      // Push took over while this poll was out: drop it.
      opts.onPollStatus({ kind: "push" });
      return;
    }

    const prevLength = series.bars.length;
    const newest = series.bars[prevLength - 1]?.t ?? null;
    const r = merge(series, tail);
    if (r.ok) {
      series = r.series;
      failures = 0;
      nextBars = tailBars();
      emit({ version: ++version, kind: "tail", from: r.from, prevLength });
    } else if (
      r.reason === "gap" &&
      !request.chart_params &&
      bars === tailBars() &&
      newest !== null
    ) {
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
      catchUp = false;
      return reload(my);
    } else if (r.reason !== "stale") {
      // A catch-up that still leaves a gap, or data that does not line
      // up with ours: start over from a full load.
      catchUp = false;
      return reload(my);
    }
    catchUp = false;
    if (sub !== null) {
      opts.onPollStatus({ kind: "push" });
      return;
    }
    opts.onPollStatus({ kind: "live", everyMs: period() });
    schedule(period());
  }

  const onVisibility = () => {
    if (hidden()) {
      // A hidden tab holds no subscription, as it does not poll.
      if (sub !== null || subscribing || pushTimer !== null) {
        dropSubscription();
        if (series) opts.onPollStatus({ kind: "paused" });
      }
      return;
    }
    if (!series || reloading) return;
    // Catch up right away; an in-flight poll reschedules itself.
    if (!polling) {
      clearTimer();
      void poll();
    }
    void subscribe();
  };
  doc.addEventListener("visibilitychange", onVisibility);

  // The page in flight; resolves true once merged, false if it failed or
  // its generation ended.
  let inflight: Promise<boolean> = Promise.resolve(false);

  /** Load the page before `first` (the oldest bar's open time) and merge it. */
  const loadPage = (req: LiveRequest, first: number): Promise<boolean> => {
    const my = gen;
    pageLoading = true;
    opts.onHistoryStatus({ kind: "loading" });
    inflight = opts
      .fetch(req, {
        bars: PAGE_BARS,
        warmup: WARMUP,
        end_time: first * 1000,
        signal: abort.signal,
      })
      .then(
        (page) => {
          if (my !== gen || !series) return false;
          pageLoading = false;
          const r = mergeOlder(series, page);
          if (!r.ok) {
            opts.onHistoryStatus({
              kind: "error",
              message: `history page rejected (${r.reason})`,
            });
            return false;
          }
          series = r.series;
          opts.onHistoryStatus(historyStatusOf(series));
          emit({ version: ++version, kind: "older", count: r.count });
          return true;
        },
        (err) => {
          if (my !== gen) return false;
          pageLoading = false;
          opts.onHistoryStatus({ kind: "error", message: message(err) });
          return false;
        },
      );
    return inflight;
  };

  return {
    reset(base, req) {
      endGeneration();
      series = base && sortChannels(base);
      // Later polls, pages and pushes stay on the box this load resolved (ADR-0047).
      request = base ? withLoadedBox(req, base) : null;
      nextBars = tailBars();
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
        void subscribe();
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
      void loadPage(request, first);
      return true;
    },

    async loadUntil(ts, isCurrent = () => true) {
      const my = gen;
      for (;;) {
        const first = series?.bars[0]?.t;
        if (my !== gen || !isCurrent() || !series || first == null) return "aborted";
        if (first <= ts) return "covered";
        if (!series.has_more) return "end";
        if (series.bars.length >= MAX_HISTORY_BARS) return "limit";
        if (!pageLoading) void loadPage(request!, first);
        if (!(await inflight)) return my !== gen ? "aborted" : "error";
      }
    },

    applyPush(id, tail) {
      if (id !== sub || !series || !request) return;
      armWatchdog();
      pushFailures = 0;
      if (reloading) return;
      const prevLength = series.bars.length;
      const r = merge(series, tail);
      if (r.ok) {
        series = r.series;
        emit({ version: ++version, kind: "tail", from: r.from, prevLength });
      } else if (r.reason !== "stale" && !polling && canReload()) {
        // Missed bars or data that does not line up: a poll catches up
        // (or reloads) exactly as when polling. Out of reloads it could do
        // neither, and pushes would turn into one poll each.
        clearTimer();
        catchUp = true;
        void poll();
      }
    },

    dispose() {
      endGeneration();
      doc.removeEventListener("visibilitychange", onVisibility);
    },
  };
}
