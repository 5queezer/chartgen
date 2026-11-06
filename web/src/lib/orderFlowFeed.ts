// Live data of an order-flow chart (ADR-0048): a load, older pages as the chart
// scrolls, and a poll of the newest items, merged by open time. One per chart. The
// poll is slow on purpose: the server reads finer bars for it, and shares them
// between charts for only a few seconds. After a hidden tab, sleep or errors a poll
// asks for all it missed, or starts over if that is more than `maxTail`.

import type { VisibilityDoc } from "./liveFeed";

/** Items kept for a chart; the server sends at most 200 bars per request. */
export const MAX_ITEMS = 600;
const MAX_BACKOFF_MS = 120_000;

export interface FlowRequest<M> {
  /** Exclusive end, ms: an older page. */
  endTime?: number;
  /** This many of the newest items only: the live poll. */
  tail?: number;
  /** What the load said (a page or a poll), so that it lines up with it. */
  meta?: M;
}

export interface FlowPage<T, M> {
  items: T[];
  meta: M;
  hasMore: boolean;
}

export interface FlowState<T, M> {
  items: ReadonlyMap<number, T>;
  meta: M | null;
  status: "loading" | "live" | "error";
  message: string | null;
}

export interface FlowFeedOptions<T extends { t: number }, M extends { row_size: number }> {
  fetch: (request: FlowRequest<M>, signal: AbortSignal) => Promise<FlowPage<T, M>>;
  onChange: (state: FlowState<T, M>) => void;
  /** What a poll or page says over what the load said: they send its row size back. */
  mergeMeta: (load: M, later: M) => M;
  pollMs: number;
  /** Items a poll needs after `gapSecs` without one. */
  tailSize: (gapSecs: number) => number;
  /** Most items a poll may ask for: past this the chart loads again. */
  maxTail: number;
  doc?: VisibilityDoc;
}

export interface FlowFeed {
  /** Start over: the symbol, interval or a setting the server uses changed. */
  restart: () => void;
  stop: () => void;
  /** The chart shows bars from `t` (s): page in older ones if they are missing. */
  ensure: (t: number) => void;
  dispose: () => void;
}

export function createFlowFeed<T extends { t: number }, M extends { row_size: number }>(
  opts: FlowFeedOptions<T, M>,
): FlowFeed {
  const doc = opts.doc ?? document;
  let items = new Map<number, T>();
  let meta: M | null = null;
  let hasMore = false;
  let abort = new AbortController();
  let timer: ReturnType<typeof setTimeout> | null = null;
  let paging = false;
  let failures = 0;
  let running = false;
  // When the newest read that worked was sent. Not the newest item's time: while a
  // market is closed that stays put, and a poll would ask for ever more of nothing.
  let readAt = 0;

  const emit = (status: FlowState<T, M>["status"], message: string | null = null) =>
    opts.onChange({ items, meta, status, message });

  const clearTimer = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };

  const schedule = () => {
    clearTimer();
    if (!running) return;
    const wait = Math.min(opts.pollMs * 2 ** failures, MAX_BACKOFF_MS);
    timer = setTimeout(() => void poll(), wait);
  };

  const merge = (page: FlowPage<T, M>) => {
    const next = new Map(items);
    for (const item of page.items) next.set(item.t, item);
    const sorted = [...next.keys()].sort((a, b) => a - b);
    // Keep the newest: the oldest go first, and scrolling back reads them again.
    for (const t of sorted.slice(0, Math.max(0, sorted.length - MAX_ITEMS))) next.delete(t);
    items = next;
    meta = meta === null ? page.meta : opts.mergeMeta(meta, page.meta);
  };

  const failed = (err: unknown, signal: AbortSignal) => {
    if (signal.aborted) return;
    failures++;
    emit("error", err instanceof Error ? err.message : String(err));
  };

  async function load(): Promise<void> {
    const { signal } = abort;
    const sent = Date.now();
    try {
      const page = await opts.fetch({}, signal);
      if (signal.aborted) return;
      merge(page);
      hasMore = page.hasMore;
      failures = 0;
      readAt = sent;
      emit("live");
    } catch (err) {
      failed(err, signal);
    }
    if (!signal.aborted) schedule();
  }

  async function poll(): Promise<void> {
    timer = null;
    if (!running) return;
    // A hidden tab, or a chart replaying, reads as hidden: nothing to ask for.
    if (doc.visibilityState === "hidden") {
      schedule();
      return;
    }
    if (meta === null) {
      await load();
      return;
    }
    const sent = Date.now();
    const tail = opts.tailSize(Math.max(0, sent - readAt) / 1000);
    if (tail > opts.maxTail) {
      start();
      return;
    }
    const { signal } = abort;
    try {
      const page = await opts.fetch({ tail, meta }, signal);
      if (signal.aborted) return;
      merge(page);
      failures = 0;
      readAt = sent;
      emit("live");
    } catch (err) {
      failed(err, signal);
    }
    if (!signal.aborted) schedule();
  }

  function start(): void {
    abort.abort();
    abort = new AbortController();
    clearTimer();
    items = new Map();
    meta = null;
    hasMore = false;
    paging = false;
    failures = 0;
    running = true;
    emit("loading");
    void load();
  }

  return {
    restart: start,

    stop() {
      running = false;
      abort.abort();
      abort = new AbortController();
      clearTimer();
      items = new Map();
      meta = null;
      opts.onChange({ items, meta, status: "live", message: null });
    },

    ensure(t) {
      if (!running || paging || !hasMore || meta === null || items.size >= MAX_ITEMS) return;
      const oldest = Math.min(...items.keys());
      if (!Number.isFinite(oldest) || t >= oldest) return;
      paging = true;
      const { signal } = abort;
      void opts
        .fetch({ endTime: oldest * 1000, meta }, signal)
        .then((page) => {
          if (signal.aborted) return;
          merge(page);
          hasMore = page.hasMore;
          emit("live");
        })
        .catch((err: unknown) => failed(err, signal))
        .finally(() => {
          if (!signal.aborted) paging = false;
        });
    },

    dispose() {
      running = false;
      abort.abort();
      clearTimer();
    },
  };
}
