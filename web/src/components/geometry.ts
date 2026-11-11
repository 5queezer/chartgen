// Pure geometry for the series channels klinecharts has no figure type
// for: `fills` (shaded band between two series), `hbars` (volume-profile
// rows) and `divlines` (divergence segments). See ADR-0007 for the wire
// shape and ADR-0010 for how these are drawn.
//
// No DOM, no klinecharts import: every function takes coordinate-mapping
// callbacks and returns pixel-space primitives, so Chart.tsx's draw
// callbacks only stroke/fill what comes back and the maths is unit-tested.
//
// Colors: the wire carries the server renderer's color per fill, hbar,
// divline and hline (ADR-0012) and those win. The theme helpers below
// mirror the Rust indicator palettes and only serve as the fallback for
// older servers that omit `color`.

import type { SeriesPayload } from "../types/responses";
import { keyName } from "../lib/instances";
import { PALETTES } from "../lib/theme";
import { theme } from "../stores/layout";

export type SeriesIndicator = SeriesPayload["indicators"][string];
export type SeriesFill = NonNullable<SeriesIndicator["fills"]>[number];
export type SeriesHBar = NonNullable<SeriesIndicator["hbars"]>[number];
export type SeriesDivLine = NonNullable<SeriesIndicator["divlines"]>[number];
export type SeriesLabel = NonNullable<SeriesIndicator["labels"]>[number];
export type SeriesPattern = NonNullable<SeriesIndicator["patterns"]>[number];

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

/**
 * Visible data-index window: klinecharts' `VisibleRange.from`/`to`, which
 * klinecharts already clamps to [0, dataList.length] (`to` exclusive), so
 * it never describes the empty space right of the last bar. Indices are
 * data indices (see `payloadToDataIndex`), not payload indices.
 */
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
  /** Wire color when valid, else the bull/bear/flat fallback. */
  color: string;
}

export interface HLineStroke {
  y: number;
  color: string;
  /** Position in the payload's `hlines`. */
  index: number;
}

const isNum = (v: number | null | undefined): v is number =>
  typeof v === "number" && Number.isFinite(v);

const sign = (d: number): -1 | 0 | 1 => (d > 0 ? 1 : d < 0 ? -1 : 0);

/**
 * Payload bar index → klinecharts data index. `seriesToKLines` drops bars
 * with `t === null`, so later bars shift left; dropped bars map to -1.
 */
export function payloadToDataIndex(
  bars: Array<{ t: number | null }>,
): number[] {
  let next = 0;
  return bars.map((b) => (b.t === null ? -1 : next++));
}

/**
 * Re-index an indicator's per-bar geometry from payload space into data
 * space with `payloadToDataIndex`'s remap: fill arrays keep only the
 * surviving bars; divline endpoints are remapped, and segments touching
 * a dropped (or out-of-range) bar are discarded. hbars are price-space
 * and pass through, except their `color` is resolved here once (wire
 * color, else the tier theme for indicator `name`) instead of per frame.
 *
 * A null `remap` means the payload already is in data space (every bar
 * has a timestamp, as in merged live data). Contract of that path: every
 * field but `hbars` is shared with `ind` (same arrays, not copies), so
 * in-place updates to `ind` show through and it costs nothing per bar;
 * and nothing is filtered, in particular divlines whose endpoints lie
 * outside the bars are kept. The caller guarantees the indices are valid
 * (the live merge only emits divlines on bars it holds).
 */
export function toDataSpace(
  ind: SeriesIndicator,
  remap: number[] | null,
  name = "",
): SeriesIndicator {
  const colors = ind.hbars ? hbarColors(ind.hbars, ind.hlines ?? [], name) : [];
  const hbars = ind.hbars?.map((h, i) => ({ ...h, color: colors[i] }));
  if (remap === null) return { ...ind, hbars };
  const kept: number[] = [];
  remap.forEach((d, p) => {
    if (d >= 0) kept.push(p);
  });
  const lines = ind.lines?.map((l) => ({ ...l, values: kept.map(p => l.values[p] ?? null) }));
  const fills = ind.fills?.map((f) => ({
    y1: kept.map((p) => f.y1[p] ?? null),
    y2: kept.map((p) => f.y2[p] ?? null),
    ...(f.color !== undefined && { color: f.color }),
    ...(f.color_below !== undefined && { color_below: f.color_below }),
  }));
  const divlines = ind.divlines?.flatMap((d) => {
    const x1 = remap[d.x1] ?? -1;
    const x2 = remap[d.x2] ?? -1;
    return x1 < 0 || x2 < 0 ? [] : [{ ...d, x1, x2 }];
  });
  const labels = ind.labels?.flatMap((l) => {
    const x = remap[l.x] ?? -1;
    return x < 0 ? [] : [{ ...l, x }];
  });
  const patterns = ind.patterns?.map((p) => ({
    ...p,
    strokes: p.strokes.flatMap((s) => {
      const x1 = remap[s.x1] ?? -1;
      const x2 = remap[s.x2] ?? -1;
      return x1 < 0 || x2 < 0 ? [] : [{ ...s, x1, x2 }];
    }),
    label: { ...p.label, x: remap[p.label.x] ?? p.label.x },
  }));
  // A boundary on a dropped bar belongs before the next retained bar.
  const line_breaks = ind.line_breaks?.flatMap((at) => {
    for (let i = at; i < remap.length; i++) {
      if (remap[i]! >= 0) return [remap[i]!];
    }
    return [];
  });
  return { ...ind, lines, fills, divlines, labels, patterns, hbars,
    ...(line_breaks && { line_breaks: [...new Set(line_breaks)] }),
  };
}

