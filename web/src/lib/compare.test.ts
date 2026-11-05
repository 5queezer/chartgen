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

  const DAY = 86_400;
  const HOUR = 3_600;

  it("joins a Binance 1D main bar to the Yahoo bar of the same session", () => {
    // BTCUSDT daily bars open 00:00 UTC; AAPL's open at 14:30 UTC.
    const d0 = 1_700_006_400 - (1_700_006_400 % DAY);
    const btc = [d0, d0 + DAY, d0 + 2 * DAY];
    const aapl = bars(
      [d0 - DAY + 14.5 * HOUR, 10],
      [d0 + 14.5 * HOUR, 11],
      [d0 + DAY + 14.5 * HOUR, 12],
    );
    expect(alignAsOf(btc, aapl, DAY)).toEqual([11, 12, 12]);
    // Joining on the open would lag one session.
    expect(alignAsOf(btc, aapl)).toEqual([10, 11, 12]);
  });

  it("joins a 1W main bar to the weekly bar of the same week", () => {
    const WEEK = 7 * DAY;
    const mon = 1_700_006_400 - (1_700_006_400 % WEEK);
    const btc = [mon, mon + WEEK];
    const aapl = bars([mon + 13.5 * HOUR, 5], [mon + WEEK + 13.5 * HOUR, 6]);
    expect(alignAsOf(btc, aapl, WEEK)).toEqual([5, 6]);
  });

  it("does not take a bar that opens at the end of the main bar", () => {
    expect(alignAsOf([0, 60], bars([0, 1], [60, 2]), 60)).toEqual([1, 2]);
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

  it("starts a late line at 0% on the axis, like the legend", () => {
    const closes = [100, 120, 130];
    const out = rebase([null, 5, 10], closes, 0);
    const axisPct = (v: number) => ((v - closes[0]!) / closes[0]!) * 100;
    expect(axisPct(out[1]!)).toBeCloseTo(0);
    expect(axisPct(out[2]!)).toBeCloseTo(100);
    expect(compareReading([null, 5, 10], 0, null)!.pct).toBeCloseTo(100);
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

  it("has no reading at a crosshair bar without a value", () => {
    expect(compareReading([null, 5, 10], 0, 0)).toBeNull();
    expect(compareReading([5, null, 10], 0, 1)).toBeNull();
  });

  it("is null without values", () => {
    expect(compareReading([null, null], 0, null)).toBeNull();
  });
});

it("formatPct signs the value", () => {
  expect(formatPct(1.234)).toBe("+1.23%");
  expect(formatPct(-0.5)).toBe("-0.50%");
});
