import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SeriesPayload } from "../types/responses";
import {
  FAST_POLL_MS,
  MAX_CATCHUP_BARS,
  MAX_HISTORY_BARS,
  MAX_RELOADS,
  PAGE_BARS,
  PUSH_RETRY_MS,
  PUSH_WATCHDOG_MS,
  RELOAD_WINDOW_MS,
  SLOW_POLL_MS,
  TAIL_BARS,
  WARMUP,
  createLiveFeed,
  type FetchArgs,
  type HistoryStatus,
  type LiveFeed,
  type LiveRequest,
  type LiveSnapshot,
  type PollStatus,
  type PushApi,
  PUSH_LIMIT_RETRY_MS,
} from "./liveFeed";

// A 1m market whose forming bar is `now` (a bar index) and whose history
// starts at bar 0. Bar i opens at BASE + 60 i.
const BASE = 1_700_000_040; // on the 1m grid
const tOf = (i: number) => BASE + i * 60;

const range = (first: number, last: number) =>
  Array.from({ length: Math.max(0, last - first + 1) }, (_, k) => first + k);

function payload(first: number, last: number, has_more = true): SeriesPayload {
  return payloadOf(range(first, last), has_more);
}

/** A payload holding exactly the bars `idx` (gaps allowed: closed hours). */
function payloadOf(idx: number[], has_more = true): SeriesPayload {
  return {
    symbol: "BTCUSDT",
    interval: "1m",
    has_more,
    bars: idx.map((i) => ({ t: tOf(i), o: i, h: i, l: i, c: i + market.tick, v: 1 })),
    indicators: {
      sma: {
        label: "SMA",
        is_overlay: true,
        lines: [{ index: 0, label: null, values: idx.map((i) => i) }],
      },
    },
  };
}

const market = { now: 1000, tick: 0, fail: false };

/** Server stand-in honoring bars / end_time like generate_chart. */
function serve(_req: LiveRequest, args: FetchArgs): Promise<SeriesPayload> {
  if (market.fail) return Promise.reject(new Error("upstream down"));
  const end = args.end_time === undefined ? market.now : (args.end_time / 1000 - BASE) / 60 - 1;
  const last = Math.min(end, market.now);
  const first = Math.max(0, last - args.bars + 1);
  return Promise.resolve(payload(first, last, first > 0));
}

class FakeDoc extends EventTarget {
  visibilityState: DocumentVisibilityState = "visible";
  set(state: DocumentVisibilityState) {
    this.visibilityState = state;
    this.dispatchEvent(new Event("visibilitychange"));
  }
}

const REQ: LiveRequest = {
  ticker: "BTCUSDT",
  timeframe: "1m",
  indicators: ["sma"],
  chart_style: "candles",
};

function setup(
  fetchImpl = serve,
  reloadImpl: (feed: LiveFeed) => Promise<void> = () => Promise.resolve(),
  push?: PushApi,
) {
  const doc = new FakeDoc();
  const snapshots: Array<LiveSnapshot | null> = [];
  const polls: PollStatus[] = [];
  const history: HistoryStatus[] = [];
  const fetch = vi.fn(fetchImpl);
  const reload = vi.fn(() => reloadImpl(feed));
  const feed = createLiveFeed({
    fetch,
    reload,
    push,
    onSnapshot: (s) => snapshots.push(s),
    onPollStatus: (s) => polls.push(s),
    onHistoryStatus: (s) => history.push(s),
    doc: doc as unknown as Document,
  });
  const last = () => snapshots.at(-1) ?? null;
  return { doc, feed, fetch, reload, snapshots, polls, history, last };
}

beforeEach(() => {
  vi.useFakeTimers();
  market.now = 1000;
  market.tick = 0;
  market.fail = false;
});
afterEach(() => vi.useRealTimers());

