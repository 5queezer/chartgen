// Keeps the close series of the compare symbols (ADR-0022) loaded and live.
//
// Per symbol: one full load sized to the main series' loaded bars, a tail
// poll at the main feed's cadence merged by timestamp, and older pages until
// the symbol covers the main series' first bar. A timeframe change drops
// everything and refetches. A symbol removed from the list has its requests
// aborted and its data dropped.
//
// No Solid and no klinecharts: App wires `onChange` to a signal.

import type { SeriesPayload } from "../types/responses";
import { mergeBars, toCompareBars, type CompareBar } from "./compare";
import { MAX_HISTORY_BARS, TAIL_BARS } from "./liveFeed";

/** Bars per older page. */
export const COMPARE_PAGE_BARS = 1_000;
/** Most bars of the initial load (the server's per-call cap). */
export const COMPARE_MAX_INITIAL_BARS = 5_000;

export interface CompareData {
  bars: CompareBar[];
  /** The source may have bars before `bars[0]`. */
  hasMore: boolean;
  /** `loading`: first load pending; `error`: first load failed, retried on the next poll. */
  state: "loading" | "ready" | "error";
}

export interface CompareSync {
  symbols: readonly string[];
  timeframe: string;
  /** Bars of the main series; 0 until it has loaded. */
  mainCount: number;
  /** Time (unix seconds) of the main series' first bar. */
  mainFirst: number | null;
}

export interface CompareFetchArgs {
  bars: number;
  end_time?: number;
  signal: AbortSignal;
}

export interface CompareFeedOptions {
  fetch: (
    request: { ticker: string; timeframe: string },
    args: CompareFetchArgs,
  ) => Promise<SeriesPayload>;
  onChange: (data: Map<string, CompareData>) => void;
  /** Poll period, the main feed's. */
  pollMs: () => number;
  doc?: Pick<Document, "visibilityState">;
}

export interface CompareFeed {
  /** Make the loaded set match `next`; cheap to call on every main-series change. */
  sync: (next: CompareSync) => void;
  dispose: () => void;
}

interface Entry {
  data: CompareData;
  abort: AbortController;
  busy: boolean;
}

export function createCompareFeed(opts: CompareFeedOptions): CompareFeed {
  const doc = opts.doc ?? document;
  const entries = new Map<string, Entry>();
  let want: CompareSync | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const publish = () =>
    opts.onChange(new Map([...entries].map(([s, e]) => [s, e.data])));

  const drop = (symbol: string) => {
    entries.get(symbol)?.abort.abort();
    entries.delete(symbol);
  };

  /** Fetch for `symbol`; `apply` runs only if the entry is still the same. */
  const run = (
    symbol: string,
    e: Entry,
    args: Omit<CompareFetchArgs, "signal">,
    apply: (page: CompareBar[], hasMore: boolean) => void,
    onError: () => void,
  ) => {
    if (!want) return;
    e.busy = true;
    opts
      .fetch(
        { ticker: symbol, timeframe: want.timeframe },
        { ...args, signal: e.abort.signal },
      )
      .then(
        (page) => {
          if (entries.get(symbol) !== e || e.abort.signal.aborted) return;
          e.busy = false;
          apply(toCompareBars(page), page.has_more);
          publish();
          step(symbol);
        },
        () => {
          if (entries.get(symbol) !== e || e.abort.signal.aborted) return;
          e.busy = false;
          onError();
          publish();
        },
      );
  };

  /** The next load `symbol` needs, if any: its first load, or older history. */
  function step(symbol: string): void {
    const e = entries.get(symbol);
    if (!e || e.busy || !want) return;
    const d = e.data;
    if (d.state !== "ready") {
      if (want.mainCount === 0) return;
      run(
        symbol,
        e,
        { bars: Math.min(want.mainCount, COMPARE_MAX_INITIAL_BARS) },
        (bars, hasMore) => {
          e.data = { bars, hasMore, state: "ready" };
        },
        () => {
          e.data = { ...d, state: "error" };
        },
      );
      return;
    }
    const first = d.bars[0]?.t;
    if (
      !d.hasMore ||
      first === undefined ||
      want.mainFirst === null ||
      first <= want.mainFirst ||
      d.bars.length >= MAX_HISTORY_BARS
    ) {
      return;
    }
    run(
      symbol,
      e,
      { bars: COMPARE_PAGE_BARS, end_time: first * 1000 },
      (page, hasMore) => {
        // An empty page ends the history whatever has_more says.
        e.data = {
          bars: mergeBars(page, d.bars),
          hasMore: hasMore && page.length > 0,
          state: "ready",
        };
      },
      () => {},
    );
  }

  const poll = () => {
    timer = null;
    if (entries.size === 0) return;
    if (doc.visibilityState !== "hidden") {
      for (const [symbol, e] of entries) {
        if (e.busy) continue;
        if (e.data.state !== "ready") {
          step(symbol);
          continue;
        }
        run(
          symbol,
          e,
          { bars: TAIL_BARS },
          (tail) => {
            const last = e.data.bars.at(-1)?.t;
            if (tail.length === 0 || last === undefined) return;
            // A gap (sleep, hidden tab): start this symbol over.
            e.data =
              tail[0]!.t > last
                ? { bars: [], hasMore: false, state: "loading" }
                : { ...e.data, bars: mergeBars(e.data.bars, tail) };
          },
          () => {},
        );
      }
    }
    timer = setTimeout(poll, opts.pollMs());
  };

  return {
    sync(next) {
      let changed = false;
      if (want && want.timeframe !== next.timeframe) {
        changed = entries.size > 0;
        for (const symbol of [...entries.keys()]) drop(symbol);
      }
      want = next;
      for (const symbol of [...entries.keys()]) {
        if (!next.symbols.includes(symbol)) {
          drop(symbol);
          changed = true;
        }
      }
      for (const symbol of next.symbols) {
        if (!entries.has(symbol)) {
          changed = true;
          entries.set(symbol, {
            data: { bars: [], hasMore: false, state: "loading" },
            abort: new AbortController(),
            busy: false,
          });
        }
      }
      if (changed) publish();
      for (const symbol of entries.keys()) step(symbol);
      if (entries.size === 0) {
        if (timer !== null) clearTimeout(timer);
        timer = null;
      } else if (timer === null) {
        timer = setTimeout(poll, opts.pollMs());
      }
    },

    dispose() {
      for (const symbol of [...entries.keys()]) drop(symbol);
      if (timer !== null) clearTimeout(timer);
      timer = null;
    },
  };
}
