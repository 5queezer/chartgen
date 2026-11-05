import { describe, expect, it } from "vitest";

import { DEFAULT_GAP, lockRatio, lockedGaps, toPaneGap } from "./priceRatio";

// Data 100..200 shown with the default gap: axis 90..220 (range 130).
const base = { vMin: 90, vMax: 220, gap: DEFAULT_GAP, height: 500 };
const ratio = lockRatio(130, 500, 8);

describe("priceRatio", () => {
  it("keeps the gaps at the locked bar space", () => {
    const g = lockedGaps({ ...base, barSpace: 8, ratio });
    expect(g.top).toBeCloseTo(0.2);
    expect(g.bottom).toBeCloseTo(0.1);
  });

  it("halves the visible range when bars are twice as wide, centered", () => {
    const g = lockedGaps({ ...base, barSpace: 16, ratio });
    // wanted 65 around the data center 150: 117.5..182.5
    expect(g.bottom).toBeCloseTo(-0.175);
    expect(g.top).toBeCloseTo(-0.175);
  });

  it("slides the window to keep the anchor in view", () => {
    const g = lockedGaps({ ...base, barSpace: 16, ratio, anchor: 195 });
    // window 65 wide, 5% margin: ends at 198.25 -> 133.25..198.25
    expect(g.top).toBeCloseTo((198.25 - 200) / 100);
    expect(g.bottom).toBeCloseTo((100 - 133.25) / 100);
  });

  it("derives the base from a gap already applied", () => {
    const g = lockedGaps({
      vMin: 117.5,
      vMax: 182.5,
      gap: { top: -0.175, bottom: -0.175 },
      height: 500,
      barSpace: 16,
      ratio,
    });
    expect(g.top).toBeCloseTo(-0.175);
    expect(g.bottom).toBeCloseTo(-0.175);
  });

  it("never crops below a tenth of the data range", () => {
    const g = lockedGaps({ ...base, barSpace: 800, ratio });
    expect(g.top + g.bottom).toBeCloseTo(-0.9);
  });

  it("converts gaps of 1 or more to pixels for klinecharts", () => {
    expect(toPaneGap({ top: 1.5, bottom: 0.4 }, 500)).toEqual({ top: 750, bottom: 0.4 });
    expect(toPaneGap({ top: 0.99, bottom: -0.2 }, 500)).toEqual({ top: 0.99, bottom: -0.2 });
  });

  it("zooming far out gives a gap of 1 or more that survives toPaneGap", () => {
    const g = lockedGaps({ ...base, barSpace: 0.8, ratio });
    expect(g.top).toBeGreaterThanOrEqual(1);
    expect(toPaneGap(g, 500).top).toBeCloseTo(g.top * 500);
  });
});
