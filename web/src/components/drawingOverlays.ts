// klinecharts overlays for the measure tools, trend-based fib extension,
// Andrews' pitchfork and long/short position tools (registered on import).
// The math lives in lib/drawingMath.ts; this file only turns it into figures.
//
// Pixels come from the overlay's `coordinates`, prices from `overlay.points`
// and the axis (`yAxis.convertToPixel`), so log, percentage and inverted
// scales need no special case here.

import {
  registerOverlay,
  type Coordinate,
  type OverlayCreateFiguresCallbackParams,
  type OverlayFigure,
} from "klinecharts";

import {
  arrowHead,
  barsBetween,
  clampPosition,
  dateRangeLabel,
  DEFAULT_POSITION_BARS,
  defaultPosition,
  FIB_EXT_LEVELS,
  fibExtensionLevels,
  formatAmount,
  pitchforkLines,
  positionLabels,
  priceRangeLabel,
  type PositionKind,
} from "../lib/drawingMath";
import { pointTimestamp, type TimeAxis } from "../stores/drawings";

type Params = OverlayCreateFiguresCallbackParams;
type Pt = { timestamp?: number; dataIndex?: number; value?: number };

const UP = { fill: "rgba(41, 98, 255, 0.15)", line: "#2962ff" };
const DOWN = { fill: "rgba(242, 54, 69, 0.15)", line: "#f23645" };
const GREEN = { fill: "rgba(8, 153, 129, 0.22)", line: "#089981" };
const RED = { fill: "rgba(242, 54, 69, 0.22)", line: "#f23645" };
const FIB_COLORS = [
  "#787b86",
  "#f23645",
  "#ff9800",
  "#4caf50",
  "#089981",
  "#00bcd4",
  "#2962ff",
  "#9c27b0",
  "#e91e63",
  "#f23645",
  "#ff5252",
];

let axisOf: () => TimeAxis | null = () => null;
/** Bar timing for the time span of points beyond the last bar. */
export function setOverlayAxis(fn: () => TimeAxis | null): void {
  axisOf = fn;
}

/** Latest labels each overlay drew, read by the E2E suite (DEV builds only). */
export const overlayLabels = new Map<string, string[]>();
const logLabels = (id: string, labels: string[]) => {
  if (import.meta.env.DEV) overlayLabels.set(id, labels);
};

/** Color the user chose (style bar), else the tool's own. */
function userOr(
  o: Params["overlay"],
  type: "line" | "polygon",
  key: "color" | "borderColor",
  fallback: string,
): string {
  const s = (o.styles as Record<string, Record<string, unknown>> | null)?.[type];
  const v = s?.[key];
  return typeof v === "string" ? v : fallback;
}

function band(c: Coordinate[], o: Params["overlay"], look: { fill: string; line: string }): OverlayFigure {
  const [a, b] = c as [Coordinate, Coordinate];
  return {
    type: "polygon",
    attrs: {
      coordinates: [
        { x: a.x, y: a.y },
        { x: b.x, y: a.y },
        { x: b.x, y: b.y },
        { x: a.x, y: b.y },
      ],
    },
    styles: {
      style: "stroke_fill",
      color: userOr(o, "polygon", "color", look.fill),
      borderColor: userOr(o, "polygon", "borderColor", look.line),
    },
  };
}

function arrow(from: Coordinate, to: Coordinate, color: string): OverlayFigure {
  return {
    type: "line",
    attrs: [
      { coordinates: [from, to] },
      ...arrowHead(from, to).map((coordinates) => ({ coordinates })),
    ],
    styles: { color },
    ignoreEvent: true,
  };
}

function label(
  x: number,
  y: number,
  text: string,
  bg: string,
  baseline: "top" | "bottom" | "middle",
  o: Params["overlay"],
): OverlayFigure {
  const size = (o.styles as { text?: { size?: number } } | null)?.text?.size;
  return {
    type: "text",
    attrs: { x, y, text, align: "center", baseline },
    styles: {
      style: "fill",
      color: "#ffffff",
      backgroundColor: bg,
      size: size ?? 12,
      paddingLeft: 6,
      paddingRight: 6,
      paddingTop: 3,
      paddingBottom: 3,
      borderRadius: 3,
    },
    ignoreEvent: true,
  };
}