describe("live tail polling", () => {
  it("polls bars=2 with warmup every 2 s for a 24/7 symbol and merges", async () => {
    const t = setup();
    t.feed.reset(payload(501, 1000), REQ);
    expect(t.last()!.update).toEqual({ version: 1, kind: "reset" });

    market.tick = 0.5;
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS);
    expect(t.fetch).toHaveBeenCalledTimes(1);
    expect(t.fetch.mock.calls[0]![1]).toMatchObject({ bars: TAIL_BARS, warmup: WARMUP });
    expect(t.fetch.mock.calls[0]![1].end_time).toBeUndefined();
    expect(t.last()!.update).toEqual({ version: 2, kind: "tail", from: 498, prevLength: 500 });
    expect(t.last()!.series.bars.at(-1)!.c).toBe(1000.5);

    market.now = 1001; // rollover
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS);
    expect(t.last()!.update).toEqual({ version: 3, kind: "tail", from: 499, prevLength: 500 });
    expect(t.last()!.series.bars).toHaveLength(501);
    expect(t.polls.at(-1)).toEqual({ kind: "live", everyMs: FAST_POLL_MS });
  });

  it("polls every 15 s for a session market", async () => {
    const t = setup();
    t.feed.reset(payload(501, 1000), { ...REQ, ticker: "AAPL" });
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS * 3);
    expect(t.fetch).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(SLOW_POLL_MS - FAST_POLL_MS * 3);
    expect(t.fetch).toHaveBeenCalledTimes(1);
  });

  it("pauses while hidden and catches up at once when visible", async () => {
    const t = setup();
    t.feed.reset(payload(501, 1000), REQ);
    t.doc.set("hidden");
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS * 10);
    expect(t.fetch).not.toHaveBeenCalled();
    expect(t.polls.at(-1)).toEqual({ kind: "paused" });

    t.doc.set("visible");
    await vi.advanceTimersByTimeAsync(0);
    expect(t.fetch).toHaveBeenCalledTimes(1);
  });

  it("backs off exponentially on errors and recovers", async () => {
    const t = setup();
    t.feed.reset(payload(501, 1000), REQ);
    market.fail = true;
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS);
    expect(t.polls.at(-1)).toMatchObject({ kind: "retrying", inMs: 4_000 });
    await vi.advanceTimersByTimeAsync(4_000);
    expect(t.polls.at(-1)).toMatchObject({ kind: "retrying", inMs: 8_000 });
    expect(t.fetch).toHaveBeenCalledTimes(2);
    // Nothing in between: no request spam during the backoff.
    await vi.advanceTimersByTimeAsync(7_999);
    expect(t.fetch).toHaveBeenCalledTimes(2);
    market.fail = false;
    await vi.advanceTimersByTimeAsync(1);
    expect(t.polls.at(-1)).toEqual({ kind: "live", everyMs: FAST_POLL_MS });
    // The chart is never cleared by a failed poll.
    expect(t.snapshots).not.toContain(null);
  });

  it("drops a response of an older generation and aborts its request", async () => {
    let release!: (s: SeriesPayload) => void;
    let signal!: AbortSignal;
    const t = setup((_r, args) => {
      signal = args.signal;
      return new Promise((res) => (release = res));
    });
    t.feed.reset(payload(501, 1000), REQ);
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS);
    expect(t.fetch).toHaveBeenCalledTimes(1);

    t.feed.reset(payload(1, 200), { ...REQ, timeframe: "5m" });
    expect(signal.aborted).toBe(true);
    const versions = t.snapshots.length;
    release(payload(999, 1000));
    await vi.advanceTimersByTimeAsync(0);
    expect(t.snapshots).toHaveLength(versions);
    expect(t.last()!.series.bars.at(-1)!.t).toBe(tOf(200));
  });

  it("suspend stops polling but keeps the last snapshot", async () => {
    const t = setup();
    t.feed.reset(payload(501, 1000), REQ);
    t.feed.suspend();
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS * 5);
    expect(t.fetch).not.toHaveBeenCalled();
    expect(t.last()).not.toBeNull();
    expect(t.polls.at(-1)).toEqual({ kind: "off" });
  });

  it("fetches a larger tail after missed bars", async () => {
    const t = setup();
    t.feed.reset(payload(501, 1000), REQ);
    market.now = 1010;
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS);
    expect(t.fetch.mock.calls.map((c) => c[1].bars)).toEqual([2, 8 + TAIL_BARS + 3]);
    const s = t.last()!.series;
    expect(s.bars.at(-1)!.t).toBe(tOf(1010));
    expect(s.bars).toHaveLength(510);
    expect(t.reload).not.toHaveBeenCalled();
  });

  it("reloads after a gap too long to catch up", async () => {
    const t = setup();
    t.feed.reset(payload(501, 1000), REQ);
    market.now = 5000;
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS);
    expect(t.reload).toHaveBeenCalledTimes(1);
    expect(t.polls).toContainEqual({ kind: "reloading" });
    await vi.advanceTimersByTimeAsync(1); // the deferred failure check
    // The fake reload never resets the feed, so polling resumes backed off.
    expect(t.polls.at(-1)).toMatchObject({ kind: "retrying", inMs: 4_000 });
  });

  it("does not poll during a reload, and a landed reload shows no error", async () => {
    let finish!: () => void;
    const t = setup(serve, (feed) => {
      // Like TanStack: the refetch promise settles, and the new data
      // reaches App (reset) in a later task.
      return new Promise<void>((res) => {
        finish = () => {
          res();
          setTimeout(() => feed.reset(payload(4501, 5000), REQ), 0);
        };
      });
    });
    t.feed.reset(payload(501, 1000), REQ);
    market.now = 5000;
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS);
    expect(t.reload).toHaveBeenCalledTimes(1);
    const calls = t.fetch.mock.calls.length;
    t.doc.set("hidden");
    t.doc.set("visible"); // would normally catch up at once
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS * 5);
    expect(t.fetch.mock.calls.length).toBe(calls);

    finish();
    await vi.advanceTimersByTimeAsync(10);
    expect(t.last()!.update.kind).toBe("reset");
    expect(t.polls.some((p) => p.kind === "retrying")).toBe(false);
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS);
    expect(t.polls.at(-1)).toEqual({ kind: "live", everyMs: FAST_POLL_MS });
  });

  it("tries one maximal tail before reloading a session market", async () => {
    // A market closed between bar 1000 and bar 3000: by the clock ~2000
    // bars are missing, but the real gap is two bars.
    let open = [...range(0, 1000), 3000, 3001];
    const t = setup((_r, args) => Promise.resolve(payloadOf(open.slice(-args.bars))));
    t.feed.reset(payload(501, 1000), { ...REQ, ticker: "AAPL" });
    await vi.advanceTimersByTimeAsync(SLOW_POLL_MS);
    expect(t.fetch.mock.calls.map((c) => c[1].bars)).toEqual([2, MAX_CATCHUP_BARS]);
    expect(t.reload).not.toHaveBeenCalled();
    expect(t.last()!.series.bars.at(-1)!.t).toBe(tOf(3001));

    // 1000 more bars after another break: the maximal tail cannot reach.
    open = [...open, ...range(9000, 9999)];
    await vi.advanceTimersByTimeAsync(SLOW_POLL_MS);
    expect(t.fetch.mock.calls.slice(2).map((c) => c[1].bars)).toEqual([2, MAX_CATCHUP_BARS]);
    expect(t.reload).toHaveBeenCalledTimes(1);
  });

  it("suspend resets the history status", () => {
    const t = setup();
    t.feed.reset(payload(501, 1000), REQ);
    t.feed.loadOlder();
    t.feed.suspend();
    expect(t.history.at(-1)).toEqual({ kind: "idle" });
  });
});

