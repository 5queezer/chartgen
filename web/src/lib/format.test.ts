import { describe, expect, it } from "vitest";

import {
  barLegend,
  formatChange,
  formatPrice,
  formatVolume,
  pricePrecision,
} from "./format";

describe("pricePrecision", () => {
  it("defaults to 2 without usable data", () => {
    expect(pricePrecision([])).toBe(2);
    expect(pricePrecision([null, NaN])).toBe(2);
  });
  it("keeps the decimals the data needs", () => {
    expect(pricePrecision([1.0842, 1.08425])).toBe(5);
    expect(pricePrecision([0.00001234])).toBe(8);
  });
  it("caps float noise by magnitude", () => {
    expect(pricePrecision([67123.456789012])).toBe(2);
    expect(pricePrecision([30.123456789])).toBe(5);
    expect(pricePrecision([0.5123456789])).toBe(6);
  });
  it("never goes below 2", () => {
    expect(pricePrecision([1500, 1510])).toBe(2);
  });
});

describe("formatPrice", () => {
  it("formats and handles missing values", () => {
    expect(formatPrice(1.5, 3)).toBe("1.500");
    expect(formatPrice(null, 2)).toBe("—");
    expect(formatPrice(NaN, 2)).toBe("—");
  });
});

describe("formatVolume", () => {
  it.each([
    [12.345, "12.35"],
    [999, "999.00"],
    [1_000, "1.00K"],
    [12_345, "12.35K"],
    [3_450_000, "3.45M"],
    [7_890_000_000, "7.89B"],
  ])("%d -> %s", (v, out) => {
    expect(formatVolume(v)).toBe(out);
  });
  it("handles null", () => {
    expect(formatVolume(null)).toBe("—");
  });
});

describe("formatChange", () => {
  it("signs gains and losses", () => {
    expect(formatChange(110, 100, 2)).toEqual({
      abs: "+10.00",
      pct: "+10.00%",
      direction: "up",
    });
    expect(formatChange(95, 100, 2)).toEqual({
      abs: "-5.00",
      pct: "-5.00%",
      direction: "down",
    });
  });
  it("is flat with no sign when unchanged", () => {
    expect(formatChange(100, 100, 2)).toEqual({
      abs: "0.00",
      pct: "0.00%",
      direction: "flat",
    });
  });
  it("is null without a usable previous close", () => {
    expect(formatChange(100, null, 2)).toBeNull();
    expect(formatChange(null, 100, 2)).toBeNull();
  });
  it("does not divide by zero", () => {
    expect(formatChange(1, 0, 2)?.pct).toBe("+0.00%");
  });
});

describe("barLegend", () => {
  const bars = [
    { t: 1, o: 10, h: 12, l: 9, c: 11, v: 1500 },
    { t: 2, o: 11, h: 13, l: 10, c: 10.5, v: 2_500_000 },
  ];
  it("builds strings with change vs previous close", () => {
    expect(barLegend(bars, 1, 2)).toEqual({
      o: "11.00",
      h: "13.00",
      l: "10.00",
      c: "10.50",
      v: "2.50M",
      change: { abs: "-0.50", pct: "-4.55%", direction: "down" },
    });
  });
  it("has no change on the first bar", () => {
    expect(barLegend(bars, 0, 2)?.change).toBeNull();
  });
  it("is null out of range", () => {
    expect(barLegend(bars, 5, 2)).toBeNull();
  });
});
