// Chart types klinecharts v9 has no candle type for (ADR-0030): Baseline,
// Columns, High-low, Step line and Line with markers. klinecharts keeps a
// hidden (transparent) price series, so its autoscale, crosshair, last price
// line and data handling stay as they are; one indicator on the candle pane
// paints the visible series from the bars in its `draw` callback.
//
// The geometry helpers are pure so they can be unit tested; `drawSeries` is
// the canvas part.

import {
  registerIndicator,
  type IndicatorDrawParams,
  type KLineData,
} from "klinecharts";

import { formatColor, parseColor } from "./color";
import type { ChartType } from "../stores/layout";

export type SeriesType =
  | "baseline"
  | "columns"
  | "high_low"
  | "step_line"
  | "line_markers";

const SERIES_TYPES: readonly ChartType[] = [
  "baseline",
  "columns",
  "high_low",
  "step_line",
  "line_markers",
];

/** True for the chart types drawn by the series indicator, not by klinecharts. */
export function isSeriesType(t: ChartType): t is SeriesType {
  return SERIES_TYPES.includes(t);
}

// Double underscore: server indicators register as `chartgen_<name>` and
// their names start with a letter, so a user script named `series` cannot
// collide with it.
export const SERIES_REG_NAME = "chartgen__series";

const TRANSPARENT = "rgba(0, 0, 0, 0)";
// TradingView's neutral gray, readable on both themes.
const BASE_LEVEL_COLOR = "rgba(120, 123, 134, 0.6)";

let registered = false;

/**
 * Register (once) the series indicator. No figures, so it adds nothing to
 * the autoscale; a negative zLevel puts it beneath the other indicators. The
 * candle pane's axis labels always use the price precision; an indicator's
 * precision only sets how far klinecharts pads a flat value range, so 10
 * keeps it from widening that padding beyond the price precision.
 */
export function ensureSeriesRegistered(): void {
  if (registered) return;
  registered = true;
  registerIndicator({
    name: SERIES_REG_NAME,
    shortName: "SERIES",
    precision: 10,
    zLevel: -1,
    figures: [],
    calc: () => [],
    draw: drawSeries,
  });
}

/**
 * klinecharts `candle` style that hides its own series, applied after the
 * settings styles. High-low keeps a candle so the autoscale spans high to
 * low; the others keep an area line, which autoscales on the closes as
 * TradingView does for close-based types.
 */
export function hiddenCandleStyle(t: SeriesType): object {
  if (t === "high_low") {
    const bar: Record<string, string> = {};
    for (const part of ["", "Border", "Wick"]) {
      for (const dir of ["up", "down", "noChange"]) {
        bar[`${dir}${part}Color`] = TRANSPARENT;
      }
    }
    return { type: "candle_solid", bar };
  }
  return {
    type: "area",
    area: {
      lineColor: TRANSPARENT,
      backgroundColor: TRANSPARENT,
      point: { show: false },
    },
  };
}

/** `extendData` of the series indicator: what to draw and in which colors. */
export interface SeriesExtend {
  type: SeriesType;
  up: string;
  down: string;
  line: string;
}