describe("history paging", () => {
  it("loads the page before the oldest bar, one at a time", async () => {
    const t = setup();
    t.feed.reset(payload(501, 1000), REQ);
    expect(t.feed.loadOlder()).toBe(true);
    expect(t.feed.loadOlder()).toBe(true); // already in flight
    expect(t.fetch).toHaveBeenCalledTimes(1);
    expect(t.fetch.mock.calls[0]![1]).toMatchObject({
      bars: PAGE_BARS,
      warmup: WARMUP,
      end_time: tOf(501) * 1000,
    });
    expect(t.history.at(-1)).toEqual({ kind: "loading" });
    await vi.advanceTimersByTimeAsync(0);
    expect(t.last()!.update).toEqual({ version: 2, kind: "older", count: 500 });
    expect(t.last()!.series.bars[0]!.t).toBe(tOf(1));
    expect(t.history.at(-1)).toEqual({ kind: "idle" });

    t.feed.loadOlder();
    await vi.advanceTimersByTimeAsync(0);
    expect(t.last()!.update).toMatchObject({ kind: "older", count: 1 });
    expect(t.history.at(-1)).toEqual({ kind: "end" });
    expect(t.feed.loadOlder()).toBe(false);
  });

  it("reports an error and retries on request", async () => {
    const t = setup();
    t.feed.reset(payload(501, 1000), REQ);
    market.fail = true;
    t.feed.loadOlder();
    await vi.advanceTimersByTimeAsync(0);
    expect(t.history.at(-1)).toEqual({ kind: "error", message: "upstream down" });
    market.fail = false;
    expect(t.feed.loadOlder()).toBe(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(t.last()!.update).toMatchObject({ kind: "older", count: 500 });
  });

  it("stops at the history limit", async () => {
    const t = setup();
    t.feed.reset(payload(MAX_HISTORY_BARS * 2 - 500, MAX_HISTORY_BARS * 2), REQ);
    market.now = MAX_HISTORY_BARS * 2;
    // Page until the cap: 501 + 39 * 500 = 20001 bars.
    for (let i = 0; i < 39; i++) {
      expect(t.feed.loadOlder()).toBe(true);
      await vi.advanceTimersByTimeAsync(0);
    }
    expect(t.last()!.series.bars.length).toBeGreaterThanOrEqual(MAX_HISTORY_BARS);
    expect(t.history.at(-1)).toEqual({ kind: "limit" });
    const calls = t.fetch.mock.calls.length;
    expect(t.feed.loadOlder()).toBe(false);
    expect(t.fetch.mock.calls.length).toBe(calls);
  });

  it("loadUntil pages back until the timestamp is covered", async () => {
    const t = setup();
    t.feed.reset(payload(501, 1000), REQ);
    const done = t.feed.loadUntil(tOf(100));
    await vi.advanceTimersByTimeAsync(0);
    await expect(done).resolves.toBe("covered");
    expect(t.last()!.series.bars[0]!.t).toBe(tOf(1));
    expect(t.fetch).toHaveBeenCalledTimes(1);
    // Already covered: no request.
    await expect(t.feed.loadUntil(tOf(500))).resolves.toBe("covered");
    expect(t.fetch).toHaveBeenCalledTimes(1);
  });

  it("loadUntil stops paging once isCurrent turns false", async () => {
    const t = setup();
    t.feed.reset(payload(501, 1000), REQ);
    let current = true;
    const done = t.feed.loadUntil(tOf(-10), () => current);
    current = false;
    await vi.advanceTimersByTimeAsync(0);
    await expect(done).resolves.toBe("aborted");
    expect(t.fetch).toHaveBeenCalledTimes(1);
  });

  it("loadUntil joins a page that is already in flight", async () => {
    const t = setup();
    t.feed.reset(payload(501, 1000), REQ);
    t.feed.loadOlder();
    const done = t.feed.loadUntil(tOf(100));
    await vi.advanceTimersByTimeAsync(0);
    await expect(done).resolves.toBe("covered");
    expect(t.fetch).toHaveBeenCalledTimes(1);
  });

  it("loadUntil stops at the start of history", async () => {
    const t = setup();
    t.feed.reset(payload(501, 1000), REQ);
    const done = t.feed.loadUntil(tOf(-10));
    await vi.advanceTimersByTimeAsync(0);
    await expect(done).resolves.toBe("end");
    expect(t.last()!.series.bars[0]!.t).toBe(tOf(0));
  });

  it("loadUntil stops at the history limit", async () => {
    const t = setup();
    t.feed.reset(payload(MAX_HISTORY_BARS * 2 - 500, MAX_HISTORY_BARS * 2), REQ);
    market.now = MAX_HISTORY_BARS * 2;
    const done = t.feed.loadUntil(0);
    await vi.advanceTimersByTimeAsync(0);
    await expect(done).resolves.toBe("limit");
    expect(t.last()!.series.bars.length).toBeGreaterThanOrEqual(MAX_HISTORY_BARS);
  });

  it("loadUntil reports a failed page and an ended generation", async () => {
    const t = setup();
    t.feed.reset(payload(501, 1000), REQ);
    market.fail = true;
    const failed = t.feed.loadUntil(tOf(0));
    await vi.advanceTimersByTimeAsync(0);
    await expect(failed).resolves.toBe("error");

    market.fail = false;
    const aborted = t.feed.loadUntil(tOf(0));
    t.feed.suspend();
    await vi.advanceTimersByTimeAsync(0);
    await expect(aborted).resolves.toBe("aborted");
  });

  it("an empty page ends the history", async () => {
    const t = setup(() => Promise.resolve(payload(1, 0, true)));
    t.feed.reset(payload(501, 1000), REQ);
    t.feed.loadOlder();
    await vi.advanceTimersByTimeAsync(0);
    expect(t.last()!.update).toMatchObject({ kind: "older", count: 0 });
    expect(t.history.at(-1)).toEqual({ kind: "end" });
    expect(t.feed.loadOlder()).toBe(false);
  });

  it("starts at end when the full load has no more history", () => {
    const t = setup();
    t.feed.reset(payload(0, 100, false), REQ);
    expect(t.history.at(-1)).toEqual({ kind: "end" });
    expect(t.feed.loadOlder()).toBe(false);
  });

  it("drops a page that arrives after a reset", async () => {
    let release!: (s: SeriesPayload) => void;
    const t = setup(() => new Promise((res) => (release = res)));
    t.feed.reset(payload(501, 1000), REQ);
    t.feed.loadOlder();
    t.feed.reset(payload(501, 1000), REQ);
    release(payload(1, 500));
    await vi.advanceTimersByTimeAsync(0);
    expect(t.last()!.update.kind).toBe("reset");
    expect(t.last()!.series.bars).toHaveLength(500);
  });
});

/** subscribe_chart stand-in: ids s1, s2, ...; `error` makes subscribes fail. */
function fakePush() {
  const f = {
    error: null as Error | null,
    hold: null as Promise<void> | null,
    count: 0,
    subscribe: vi.fn(async (_req: LiveRequest, _warmup: number) => {
      if (f.hold) await f.hold;
      if (f.error) throw f.error;
      return `s${++f.count}`;
    }),
    unsubscribe: vi.fn((_id: string) => Promise.resolve()),
  };
  return f;
}

function setupPush() {
  const push = fakePush();
  const t = setup(serve, () => Promise.resolve(), push);
  return { ...t, push };
}

describe("push mode", () => {
  it("subscribes after a full load, stops polling and merges pushed tails", async () => {
    const t = setupPush();
    t.feed.reset(payload(501, 1000), REQ);
    await vi.advanceTimersByTimeAsync(0);
    expect(t.push.subscribe).toHaveBeenCalledWith(REQ, WARMUP);
    expect(t.polls.at(-1)).toEqual({ kind: "push" });

    for (let i = 1; i <= 6; i++) {
      market.tick = i / 10;
      t.feed.applyPush("s1", payload(999, 1000));
      expect(t.last()!.update).toMatchObject({ kind: "tail", from: 498 });
      expect(t.last()!.series.bars.at(-1)!.c).toBeCloseTo(1000 + i / 10);
      await vi.advanceTimersByTimeAsync(1_000);
    }
    // Six seconds of pushes, not a single poll.
    expect(t.fetch).not.toHaveBeenCalled();

    // A rollover arrives as a two-bar tail too.
    t.feed.applyPush("s1", payload(1000, 1001));
    expect(t.last()!.series.bars).toHaveLength(501);
    // Ticks of another subscription are not ours.
    const n = t.snapshots.length;
    t.feed.applyPush("s9", payload(1000, 1001));
    expect(t.snapshots).toHaveLength(n);
  });

  it("falls back to polling when pushes stop, and resubscribes with backoff", async () => {
    const t = setupPush();
    t.feed.reset(payload(501, 1000), REQ);
    await vi.advanceTimersByTimeAsync(0);
    t.push.error = new Error("MCP tool error: no open SSE stream");

    await vi.advanceTimersByTimeAsync(PUSH_WATCHDOG_MS);
    expect(t.push.unsubscribe).toHaveBeenCalledWith("s1");
    // Caught up at once, then polled on the usual period.
    expect(t.fetch).toHaveBeenCalledTimes(1);
    expect(t.polls.at(-1)).toEqual({ kind: "live", everyMs: FAST_POLL_MS });

    await vi.advanceTimersByTimeAsync(PUSH_RETRY_MS);
    expect(t.push.subscribe).toHaveBeenCalledTimes(2);
    // Failed again: the next retry waits twice as long.
    await vi.advanceTimersByTimeAsync(PUSH_RETRY_MS * 2 - 1);
    expect(t.push.subscribe).toHaveBeenCalledTimes(2);
    t.push.error = null;
    await vi.advanceTimersByTimeAsync(1);
    expect(t.push.subscribe).toHaveBeenCalledTimes(3);
    expect(t.polls.at(-1)).toEqual({ kind: "push" });
    const polled = t.fetch.mock.calls.length;
    t.feed.applyPush("s2", payload(999, 1000));
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS * 2);
    expect(t.fetch.mock.calls.length).toBe(polled);
  });

  it("keeps polling a chart the server cannot push, without retrying", async () => {
    const t = setupPush();
    t.push.error = new Error("MCP tool error: not pushable: AAPL is not a Binance symbol");
    t.feed.reset(payload(501, 1000), { ...REQ, ticker: "AAPL" });
    await vi.advanceTimersByTimeAsync(SLOW_POLL_MS * 4);
    expect(t.push.subscribe).toHaveBeenCalledTimes(1);
    expect(t.fetch).toHaveBeenCalledTimes(4);
    expect(t.polls.at(-1)).toEqual({ kind: "live", everyMs: SLOW_POLL_MS });
  });

  it("polls when a push limit refuses the chart, and tries push again later", async () => {
    const t = setupPush();
    t.push.error = new Error("MCP tool error: push limit reached: at most 4 per token");
    t.feed.reset(payload(501, 1000), REQ);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(t.push.subscribe).toHaveBeenCalledTimes(1);
    expect(t.polls.at(-1)).toEqual({ kind: "live", everyMs: FAST_POLL_MS });
    // Still refused a few minutes on: it asks once and goes on polling.
    await vi.advanceTimersByTimeAsync(PUSH_LIMIT_RETRY_MS - 60_000);
    expect(t.push.subscribe).toHaveBeenCalledTimes(2);
    expect(t.polls.at(-1)).toEqual({ kind: "live", everyMs: FAST_POLL_MS });
    // The limit has passed: push takes over, with no reload of the symbol or interval.
    t.push.error = null;
    await vi.advanceTimersByTimeAsync(PUSH_LIMIT_RETRY_MS);
    expect(t.push.subscribe).toHaveBeenCalledTimes(3);
    expect(t.polls.at(-1)).toEqual({ kind: "push" });
  });

  it("does not leave a push retry behind for the next chart", async () => {
    const t = setupPush();
    t.push.error = new Error("MCP tool error: push limit reached: at most 4 per token");
    t.feed.reset(payload(501, 1000), REQ);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(t.push.subscribe).toHaveBeenCalledTimes(1);
    // The next chart may try at once, and the old retry, a minute ahead of its own,
    // dies with its chart.
    t.feed.reset(payload(501, 1000), { ...REQ, timeframe: "5m" });
    await vi.advanceTimersByTimeAsync(0);
    expect(t.push.subscribe).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(PUSH_LIMIT_RETRY_MS - 1);
    expect(t.push.subscribe).toHaveBeenCalledTimes(2);
  });

  it("drops a routine poll that lands after push took over", async () => {
    let release!: (s: SeriesPayload) => void;
    const push = fakePush();
    let subscribed!: () => void;
    push.hold = new Promise((r) => (subscribed = r));
    const t = setup(() => new Promise((res) => (release = res)), () => Promise.resolve(), push);
    t.feed.reset(payload(501, 1000), REQ);
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS);
    expect(t.fetch).toHaveBeenCalledTimes(1); // in flight
    subscribed();
    await vi.advanceTimersByTimeAsync(0);
    market.tick = 0.9;
    t.feed.applyPush("s1", payload(999, 1000));
    // The poll was answered before that push and lands after it.
    market.tick = 0.1;
    release(payload(999, 1000));
    await vi.advanceTimersByTimeAsync(0);
    expect(t.last()!.series.bars.at(-1)!.c).toBeCloseTo(1000.9);
    expect(t.polls.at(-1)).toEqual({ kind: "push" });
  });

  it("a generation change unsubscribes and subscribes the new chart", async () => {
    const t = setupPush();
    t.feed.reset(payload(501, 1000), REQ);
    await vi.advanceTimersByTimeAsync(0);
    const m5 = { ...REQ, timeframe: "5m" };
    t.feed.reset(payload(501, 1000), m5);
    expect(t.push.unsubscribe).toHaveBeenCalledWith("s1");
    await vi.advanceTimersByTimeAsync(0);
    expect(t.push.subscribe).toHaveBeenLastCalledWith(m5, WARMUP);
    // A late tick of the old subscription is dropped.
    const n = t.snapshots.length;
    t.feed.applyPush("s1", payload(999, 1000));
    expect(t.snapshots).toHaveLength(n);
    t.feed.dispose();
    expect(t.push.unsubscribe).toHaveBeenLastCalledWith("s2");
  });

  it("undoes a subscribe that lands after the chart changed", async () => {
    const t = setupPush();
    let release!: () => void;
    t.push.hold = new Promise((r) => (release = r));
    t.feed.reset(payload(501, 1000), REQ);
    t.feed.suspend();
    release();
    await vi.advanceTimersByTimeAsync(0);
    expect(t.push.unsubscribe).toHaveBeenCalledWith("s1");
    expect(t.polls.at(-1)).toEqual({ kind: "off" });
  });

  it("a hidden tab unsubscribes and resubscribes when visible", async () => {
    const t = setupPush();
    t.feed.reset(payload(501, 1000), REQ);
    await vi.advanceTimersByTimeAsync(0);
    t.doc.set("hidden");
    expect(t.push.unsubscribe).toHaveBeenCalledWith("s1");
    expect(t.polls.at(-1)).toEqual({ kind: "paused" });
    await vi.advanceTimersByTimeAsync(PUSH_WATCHDOG_MS * 3);
    expect(t.push.subscribe).toHaveBeenCalledTimes(1);
    expect(t.fetch).not.toHaveBeenCalled();

    t.doc.set("visible");
    await vi.advanceTimersByTimeAsync(0);
    // Catches up with a poll and subscribes again.
    expect(t.fetch).toHaveBeenCalledTimes(1);
    expect(t.push.subscribe).toHaveBeenCalledTimes(2);
    expect(t.polls.at(-1)).toEqual({ kind: "push" });
  });

  it("a pushed tail after missed bars catches up with a poll", async () => {
    const t = setupPush();
    t.feed.reset(payload(501, 1000), REQ);
    await vi.advanceTimersByTimeAsync(0);
    market.now = 1010;
    t.feed.applyPush("s1", payload(1009, 1010));
    await vi.advanceTimersByTimeAsync(0);
    expect(t.fetch.mock.calls.map((c) => c[1].bars)).toEqual([2, 8 + TAIL_BARS + 3]);
    expect(t.last()!.series.bars.at(-1)!.t).toBe(tOf(1010));
    expect(t.polls.at(-1)).toEqual({ kind: "push" });
    // Still pushing: no polls follow.
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS * 2);
    expect(t.fetch).toHaveBeenCalledTimes(2);
  });
});

