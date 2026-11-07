import { describe, expect, it } from "vitest";

import {
  alertInterval,
  chartParamsWire,
  decodeChartParams,
  DEFAULT_CHART_PARAMS,
  encodeChartParams,
  isDefaultChartParams,
  isPriceBarType,
  priceBarLabel,
  sanitizeChartParams,
} from "./priceBars";

describe("sanitizeChartParams", () => {
  it("keeps what is valid and defaults the rest", () => {
    expect(sanitizeChartParams({ box: "percent", percent: 2, reversal: 5, lines: 99 })).toEqual({
      ...DEFAULT_CHART_PARAMS,
      box: "percent",
      percent: 2,
      reversal: 5,
    });
  });

  it("will not take a fixed box with no size, or garbage", () => {
    expect(sanitizeChartParams({ box: "fixed" }).box).toBe("atr");
    expect(sanitizeChartParams({ box: "fixed", boxSize: -1 }).box).toBe("atr");
    expect(sanitizeChartParams({ box: "fixed", boxSize: 5 })).toMatchObject({ box: "fixed", boxSize: 5 });
    for (const bad of [null, 3, "x", []]) {
      expect(sanitizeChartParams(bad)).toEqual(DEFAULT_CHART_PARAMS);
    }
    expect(sanitizeChartParams({ atrLength: 0 }).atrLength).toBe(14);
    expect(sanitizeChartParams({ atrLength: 1.5 }).atrLength).toBe(14);
    expect(sanitizeChartParams({ percent: 0 }).percent).toBe(1);
    expect(sanitizeChartParams({ percent: 101 }).percent).toBe(1);
  });
});

describe("chartParamsWire", () => {
  it("sends what the type reads", () => {
    const p = { ...DEFAULT_CHART_PARAMS, reversal: 2, lines: 5 };
    expect(chartParamsWire("candles", p)).toBeUndefined();
    expect(chartParamsWire("heikin_ashi", p)).toBeUndefined();
    expect(chartParamsWire("renko", p)).toEqual({ box: "atr", atr_length: 14 });
    expect(chartParamsWire("kagi", p)).toEqual({ box: "atr", atr_length: 14 });
    expect(chartParamsWire("range", p)).toEqual({ box: "atr", atr_length: 14 });
    expect(chartParamsWire("point_figure", p)).toEqual({ box: "atr", atr_length: 14, reversal: 2 });
    expect(chartParamsWire("line_break", p)).toEqual({ lines: 5 });
    expect(chartParamsWire("renko", { ...p, box: "percent", percent: 3 })).toEqual({
      box: "percent",
      percent: 3,
    });
    expect(chartParamsWire("renko", { ...p, box: "fixed", boxSize: 50 })).toEqual({
      box: "fixed",
      box_size: 50,
    });
  });
});

describe("share link inputs", () => {
  it("writes only what differs and reads it back", () => {
    expect(encodeChartParams(DEFAULT_CHART_PARAMS)).toBe("");
    const p = { ...DEFAULT_CHART_PARAMS, box: "fixed" as const, boxSize: 0.25, reversal: 2 };
    expect(encodeChartParams(p)).toBe("box:fixed;size:0.25;rev:2");
    expect(decodeChartParams(encodeChartParams(p))).toEqual(p);
    expect(decodeChartParams("")).toEqual(DEFAULT_CHART_PARAMS);
  });

  it("ignores what it does not understand", () => {
    expect(decodeChartParams("box:tick;size:abc;len:;rev:999;nope:1;__proto__:2")).toEqual(
      DEFAULT_CHART_PARAMS,
    );
    expect(decodeChartParams("box:fixed;size:-3")).toEqual(DEFAULT_CHART_PARAMS);
  });
});

describe("names", () => {
  it("knows the defaults", () => {
    expect(isDefaultChartParams(DEFAULT_CHART_PARAMS)).toBe(true);
    expect(isDefaultChartParams({ ...DEFAULT_CHART_PARAMS, lines: 2 })).toBe(false);
    expect(isPriceBarType("renko")).toBe(true);
    expect(isPriceBarType("heikin_ashi")).toBe(false);
  });

  it("labels the legend with the size in use", () => {
    expect(priceBarLabel("renko", { box: "atr", box_size: 670 })).toBe("Renko 670");
    expect(priceBarLabel("point_figure", { box_size: 5, reversal: 2 })).toBe("P&F 5 x 2");
    expect(priceBarLabel("line_break", { lines: 4 })).toBe("Line Break 4");
    expect(priceBarLabel("candles", undefined)).toBeNull();
    expect(priceBarLabel("heikin_ashi", { box_size: 1 })).toBeNull();
  });

  it("ties an alert to the type, within what stored alerts allow", () => {
    expect(alertInterval({ interval: "1h" })).toBe("1h");
    expect(alertInterval({ interval: "1h", chart_style: "candles" })).toBe("1h");
    expect(alertInterval({ interval: "1h", chart_style: "heikin_ashi" })).toBe("1h");
    expect(alertInterval({ interval: "1h", chart_style: "renko" })).toBe("1h renko");
    for (const style of ["renko", "kagi", "point_figure", "line_break", "range"]) {
      expect(alertInterval({ interval: "365d", chart_style: style }).length).toBeLessThanOrEqual(16);
    }
  });
});
