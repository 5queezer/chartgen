import { describe, expect, it } from "vitest";

import { effectiveMagnet, magnetValue } from "./magnet";

describe("magnetValue", () => {
  const bar = { open: 100, high: 120, low: 90, close: 110 };
  const toY = (v: number) => 1000 - v; // 1 px per unit

  it("leaves the value alone when the magnet is off", () => {
    expect(magnetValue(101, bar, "normal", 8, toY)).toBe(101);
  });

  it.each([
    [500, 120],
    [121, 120],
    [116, 120],
    [114, 110],
    [106, 110],
    [104, 100],
    [96, 100],
    [94, 90],
    [1, 90],
  ])("strong takes %d to the nearest of OHLC, %d", (value, want) => {
    expect(magnetValue(value, bar, "strong_magnet", 8, toY)).toBe(want);
  });

  it("weak only snaps within the sensitivity, on either side of the bar", () => {
    expect(magnetValue(128, bar, "weak_magnet", 8, toY)).toBe(120);
    expect(magnetValue(128.5, bar, "weak_magnet", 8, toY)).toBe(128.5);
    expect(magnetValue(82, bar, "weak_magnet", 8, toY)).toBe(90);
    expect(magnetValue(81.5, bar, "weak_magnet", 8, toY)).toBe(81.5);
    expect(magnetValue(115, bar, "weak_magnet", 2, toY)).toBe(115);
  });

  it("weak measures on the axis, whichever way it runs", () => {
    expect(magnetValue(128, bar, "weak_magnet", 8, (v) => v)).toBe(120);
    expect(magnetValue(128, bar, "weak_magnet", 8, (v) => 2 * v)).toBe(128);
  });

  it("weak does not snap when the axis has no pixel for the value", () => {
    expect(magnetValue(128, bar, "weak_magnet", 8, () => NaN)).toBe(128);
  });
});

describe("effectiveMagnet", () => {
  it("keeps the chosen magnet without the modifier", () => {
    expect(effectiveMagnet("normal", false)).toBe("normal");
    expect(effectiveMagnet("weak_magnet", false)).toBe("weak_magnet");
    expect(effectiveMagnet("strong_magnet", false)).toBe("strong_magnet");
  });

  it("inverts it while the modifier is held", () => {
    expect(effectiveMagnet("normal", true)).toBe("strong_magnet");
    expect(effectiveMagnet("weak_magnet", true)).toBe("normal");
    expect(effectiveMagnet("strong_magnet", true)).toBe("normal");
  });
});