describe("reload bound", () => {
  // Every tail misses an indicator the full load has, so each one asks for a
  // reload, and each reload lands in reset(): a new generation every time.
  const misfitTail = (req: LiveRequest, args: FetchArgs) =>
    serve(req, args).then((p) => ({ ...p, indicators: {} }));
  const otherSymbol = (req: LiveRequest, args: FetchArgs) =>
    serve(req, args).then((p) => ({ ...p, symbol: "OTHER" }));
  const landing = (feed: LiveFeed) => {
    feed.reset(payload(0, market.now), REQ);
    return Promise.resolve();
  };

  it("holds across generations, and allows more after the window", async () => {
    const t = setup(misfitTail, landing);
    t.feed.reset(payload(0, market.now), REQ);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(t.reload).toHaveBeenCalledTimes(MAX_RELOADS);

    await vi.advanceTimersByTimeAsync(RELOAD_WINDOW_MS);
    const later = t.reload.mock.calls.length;
    expect(later).toBeGreaterThan(MAX_RELOADS);
    expect(later).toBeLessThanOrEqual(MAX_RELOADS * 2);
    t.feed.dispose();
  });

  it("out of reloads, a misfit keeps the bars live and fails only its indicator", async () => {
    const t = setup(misfitTail, landing);
    t.feed.reset(payload(0, market.now), REQ);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(t.reload).toHaveBeenCalledTimes(MAX_RELOADS);

    market.now += 3;
    await vi.advanceTimersByTimeAsync(10_000);
    const shown = t.last()!.series;
    expect(shown.bars.at(-1)!.t).toBe(tOf(market.now));
    expect(shown.indicator_errors?.sma?.step).toBe("shape");
    // Aligned with the new bars, and live rather than backing off.
    expect(shown.indicators.sma!.lines![0]!.values).toHaveLength(shown.bars.length);
    expect(t.polls.at(-1)?.kind).toBe("live");
    t.feed.dispose();
  });

  it("out of reloads, data that cannot merge at all backs off and says so", async () => {
    const t = setup(otherSymbol, landing);
    t.feed.reset(payload(0, market.now), REQ);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(t.reload).toHaveBeenCalledTimes(MAX_RELOADS);
    const status = t.polls.at(-1);
    expect(status?.kind).toBe("retrying");
    expect(status?.kind === "retrying" && status.message).toMatch(/not lining up/);
    t.feed.dispose();
  });

  it("counts per chart: another request starts a new count", async () => {
    const t = setup(misfitTail, landing);
    t.feed.reset(payload(0, market.now), REQ);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(t.reload).toHaveBeenCalledTimes(MAX_RELOADS);
    t.feed.reset(payload(0, market.now), { ...REQ, timeframe: "5m" });
    await vi.advanceTimersByTimeAsync(SLOW_POLL_MS * 2);
    expect(t.reload.mock.calls.length).toBeGreaterThan(MAX_RELOADS);
    t.feed.dispose();
  });

  it("out of reloads, pushes that cannot merge do not each cause a poll", async () => {
    const push: PushApi = {
      subscribe: () => Promise.resolve("sub"),
      unsubscribe: () => Promise.resolve(),
    };
    const t = setup(otherSymbol, landing, push);
    t.feed.reset(payload(0, market.now), REQ);
    await vi.advanceTimersByTimeAsync(0);
    const bad = () => t.feed.applyPush("sub", { ...payload(market.now - 1, market.now), symbol: "OTHER" });
    // Use up the reloads: each lands in a new generation, which subscribes again.
    for (let i = 0; i < MAX_RELOADS; i++) {
      bad();
      await vi.advanceTimersByTimeAsync(100);
    }
    expect(t.reload).toHaveBeenCalledTimes(MAX_RELOADS);

    await vi.advanceTimersByTimeAsync(0);
    const before = t.fetch.mock.calls.length;
    for (let i = 0; i < 20; i++) {
      bad();
      await vi.advanceTimersByTimeAsync(100);
    }
    expect(t.fetch.mock.calls.length).toBe(before);
    t.feed.dispose();
  });
});
