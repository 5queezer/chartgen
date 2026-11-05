import { describe, expect, it } from "vitest";

import {
  arrowHead,
  barsBetween,
  clampPosition,
  dateRangeLabel,
  defaultPosition,
  extendRay,
  fibExtensionLevels,
  formatDuration,
  pitchforkLines,
  positionBars,
  positionLabels,
  positionReward,
  positionStats,
  priceRangeLabel,
} from "./drawingMath";

describe("price range", () => {
  it("labels an up move with sign, separators and percent", () => {
    expect(priceRangeLabel(53_400, 54_634.5, 2)).toBe("+1,234.50 (+2.31%)");
  });
  it("labels a down move", () => {
    expect(priceRangeLabel(200, 150, 2)).toBe("-50.00 (-25.00%)");
  });
  it("survives a zero start", () => {
    expect(priceRangeLabel(0, 5, 1)).toBe("+5.0 (+0.00%)");
  });
});

describe("date range", () => {
  it("formats durations with the two largest units", () => {
    expect(formatDuration(3 * 86_400_000 + 4 * 3_600_000)).toBe("3d 4h");
    expect(formatDuration(2 * 86_400_000)).toBe("2d");
    expect(formatDuration(5.5 * 3_600_000)).toBe("5h 30m");
    expect(formatDuration(45 * 60_000)).toBe("45m");
    expect(formatDuration(-3_600_000)).toBe("1h");
  });
  it("labels bars and span", () => {
    expect(dateRangeLabel(12, 12 * 3_600_000)).toBe("12 bars, 12h");
    expect(dateRangeLabel(1, 60_000)).toBe("1 bar, 1m");
  });
  it("counts bars from pixels", () => {
    expect(barsBetween(100, 180, 8)).toBe(10);
    expect(barsBetween(180, 100, 8)).toBe(10);
    expect(barsBetween(0, 10, 0)).toBe(0);
  });
});

describe("arrow head", () => {
  it("ends both strokes at the tip", () => {
    const [a, b] = arrowHead({ x: 0, y: 0 }, { x: 10, y: 0 }, 4);
    expect(a![1]).toEqual({ x: 10, y: 0 });
    expect(b![1]).toEqual({ x: 10, y: 0 });
    // Wings sit behind the tip, mirrored around the shaft.
    expect(a![0].x).toBeLessThan(10);
    expect(a![0].y).toBeCloseTo(-b![0].y);
  });
});

describe("fib extension", () => {
  it("projects the A to B move from C", () => {
    const lv = fibExtensionLevels(100, 150, 120);
    expect(lv).toHaveLength(11);
    expect(lv[0]).toEqual({ level: 0, value: 120 });
    expect(lv.find((l) => l.level === 1)?.value).toBe(170);
    expect(lv.find((l) => l.level === 1.618)?.value).toBeCloseTo(200.9);
    expect(lv.find((l) => l.level === 4.236)?.value).toBeCloseTo(331.8);
  });
  it("projects downwards for a falling A to B", () => {
    const lv = fibExtensionLevels(150, 100, 130, [0, 1, 2]);
    expect(lv.map((l) => l.value)).toEqual([130, 80, 30]);
  });
});

