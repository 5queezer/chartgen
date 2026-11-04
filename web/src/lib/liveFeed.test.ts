import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SeriesPayload } from "../types/responses";
import {
  FAST_POLL_MS,
  PAGE_BARS,
  SLOW_POLL_MS,
  TAIL_BARS,
  WARMUP,
  createLiveFeed,
  type FetchArgs,
  type HistoryStatus,
  type LiveRequest,
  type LiveSnapshot,
  type PollStatus,
} from "./liveFeed";

// A 1m market whose forming bar is `now` (a bar index) and whose history
// starts at bar 0. Bar i opens at BASE + 60 i.
const BASE = 1_700_000_040; // on the 1m grid
const tOf = (i: number) => BASE + i * 60;

function payload(first: number, last: number, has_more = true): SeriesPayload {
  const idx = Array.from({ length: Math.max(0, last - first + 1) }, (_, k) => first + k);
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

const REQ: LiveRequest = { ticker: "BTCUSDT", timeframe: "1m", indicators: ["sma"] };

function setup(fetchImpl = serve) {
  const doc = new FakeDoc();
  const snapshots: Array<LiveSnapshot | null> = [];
  const polls: PollStatus[] = [];
  const history: HistoryStatus[] = [];
  const fetch = vi.fn(fetchImpl);
  const reload = vi.fn(() => Promise.resolve());
  const feed = createLiveFeed({
    fetch,
    reload,
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
    // The fake reload never resets the feed, so polling resumes backed off.
    expect(t.polls.at(-1)).toMatchObject({ kind: "retrying", inMs: 4_000 });
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