const pricePrecision = ({ precision, yAxis }: Params) =>
  (yAxis?.isInCandle() ?? true) ? precision.price : precision.excludePriceVolumeMax;

const val = (p: Pt | undefined) => (typeof p?.value === "number" ? p.value : null);

function timeOf(p: Pt): number | undefined {
  if (typeof p.timestamp === "number") return p.timestamp;
  return pointTimestamp(p, axisOf());
}

/** Bars (from pixels) and time span (from timestamps) between two points. */
function span(p: Params, a: Pt, b: Pt, ca: Coordinate, cb: Coordinate): string {
  const bars = barsBetween(ca.x, cb.x, p.barSpace.bar);
  const ta = timeOf(a);
  const tb = timeOf(b);
  const ms =
    ta !== undefined && tb !== undefined ? Math.abs(tb - ta) : bars * (axisOf()?.intervalMs ?? 0);
  return dateRangeLabel(bars, ms);
}

function twoPoints(p: Params): { a: Coordinate; b: Coordinate; pa: Pt; pb: Pt } | null {
  const [a, b] = p.coordinates;
  const [pa, pb] = p.overlay.points;
  return a && b && pa && pb ? { a, b, pa, pb } : null;
}

const measure = (name: string, price: boolean, date: boolean) =>
  registerOverlay({
    name,
    totalStep: 3,
    needDefaultPointFigure: true,
    needDefaultXAxisFigure: true,
    needDefaultYAxisFigure: true,
    createPointFigures: (p) => {
      const t = twoPoints(p);
      if (!t) return [];
      const { a, b, pa, pb } = t;
      const { overlay } = p;
      const up = date && !price ? b.x >= a.x : (val(pb) ?? 0) >= (val(pa) ?? 0);
      const look = up ? UP : DOWN;
      const line = userOr(overlay, "line", "color", look.line);
      const xm = (a.x + b.x) / 2;
      const ym = (a.y + b.y) / 2;
      const figs: OverlayFigure[] = [band([a, b], overlay, look)];
      if (price) figs.push(arrow({ x: xm, y: a.y }, { x: xm, y: b.y }, line));
      if (date) figs.push(arrow({ x: a.x, y: ym }, { x: b.x, y: ym }, line));
      const top = Math.min(a.y, b.y);
      const texts: string[] = [];
      if (price) texts.push(priceRangeLabel(val(pa) ?? 0, val(pb) ?? 0, pricePrecision(p)));
      if (date) texts.push(span(p, pa, pb, a, b));
      // Stacked above the band: the last text sits closest to it.
      texts.forEach((text, i) => {
        figs.push(label(xm, top - 4 - (texts.length - 1 - i) * 22, text, line, "bottom", overlay));
      });
      logLabels(overlay.id, texts);
      return figs;
    },
  });

measure("cgPriceRange", true, false);
measure("cgDateRange", false, true);
measure("cgDatePriceRange", true, true);

registerOverlay({
  name: "cgFibExt",
  totalStep: 4,
  needDefaultPointFigure: true,
  needDefaultXAxisFigure: true,
  needDefaultYAxisFigure: true,
  createPointFigures: (p) => {
    const { coordinates: c, overlay, bounding, yAxis } = p;
    const guide: OverlayFigure = {
      type: "line",
      attrs: { coordinates: c.slice(0, 3) },
      styles: { style: "dashed", dashedValue: [4, 4] },
    };
    const [a, b, k] = overlay.points.map(val);
    const ck = c[2];
    if (!ck || a == null || b == null || k == null) return [guide];
    const prec = pricePrecision(p);
    const text = (overlay.styles as { text?: { color?: string; size?: number } } | null)?.text;
    const labels: string[] = [];
    const figs = fibExtensionLevels(a, b, k).flatMap(({ level, value }) => {
      const y = yAxis?.convertToPixel(value) ?? ck.y + (c[1]!.y - c[0]!.y) * level;
      const color = userOr(overlay, "line", "color", FIB_COLORS[FIB_EXT_LEVELS.indexOf(level as never)] ?? "#787b86");
      const caption = `${level} (${formatAmount(value, prec)})`;
      labels.push(caption);
      return [
        {
          type: "line",
          attrs: { coordinates: [{ x: ck.x, y }, { x: Math.max(bounding.width, ck.x), y }] },
          styles: { color },
        },
        {
          type: "text",
          attrs: { x: ck.x + 4, y, text: caption, baseline: "bottom" },
          styles: { color: text?.color ?? color, size: text?.size ?? 11, backgroundColor: "transparent" },
          ignoreEvent: true,
        },
      ] as OverlayFigure[];
    });
    logLabels(overlay.id, labels);
    return [guide, ...figs];
  },
});

