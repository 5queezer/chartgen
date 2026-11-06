// Price-based chart types (ADR-0047): what they ask the server for, and how those
// inputs are stored, shared and named. The bars are built on the server
// (src/price_bars.rs). Pure, so it can be unit tested.

import type { ChartType } from "../stores/layout";

export type PriceBarType = "renko" | "kagi" | "point_figure" | "line_break" | "range";

export const PRICE_BAR_TYPES: readonly ChartType[] = [
  "renko",
  "kagi",
  "point_figure",
  "line_break",
  "range",
];

export function isPriceBarType(t: ChartType): t is PriceBarType {
  return PRICE_BAR_TYPES.includes(t);
}

export type BoxMode = "atr" | "fixed" | "percent";

/** The inputs of the price-based types of one chart. One set, kept across them. */
export interface ChartParams {
  box: BoxMode;
  atrLength: number;
  percent: number;
  /** The fixed size; only read when `box` is "fixed". */
  boxSize?: number;
  reversal: number;
  lines: number;
}

export const DEFAULT_CHART_PARAMS: ChartParams = {
  box: "atr",
  atrLength: 14,
  percent: 1,
  reversal: 3,
  lines: 3,
};

export const MAX_ATR_LENGTH = 500;
export const MAX_REVERSAL = 20;
export const MAX_LINES = 20;

const whole = (x: unknown, max: number): number | undefined =>
  typeof x === "number" && Number.isInteger(x) && x >= 1 && x <= max ? x : undefined;

const validPercent = (x: unknown): number | undefined =>
  typeof x === "number" && x > 0 && x <= 100 ? x : undefined;

const validSize = (x: unknown): number | undefined =>
  typeof x === "number" && Number.isFinite(x) && x > 0 ? x : undefined;

/** Stored or imported inputs: each one that is not valid falls back to its default. */
export function sanitizeChartParams(x: unknown): ChartParams {
  const r = (typeof x === "object" && x !== null ? x : {}) as Record<string, unknown>;
  const boxSize = validSize(r.boxSize);
  const box = r.box === "percent" || (r.box === "fixed" && boxSize !== undefined) ? r.box : "atr";
  return {
    box,
    atrLength: whole(r.atrLength, MAX_ATR_LENGTH) ?? DEFAULT_CHART_PARAMS.atrLength,
    percent: validPercent(r.percent) ?? DEFAULT_CHART_PARAMS.percent,
    ...(boxSize !== undefined ? { boxSize } : {}),
    reversal: whole(r.reversal, MAX_REVERSAL) ?? DEFAULT_CHART_PARAMS.reversal,
    lines: whole(r.lines, MAX_LINES) ?? DEFAULT_CHART_PARAMS.lines,
  };
}

export const isDefaultChartParams = (p: ChartParams): boolean =>
  p.box === DEFAULT_CHART_PARAMS.box &&
  p.atrLength === DEFAULT_CHART_PARAMS.atrLength &&
  p.percent === DEFAULT_CHART_PARAMS.percent &&
  p.boxSize === undefined &&
  p.reversal === DEFAULT_CHART_PARAMS.reversal &&
  p.lines === DEFAULT_CHART_PARAMS.lines;

/** `chart_params` on the wire: what this type reads. */
export interface ChartParamsWire {
  box?: BoxMode;
  atr_length?: number;
  percent?: number;
  box_size?: number;
  reversal?: number;
  lines?: number;
}

/** The request's `chart_params`; undefined for a type that is not price-based. */
export function chartParamsWire(type: ChartType, p: ChartParams): ChartParamsWire | undefined {
  if (!isPriceBarType(type)) return undefined;
  if (type === "line_break") return { lines: p.lines };
  const box: ChartParamsWire =
    p.box === "fixed" && p.boxSize !== undefined
      ? { box: "fixed", box_size: p.boxSize }
      : p.box === "percent"
        ? { box: "percent", percent: p.percent }
        : { box: "atr", atr_length: p.atrLength };
  return type === "point_figure" ? { ...box, reversal: p.reversal } : box;
}

/** A share link's `tp`: only what differs from the defaults. Empty for all of them. */
export function encodeChartParams(p: ChartParams): string {
  const d = DEFAULT_CHART_PARAMS;
  const pairs: string[] = [];
  if (p.box !== d.box) pairs.push(`box:${p.box}`);
  if (p.boxSize !== undefined) pairs.push(`size:${p.boxSize}`);
  if (p.atrLength !== d.atrLength) pairs.push(`len:${p.atrLength}`);
  if (p.percent !== d.percent) pairs.push(`pct:${p.percent}`);
  if (p.reversal !== d.reversal) pairs.push(`rev:${p.reversal}`);
  if (p.lines !== d.lines) pairs.push(`lines:${p.lines}`);
  return pairs.join(";");
}

/** A share link's `tp`; malformed parts are ignored and take their default. */
export function decodeChartParams(raw: string): ChartParams {
  const out: Record<string, unknown> = {};
  const keys: Record<string, string> = {
    size: "boxSize",
    len: "atrLength",
    pct: "percent",
    rev: "reversal",
    lines: "lines",
  };
  for (const part of raw.split(";")) {
    const [key, value] = part.split(":");
    if (key === "box") out.box = value;
    else if (key && value !== undefined && Object.hasOwn(keys, key) && value.trim() !== "") {
      out[keys[key]!] = Number(value);
    }
  }
  return sanitizeChartParams(out);
}

const SHORT_NAMES: Record<PriceBarType, string> = {
  renko: "Renko",
  kagi: "Kagi",
  point_figure: "P&F",
  line_break: "Line Break",
  range: "Range",
};

/** What the legend says after the interval: the type and the size in use. */
export function priceBarLabel(
  style: string | undefined,
  params: ChartParamsWire | undefined,
): string | null {
  if (!style || !params || !(style in SHORT_NAMES)) return null;
  const name = SHORT_NAMES[style as PriceBarType];
  if (style === "line_break") return `${name} ${params.lines ?? DEFAULT_CHART_PARAMS.lines}`;
  const size = params.box_size ?? "";
  return style === "point_figure"
    ? `${name} ${size} x ${params.reversal ?? DEFAULT_CHART_PARAMS.reversal}`
    : `${name} ${size}`;
}

const ALERT_NAMES: Record<PriceBarType, string> = {
  renko: "renko",
  kagi: "kagi",
  point_figure: "pnf",
  line_break: "linebreak",
  range: "range",
};

/**
 * What an indicator or sloped drawing alert is tied to (ADR-0034). Its values
 * and bar indices mean something else on these bars than on the interval's candles,
 * so a chart of them is a chart of its own: `1h renko`. At most 16 characters, as
 * stored alerts require.
 */
export function alertInterval(series: { interval: string | null; chart_style?: string }): string {
  const interval = series.interval ?? "";
  const style = series.chart_style as PriceBarType | undefined;
  return style && style in ALERT_NAMES ? `${interval} ${ALERT_NAMES[style]}` : interval;
}