export interface ValueExtent {
  lo: number | null;
  hi: number | null;
}

/**
 * Accepts only the "rgba(r,g,b,a)" / "rgb(r,g,b)" strings the server
 * emits. A canvas silently keeps its previous style when handed an
 * unparsable color, which would repaint with a stale one, so anything
 * else is treated as absent and the caller falls back to the theme.
 */
const CSS_RGB =
  /^rgba?\(\s*\d{1,3}\s*,\s*\d{1,3}\s*,\s*\d{1,3}\s*(,\s*(\d+\.?\d*|\.\d+)\s*)?\)$/;
export function wireColor(c: string | undefined): string | undefined {
  return c !== undefined && CSS_RGB.test(c) ? c : undefined;
}

/**
 * Per-bar values the y-axis has to include beyond the line/bar figures:
 * the min/max of every fill edge at that bar and, for oscillator panes,
 * the payload's `y_range`. That keeps fixed ranges such as RSI 0–100 or
 * Kalman ±2.5 (whose OB/OS zones live at the edges) and gives
 * geometry-only panes a real axis. klinecharts auto-scales over figure
 * values only, so Chart.tsx feeds these through two invisible figures.
 * Overlays ignore `y_range`, since pinning the price axis would be wrong.
 * hlines are deliberately not included, in any pane: the PNG renderer's
 * auto-range ignores them too, and a level far from the data (e.g. CVD's
 * zero line) would squash the series. Off-range hlines are just clipped.
 */