describe("pitchfork", () => {
  const w = 1000;
  const h = 500;
  it("runs the median through the midpoint of P2-P3", () => {
    const { median, upper, lower, handle } = pitchforkLines(
      { x: 100, y: 300 },
      { x: 200, y: 100 },
      { x: 300, y: 300 },
      w,
      h,
    );
    // mid = (250, 200), dir = (150, -100)
    expect(median[0]).toEqual({ x: 100, y: 300 });
    const dx = median[1].x - median[0].x;
    const dy = median[1].y - median[0].y;
    expect(dy / dx).toBeCloseTo(-100 / 150);
    // Leaves the box at the top edge (y = 0).
    expect(median[1].y).toBeCloseTo(0);
    // Parallels share the slope and start at P2 / P3.
    for (const [from, to] of [upper, lower]) {
      expect((to.y - from.y) / (to.x - from.x)).toBeCloseTo(-100 / 150);
    }
    expect(upper[0]).toEqual({ x: 200, y: 100 });
    expect(lower[0]).toEqual({ x: 300, y: 300 });
    expect(handle).toEqual([{ x: 200, y: 100 }, { x: 300, y: 300 }]);
  });
  it("extends to the right edge for a flat fork", () => {
    const { median } = pitchforkLines(
      { x: 0, y: 250 },
      { x: 100, y: 200 },
      { x: 100, y: 300 },
      w,
      h,
    );
    expect(median[1]).toEqual({ x: 1000, y: 250 });
  });
  it("extends a vertical fork to the pane edge", () => {
    expect(extendRay({ x: 10, y: 10 }, { x: 0, y: 1 }, 100, 50)).toEqual({ x: 10, y: 50 });
  });
  it("keeps the origin for a zero direction", () => {
    expect(extendRay({ x: 10, y: 10 }, { x: 0, y: 0 }, 100, 50)).toEqual({ x: 10, y: 10 });
  });
});

describe("positions", () => {
  it("defaults to +2% / -1% for a long and mirrors for a short", () => {
    const l = defaultPosition("long", 100);
    expect(l.target).toBeCloseTo(102);
    expect(l.stop).toBeCloseTo(99);
    const s = defaultPosition("short", 100);
    expect(s.target).toBeCloseTo(98);
    expect(s.stop).toBeCloseTo(101);
  });
  it("sizes a fresh position from the visible span: reward 10%, stop half", () => {
    expect(positionReward(200)).toBe(20);
    expect(positionReward(null)).toBeNull();
    expect(positionReward(0)).toBeNull();
    const l = defaultPosition("long", 100, 20);
    expect(l).toEqual({ target: 120, stop: 90 });
    expect(defaultPosition("short", 100, 20)).toEqual({ target: 80, stop: 110 });
    // A stop that would reach zero falls back to the percent default.
    expect(defaultPosition("long", 5, 20).stop).toBeCloseTo(4.95);
  });
  it("widens a fresh position with the visible bars, 20 at least", () => {
    expect(positionBars(100)).toBe(20);
    expect(positionBars(400)).toBe(40);
    expect(positionBars(null)).toBe(20);
  });
  it("computes risk/reward for both sides", () => {
    const l = positionStats("long", 100, 104, 98);
    expect(l.ratio).toBeCloseTo(2);
    expect(l.rewardPct).toBeCloseTo(4);
    expect(l.riskPct).toBeCloseTo(2);
    const s = positionStats("short", 100, 94, 102);
    expect(s.ratio).toBeCloseTo(3);
    expect(s.reward).toBeCloseTo(6);
  });
  it("has no ratio without risk", () => {
    expect(positionStats("long", 100, 110, 100).ratio).toBeNull();
  });
  it("labels the default long", () => {
    const { target, stop } = defaultPosition("long", 100);
    expect(positionLabels("long", 100, target, stop, 2)).toEqual({
      target: "Target: 102.00 (2.00%)",
      stop: "Stop: 99.00 (1.00%)",
      ratio: "Risk/Reward Ratio: 2.00",
    });
  });
  it("clamps dragged handles to their side of the entry", () => {
    expect(clampPosition("long", [100, 95, 99], 1)).toEqual([100, 100, 99]);
    expect(clampPosition("long", [100, 105, 101], 2)).toEqual([100, 105, 100]);
    expect(clampPosition("long", [110, 105, 99], 0)).toEqual([105, 105, 99]);
    expect(clampPosition("short", [100, 105, 102], 1)).toEqual([100, 100, 102]);
    expect(clampPosition("short", [100, 95, 99], 2)).toEqual([100, 95, 100]);
    expect(clampPosition("short", [90, 95, 102], 0)).toEqual([95, 95, 102]);
  });
});
