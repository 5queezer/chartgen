// Drawing persistence — pure serialize / validate helpers plus a thin
// localStorage layer keyed per symbol (`chartgen.drawings.<SYMBOL>`).
//
// Points are stored as (timestamp, value) only, never as dataIndex:
// the chart pages bars in and out, so an index is meaningless across
// loads, while a timestamp maps onto any timeframe.

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
] as const;

export type DrawingName = (typeof DRAWING_NAMES)[number];

export interface DrawingPoint {
  timestamp: number;
  value: number;
}

export interface DrawingRecord {
  name: DrawingName;
  points: DrawingPoint[];
  styles?: Record<string, unknown>;
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

/** Validate one stored entry; null if malformed. */
export function parseRecord(x: unknown): DrawingRecord | null {
  if (!isPlainObject(x) || !isDrawingName(x.name)) return null;
  if (!Array.isArray(x.points) || x.points.length === 0) return null;
  if (x.points.length > MAX_POINTS) return null;
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

function pointTimestamp(
  p: { timestamp?: number; dataIndex?: number },
  axis: TimeAxis | null,
): number | undefined {
  if (p.timestamp !== undefined) return p.timestamp;
  if (axis === null || p.dataIndex === undefined) return undefined;
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