export interface Point {
  x: number;
  y: number;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Data indices to draw, `[start, end)`: the visible range (`to` exclusive)
 * plus one bar on each side, so lines run to the pane edges.
 */
export function drawSpan(
  range: { from: number; to: number },
  n: number,
): [number, number] {
  return [Math.max(0, range.from - 1), Math.min(n, range.to + 1)];
}

/** Step line through `points`: horizontal to the next bar, then vertical. */
export function stepPoints(points: Point[]): Point[] {
  const out: Point[] = [];
  points.forEach((p, k) => {
    if (k > 0) out.push({ x: p.x, y: points[k - 1]!.y });
    out.push(p);
  });
  return out;
}

/** TradingView's default base level: the middle of the visible close range. */
export function baselineValue(closes: number[]): number | null {
  if (closes.length === 0) return null;
  return (Math.min(...closes) + Math.max(...closes)) / 2;
}

/** Up bar: closed at or above the previous close (the first bar: its open). */
export function isUpBar(bars: readonly KLineData[], i: number): boolean {
  const ref = i > 0 ? bars[i - 1]!.close : bars[i]!.open;
  return bars[i]!.close >= ref;
}

/** Bar body from `y1` to `y2` centred on `x`, at least 1px wide and tall. */
export function barRect(x: number, y1: number, y2: number, width: number): Rect {
  const w = Math.max(1, Math.round(width));
  const top = Math.round(Math.min(y1, y2));
  return {
    x: Math.round(x - w / 2),
    y: top,
    width: w,
    height: Math.max(1, Math.round(Math.max(y1, y2)) - top),
  };
}

/** Dot radius of Line with markers: smaller when bars are packed. */
export function markerRadius(barSpace: number): number {
  return Math.min(3.5, Math.max(1.5, barSpace * 0.3));
}

const withAlpha = (color: string, a: number) => {
  const { hex, alpha } = parseColor(color);
  return formatColor(hex, alpha * a);
};

function tracePath(ctx: CanvasRenderingContext2D, points: Point[]): void {
  ctx.beginPath();
  points.forEach((p, k) => (k === 0 ? ctx.moveTo(p.x, p.y) : ctx.lineTo(p.x, p.y)));
}

/**
 * `draw` callback of the series indicator. The indicator has a negative
 * zLevel, so klinecharts paints it with `destination-over`: beneath the last
 * price line and the other indicators, as candles would be. Each paint lands
 * beneath the previous one, so the topmost part is painted first.
 */
export function drawSeries({
  ctx,
  kLineDataList: bars,
  indicator,
  visibleRange,
  bounding,
  barSpace,
  xAxis,
  yAxis,
}: IndicatorDrawParams): boolean {
  const ext = indicator.extendData as SeriesExtend | undefined;
  const [start, end] = drawSpan(visibleRange, bars.length);
  if (!ext || end <= start) return true;
  const point = (i: number, v: number): Point => ({
    x: xAxis.convertToPixel(i),
    y: yAxis.convertToPixel(v),
  });
  const closes: Point[] = [];
  for (let i = start; i < end; i++) closes.push(point(i, bars[i]!.close));
  // The context carries over the dash of klinecharts' last price line.
  ctx.setLineDash([]);
  ctx.lineJoin = "round";
  ctx.lineCap = "round";

  switch (ext.type) {
    case "columns":
    case "high_low": {
      const up = ext.type === "high_low" ? ext.line : withAlpha(ext.up, 0.5);
      const down = ext.type === "high_low" ? ext.line : withAlpha(ext.down, 0.5);
      for (let i = start; i < end; i++) {
        const b = bars[i]!;
        const x = xAxis.convertToPixel(i);
        const r =
          ext.type === "columns"
            ? barRect(x, yAxis.convertToPixel(b.close), bounding.height, barSpace.gapBar)
            : barRect(x, yAxis.convertToPixel(b.high), yAxis.convertToPixel(b.low), barSpace.gapBar);
        ctx.fillStyle = ext.type === "columns" && !isUpBar(bars, i) ? down : up;
        ctx.fillRect(r.x, r.y, r.width, r.height);
      }
      break;
    }
    case "step_line":
    case "line_markers": {
      if (ext.type === "line_markers") {
        ctx.fillStyle = ext.line;
        const r = markerRadius(barSpace.bar);
        ctx.beginPath();
        for (const p of closes) {
          ctx.moveTo(p.x + r, p.y);
          ctx.arc(p.x, p.y, r, 0, Math.PI * 2);
        }
        ctx.fill();
      }
      ctx.strokeStyle = ext.line;
      ctx.lineWidth = 2;
      tracePath(ctx, ext.type === "step_line" ? stepPoints(closes) : closes);
      ctx.stroke();
      break;
    }
    case "baseline": {
      const visible = [];
      for (let i = visibleRange.from; i < visibleRange.to; i++) visible.push(bars[i]!.close);
      const base = baselineValue(visible);
      if (base === null) break;
      const baseY = yAxis.convertToPixel(base);
      const halves = [
        { color: ext.up, clip: [0, baseY], edge: 0 },
        { color: ext.down, clip: [baseY, bounding.height], edge: bounding.height },
      ] as const;
      // Lines first (on top), then the fills beneath them.
      for (const h of halves) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(0, h.clip[0], bounding.width, h.clip[1] - h.clip[0]);
        ctx.clip();
        ctx.strokeStyle = h.color;
        ctx.lineWidth = 2;
        tracePath(ctx, closes);
        ctx.stroke();
        ctx.restore();
      }
      for (const h of halves) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(0, h.clip[0], bounding.width, h.clip[1] - h.clip[0]);
        ctx.clip();
        const g = ctx.createLinearGradient(0, h.edge, 0, baseY);
        g.addColorStop(0, withAlpha(h.color, 0.28));
        g.addColorStop(1, withAlpha(h.color, 0.05));
        ctx.fillStyle = g;
        tracePath(ctx, closes);
        ctx.lineTo(closes.at(-1)!.x, baseY);
        ctx.lineTo(closes[0]!.x, baseY);
        ctx.closePath();
        ctx.fill();
        ctx.restore();
      }
      // The base level itself, dotted as in TradingView.
      ctx.strokeStyle = BASE_LEVEL_COLOR;
      ctx.lineWidth = 1;
      ctx.setLineDash([1, 3]);
      ctx.beginPath();
      ctx.moveTo(0, Math.round(baseY) + 0.5);
      ctx.lineTo(bounding.width, Math.round(baseY) + 0.5);
      ctx.stroke();
      break;
    }
  }
  return true;
}
