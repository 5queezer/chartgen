// Pure geometry for the series channels klinecharts has no figure type
// for: `fills` (shaded band between two series), `hbars` (volume-profile
// rows) and `divlines` (divergence segments). See ADR-0007 for the wire
// shape and ADR-0010 for how these are drawn.
//
// No DOM, no klinecharts import: every function takes coordinate-mapping
// callbacks and returns pixel-space primitives, so Chart.tsx's draw
// callbacks only stroke/fill what comes back and the maths is unit-tested.
//
// Colors: the wire drops server-side colors (ADR-0007, "the client themes
// per indicator"), so the theme helpers below mirror the Rust indicator
// palettes. Swap them for payload fields once the wire carries colors.

import type { SeriesPayload } from "../types/responses";

export type SeriesIndicator = SeriesPayload["indicators"][string];
export type SeriesFill = NonNullable<SeriesIndicator["fills"]>[number];
export type SeriesHBar = NonNullable<SeriesIndicator["hbars"]>[number];
export type SeriesDivLine = NonNullable<SeriesIndicator["divlines"]>[number];

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

/** Data index → pixel x, value → pixel y (klinecharts xAxis/yAxis). */
export interface Mapper {
  x: (index: number) => number;
  y: (value: number) => number;
}

/** Visible data-index window, `to` exclusive (klinecharts VisibleRange). */
export interface IndexRange {
  from: number;
  to: number;
}

export interface FillPolygon {
  /** Closed outline: the y1 edge left→right, then the y2 edge right→left. */
  points: Point[];
  /** True where y1 ≥ y2 (e.g. Senkou A above Senkou B → bullish cloud). */
  up: boolean;
}

export interface HBarRect extends Rect {
  /** Index into the payload's `hbars` array (for per-bar styling). */
  index: number;
}

export type DivergenceKind = "bull" | "bear" | "flat";

export interface DivSegment {
  from: Point;
  to: Point;
  dashed: boolean;
  kind: DivergenceKind;
}

const isNum = (v: number | null | undefined): v is number =>
  typeof v === "number" && Number.isFinite(v);

const sign = (d: number): -1 | 0 | 1 => (d > 0 ? 1 : d < 0 ? -1 : 0);

/**
 * Polygons between `y1` and `y2`, clipped to the visible range plus one
 * bar on each side (so the band runs to the pane edge; the canvas clips
 * the overhang). A polygon breaks at any bar where either series is null
 * and at every point where the two series cross, so each polygon has a
 * single `up` orientation and can be colored bull/bear.
 */
export function fillPolygons(
  y1: Array<number | null>,
  y2: Array<number | null>,
  range: IndexRange,
  map: Mapper,
): FillPolygon[] {
  const n = Math.min(y1.length, y2.length);
  const start = Math.max(0, Math.floor(range.from) - 1);
  const end = Math.min(n - 1, Math.ceil(range.to));
  const out: FillPolygon[] = [];
  let edgeA: Point[] = [];
  let edgeB: Point[] = [];
  let orient: -1 | 0 | 1 = 0;

  const flush = () => {
    if (edgeA.length >= 2) {
      out.push({ points: [...edgeA, ...edgeB.reverse()], up: orient >= 0 });
    }
    edgeA = [];
    edgeB = [];
  };

  for (let i = start; i <= end; i++) {
    const a = y1[i];
    const b = y2[i];
    if (!isNum(a) || !isNum(b)) {
      flush();
      orient = 0;
      continue;
    }
    const s = sign(a - b);
    if (s !== 0 && orient !== 0 && s !== orient) {
      // Crossing between i-1 and i. edgeA is non-empty and the previous
      // bar is finite, because a null bar always flushes the run.
      const pa = y1[i - 1]!;
      const pd = pa - y2[i - 1]!;
      const t = pd / (pd - (a - b));
      const x0 = map.x(i - 1);
      const cross = { x: x0 + t * (map.x(i) - x0), y: map.y(pa + t * (a - pa)) };
      edgeA.push(cross);
      edgeB.push(cross);
      flush();
      edgeA.push(cross);
      edgeB.push(cross);
    }
    if (s !== 0) orient = s;
    const x = map.x(i);
    edgeA.push({ x, y: map.y(a) });
    edgeB.push({ x, y: map.y(b) });
  }
  flush();
  return out;
}

/**
 * Pixel rects for horizontal bars. `width`/`offset` are fractions of the
 * pane width measured from the anchored edge (`left` true → left edge,
 * false → right edge); `y`/`height` are a price-space center and extent.
 * Rects entirely above or below the pane are dropped.
 */
export function hbarRects(
  hbars: SeriesHBar[],
  paneWidth: number,
  paneHeight: number,
  mapY: (value: number) => number,
): HBarRect[] {
  const out: HBarRect[] = [];
  hbars.forEach((h, index) => {
    const yTop = mapY(h.y + h.height / 2);
    const yBot = mapY(h.y - h.height / 2);
    const y = Math.min(yTop, yBot);
    const height = Math.abs(yBot - yTop);
    if (y > paneHeight || y + height < 0) return;
    const width = h.width * paneWidth;
    const x = h.left
      ? h.offset * paneWidth
      : paneWidth - (h.offset + h.width) * paneWidth;
    out.push({ x, y, width, height, index });
  });
  return out;
}

