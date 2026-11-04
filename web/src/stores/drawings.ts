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
  "simpleAnnotation",
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
const MAX_POINTS = 3;
const MAX_DRAWINGS = 500;
const MAX_TEXT = 200;

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
  if (isPlainObject(x.styles)) rec.styles = x.styles;
  if (typeof x.extendData === "string" && x.extendData.length <= MAX_TEXT) {
    rec.extendData = x.extendData;
  }
  return rec;
}

/** Validate a whole stored list, dropping malformed entries. */
export function parseRecords(raw: unknown): DrawingRecord[] {
  if (!Array.isArray(raw)) return [];
  return raw
    .slice(0, MAX_DRAWINGS)
    .map(parseRecord)
    .filter((r): r is DrawingRecord => r !== null);
}

/** Minimal shape of a klinecharts overlay that we serialize. */
export interface OverlayLike {
  name: string;
  points: ReadonlyArray<{ timestamp?: number; value?: number }>;
  styles?: unknown;
  extendData?: unknown;
  lock: boolean;
  visible: boolean;
}

/** Convert live overlays to records; anything that does not validate is dropped. */
export function serialize(overlays: readonly OverlayLike[]): DrawingRecord[] {
  return parseRecords(
    overlays.map((o) => ({
      name: o.name,
      points: o.points.map((p) => ({
        timestamp: p.timestamp,
        value: p.value,
      })),
      styles: o.styles ?? undefined,
      extendData: o.extendData ?? undefined,
      lock: o.lock,
      visible: o.visible,
    })),
  );
}

export function loadDrawings(symbol: string): DrawingRecord[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(storageKey(symbol));
    return raw === null ? [] : parseRecords(JSON.parse(raw));
  } catch {
    return [];
  }
}

export function saveDrawings(
  symbol: string,
  records: readonly DrawingRecord[],
): void {
  if (typeof localStorage === "undefined") return;
  try {
    if (records.length === 0) localStorage.removeItem(storageKey(symbol));
    else localStorage.setItem(storageKey(symbol), JSON.stringify(records));
  } catch {
    // Quota exceeded or private-mode storage: best-effort only.
  }
}