export function valueExtents(
  ind: SeriesIndicator,
  length: number,
): ValueExtent[] {
  const [rlo, rhi] =
    !ind.is_overlay && ind.y_range ? ind.y_range : [null, null];
  const out: ValueExtent[] = [];
  for (let i = 0; i < length; i++) {
    let lo = isNum(rlo) ? rlo : null;
    let hi = isNum(rhi) ? rhi : null;
    for (const f of ind.fills ?? []) {
      for (const v of [f.y1[i], f.y2[i]]) {
        if (!isNum(v)) continue;
        lo = lo === null ? v : Math.min(lo, v);
        hi = hi === null ? v : Math.max(hi, v);
      }
    }
    out.push({ lo, hi });
  }
  // A projection past the last bar (ADR-0033) scales with the last bar, so
  // the cloud ahead stays on the axis while the last bar is in view.
  const last = out[length - 1];
  for (const f of last ? (ind.future?.fills ?? []) : []) {
    for (const v of [...f.y1, ...f.y2]) {
      if (!isNum(v)) continue;
      last!.lo = last!.lo === null ? v : Math.min(last!.lo, v);
      last!.hi = last!.hi === null ? v : Math.max(last!.hi, v);
    }
  }
  return out;
}

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
  steps = false,
  breaks: number[] = [],
): FillPolygon[] {
  const n = Math.min(y1.length, y2.length);
  const start = Math.max(0, Math.floor(range.from) - 1);
  const end = Math.min(n - 1, Math.ceil(range.to));
  const out: FillPolygon[] = [];
  let edgeA: Point[] = [];
  let edgeB: Point[] = [];
  let orient: -1 | 0 | 1 = 0;
  const boundaries = new Set(breaks);

  const flush = () => {
    if (edgeA.length >= 2) {
      out.push({ points: [...edgeA, ...edgeB.reverse()], up: orient >= 0 });
    }
    edgeA = [];
    edgeB = [];
  };

  for (let i = start; i <= end; i++) {
    if (boundaries.has(i)) {
      flush();
      orient = 0;
    }
    const a = y1[i];
    const b = y2[i];
    if (!isNum(a) || !isNum(b)) {
      flush();
      orient = 0;
      continue;
    }
    const s = sign(a - b);
    if (steps && edgeA.length > 0) {
      edgeA.push({ x: map.x(i), y: map.y(y1[i - 1]!) });
      edgeB.push({ x: map.x(i), y: map.y(y2[i - 1]!) });
      if (s !== 0 && orient !== 0 && s !== orient) flush();
    }
    if (!steps && s !== 0 && orient !== 0 && s !== orient) {
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
    const kind = divergenceKind(d, isOverlay);
    out.push({
      from: { x: map.x(d.x1), y: map.y(d.y1) },
      to: { x: map.x(d.x2), y: map.y(d.y2) },
      dashed: d.dashed,
      kind,
      color: wireColor(d.color) ?? DIVLINE_COLORS[kind],
    });
  }
  return out;
}

export type Bias = SeriesLabel["bias"];

export const BIAS_COLORS: Record<Bias, string> = {
  bullish: "rgba(41,98,255,1)",
  bearish: "rgba(242,54,69,1)",
  neutral: "rgba(120,123,134,1)",
};

/** A text mark on the chart (ADR-0046) and what its tooltip says. */
export interface MarkBox extends Rect {
  text: string;
  bias: Bias;
  /** Wire color when valid, else the bias's. */
  color: string;
  tooltip: string;
  /** Faded: on the forming bar, or a pattern that is forming or failed. */
  faint: boolean;
  /** Open for a label on the forming bar: visibly not final. */
  hollow: boolean;
}

export const MARK_HEIGHT = 15;
const MARK_PAD = 3;
const MARK_GAP = 5;

/**
 * Boxes of the labels on the visible bars, past the bar's high or low. Labels of
 * one bar and side stack away from it, in server order.
 */
export function labelBoxes(
  labels: readonly SeriesLabel[],
  range: IndexRange,
  map: Mapper,
  measure: (text: string) => number,
): MarkBox[] {
  const out: MarkBox[] = [];
  const stacks = new Map<string, number>();
  for (const l of labels) {
    if (l.x < range.from - 1 || l.x > range.to) continue;
    const side = `${l.x}|${l.above}`;
    const k = stacks.get(side) ?? 0;
    stacks.set(side, k + 1);
    const width = measure(l.text) + 2 * MARK_PAD;
    const step = k * (MARK_HEIGHT + 1);
    const py = map.y(l.y);
    out.push({
      x: map.x(l.x) - width / 2,
      y: l.above ? py - MARK_GAP - MARK_HEIGHT - step : py + MARK_GAP + step,
      width,
      height: MARK_HEIGHT,
      text: l.text,
      bias: l.bias,
      color: wireColor(l.color) ?? BIAS_COLORS[l.bias],
      tooltip: l.tooltip,
      faint: l.tentative === true,
      hollow: l.tentative === true,
    });
  }
  return out;
}

/** A pattern's strokes in pixels, for those reaching into the visible bars. */
export interface PatternDrawing {
  strokes: Array<{ from: Point; to: Point; dashed: boolean }>;
  box: MarkBox;
  bias: Bias;
  status: SeriesPattern["status"];
}

/** The status in a label's words: nothing hides that a forming one can change. */
export const patternText = (p: SeriesPattern): string =>
  p.key === "fib_swing" ? p.name : `${p.name} · ${p.status}`;

export function patternDrawings(
  patterns: readonly SeriesPattern[],
  range: IndexRange,
  map: Mapper,
  measure: (text: string) => number,
): PatternDrawing[] {
  const out: PatternDrawing[] = [];
  for (const p of patterns) {
    const xs = p.strokes.flatMap((s) => [s.x1, s.x2]);
    if (xs.length === 0) continue;
    if (Math.max(...xs) < range.from - 1 || Math.min(...xs) > range.to) continue;
    const text = patternText(p);
    const width = measure(text) + 2 * MARK_PAD;
    const py = map.y(p.label.y);
    out.push({
      strokes: p.strokes.map((s) => ({
        from: { x: map.x(s.x1), y: map.y(s.y1) },
        to: { x: map.x(s.x2), y: map.y(s.y2) },
        dashed: s.dashed,
      })),
      box: {
        x: map.x(p.label.x) - width / 2,
        y: p.label.above ? py - MARK_GAP - MARK_HEIGHT : py + MARK_GAP,
        width,
        height: MARK_HEIGHT,
        text,
        bias: p.bias,
        color: wireColor(p.color) ?? BIAS_COLORS[p.bias],
        tooltip: p.tooltip,
        faint: p.status !== "confirmed",
        hollow: false,
      },
      bias: p.bias,
      status: p.status,
    });
  }
  return out;
}

/** The topmost box under a point. Later boxes are drawn over earlier ones. */
export function markAt(boxes: readonly MarkBox[], x: number, y: number): MarkBox | null {
  for (let i = boxes.length - 1; i >= 0; i--) {
    const b = boxes[i]!;
    if (x >= b.x && x <= b.x + b.width && y >= b.y && y <= b.y + b.height) return b;
  }
  return null;
}

/** hlines inside the pane, with the wire color (gray fallback). */
export function hlineStrokes(
  hlines: Array<{ y: number; color?: string }>,
  paneHeight: number,
  mapY: (value: number) => number,
): HLineStroke[] {
  const out: HLineStroke[] = [];
  hlines.forEach((h, index) => {
    if (!isNum(h.y)) return;
    const y = mapY(h.y);
    if (y < 0 || y > paneHeight) return;
    out.push({ y, color: wireColor(h.color) ?? HLINE_FALLBACK, index });
  });
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
const waveFills = (): FillTheme[] => [
  single("rgba(73, 148, 236, 0.25)"),
  single(PALETTES[theme()].neutralFill),
  single("rgba(33, 186, 243, 0.05)"),
];

// Kalman Volume Filter emits: bull band, bear band, OB zone, OS zone
// (0x00BCD4 cyan / 0xFF1744 red server-side).
const KALMAN_FILLS: FillTheme[] = [
  single("rgba(0, 188, 212, 0.15)"),
  single("rgba(255, 23, 68, 0.15)"),
  single("rgba(255, 23, 68, 0.10)"),
  single("rgba(0, 188, 212, 0.10)"),
];

/** Fill colors for the `index`-th fill of indicator `name`. */
export function fillTheme(name: string, index: number): FillTheme {
  switch (name) {
    case "ichimoku":
      // TradingView cloud: green while Senkou A ≥ Senkou B, red below.
      return { up: "rgba(38, 166, 154, 0.20)", down: "rgba(239, 83, 80, 0.20)" };
    case "cipher_b":
    case "wavetrend":
      return waveFills()[index] ?? single(BAND_FILL);
    case "kalman":
    case "kalman_volume":
    case "kvf":
      return KALMAN_FILLS[index] ?? single(BAND_FILL);
    case "long_short":
    case "ls_ratio":
    case "top_long_short":
    case "top_ls":
      return single("rgba(38, 166, 154, 0.10)");
    default:
      return single(BAND_FILL);
  }
}

const HLINE_FALLBACK = "rgba(120, 123, 134, 0.80)";

/**
 * Fill colors: the wire `color` where y1 >= y2 and `color_below` (when
 * present) where y1 < y2, else the theme.
 */
export function fillColors(
  fill: SeriesFill,
  name: string,
  index: number,
): FillTheme {
  const c = wireColor(fill.color);
  return c
    ? { up: c, down: wireColor(fill.color_below) ?? c }
    : fillTheme(keyName(name), index);
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

/** One color per hbar: the wire color, else the POC/value-area tier theme. */
export function hbarColors(
  hbars: SeriesHBar[],
  hlines: Array<{ y: number }>,
  name: string,
): string[] {
  const tiers = hbarTiers(hbars, hlines);
  const theme = hbarTheme(keyName(name));
  return hbars.map((h, i) => wireColor(h.color) ?? theme[tiers[i]!]);
}

export const DIVLINE_COLORS: Record<DivergenceKind, string> = {
  bull: "rgba(0, 230, 118, 0.80)",
  bear: "rgba(255, 0, 0, 0.80)",
  flat: "rgba(120, 123, 134, 0.80)",
};

/** Numeric plots rendered with steps, dots, or reset boundaries; values are never altered. */
export function presentationRuns(
  values: Array<number | null>,
  range: IndexRange,
  map: Mapper,
  steps = false,
  breaks: number[] = [],
): Point[][] {
  const runs: Point[][] = [];
  const boundaries = new Set(breaks);
  let run: Point[] = [];
  const flush = () => {
    if (run.length) runs.push(run);
    run = [];
  };
  const start = Math.max(0, Math.floor(range.from) - 1);
  const end = Math.min(values.length - 1, Math.ceil(range.to));
  for (let i = start; i <= end; i++) {
    const value = values[i];
    if (boundaries.has(i) || !isNum(value)) flush();
    if (!isNum(value)) continue;
    const point = { x: map.x(i), y: map.y(value) };
    if (steps && run.length) run.push({ x: point.x, y: run[run.length - 1]!.y });
    run.push(point);
  }
  flush();
  return runs;
}
