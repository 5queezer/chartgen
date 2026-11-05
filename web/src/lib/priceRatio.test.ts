import { describe, expect, it } from "vitest";

import { DEFAULT_GAP, lockRatio, lockedGap } from "./priceRatio";

describe("priceRatio", () => {
  it("keeps the range unchanged at the locked bar space", () => {
    const ratio = lockRatio(130, 500, 8);
    const g = lockedGap({ range: 130, gap: DEFAULT_GAP, height: 500, barSpace: 8, ratio });
    expect(g).toBeCloseTo((DEFAULT_GAP.top + DEFAULT_GAP.bottom) / 2);
  });

  it("halves the visible range when bars are twice as wide", () => {
    const ratio = lockRatio(130, 500, 8);
    const g = lockedGap({ range: 130, gap: DEFAULT_GAP, height: 500, barSpace: 16, ratio });
    // base 100, wanted 65 -> total (1 + 2g) = 0.65
    expect(g).toBeCloseTo(-0.175);
  });

  it("derives the base from a gap already applied", () => {
    const ratio = lockRatio(130, 500, 8);
    const g = lockedGap({
      range: 65,
      gap: { top: -0.175, bottom: -0.175 },
      height: 500,
      barSpace: 16,
      ratio,
    });
    expect(g).toBeCloseTo(-0.175);
  });

  it("clamps so the range never collapses", () => {
    const ratio = lockRatio(130, 500, 8);
    expect(
      lockedGap({ range: 130, gap: DEFAULT_GAP, height: 500, barSpace: 800, ratio }),
    ).toBe(-0.45);
  });
});
