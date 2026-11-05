// Drawing persistence — pure serialize / validate helpers plus a thin
// localStorage layer keyed per symbol (`chartgen.drawings.<SYMBOL>`).
//
// Points are stored as (timestamp, value) only, never as dataIndex:
// the chart pages bars in and out, so an index is meaningless across
// loads, while a timestamp maps onto any timeframe.

import { isColor } from "../lib/color";

export const DRAWING_NAMES = [
  "segment",
  "rayLine",
  "straightLine",
  "horizontalStraightLine",
  "horizontalRayLine",
  "verticalStraightLine",
  "parallelStraightLine",
  "fibonacciLine",
  "cgRect",
  "cgText",
  "simpleTag",
  "cgFibExt",
  "cgPitchfork",
  "cgPriceRange",
  "cgDateRange",
  "cgDatePriceRange",
  "cgLong",
  "cgShort",
] as const;

export type DrawingName = (typeof DRAWING_NAMES)[number];

/** Exact point count of the multi-point tools; stored records with another count are dropped. */
const EXACT_POINTS: Partial<Record<DrawingName, number>> = {
  cgFibExt: 3,
  cgPitchfork: 3,
  cgPriceRange: 2,
  cgDateRange: 2,
  cgDatePriceRange: 2,
  cgLong: 3,
  cgShort: 3,
};

/** Drawings with a translucent fill the user can recolor. */
export const FILLED: readonly DrawingName[] = [
  "cgRect",
  "cgPriceRange",
  "cgDateRange",
  "cgDatePriceRange",
];

export interface DrawingPoint {
  timestamp: number;
  value: number;
}

export const DASH_STYLES = ["solid", "dashed", "dotted"] as const;
export type DashStyle = (typeof DASH_STYLES)[number];

export interface DrawingStyle {
  color?: string;
  /** Line width in px, 1-4. */
  width?: number;
  dash?: DashStyle;
  /** Fill of a rectangle (may carry alpha). */
  fill?: string;
  /** Text size in px. */
  fontSize?: number;
}

export const MAX_WIDTH = 4;
export const FONT_SIZES = [10, 12, 14, 18, 24] as const;

export interface DrawingRecord {
  name: DrawingName;
  points: DrawingPoint[];
  styles?: Record<string, unknown>;
  /** User-chosen look; absent fields follow the chart's defaults and theme. */
  style?: DrawingStyle;
  extendData?: string;
  lock: boolean;
  visible: boolean;
}

const PREFIX = "chartgen.drawings.";
const INDEX_KEY = PREFIX + "index";
const MAX_POINTS = 3;
/** Applied on load only; arming a tool refuses to go past it. */
export const MAX_DRAWINGS = 500;
const MAX_TEXT = 200;
/** Serialized size cap per symbol. */
export const MAX_BYTES = 200_000;
/** Symbols kept in storage; the least recently used are evicted. */
export const MAX_SYMBOLS = 100;

export function storageKey(symbol: string): string {
  return PREFIX + symbol;
}

function isDrawingName(x: unknown): x is DrawingName {
  return (DRAWING_NAMES as readonly unknown[]).includes(x);
}

function isPlainObject(x: unknown): x is Record<string, unknown> {
  return typeof x === "object" && x !== null && !Array.isArray(x);
}

function parsePoint(x: unknown): DrawingPoint | null {
  if (!isPlainObject(x)) return null;
  const { timestamp, value } = x;
  if (
    typeof timestamp !== "number" ||
    typeof value !== "number" ||
    !Number.isFinite(timestamp) ||
    !Number.isFinite(value)
  ) {
    return null;
  }
  return { timestamp, value };
}

