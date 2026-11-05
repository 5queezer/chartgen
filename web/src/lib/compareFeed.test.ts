import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SeriesPayload } from "../types/responses";
import {
  COMPARE_PAGE_BARS,
  createCompareFeed,
  type CompareData,
  type CompareFetchArgs,
  type CompareFeed,
} from "./compareFeed";
import { FAST_POLL_MS, SLOW_POLL_MS, TAIL_BARS } from "./liveFeed";

// A 1m market whose forming bar is `market.now` (a bar index), history from 0.
const BASE = 1_700_000_040;
const tOf = (i: number) => BASE + i * 60;
const market = { now: 1000, fail: false };

function payload(first: number, last: number, has_more: boolean): SeriesPayload {
  const idx = Array.from({ length: Math.max(0, last - first + 1) }, (_, k) => first + k);
  return {
    symbol: "X",
    interval: "1m",
    has_more,
    bars: idx.map((i) => ({ t: tOf(i), o: i, h: i, l: i, c: i, v: 1 })),
    indicators: {},
  };
}

interface Call {
  ticker: string;
  timeframe: string;
  args: CompareFetchArgs;
}
let calls: Call[];

function serve(
  req: { ticker: string; timeframe: string },
  args: CompareFetchArgs,
): Promise<SeriesPayload> {
  calls.push({ ...req, args });
  if (market.fail) return Promise.reject(new Error("down"));
  const end =
    args.end_time === undefined ? market.now : (args.end_time / 1000 - BASE) / 60 - 1;
  const last = Math.min(end, market.now);
  const first = Math.max(0, last - args.bars + 1);
  return Promise.resolve(payload(first, last, first > 0));
}

let latest: Map<string, CompareData>;
let feed: CompareFeed;
let visibility: "visible" | "hidden";

const sync = (over: Partial<Parameters<CompareFeed["sync"]>[0]> = {}) =>
  feed.sync({
    symbols: ["ETHUSDT"],
    timeframe: "1m",
    mainCount: 100,
    mainFirst: tOf(901),
    ...over,
  });

const settle = () => vi.advanceTimersByTimeAsync(0);

beforeEach(() => {
  vi.useFakeTimers();
  calls = [];
  market.now = 1000;
  market.fail = false;
  visibility = "visible";
  latest = new Map();
  feed = createCompareFeed({
    fetch: serve,
    onChange: (d) => (latest = d),
    pollMs: (s) => (s.endsWith("USDT") ? FAST_POLL_MS : SLOW_POLL_MS),
    doc: { get visibilityState() { return visibility; } },
  });
});

afterEach(() => {
  feed.dispose();
  vi.useRealTimers();
});