/**
 * Bull/bear classification of a regular divergence from its slope. In an
 * oscillator pane a bearish divergence joins lower highs (slope down) and
 * a bullish one joins higher lows (slope up); in the price pane the
 * divergence line sits on price, so the slopes are reversed (bearish =
 * higher highs).
 */
export function divergenceKind(
  d: SeriesDivLine,
  isOverlay: boolean,
): DivergenceKind {
  const [ya, yb] = d.x1 <= d.x2 ? [d.y1, d.y2] : [d.y2, d.y1];
  const rising = sign(yb - ya);
  if (rising === 0) return "flat";
  return (rising > 0) !== isOverlay ? "bull" : "bear";
}

/** Pixel segments for divergence lines overlapping the visible range. */
export function divlineSegments(
  divlines: SeriesDivLine[],
  range: IndexRange,
  map: Mapper,
  isOverlay: boolean,
): DivSegment[] {
  const out: DivSegment[] = [];
  for (const d of divlines) {
    if (Math.max(d.x1, d.x2) < range.from - 1) continue;
    if (Math.min(d.x1, d.x2) > range.to) continue;
    out.push({
      from: { x: map.x(d.x1), y: map.y(d.y1) },
      to: { x: map.x(d.x2), y: map.y(d.y2) },
      dashed: d.dashed,
      kind: divergenceKind(d, isOverlay),
    });
  }
  return out;
}

// ---------------------------------------------------------------------------
// Theme — client-side mirror of the Rust palettes (ADR-0006 / ADR-0007).
// ---------------------------------------------------------------------------

export interface FillTheme {
  up: string;
  down: string;
}

const BAND_FILL = "rgba(33, 150, 243, 0.10)"; // 0x2196F3 @ 0.1
const single = (c: string): FillTheme => ({ up: c, down: c });

// Cipher B / WaveTrend emit their fills in a fixed order: WT1/WT2 band,
// VWAP-to-zero area, then (Cipher B only) Stoch K/D band.
const WAVE_FILLS: FillTheme[] = [
  single("rgba(73, 148, 236, 0.25)"),
  single("rgba(255, 255, 255, 0.15)"),
  single("rgba(33, 186, 243, 0.05)"),
];

/** Fill colors for the `index`-th fill of indicator `name`. */
export function fillTheme(name: string, index: number): FillTheme {
  switch (name) {
    case "ichimoku":
      // TradingView cloud: green while Senkou A ≥ Senkou B, red below.
      return { up: "rgba(38, 166, 154, 0.20)", down: "rgba(239, 83, 80, 0.20)" };
    case "cipher_b":
    case "wavetrend":
      return WAVE_FILLS[index] ?? single(BAND_FILL);
    case "long_short":
    case "ls_ratio":
      return single("rgba(38, 166, 154, 0.10)");
    default:
      return single(BAND_FILL);
  }
}

export type HBarTier = "poc" | "va" | "out";

const HBAR_COLORS: Record<HBarTier, string> = {
  poc: "rgba(255, 215, 0, 0.70)",
  va: "rgba(41, 98, 255, 0.40)",
  out: "rgba(41, 98, 255, 0.20)",
};

// TPO is purple (0x9C27B0) server-side so it reads apart from VPVR.
const TPO_COLORS: Record<HBarTier, string> = {
  poc: "rgba(255, 215, 0, 0.70)",
  va: "rgba(156, 39, 176, 0.42)",
  out: "rgba(156, 39, 176, 0.19)",
};

/** Per-tier hbar colors for indicator `name`. */
export function hbarTheme(name: string): Record<HBarTier, string> {
  return name === "tpo" || name === "market_profile" ? TPO_COLORS : HBAR_COLORS;
}

/**
 * Volume-profile tiers. The widest bar is the point of control; when the
 * indicator also emits POC/VAH/VAL hlines (in that order, as
 * volume_profile and tpo do), bars centred inside [VAL, VAH] are the
 * value area.
 */
export function hbarTiers(
  hbars: SeriesHBar[],
  hlines: Array<{ y: number }>,
): HBarTier[] {
  let poc = -1;
  hbars.forEach((h, i) => {
    if (poc < 0 || h.width > hbars[poc]!.width) poc = i;
  });
  const va =
    hlines.length >= 3
      ? [Math.min(hlines[1]!.y, hlines[2]!.y), Math.max(hlines[1]!.y, hlines[2]!.y)]
      : null;
  return hbars.map((h, i) => {
    if (i === poc) return "poc";
    if (va && h.y >= va[0]! && h.y <= va[1]!) return "va";
    return "out";
  });
}

export const DIVLINE_COLORS: Record<DivergenceKind, string> = {
  bull: "rgba(0, 230, 118, 0.80)",
  bear: "rgba(255, 0, 0, 0.80)",
  flat: "rgba(120, 123, 134, 0.80)",
};
