import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SeriesPayload } from "../types/responses";
import {
  RECOMPUTE_BARS,
  REPLAY_SPEEDS,
  closeOf,
  createThrottle,
  cursorIndexAt,
  frameShape,
  frameUpdate,
  indexClosedBy,
  isRepainting,
  playDelayMs,
  replayFrame,
  sliceSeries,
  stepCursor,
  windowStart,
} from "./replay";

const BASE = 1_700_000_000;
const STEP = 60;
const tOf = (i: number) => BASE + i * STEP;

/** Bars `first`..`first + n - 1` with every channel type on two indicators. */
function series(n: number, first = 0): SeriesPayload {
  const idx = Array.from({ length: n }, (_, k) => first + k);
  const v = (f: (i: number) => number) => idx.map(f);
  return {
    symbol: "BTCUSDT",
    interval: "1m",
    bars: idx.map((i) => ({ t: tOf(i), o: i, h: i + 1, l: i - 1, c: i, v: 10 })),
    has_more: true,
    indicators: {
      rsi: {
        label: "RSI",
        is_overlay: false,
        y_range: [0, 100],
        lines: [{ index: 0, label: "RSI", values: v((i) => i % 100) }],
        histogram: [{ index: 0, values: v((i) => -i) }],
        fills: [{ y1: v((i) => i + 2), y2: v((i) => i - 2), color: "rgba(1,2,3,0.5)" }],
        hlines: [{ y: 70 }, { y: 30 }],
        signals: [
          { x: 1, t: null, y: 1, label: "buy" },
          { x: n - 1, t: null, y: 2, label: "sell" },
        ],
        divlines: [
          { x1: 0, t1: null, y1: 1, x2: 2, t2: null, y2: 2, dashed: false },
          { x1: 1, t1: null, y1: 1, x2: n - 1, t2: null, y2: 2, dashed: true },
        ],
      },
      vpvr: {
        label: "VPVR",
        is_overlay: true,
        lines: [{ index: 0, label: "POC", values: v((i) => 1000 + i) }],
        hbars: [{ y: 5, height: 1, width: 0.5, offset: 0, left: false }],
        hlines: [{ y: 5 }],
      },
    },
  };
}

describe("cursorIndexAt / stepCursor", () => {
  const bars = series(5).bars;
  it("finds the last bar opening at or before a time", () => {
    expect(cursorIndexAt(bars, tOf(2))).toBe(2);
    expect(cursorIndexAt(bars, tOf(2) + 30)).toBe(2);
    expect(cursorIndexAt(bars, tOf(0) - 1)).toBe(-1);
    expect(cursorIndexAt(bars, tOf(99))).toBe(4);
    expect(cursorIndexAt([], 0)).toBe(-1);
  });
  it("finds the last bar closed by a replay clock", () => {
    expect(closeOf(tOf(2), "1m")).toBe(tOf(3));
    expect(closeOf(tOf(2), null)).toBe(tOf(2));
    expect(indexClosedBy(bars, tOf(3), "1m")).toBe(2);
    expect(indexClosedBy(bars, tOf(3) - 1, "1m")).toBe(1);
    expect(indexClosedBy(bars, tOf(0), "1m")).toBe(-1);
    expect(indexClosedBy(bars, tOf(99), "1m")).toBe(4);
    // 1m bars under a 5m clock: everything complete by then.
    expect(indexClosedBy(bars, tOf(0) + 300, "1m")).toBe(4);
    expect(indexClosedBy(bars, tOf(2), null)).toBe(2);
  });
  it("steps until the last bar", () => {
    expect(stepCursor(0, 5)).toBe(1);
    expect(stepCursor(3, 5)).toBe(4);
    expect(stepCursor(4, 5)).toBeNull();
    expect(stepCursor(-1, 5)).toBe(0);
    expect(stepCursor(1, 5, 10)).toBe(4);
    expect(stepCursor(0, 0)).toBeNull();
  });
  it("plays at the documented speeds", () => {
    expect(REPLAY_SPEEDS).toEqual([1, 3, 10]);
    expect(playDelayMs(1)).toBe(1000);
    expect(playDelayMs(10)).toBe(100);
  });
});

