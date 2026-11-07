import { describe, expect, it } from "vitest";
import type { IndicatorDrawParams, KLineData } from "klinecharts";

import {
  barRect,
  baselineValue,
  drawSeries,
  drawSpan,
  hiddenCandleStyle,
  isSeriesType,
  isUpBar,
  markerRadius,
  stepPoints,
} from "./chartTypes";
import { CHART_TYPES } from "../stores/layout";

// A y axis at BTC-like prices that rounds to whole pixels as klinecharts does:
// 2000 price units over 400px, so one unit is 0.2px.
const HEIGHT = 400;
const LO = 99_000;
const SPAN = 2000;
const mockYAxis = (inverted: boolean) =>
  ({
    convertToPixel: (v: number) => {
      const f = (v - LO) / SPAN;
      return Math.round((inverted ? f : 1 - f) * HEIGHT);
    },
    convertFromPixel: (p: number) => {
      const f = p / HEIGHT;
      return LO + (inverted ? f : 1 - f) * SPAN;
    },
  }) as unknown as IndicatorDrawParams["yAxis"];

/** Run drawSeries on a recording canvas; returns what it filled and stroked. */
function paint(type: "columns" | "baseline", inverted: boolean) {
  const fills: { y: number; h: number }[] = [];
  const strokes: { color: string; clip: [number, number] }[] = [];
  let rect: [number, number] = [0, 0];
  let clip: [number, number] = [0, 0];
  const ctx = {
    fillStyle: "",
    strokeStyle: "",
    setLineDash() {},
    save() {},
    restore() {},
    beginPath() {},
    moveTo() {},
    lineTo() {},
    closePath() {},
    arc() {},
    fill() {},
    createLinearGradient: () => ({ addColorStop() {} }),
    rect: (_x: number, y: number, _w: number, h: number) => (rect = [y, h]),
    clip: () => (clip = rect),
    stroke() {
      strokes.push({ color: ctx.strokeStyle, clip });
    },
    fillRect: (_x: number, y: number, _w: number, h: number) => fills.push({ y, h }),
  };
  const bars = [100_000, 100_400, 99_800, 100_200].map((c) => bar(c - 50, c));
  drawSeries({
    ctx: ctx as unknown as CanvasRenderingContext2D,
    kLineDataList: bars,
    indicator: { extendData: { type, up: "#26a69a", down: "#ef5350", line: "#2196f3" } },
    visibleRange: { from: 0, to: 4, realFrom: 0, realTo: 4 },
    bounding: { width: 600, height: HEIGHT, left: 0, top: 0, right: 600, bottom: HEIGHT },
    barSpace: { bar: 8, halfBar: 4, gapBar: 6, halfGapBar: 3 },
    xAxis: { convertToPixel: (i: number) => 20 + i * 10, convertFromPixel: () => 0 },
    yAxis: mockYAxis(inverted),
  } as unknown as IndicatorDrawParams);
  return { fills, strokes };
}

describe("series types on a rounding axis at high prices", () => {
  it("Columns hang from the bottom edge on a normal axis", () => {
    const { fills } = paint("columns", false);
    expect(fills.length).toBe(4);
    for (const f of fills) expect(f.y + f.h).toBe(HEIGHT);
  });

  it("Columns hang from the top edge on an inverted axis", () => {
    const { fills } = paint("columns", true);
    for (const f of fills) expect(f.y).toBe(0);
  });

  it("Baseline paints the up color above the base line on a normal axis", () => {
    const { strokes } = paint("baseline", false);
    const up = strokes.find((s) => s.color === "#26a69a")!;
    expect(up.clip[0]).toBe(0);
  });

  it("Baseline paints the up color below the base line when inverted", () => {
    const { strokes } = paint("baseline", true);
    const up = strokes.find((s) => s.color === "#26a69a")!;
    expect(up.clip[0]).toBeGreaterThan(0);
    expect(up.clip[0] + up.clip[1]).toBe(HEIGHT);
  });
});

const bar = (open: number, close: number): KLineData => ({
  timestamp: 0,
  open,
  high: Math.max(open, close),
  low: Math.min(open, close),
  close,
});

describe("chart types", () => {
  it("draws exactly the types klinecharts has no candle type for", () => {
    expect(CHART_TYPES.filter(isSeriesType)).toEqual([
      "line_markers",
      "step_line",
      "baseline",
      "columns",
      "high_low",
      "volume_candles",
    ]);
  });

  it("hides klinecharts' series: candles for high-low, an area line otherwise", () => {
    const hl = hiddenCandleStyle("high_low") as {
      type: string;
      bar: Record<string, string>;
    };
    expect(hl.type).toBe("candle_solid");
    expect(Object.keys(hl.bar)).toHaveLength(9);
    expect(new Set(Object.values(hl.bar))).toEqual(new Set(["rgba(0, 0, 0, 0)"]));
    expect(hiddenCandleStyle("baseline")).toMatchObject({
      type: "area",
      area: { lineColor: "rgba(0, 0, 0, 0)", backgroundColor: "rgba(0, 0, 0, 0)" },
    });
  });

  it("draws one bar past each edge of the visible range", () => {
    expect(drawSpan({ from: 10, to: 20 }, 100)).toEqual([9, 21]);
    expect(drawSpan({ from: 0, to: 100 }, 100)).toEqual([0, 100]);
    expect(drawSpan({ from: 0, to: 0 }, 0)).toEqual([0, 0]);
  });

  it("steps horizontally to the next bar, then vertically", () => {
    expect(
      stepPoints([
        { x: 0, y: 10 },
        { x: 5, y: 20 },
        { x: 10, y: 15 },
      ]),
    ).toEqual([
      { x: 0, y: 10 },
      { x: 5, y: 10 },
      { x: 5, y: 20 },
      { x: 10, y: 20 },
      { x: 10, y: 15 },
    ]);
    expect(stepPoints([])).toEqual([]);
  });

  it("puts the base level in the middle of the visible closes", () => {
    expect(baselineValue([4, 10, 6])).toBe(7);
    expect(baselineValue([5])).toBe(5);
    expect(baselineValue([])).toBeNull();
  });

  it("colors a bar against the previous close, the first against its open", () => {
    const bars = [bar(10, 9), bar(9, 12), bar(13, 12), bar(11, 12)];
    expect(isUpBar(bars, 0)).toBe(false);
    expect(isUpBar(bars, 1)).toBe(true);
    // Red candle, but it closed at the previous close: up, as in TradingView.
    expect(isUpBar(bars, 2)).toBe(true);
    expect(isUpBar([bar(10, 10), bar(12, 9)], 1)).toBe(false);
  });

  it("snaps bar rects to whole pixels with at least 1px each way", () => {
    expect(barRect(50, 20.4, 80.6, 6)).toEqual({ x: 47, y: 20, width: 6, height: 61 });
    // Either edge order.
    expect(barRect(50, 80.6, 20.4, 6)).toEqual({ x: 47, y: 20, width: 6, height: 61 });
    expect(barRect(10.2, 30, 30, 0.3)).toEqual({ x: 10, y: 30, width: 1, height: 1 });
  });

  it("shrinks the markers when bars are packed", () => {
    expect(markerRadius(2)).toBe(1.5);
    expect(markerRadius(8)).toBeCloseTo(2.4);
    expect(markerRadius(40)).toBe(3.5);
  });
});