describe("compare feed", () => {
  it("loads as many bars as the main series holds", async () => {
    sync();
    await settle();
    expect(calls[0]!.args.bars).toBe(100);
    expect(latest.get("ETHUSDT")!.state).toBe("ready");
    expect(latest.get("ETHUSDT")!.bars).toHaveLength(100);
  });

  it("waits for the main series before the first load", async () => {
    sync({ mainCount: 0, mainFirst: null });
    await settle();
    expect(calls).toHaveLength(0);
    sync();
    await settle();
    expect(calls).toHaveLength(1);
  });

  it("merges tails by timestamp and keeps the history", async () => {
    sync();
    await settle();
    market.now = 1001;
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS);
    const tail = calls.at(-1)!;
    expect(tail.args.bars).toBe(TAIL_BARS);
    const bars = latest.get("ETHUSDT")!.bars;
    expect(bars).toHaveLength(101);
    expect(bars.at(-1)!.t).toBe(tOf(1001));
    expect(new Set(bars.map((b) => b.t)).size).toBe(bars.length);
  });

  it("starts a symbol over after a gap", async () => {
    sync();
    await settle();
    market.now = 1500;
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS);
    await settle();
    expect(calls.filter((c) => c.args.end_time === undefined).at(-1)!.args.bars).toBe(100);
    expect(latest.get("ETHUSDT")!.bars.at(-1)!.t).toBe(tOf(1500));
  });

  it("polls each symbol at its own market's cadence", async () => {
    sync({ symbols: ["ETHUSDT", "AAPL"] });
    await settle();
    calls = [];
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS);
    expect(calls.map((c) => c.ticker)).toEqual(["ETHUSDT"]);
    await vi.advanceTimersByTimeAsync(SLOW_POLL_MS - FAST_POLL_MS);
    expect(calls.map((c) => c.ticker)).toContain("AAPL");
  });

  it("does not poll while the tab is hidden", async () => {
    sync();
    await settle();
    calls = [];
    visibility = "hidden";
    await vi.advanceTimersByTimeAsync(FAST_POLL_MS * 3);
    expect(calls).toHaveLength(0);
  });

  describe("paging", () => {
    it("pages older bars until the main first bar is covered, then stops", async () => {
      sync({ mainCount: 100, mainFirst: tOf(0) });
      await settle();
      await settle();
      const page = calls.find((c) => c.args.end_time !== undefined)!;
      expect(page.args.bars).toBe(COMPARE_PAGE_BARS);
      expect(page.args.end_time).toBe(tOf(901) * 1000);
      // 100 + 1000 bars would pass bar 0: the page is cut by the source.
      expect(latest.get("ETHUSDT")!.bars[0]!.t).toBe(tOf(0));
      expect(latest.get("ETHUSDT")!.hasMore).toBe(false);
      const n = calls.length;
      sync({ mainCount: 100, mainFirst: tOf(0) });
      await settle();
      expect(calls).toHaveLength(n);
    });

    it("does not page when the compare series already starts before the main one", async () => {
      sync({ mainFirst: tOf(950) });
      await settle();
      await settle();
      expect(calls.filter((c) => c.args.end_time !== undefined)).toHaveLength(0);
    });

    it("stops when the source has no more bars", async () => {
      sync({ mainFirst: tOf(-5) });
      await settle();
      await settle();
      await settle();
      const paged = calls.filter((c) => c.args.end_time !== undefined);
      expect(paged.length).toBeGreaterThan(0);
      expect(latest.get("ETHUSDT")!.hasMore).toBe(false);
      const n = calls.length;
      await settle();
      expect(calls).toHaveLength(n);
    });
  });

  describe("lifecycle", () => {
    it("aborts and refetches everything on a timeframe change", async () => {
      sync({ symbols: ["ETHUSDT", "SOLUSDT"] });
      const first = calls.map((c) => c.args.signal);
      expect(first).toHaveLength(2);
      sync({ symbols: ["ETHUSDT", "SOLUSDT"], timeframe: "5m" });
      expect(first.every((s) => s.aborted)).toBe(true);
      await settle();
      const fresh = calls.slice(2);
      expect(fresh.map((c) => c.timeframe)).toEqual(["5m", "5m"]);
      expect(latest.get("ETHUSDT")!.state).toBe("ready");
    });

    it("drops a removed symbol and aborts its request", async () => {
      sync({ symbols: ["ETHUSDT", "SOLUSDT"] });
      const sol = calls.find((c) => c.ticker === "SOLUSDT")!.args.signal;
      sync({ symbols: ["ETHUSDT"] });
      expect(sol.aborted).toBe(true);
      await settle();
      expect([...latest.keys()]).toEqual(["ETHUSDT"]);
    });

    it("retries a failed first load on the next poll", async () => {
      market.fail = true;
      sync();
      await settle();
      expect(latest.get("ETHUSDT")!.state).toBe("error");
      market.fail = false;
      await vi.advanceTimersByTimeAsync(FAST_POLL_MS);
      await settle();
      expect(latest.get("ETHUSDT")!.state).toBe("ready");
    });
  });
});
