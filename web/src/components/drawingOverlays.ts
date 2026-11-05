// klinecharts overlays for the measure tools, trend-based fib extension,
// Andrews' pitchfork and long/short position tools (registered on import).
// The math lives in lib/drawingMath.ts; this file only turns it into figures.
//
// Pixels come from the overlay's `coordinates`, prices from `overlay.points`
// and the axis (`yAxis.convertToPixel`), so log, percentage and inverted
// scales need no special case here.

import {
  registerOverlay,
  utils,
  type Bounding,
  type Coordinate,
  type OverlayCreateFiguresCallbackParams,
  type OverlayFigure,
} from "klinecharts";

import { formatColor } from "../lib/color";
import {
  arrowHead,
  barsBetween,
  clampPosition,
  dateRangeLabel,
  defaultPosition,
  FIB_EXT_LEVELS,
  fibExtensionLevels,
  formatAmount,
  pitchforkLines,
  positionBars,
  positionLabels,
  priceRangeLabel,
  type PositionKind,
} from "../lib/drawingMath";
import { FONT_FAMILY, PALETTES } from "../lib/theme";
import { theme } from "../stores/layout";
import { pointTimestamp, type TimeAxis } from "../stores/drawings";

type Params = OverlayCreateFiguresCallbackParams;
type Pt = { timestamp?: number; dataIndex?: number; value?: number };

const UP = { fill: "rgba(41, 98, 255, 0.15)", line: "#2962ff" };
const DOWN = { fill: "rgba(242, 54, 69, 0.15)", line: "#f23645" };
const GREEN = { fill: "rgba(8, 153, 129, 0.22)", line: "#089981" };
const RED = { fill: "rgba(242, 54, 69, 0.22)", line: "#f23645" };
// Level colors in TradingView's order; the dark theme gets lighter blue and
// purple so they stay readable on the dark background.
const FIB_COLORS_LIGHT = [
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
const FIB_COLORS_DARK = FIB_COLORS_LIGHT.map((c) =>
  c === "#2962ff" ? "#5b8cff" : c === "#9c27b0" ? "#ba68c8" : c,
);

let axisOf: () => TimeAxis | null = () => null;
/** Bar timing for the time span of points beyond the last bar. */
export function setOverlayAxis(fn: () => TimeAxis | null): void {
  axisOf = fn;
}

// What a fresh long/short position is sized from, read once when the tool is armed.
let positionSize: { reward: number | null; bars: number | null } = { reward: null, bars: null };
export function setPositionSize(size: { reward: number | null; bars: number | null }): void {
  positionSize = size;
}

const TEXT_PAD = 6;
/**
 * Center of a box [left, right] for a label: the middle of the part inside
 * the pane (so it stays readable when the box runs off an edge), then
 * clamped so a label of `text` fits inside the pane.
 */
function labelX(left: number, right: number, text: string, size: number, b: Bounding): number {
  const lo = Math.max(left, 0);
  const hi = Math.min(right, b.width);
  const mid = lo <= hi ? (lo + hi) / 2 : (left + right) / 2;
  const half = (utils.calcTextWidth(text, size, "normal", FONT_FAMILY) + 2 * TEXT_PAD) / 2;
  return Math.min(Math.max(mid, half), Math.max(half, b.width - half));
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
  return {
    type: "text",
    attrs: { x, y, text, align: "center", baseline },
    styles: {
      style: "fill",
      color: "#ffffff",
      backgroundColor: bg,
      size: textSize(o),
      family: FONT_FAMILY,
      paddingLeft: TEXT_PAD,
      paddingRight: TEXT_PAD,
      paddingTop: 3,
      paddingBottom: 3,
      borderRadius: 3,
    },
    ignoreEvent: true,
  };
}

const textSize = (o: Params["overlay"], fallback = 12): number =>
  (o.styles as { text?: { size?: number } } | null)?.text?.size ?? fallback;

const LABEL_H = 22;

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
      const bottom = Math.max(a.y, b.y);
      const texts: string[] = [];
      if (price) texts.push(priceRangeLabel(val(pa) ?? 0, val(pb) ?? 0, pricePrecision(p)));
      if (date) texts.push(span(p, pa, pb, a, b));
      // Stacked above the band (the last text closest to it); below it when
      // there is no room above, e.g. a band at the top of the pane.
      const below = top - 4 - texts.length * LABEL_H < 0;
      texts.forEach((text, i) => {
        const x = labelX(Math.min(a.x, b.x), Math.max(a.x, b.x), text, textSize(overlay), p.bounding);
        const y = below ? bottom + 4 + i * LABEL_H : top - 4 - (texts.length - 1 - i) * LABEL_H;
        figs.push(label(x, y, text, line, below ? "top" : "bottom", overlay));
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
    const colors = theme() === "dark" ? FIB_COLORS_DARK : FIB_COLORS_LIGHT;
    const capBg = formatColor(PALETTES[theme()].bg, 0.85);
    const figs = fibExtensionLevels(a, b, k).flatMap(({ level, value }) => {
      const y = yAxis?.convertToPixel(value) ?? ck.y + (c[1]!.y - c[0]!.y) * level;
      const color = userOr(overlay, "line", "color", colors[FIB_EXT_LEVELS.indexOf(level as never)] ?? "#787b86");
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
          styles: {
            style: "fill",
            color: text?.color ?? color,
            size: text?.size ?? 12,
            weight: "500",
            family: FONT_FAMILY,
            backgroundColor: capBg,
            paddingLeft: 2,
            paddingRight: 2,
            paddingTop: 2,
            paddingBottom: 2,
          },
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
      // Labels sit on the visible part of the box.
      const at = (text: string) => labelX(left, right, text, textSize(overlay), p.bounding);
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
        label(at(text.target), t.y, text.target, GREEN.line, t.y <= e.y ? "bottom" : "top", overlay),
        label(at(text.stop), s.y, text.stop, RED.line, s.y >= e.y ? "top" : "bottom", overlay),
        label(at(text.ratio), e.y, text.ratio, line, "middle", overlay),
      ];
    },
    performEventMoveForDrawing: ({ currentStep, points, performPoint }) => {
      // setPoints replays this with step 2 when a stored drawing is restored.
      if (currentStep !== 1) return;
      const entry = performPoint.value;
      const at = performPoint.dataIndex;
      if (typeof entry !== "number" || typeof at !== "number") return;
      const d = defaultPosition(kind, entry, positionSize.reward);
      const x = at + positionBars(positionSize.bars);
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
      // The dragged value first, then both others, so a restored record
      // with any value on the wrong side of the entry is repaired too.
      let v: [number, number, number] = [e.value, t.value, s.value];
      for (const moved of [performPointIndex, 1, 2] as const) {
        v = clampPosition(kind, v, moved as 0 | 1 | 2);
      }
      [e.value, t.value, s.value] = v;
    },
  });
}