describe("sliceSeries", () => {
  const full = series(10);

  it("cuts every per-bar channel at the cursor", () => {
    const s = sliceSeries(full, 4);
    expect(s.bars.map((b) => b.t)).toEqual([0, 1, 2, 3, 4].map(tOf));
    const rsi = s.indicators.rsi!;
    expect(rsi.lines![0]!.values).toEqual([0, 1, 2, 3, 4]);
    expect(rsi.histogram![0]!.values).toEqual([-0, -1, -2, -3, -4]);
    expect(rsi.fills![0]!.y1).toEqual([2, 3, 4, 5, 6]);
    expect(rsi.fills![0]!.y2).toEqual([-2, -1, 0, 1, 2]);
    // Colors, labels and window aggregates pass through.
    expect(rsi.fills![0]!.color).toBe("rgba(1,2,3,0.5)");
    expect(rsi.lines![0]!.label).toBe("RSI");
    expect(rsi.hlines).toEqual([{ y: 70 }, { y: 30 }]);
    expect(rsi.y_range).toEqual([0, 100]);
    expect(s.has_more).toBe(true);
    expect(s.symbol).toBe("BTCUSDT");
  });

  it("keeps signals on or before the cursor", () => {
    expect(sliceSeries(full, 4).indicators.rsi!.signals!.map((s) => s.x)).toEqual([1]);
    expect(sliceSeries(full, 9).indicators.rsi!.signals!.map((s) => s.x)).toEqual([1, 9]);
  });

  it("drops divlines ending after the cursor instead of clipping them", () => {
    const at = (c: number) =>
      sliceSeries(full, c).indicators.rsi!.divlines!.map((d) => [d.x1, d.x2]);
    expect(at(1)).toEqual([]);
    expect(at(2)).toEqual([[0, 2]]);
    expect(at(8)).toEqual([[0, 2]]);
    expect(at(9)).toEqual([
      [0, 2],
      [1, 9],
    ]);
  });

  it("handles divlines drawn right to left", () => {
    const s = series(10);
    s.indicators.rsi!.divlines = [
      { x1: 6, t1: null, y1: 1, x2: 3, t2: null, y2: 2, dashed: false },
    ];
    expect(sliceSeries(s, 5).indicators.rsi!.divlines).toEqual([]);
    expect(sliceSeries(s, 6).indicators.rsi!.divlines).toHaveLength(1);
  });

  it("at bar 0 holds exactly one bar", () => {
    const s = sliceSeries(full, 0);
    expect(s.bars).toHaveLength(1);
    expect(s.indicators.rsi!.lines![0]!.values).toEqual([0]);
    expect(s.indicators.rsi!.fills![0]!.y1).toEqual([2]);
    expect(s.indicators.rsi!.signals).toEqual([]);
    expect(s.indicators.rsi!.divlines).toEqual([]);
  });

  it("at the last bar equals the full series", () => {
    const s = sliceSeries(full, 9);
    expect(s).toEqual(full);
    expect(s.bars).not.toBe(full.bars);
  });

  it("clamps a cursor outside the bars", () => {
    expect(sliceSeries(full, -3).bars).toHaveLength(1);
    expect(sliceSeries(full, 99).bars).toHaveLength(10);
    expect(sliceSeries(series(0), 0).bars).toEqual([]);
  });

  it("never shares or modifies the input's arrays", () => {
    const before = structuredClone(full);
    const s = sliceSeries(full, 9);
    s.indicators.rsi!.lines![0]!.values[0] = 42;
    s.indicators.rsi!.hlines!.push({ y: 1 });
    s.indicators.vpvr!.hbars!.pop();
    expect(full).toEqual(before);
    expect(s.indicators.rsi!.fills![0]!.y1).not.toBe(full.indicators.rsi!.fills![0]!.y1);
  });

  it("leaves channels the payload omits absent", () => {
    const s = series(3);
    s.indicators = { ema: { label: "EMA", is_overlay: true, lines: [{ index: 0, label: null, values: [1, 2, 3] }] } };
    const ind = sliceSeries(s, 1).indicators.ema!;
    expect(Object.keys(ind).sort()).toEqual(["is_overlay", "label", "lines"]);
  });
});

