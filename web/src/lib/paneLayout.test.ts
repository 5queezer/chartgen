import { describe, expect, it } from "vitest";

import { compactPaneHeights, fitSubPanes } from "./paneLayout";

describe("fitSubPanes", () => {
  it("leaves a roomy layout alone", () => {
    expect(fitSubPanes(400, [120, 120])).toBeNull();
    expect(fitSubPanes(300, [])).toBeNull();
  });

  it("shrinks sub-panes to keep the price pane at 40% / 200px", () => {
    const out = fitSubPanes(45, [120, 120, 120, 120, 120])!;
    expect(out.every((h) => h >= 50)).toBe(true);
    expect(645 - out.reduce((a, b) => a + b, 0)).toBeGreaterThanOrEqual(200);
  });

  it("never takes sub-panes below the minimum", () => {
    const out = fitSubPanes(10, [60, 60, 60, 60, 60, 60, 60, 60])!;
    expect(out.every((h) => h === 50)).toBe(true);
  });
});

describe("compactPaneHeights", () => {
  it("gives the price pane most of a short stack", () => {
    const out = compactPaneHeights(217, [120, 120]);
    expect(out.every((h) => h >= 30)).toBe(true);
    expect(217 - out[0]! - out[1]!).toBeGreaterThanOrEqual(217 * 0.65);
  });

  it("returns the natural heights on a tall stack", () => {
    expect(compactPaneHeights(500, [120, 120])).toEqual([120, 120]);
  });

  it("does not grow panes that already fit a short stack", () => {
    expect(compactPaneHeights(300, [40, 40])).toEqual([40, 40]);
  });

  it("never takes a sub-pane below 30px, whatever the count", () => {
    const out = compactPaneHeights(217, [120, 120, 120, 120, 120]);
    expect(out).toEqual([30, 30, 30, 30, 30]);
  });

  it("has nothing to do without sub-panes", () => {
    expect(compactPaneHeights(217, [])).toEqual([]);
  });
});
