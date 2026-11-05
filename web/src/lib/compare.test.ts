import { describe, expect, it } from "vitest";

import {
  alignAsOf,
  compareReading,
  formatPct,
  mergeBars,
  rebase,
} from "./compare";

const bars = (...pairs: Array<[number, number]>) =>
  pairs.map(([t, c]) => ({ t, c }));

describe("alignAsOf", () => {
  it("takes the last compare close at or before each main bar", () => {
    const out = alignAsOf([10, 20, 30, 40], bars([10, 1], [25, 2], [30, 3]));
    expect(out).toEqual([1, 1, 3, 3]);
  });

  it("is null before the compare series starts", () => {
    expect(alignAsOf([5, 10], bars([10, 7]))).toEqual([null, 7]);
  });

  it("carries the last close past the end and keeps null times null", () => {
    expect(alignAsOf([10, null, 99], bars([10, 7]))).toEqual([7, null, 7]);
  });

  it("is all null for an empty compare series", () => {
    expect(alignAsOf([1, 2], [])).toEqual([null, null]);
  });
});

describe("mergeBars", () => {
  it("replaces a shared timestamp with the newer bar and appends", () => {
    expect(mergeBars(bars([1, 1], [2, 2]), bars([2, 9], [3, 3]))).toEqual(
      bars([1, 1], [2, 9], [3, 3]),
    );
  });

  it("merges an older page in front", () => {
    expect(mergeBars(bars([0, 0], [1, 1]), bars([1, 5], [2, 2]))).toEqual(
      bars([0, 0], [1, 5], [2, 2]),
    );
    expect(mergeBars(bars([2, 2]), bars([0, 0], [1, 1]))).toEqual(
      bars([0, 0], [1, 1], [2, 2]),
    );
  });
});

describe("rebase", () => {
  it("puts the first visible compare value at the main close of that bar", () => {
    const out = rebase([3000, 3300, 2700], [100, 105, 95], 0);
    expect(out[0]).toBeCloseTo(100);
    // Percent as klinecharts computes it: (v - c0) / c0 * 100.
    expect(((out[1]! - 100) / 100) * 100).toBeCloseTo(10);
    expect(((out[2]! - 100) / 100) * 100).toBeCloseTo(-10);
  });

  it("re-anchors at the first visible bar", () => {
    const out = rebase([10, 20, 40], [50, 60, 70], 1);
    expect(out[1]).toBeCloseTo(60);
    expect(out[2]).toBeCloseTo(120);
  });

  it("anchors at the first bar with a value and keeps nulls", () => {
    expect(rebase([null, 5, 10], [100, 100, 100], 0)).toEqual([null, 100, 200]);
  });

  it("is all null without any anchor", () => {
    expect(rebase([null, null], [1, 2], 0)).toEqual([null, null]);
    expect(rebase([1, 2], [null, null], 0)).toEqual([null, null]);
  });
});

describe("compareReading", () => {
  it("reads the last value and its change since the first visible bar", () => {
    const r = compareReading([100, 110, 121], 0, null)!;
    expect(r.value).toBe(121);
    expect(r.pct).toBeCloseTo(21);
  });

  it("follows the crosshair bar and the view start", () => {
    expect(compareReading([100, 110, 121], 1, 2)!.pct).toBeCloseTo(10);
    expect(compareReading([100, 110, 121], 0, 1)!.value).toBe(110);
  });

  it("is null without values", () => {
    expect(compareReading([null, null], 0, null)).toBeNull();
  });
});

it("formatPct signs the value", () => {
  expect(formatPct(1.234)).toBe("+1.23%");
  expect(formatPct(-0.5)).toBe("-0.50%");
});