describe("replayFrame", () => {
  it("is the slice when nothing repaints", () => {
    const s = series(10);
    delete s.indicators.vpvr;
    expect(replayFrame(s, 5, null)).toEqual(sliceSeries(s, 5));
  });

  it("blanks repainting outputs in the window until a recompute arrives", () => {
    const f = replayFrame(series(10), 5, null);
    const vp = f.indicators.vpvr!;
    expect(vp.lines![0]!.values).toEqual([null, null, null, null, null, null]);
    expect(vp.hbars).toEqual([]);
    expect(vp.hlines).toEqual([]);
    // Causal indicators are untouched.
    expect(f.indicators.rsi!.lines![0]!.values).toEqual([0, 1, 2, 3, 4, 5]);
  });

  it("takes the window from a recompute ending at the cursor", () => {
    const rc = series(3, 3); // bars 3..5
    rc.indicators.vpvr!.lines![0]!.values = [7, 8, 9];
    rc.indicators.vpvr!.hbars = [{ y: 9, height: 2, width: 1, offset: 0, left: true }];
    rc.indicators.vpvr!.hlines = [{ y: 9, color: "red" }];
    rc.indicators.vpvr!.y_range = [1, 2];
    const vp = replayFrame(series(10), 5, rc).indicators.vpvr!;
    // Bars 0..2 are in the window too (it is RECOMPUTE_BARS wide) but not covered.
    expect(vp.lines![0]!.values).toEqual([null, null, null, 7, 8, 9]);
    expect(vp.hbars).toEqual(rc.indicators.vpvr!.hbars);
    expect(vp.hbars).not.toBe(rc.indicators.vpvr!.hbars);
    expect(vp.hlines).toEqual([{ y: 9, color: "red" }]);
    expect(vp.y_range).toEqual([1, 2]);
  });

  it("leaves bars after an older recompute blank", () => {
    const rc = series(3, 1); // bars 1..3, cursor moved on to 5 since
    const vp = replayFrame(series(10), 5, rc).indicators.vpvr!;
    expect(vp.lines![0]!.values).toEqual([null, 1001, 1002, 1003, null, null]);
  });

  it("keeps the full series' values before the window", () => {
    const n = RECOMPUTE_BARS + 20;
    const cursor = n - 1;
    const from = windowStart(cursor);
    expect(from).toBe(20);
    const rc = series(RECOMPUTE_BARS, from);
    rc.indicators.vpvr!.lines![0]!.values = rc.bars.map(() => -1);
    const values = replayFrame(series(n), cursor, rc).indicators.vpvr!.lines![0]!.values;
    expect(values.slice(0, from)).toEqual(Array.from({ length: from }, (_, i) => 1000 + i));
    expect(values.slice(from).every((v) => v === -1)).toBe(true);
  });

  it("maps recompute signals and divlines into the slice and keeps earlier ones", () => {
    const n = RECOMPUTE_BARS + 10;
    const s = series(n);
    s.indicators.zigzag = {
      label: "ZigZag",
      is_overlay: true,
      lines: [{ index: 0, label: null, values: s.bars.map(() => 1) }],
      signals: [
        { x: 2, t: null, y: 1, label: "old" },
        { x: 400, t: null, y: 1, label: "future-leg" },
      ],
      divlines: [
        { x1: 1, t1: null, y1: 1, x2: 3, t2: null, y2: 1, dashed: false },
        { x1: 5, t1: null, y1: 1, x2: 300, t2: null, y2: 1, dashed: false },
      ],
    };
    const cursor = n - 1;
    const from = windowStart(cursor); // 10
    const rc = series(RECOMPUTE_BARS + 5, from - 5); // starts 5 bars before the window
    rc.indicators.zigzag = {
      label: "ZigZag",
      is_overlay: true,
      lines: [{ index: 0, label: null, values: rc.bars.map(() => 2) }],
      // Recompute index 7 is slice bar 12; index 2 is slice bar 7 (before the window).
      signals: [
        { x: 7, t: null, y: 1, label: "new" },
        { x: 2, t: null, y: 1, label: "before-window" },
      ],
      divlines: [
        { x1: 2, t1: null, y1: 1, x2: 7, t2: null, y2: 1, dashed: true },
        { x1: 0, t1: null, y1: 1, x2: 3, t2: null, y2: 1, dashed: true },
      ],
    };
    const zz = replayFrame(s, cursor, rc).indicators.zigzag!;
    expect(zz.signals!.map((x) => [x.x, x.label])).toEqual([
      [2, "old"],
      [12, "new"],
    ]);
    expect(zz.divlines!.map((d) => [d.x1, d.x2, d.dashed])).toEqual([
      [1, 3, false],
      [7, 12, true],
    ]);
  });

  it("ignores a recompute whose channels do not fit", () => {
    const rc = series(3, 3);
    rc.indicators.vpvr!.lines!.push({ index: 1, label: null, values: [1, 2, 3] });
    const vp = replayFrame(series(10), 5, rc).indicators.vpvr!;
    expect(vp.lines![0]!.values.every((v) => v === null)).toBe(true);
    expect(vp.hbars).toEqual([]);
  });

  it("repaints fills and histograms too", () => {
    const s = series(4);
    s.indicators.ichimoku = {
      label: "Ichimoku",
      is_overlay: true,
      histogram: [{ index: 0, values: [1, 2, 3, 4] }],
      fills: [{ y1: [1, 2, 3, 4], y2: [0, 0, 0, 0] }],
    };
    const rc = series(2, 1);
    rc.indicators.ichimoku = {
      label: "Ichimoku",
      is_overlay: true,
      histogram: [{ index: 0, values: [8, 9] }],
      fills: [{ y1: [5, 6], y2: [7, 8] }],
    };
    const ich = replayFrame(s, 3, rc).indicators.ichimoku!;
    expect(ich.histogram![0]!.values).toEqual([null, 8, 9, null]);
    expect(ich.fills![0]!.y1).toEqual([null, 5, 6, null]);
    expect(ich.fills![0]!.y2).toEqual([null, 7, 8, null]);
  });

  it("does not modify its inputs", () => {
    const s = series(10);
    const rc = series(3, 3);
    const a = structuredClone(s);
    const b = structuredClone(rc);
    replayFrame(s, 5, rc);
    expect(s).toEqual(a);
    expect(rc).toEqual(b);
  });
});

