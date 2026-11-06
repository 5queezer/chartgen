// Chart types klinecharts v9 has no candle type for (ADR-0030): Baseline,
// Columns, High-low, HLC area, Step line, Line with markers, and the Kagi and
// Point & Figure bars the server builds (ADR-0047). klinecharts keeps a
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
  | "hlc_area"
  | "step_line"
  | "line_markers"
  | "kagi"
  | "point_figure";

const SERIES_TYPES: readonly ChartType[] = [
  "baseline",
  "columns",
  "high_low",
  "hlc_area",
  "step_line",
  "line_markers",
  "kagi",
  "point_figure",
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
 * settings styles. The types that show highs and lows keep a candle so the
 * autoscale spans them; the others keep an area line, which autoscales on the
 * closes as TradingView does for close-based types.
 */
export function hiddenCandleStyle(t: SeriesType): object {
  if (t === "high_low" || t === "hlc_area" || t === "kagi" || t === "point_figure") {
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
  /** Point & Figure: the box its columns are built on (ADR-0047). */
  boxSize?: number;
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

type Hl = Pick<KLineData, "open" | "high" | "low" | "close">;

/** A Kagi line runs up from its open, or down: the server's bars say which by reach. */
export const isKagiUp = (b: Hl): boolean => b.high - b.open >= b.open - b.low;

/**
 * Thick or thin for each Kagi line: thick from the line that passes the last up
 * line's top until a line passes the last down line's bottom. A line takes one
 * weight along its whole length, where TradingView changes it at the crossing.
 */
export function kagiThick(bars: readonly Hl[]): boolean[] {
  const out: boolean[] = [];
  let thick = true;
  for (let i = 0; i < bars.length; i++) {
    const b = bars[i]!;
    const before = bars[i - 2];
    if (before) {
      if (isKagiUp(b) && b.high > before.high) thick = true;
      else if (!isKagiUp(b) && b.low < before.low) thick = false;
    } else {
      thick = isKagiUp(b);
    }
    out.push(thick);
  }
  return out;
}

/** The levels on the box's grid inside a column, lowest first. */
export function columnLevels(low: number, high: number, box: number): number[] {
  const from = Math.ceil(low / box - 1e-9);
  const to = Math.floor(high / box + 1e-9);
  const out: number[] = [];
  for (let k = from; k <= to; k++) out.push(k * box);
  return out;
}

/** Xs or Os: a column of Os opens at its top. A single box goes against the last column. */
export function isXColumn(bars: readonly Hl[], i: number): boolean {
  const b = bars[i]!;
  if (b.high === b.low) {
    const prev = bars[i - 1];
    return prev ? !isXColumn(bars, i - 1) : b.close >= b.open;
  }
  return b.open !== b.high;
}

/** A column of marks this close together, or this long, is drawn as one bar. */
const MIN_MARK_PX = 4;
const MAX_MARKS = 200;

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
  // Compare prices, not pixels: klinecharts rounds pixels to whole numbers,
  // so one price unit can map to the same pixel at high prices.
  const inverted = yAxis.convertFromPixel(0) < yAxis.convertFromPixel(bounding.height);
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
      // Columns grow from the bottom edge, or from the top on an inverted axis.
      const columnBase = inverted ? 0 : bounding.height;
      for (let i = start; i < end; i++) {
        const b = bars[i]!;
        const x = xAxis.convertToPixel(i);
        const r =
          ext.type === "columns"
            ? barRect(x, yAxis.convertToPixel(b.close), columnBase, barSpace.gapBar)
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
    case "hlc_area": {
      const line = (key: "high" | "low") => {
        const out: Point[] = [];
        for (let i = start; i < end; i++) out.push(point(i, bars[i]![key]));
        return out;
      };
      const highs = line("high");
      const lows = line("low");
      // Topmost first (see above): the close, the edges, then the fills beneath.
      ctx.strokeStyle = ext.line;
      ctx.lineWidth = 2;
      tracePath(ctx, closes);
      ctx.stroke();
      for (const [edge, color] of [
        [highs, ext.up],
        [lows, ext.down],
      ] as const) {
        ctx.strokeStyle = color;
        ctx.lineWidth = 1;
        tracePath(ctx, edge);
        ctx.stroke();
        ctx.fillStyle = withAlpha(color, 0.2);
        tracePath(ctx, [...edge, ...[...closes].reverse()]);
        ctx.closePath();
        ctx.fill();
      }
      break;
    }
    case "kagi": {
      const thick = kagiThick(bars);
      for (let i = start; i < end; i++) {
        const b = bars[i]!;
        const x = xAxis.convertToPixel(i);
        const y = yAxis.convertToPixel(isKagiUp(b) ? b.high : b.low);
        ctx.strokeStyle = thick[i] ? ext.up : ext.down;
        ctx.lineWidth = thick[i] ? 3 : 1;
        ctx.beginPath();
        ctx.moveTo(x, yAxis.convertToPixel(b.open));
        ctx.lineTo(x, y);
        if (i + 1 < bars.length) ctx.lineTo(xAxis.convertToPixel(i + 1), y);
        ctx.stroke();
      }
      break;
    }
    case "point_figure": {
      const box = ext.boxSize;
      if (!box) break;
      const cell = Math.abs(yAxis.convertToPixel(box * 2) - yAxis.convertToPixel(box));
      for (let i = start; i < end; i++) {
        const b = bars[i]!;
        const x = xAxis.convertToPixel(i);
        const isX = isXColumn(bars, i);
        const color = isX ? ext.up : ext.down;
        const levels = columnLevels(b.low, b.high, box);
        if (cell < MIN_MARK_PX || levels.length > MAX_MARKS) {
          const r = barRect(x, yAxis.convertToPixel(b.high), yAxis.convertToPixel(b.low), barSpace.gapBar);
          ctx.fillStyle = withAlpha(color, 0.5);
          ctx.fillRect(r.x, r.y, r.width, r.height);
          continue;
        }
        const half = Math.max(1.5, Math.min(barSpace.bar * 0.4, cell * 0.45));
        ctx.strokeStyle = color;
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        for (const level of levels) {
          const y = yAxis.convertToPixel(level);
          if (y < -half || y > bounding.height + half) continue;
          if (isX) {
            ctx.moveTo(x - half, y - half);
            ctx.lineTo(x + half, y + half);
            ctx.moveTo(x + half, y - half);
            ctx.lineTo(x - half, y + half);
          } else {
            ctx.moveTo(x + half, y);
            ctx.arc(x, y, half, 0, Math.PI * 2);
          }
        }
        ctx.stroke();
      }
      break;
    }
    case "baseline": {
      const visible = [];
      for (let i = visibleRange.from; i < visibleRange.to; i++) visible.push(bars[i]!.close);
      const base = baselineValue(visible);
      if (base === null) break;
      const baseY = yAxis.convertToPixel(base);
      // On an inverted axis higher prices are below the base line.
      const rev = inverted;
      const above = { clip: [0, baseY], edge: 0 } as const;
      const below = { clip: [baseY, bounding.height], edge: bounding.height } as const;
      const halves = rev
        ? ([{ color: ext.up, ...below }, { color: ext.down, ...above }] as const)
        : ([{ color: ext.up, ...above }, { color: ext.down, ...below }] as const);
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
