import { describe, expect, it } from "vitest";

import {
  barLegend,
  lastFiniteCloseIndex,
  formatChange,
  abbreviate,
  formatIndicatorValue,
  formatPrice,
  formatVolume,
  hasNoVolume,
  indicatorValueKind,
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
  it("keeps significant digits for tiny prices", () => {
    expect(pricePrecision([1.234e-9])).toBe(12);
    expect(pricePrecision([0.000001234])).toBe(9);
  });
  it("ignores float32 noise (Yahoo equities get 2 decimals)", () => {
    expect(pricePrecision([385.19000244, 384.5, 386.12000122])).toBe(2);
    expect(pricePrecision([1.1238500118, 1.12360002])).toBe(5);
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
    [999, "999"],
    [1_000, "1K"],
    [4_200, "4.2K"],
    [180_000_000, "180M"],
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
      v: "2.5M",
      change: { abs: "-0.50", pct: "-4.55%", direction: "down" },
    });
  });
  it("walks back to the last finite previous close", () => {
    const gappy = [
      { t: 1, o: 1, h: 1, l: 1, c: 10, v: 1 },
      { t: 2, o: null, h: null, l: null, c: null, v: null },
      { t: 3, o: 1, h: 1, l: 1, c: 11, v: 1 },
    ];
    expect(barLegend(gappy, 2, 2)?.change?.abs).toBe("+1.00");
  });
  it("has no change on the first bar", () => {
    expect(barLegend(bars, 0, 2)?.change).toBeNull();
  });
  it("is null out of range", () => {
    expect(barLegend(bars, 5, 2)).toBeNull();
  });
});

describe("lastFiniteCloseIndex", () => {
  it("skips trailing bars without a close", () => {
    const bars = [
      { t: 1, o: 1, h: 1, l: 1, c: 5, v: 1 },
      { t: 2, o: null, h: null, l: null, c: null, v: null },
    ];
    expect(lastFiniteCloseIndex(bars)).toBe(0);
    expect(lastFiniteCloseIndex([])).toBe(-1);
  });
});

describe("abbreviate", () => {
  it.each([
    [0, "0"],
    [4200, "4.2K"],
    [180_000_000, "180M"],
    [1_234_567, "1.23M"],
    [2_500_000_000, "2.5B"],
    [-4200, "-4.2K"],
    [12.5, "12.5"],
    [999_995, "1M"],
    [999_999_999, "1B"],
    [999.996, "1K"],
    [999_994, "999.99K"],
    [0.0004, "0.0004"],
    [-0.00031, "-0.00031"],
  ])("%d -> %s", (v, out) => expect(abbreviate(v)).toBe(out));
});

describe("formatIndicatorValue", () => {
  it("uses the price precision for overlays, never K", () => {
    expect(formatIndicatorValue(80452.1234, "overlay", 2)).toBe("80452.12");
    expect(formatIndicatorValue(1.1238512, "overlay", 5)).toBe("1.12385");
  });
  it("abbreviates volume and keeps oscillator precision", () => {
    expect(formatIndicatorValue(1_500_000, "volume", 2)).toBe("1.5M");
    expect(formatIndicatorValue(47.5678, "oscillator", 5)).toBe("47.57");
  });
  it("price-unit panes use the price precision, with significant digits for tiny values", () => {
    expect(formatIndicatorValue(0.00083, "priceUnit", 5)).toBe("0.00083");
    // ATR of a SHIB-like coin: a 6-decimal price would print 0.000000.
    expect(formatIndicatorValue(3.456e-7, "priceUnit", 6)).toBe("0.00000035");
    expect(formatIndicatorValue(123.456, "priceUnit", 2)).toBe("123.46");
  });
  it("picks the kind from the instance key", () => {
    expect(indicatorValueKind("atr", false)).toBe("priceUnit");
    expect(indicatorValueKind("macd#2", false)).toBe("priceUnit");
    for (const k of ["obv", "cvd", "oi", "elder_force_index"]) expect(indicatorValueKind(k, false)).toBe("volume");
    expect(indicatorValueKind("volume#2", false)).toBe("volume");
    expect(indicatorValueKind("ema_stack", true)).toBe("overlay");
    expect(indicatorValueKind("rsi", false)).toBe("oscillator");
  });
});

describe("hasNoVolume", () => {
  it("is true for all-zero or null volume", () => {
    expect(hasNoVolume([{ v: 0 }, { v: null }])).toBe(true);
    expect(hasNoVolume([{ v: 0 }, { v: 3 }])).toBe(false);
  });
});