describe("frameShape", () => {
  it("lists the indicators with any output", () => {
    const s = series(10);
    expect(frameShape(s)).toBe("rsi\nvpvr");
    // A profile before its first recompute has none.
    delete s.indicators.vpvr!.lines;
    const rc = series(3, 3);
    delete rc.indicators.vpvr!.lines;
    expect(frameShape(replayFrame(s, 5, null))).toBe("rsi");
    expect(frameShape(replayFrame(s, 5, rc))).toBe("rsi\nvpvr");
    s.indicators.rsi = { label: "RSI", is_overlay: false, lines: [], divlines: [] };
    expect(frameShape(s)).toBe("vpvr");
  });
});

describe("isRepainting", () => {
  it("covers profiles, zigzag, linreg and ichimoku with aliases", () => {
    for (const n of ["vpvr", "vp", "volume_profile", "tpo", "market_profile", "hvn_lvn", "svp", "npoc", "zigzag", "linreg_channel", "ichimoku", "VPVR"]) {
      expect(isRepainting(n)).toBe(true);
    }
    for (const n of ["rsi", "ema", "macd", "pivot", "cipher_b"]) {
      expect(isRepainting(n)).toBe(false);
    }
  });
});

describe("frameUpdate", () => {
  const key = (baseVersion: number, cursor: number, recompute = 0, shape = "rsi") => ({
    baseVersion,
    cursor,
    recompute,
    shape,
  });
  const reset = { version: 1, kind: "reset" } as const;

  it("resets when the indicators with output change", () => {
    expect(frameUpdate(key(1, 5), key(1, 5, 1, "rsi\nvpvr"), reset, true, 8)).toEqual({
      version: 8,
      kind: "reset",
    });
    const older = { version: 2, kind: "older", count: 10 } as const;
    expect(frameUpdate(key(1, 5), key(2, 15, 0, ""), older, false, 8)!.kind).toBe("reset");
  });

  it("starts with a reset", () => {
    expect(frameUpdate(null, key(1, 5), reset, false, 7)).toEqual({ version: 7, kind: "reset" });
  });

  it("is a tail from the previous cursor for a step", () => {
    expect(frameUpdate(key(1, 5), key(1, 6), reset, false, 8)).toEqual({
      version: 8,
      kind: "tail",
      from: 5,
      prevLength: 6,
    });
  });

  it("re-applies the recompute window when something repaints", () => {
    const prev = key(1, RECOMPUTE_BARS + 50);
    expect(frameUpdate(prev, key(1, RECOMPUTE_BARS + 51), reset, true, 8)).toMatchObject({
      kind: "tail",
      from: 51,
      prevLength: RECOMPUTE_BARS + 51,
    });
    // A new recompute without a step.
    expect(frameUpdate(prev, key(1, RECOMPUTE_BARS + 50, 3), reset, true, 8)).toMatchObject({
      kind: "tail",
      from: 51,
    });
  });

  it("is null when nothing changed", () => {
    expect(frameUpdate(key(1, 5), key(1, 5), reset, true, 8)).toBeNull();
  });

  it("resets when the cursor moves back", () => {
    expect(frameUpdate(key(1, 5), key(1, 4), reset, false, 8)).toEqual({ version: 8, kind: "reset" });
  });

  it("prepends a page that shifted the cursor by its size", () => {
    const older = { version: 2, kind: "older", count: 100 } as const;
    expect(frameUpdate(key(1, 5), key(2, 105), older, true, 8)).toEqual({
      version: 8,
      kind: "older",
      count: 100,
    });
  });

  it("resets for a page that does not line up or new data", () => {
    const older = { version: 2, kind: "older", count: 100 } as const;
    // The cursor moved as well.
    expect(frameUpdate(key(1, 5), key(2, 106), older, false, 8)!.kind).toBe("reset");
    // A recompute landed at once.
    expect(frameUpdate(key(1, 5), key(2, 105, 1), older, false, 8)!.kind).toBe("reset");
    // A snapshot was skipped.
    expect(frameUpdate(key(1, 5), key(3, 105), { ...older, version: 3 }, false, 8)!.kind).toBe("reset");
    // A full reload.
    expect(frameUpdate(key(1, 5), key(2, 5), { version: 2, kind: "reset" }, false, 8)!.kind).toBe("reset");
  });
});

describe("createThrottle", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("runs the first call at once and at most once per period after", () => {
    const fn = vi.fn();
    const t = createThrottle(fn, 500);
    t.call();
    expect(fn).toHaveBeenCalledTimes(1);
    t.call();
    t.call();
    expect(fn).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(499);
    expect(fn).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1);
    expect(fn).toHaveBeenCalledTimes(2);
    vi.advanceTimersByTime(5000);
    expect(fn).toHaveBeenCalledTimes(2);
  });

  it("calls through when the period has passed", () => {
    const fn = vi.fn();
    const t = createThrottle(fn, 500);
    t.call();
    vi.advanceTimersByTime(600);
    t.call();
    expect(fn).toHaveBeenCalledTimes(2);
  });

  it("flushes a pending call and cancels one", () => {
    const fn = vi.fn();
    const t = createThrottle(fn, 500);
    t.call();
    t.call();
    t.flush();
    expect(fn).toHaveBeenCalledTimes(2);
    t.flush();
    expect(fn).toHaveBeenCalledTimes(2);
    t.call();
    t.cancel();
    vi.advanceTimersByTime(1000);
    expect(fn).toHaveBeenCalledTimes(2);
  });
});