registerOverlay({
  name: "cgPitchfork",
  totalStep: 4,
  needDefaultPointFigure: true,
  needDefaultXAxisFigure: true,
  needDefaultYAxisFigure: true,
  createPointFigures: ({ coordinates: c, bounding }) => {
    const [p1, p2, p3] = c;
    if (!p1 || !p2) return [];
    if (!p3) return { type: "line", attrs: { coordinates: [p1, p2] } };
    const f = pitchforkLines(p1, p2, p3, bounding.width, bounding.height);
    return [
      { type: "line", attrs: [f.median, f.upper, f.lower].map((coordinates) => ({ coordinates })) },
      {
        type: "line",
        attrs: { coordinates: f.handle },
        styles: { style: "dashed", dashedValue: [4, 4] },
      },
    ];
  },
});

const POSITION_KINDS: Record<string, PositionKind> = { cgLong: "long", cgShort: "short" };

for (const [name, kind] of Object.entries(POSITION_KINDS)) {
  registerOverlay({
    name,
    // One click places the entry; the target and stop are derived points.
    totalStep: 2,
    needDefaultPointFigure: true,
    needDefaultXAxisFigure: true,
    needDefaultYAxisFigure: true,
    createPointFigures: (p) => {
      const { coordinates: c, overlay } = p;
      const [e, t, s] = c;
      const [pe, pt, ps] = overlay.points.map(val);
      if (!e || !t || !s || pe == null || pt == null || ps == null) return [];
      const left = Math.min(e.x, t.x);
      const right = Math.max(e.x, t.x);
      const mid = (left + right) / 2;
      const rect = (y0: number, y1: number, look: typeof GREEN): OverlayFigure => ({
        type: "polygon",
        attrs: {
          coordinates: [
            { x: left, y: y0 },
            { x: right, y: y0 },
            { x: right, y: y1 },
            { x: left, y: y1 },
          ],
        },
        styles: { style: "stroke_fill", color: look.fill, borderColor: look.line },
      });
      const text = positionLabels(kind, pe, pt, ps, pricePrecision(p));
      logLabels(overlay.id, [text.target, text.stop, text.ratio]);
      const line = userOr(overlay, "line", "color", "#787b86");
      return [
        rect(e.y, t.y, GREEN),
        rect(e.y, s.y, RED),
        { type: "line", attrs: { coordinates: [{ x: left, y: e.y }, { x: right, y: e.y }] }, styles: { color: line } },
        label(mid, t.y, text.target, GREEN.line, t.y <= e.y ? "bottom" : "top", overlay),
        label(mid, s.y, text.stop, RED.line, s.y >= e.y ? "top" : "bottom", overlay),
        label(mid, e.y, text.ratio, line, "middle", overlay),
      ];
    },
    performEventMoveForDrawing: ({ currentStep, points, performPoint }) => {
      // setPoints replays this with step 2 when a stored drawing is restored.
      if (currentStep !== 1) return;
      const entry = performPoint.value;
      const at = performPoint.dataIndex;
      if (typeof entry !== "number" || typeof at !== "number") return;
      const d = defaultPosition(kind, entry);
      const x = at + DEFAULT_POSITION_BARS;
      points[1] = { dataIndex: x, value: d.target };
      points[2] = { dataIndex: x, value: d.stop };
    },
    performEventPressedMove: ({ points, performPointIndex, performPoint }) => {
      const [e, t, s] = points;
      if (!e || !t || !s || performPointIndex > 2) return;
      // Target and stop share one right edge.
      if (performPointIndex > 0) {
        const other = performPointIndex === 1 ? s : t;
        other.timestamp = performPoint.timestamp;
        other.dataIndex = performPoint.dataIndex;
      }
      if (e.value === undefined || t.value === undefined || s.value === undefined) return;
      [e.value, t.value, s.value] = clampPosition(
        kind,
        [e.value, t.value, s.value],
        performPointIndex as 0 | 1 | 2,
      );
    },
  });
}