const COLOR = /^(#[0-9a-f]{3,8}|rgba?\([\d\s.,%]+\))$/i;
const POLYGON_STYLES = ["fill", "stroke", "stroke_fill"];

/**
 * Keep only the style keys chartgen sets (the rectangle's polygon look);
 * anything else found in storage is ignored.
 */
export function sanitizeStyles(
  x: unknown,
): Record<string, unknown> | undefined {
  if (!isPlainObject(x) || !isPlainObject(x.polygon)) return undefined;
  const { style, color, borderColor, borderSize } = x.polygon;
  const polygon: Record<string, unknown> = {};
  if (typeof style === "string" && POLYGON_STYLES.includes(style)) {
    polygon.style = style;
  }
  for (const [k, v] of [
    ["color", color],
    ["borderColor", borderColor],
  ] as const) {
    if (typeof v === "string" && v.length <= 64 && COLOR.test(v)) {
      polygon[k] = v;
    }
  }
  if (typeof borderSize === "number" && borderSize >= 0 && borderSize <= 20) {
    polygon.borderSize = borderSize;
  }
  return Object.keys(polygon).length > 0 ? { polygon } : undefined;
}

/** Keep only valid style fields; undefined when none is left. */
export function sanitizeStyle(x: unknown): DrawingStyle | undefined {
  if (!isPlainObject(x)) return undefined;
  const out: DrawingStyle = {};
  for (const k of ["color", "fill"] as const) {
    const v = x[k];
    if (isColor(v)) out[k] = v;
  }
  if (
    typeof x.width === "number" &&
    Number.isInteger(x.width) &&
    x.width >= 1 &&
    x.width <= MAX_WIDTH
  ) {
    out.width = x.width;
  }
  if (DASH_STYLES.includes(x.dash as DashStyle)) out.dash = x.dash as DashStyle;
  if (typeof x.fontSize === "number" && (FONT_SIZES as readonly number[]).includes(x.fontSize)) {
    out.fontSize = x.fontSize;
  }
  return Object.keys(out).length > 0 ? out : undefined;
}

/**
 * klinecharts overlay styles for a drawing's look. `selected` thickens
 * strokes by 2 px (the selection highlight). Unset fields are omitted so
 * the chart's themed defaults apply.
 */
export function overlayStyles(
  name: DrawingName,
  style: DrawingStyle | undefined,
  selected: boolean,
): Record<string, Record<string, unknown>> {
  const s = style ?? {};
  const size = (s.width ?? 1) + (selected ? 2 : 0);
  const line: Record<string, unknown> = { size };
  const polygon: Record<string, unknown> = { borderSize: size };
  const text: Record<string, unknown> = {};
  if (s.dash !== undefined) {
    const dashedValue = s.dash === "dotted" ? [1, 3] : [6, 4];
    const kind = s.dash === "solid" ? "solid" : "dashed";
    line.style = kind;
    line.dashedValue = dashedValue;
    polygon.borderStyle = kind;
    polygon.borderDashedValue = dashedValue;
  }
  if (s.color !== undefined) {
    line.color = s.color;
    polygon.borderColor = s.color;
    if (name === "cgText" || name === "fibonacciLine" || name === "cgFibExt") text.color = s.color;
    if (name === "simpleTag") {
      text.backgroundColor = s.color;
      text.borderColor = s.color;
    }
  }
  if (s.fill !== undefined) polygon.color = s.fill;
  if (s.fontSize !== undefined) text.size = s.fontSize;
  return { line, polygon, text };
}

/** Validate one stored entry; null if malformed. */
export function parseRecord(x: unknown): DrawingRecord | null {
  if (!isPlainObject(x) || !isDrawingName(x.name)) return null;
  if (!Array.isArray(x.points) || x.points.length === 0) return null;
  if (x.points.length > MAX_POINTS) return null;
  const exact = EXACT_POINTS[x.name];
  if (exact !== undefined && x.points.length !== exact) return null;
  const points = x.points.map(parsePoint);
  if (points.some((p) => p === null)) return null;
  const rec: DrawingRecord = {
    name: x.name,
    points: points as DrawingPoint[],
    lock: x.lock === true,
    visible: x.visible !== false,
  };
  const styles = sanitizeStyles(x.styles);
  if (styles) rec.styles = styles;
  const style = sanitizeStyle(x.style);
  if (style) rec.style = style;
  if (typeof x.extendData === "string" && x.extendData.length <= MAX_TEXT) {
    rec.extendData = x.extendData;
  }
  return rec;
}

/** Validate a whole stored list, dropping malformed entries (capped at MAX_DRAWINGS). */
export function parseRecords(raw: unknown): DrawingRecord[] {
  if (!Array.isArray(raw)) return [];
  return raw
    .slice(0, MAX_DRAWINGS)
    .map(parseRecord)
    .filter((r): r is DrawingRecord => r !== null);
}

/**
 * Bar timing of the loaded data. Points outside the loaded bars carry no
 * timestamp in klinecharts, so they are extrapolated from the last (or
 * first) bar at one bar interval per index step.
 */
export interface TimeAxis {
  firstTs: number;
  lastTs: number;
  count: number;
  /** The loaded bar timestamps: real times for indices inside the data (gaps included). */
  timestamps: readonly number[];
  /** Most common recent bar spacing in ms (robust against weekend gaps). */
  intervalMs: number;
}

/** Derive a TimeAxis from ascending bar timestamps; null if unusable. */
export function timeAxisOf(timestamps: readonly number[]): TimeAxis | null {
  const n = timestamps.length;
  if (n < 2) return null;
  const counts = new Map<number, number>();
  for (let i = Math.max(1, n - 100); i < n; i++) {
    const d = (timestamps[i] as number) - (timestamps[i - 1] as number);
    if (d > 0) counts.set(d, (counts.get(d) ?? 0) + 1);
  }
  let intervalMs = 0;
  let best = 0;
  for (const [d, c] of counts) {
    if (c > best) {
      best = c;
      intervalMs = d;
    }
  }
  if (intervalMs === 0) return null;
  return {
    firstTs: timestamps[0] as number,
    lastTs: timestamps[n - 1] as number,
    count: n,
    timestamps,
    intervalMs,
  };
}

/** A point as klinecharts accepts it: by timestamp, or by index beyond the last bar. */
export type ChartPoint =
  | { timestamp: number; value: number }
  | { dataIndex: number; value: number };

/**
 * Map a stored point onto the chart. Null means the point lies before the
 * first loaded bar (it would be clamped there), so the drawing must stay
 * hidden until older bars are loaded.
 */
export function toChartPoint(
  axis: TimeAxis,
  p: DrawingPoint,
): ChartPoint | null {
  if (p.timestamp < axis.firstTs) return null;
  if (p.timestamp <= axis.lastTs) return p;
  return {
    dataIndex:
      axis.count -
      1 +
      Math.round((p.timestamp - axis.lastTs) / axis.intervalMs),
    value: p.value,
  };
}

/** Minimal shape of a klinecharts overlay that we serialize. */
export interface OverlayLike {
  name: string;
  points: ReadonlyArray<{
    timestamp?: number;
    dataIndex?: number;
    value?: number;
  }>;
  styles?: unknown;
  extendData?: unknown;
  lock: boolean;
  visible: boolean;
}

export function pointTimestamp(
  p: { timestamp?: number; dataIndex?: number },
  axis: TimeAxis | null,
): number | undefined {
  if (p.timestamp !== undefined) return p.timestamp;
  if (axis === null || p.dataIndex === undefined) return undefined;
  const real = axis.timestamps[p.dataIndex];
  if (real !== undefined && Number.isInteger(p.dataIndex)) return real;
  if (p.dataIndex < 0) return axis.firstTs + p.dataIndex * axis.intervalMs;
  return axis.lastTs + (p.dataIndex - (axis.count - 1)) * axis.intervalMs;
}

/** Convert live overlays to records; anything that does not validate is dropped. */
export function serialize(
  overlays: readonly OverlayLike[],
  axis: TimeAxis | null,
): DrawingRecord[] {
  return parseRecords(
    overlays.map((o) => ({
      name: o.name,
      points: o.points.map((p) => ({
        timestamp: pointTimestamp(p, axis),
        value: p.value,
      })),
      styles: o.styles ?? undefined,
      extendData: o.extendData ?? undefined,
      lock: o.lock,
      visible: o.visible,
    })),
  );
}

function readIndex(): string[] {
  try {
    const raw = JSON.parse(localStorage.getItem(INDEX_KEY) ?? "[]") as unknown;
    return Array.isArray(raw)
      ? [...new Set(raw.filter((x): x is string => typeof x === "string"))]
      : [];
  } catch {
    return [];
  }
}

/** Move `symbol` to the front of the LRU index (or drop it); evict beyond MAX_SYMBOLS. */
function touch(symbol: string, keep: boolean): void {
  const rest = readIndex().filter((s) => s !== symbol);
  const index = keep ? [symbol, ...rest] : rest;
  for (const evicted of index.splice(MAX_SYMBOLS)) {
    localStorage.removeItem(storageKey(evicted));
  }
  localStorage.setItem(INDEX_KEY, JSON.stringify(index));
}

export function loadDrawings(symbol: string): DrawingRecord[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(storageKey(symbol));
    if (raw === null) return [];
    const records = parseRecords(JSON.parse(raw));
    try {
      touch(symbol, true);
    } catch {
      // LRU bookkeeping is best-effort.
    }
    return records;
  } catch {
    return [];
  }
}

/** Persist a symbol's drawings. Returns false (after warning) if they could not be saved. */
export function saveDrawings(
  symbol: string,
  records: readonly DrawingRecord[],
): boolean {
  if (typeof localStorage === "undefined") return true;
  try {
    if (records.length === 0) {
      localStorage.removeItem(storageKey(symbol));
      touch(symbol, false);
      return true;
    }
    const json = JSON.stringify(records);
    if (json.length > MAX_BYTES) {
      console.warn(`drawings for ${symbol} exceed ${MAX_BYTES} bytes`);
      return false;
    }
    localStorage.setItem(storageKey(symbol), json);
    touch(symbol, true);
    return true;
  } catch (err) {
    console.warn("failed to save drawings", err);
    return false;
  }
}
