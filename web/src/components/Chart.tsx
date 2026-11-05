// klinecharts wrapper. Every indicator in the SeriesPayload is rendered
// via a generic registration path: the server returns per-bar values in
// the payload, we stash them on each KLineData under per-indicator keys
// (`chartgen_<name>_line<i>` / `chartgen_<name>_hist<i>`), and we
// `registerIndicator` lazily for each name the first time we see it.
// The indicator's `calc` callback reads its stashed values back off the
// bars — no math runs in the browser.
//
// Fills, hbars and divlines have no klinecharts figure type. Each payload
// entry, re-indexed into data space, sits in the module-level
// `geometryStore`; the template's `draw` callback paints it from
// geometry.ts primitives before the default line/bar figures (ADR-0010).
//
// Pane lifecycle: we track created panes in a signal keyed by indicator
// name. When a new indicator shows up we `createIndicator`; when one
// drops out of the payload we `removeIndicator` and delete the entry.
//
// Tooltip: klinecharts's canvas-drawn candle tooltip is suppressed and
// the DOM PaneHeader becomes the single source of truth. When the
// crosshair is active the header shows the values at that bar index;
// otherwise it shows the last finite value. Handler is rAF-debounced.
//
// Live data: `series` is the live feed's merged series (lib/liveFeed.ts)
// and `update` says what changed. Older pages are prepended through
// klinecharts's load-data callback and tail polls go through updateData,
// so neither moves the view; everything else is a full applyNewData.
//
// Visual styling is driven by ADR-0006 (TradingView visual parity).

import {
  createEffect,
  createSignal,
  createMemo,
  Index,
  Match,
  onCleanup,
  onMount,
  Show,
  Switch,
  untrack,
  type JSX,
} from "solid-js";
import {
  dispose,
  init,
  LoadDataType,
  registerIndicator,
  type Chart as KChart,
  type IndicatorDrawCallback,
  type KLineData,
} from "klinecharts";

import {
  figureOverrides,
  figureStyle,
  plotsOf,
  resolveOverride,
  resolvePlot,
} from "../lib/plotStyle";
import {
  indicatorName,
  type ActiveIndicator,
  type IndicatorStyle,
} from "../stores/indicators";
import {
  canPageBefore,
  type HistoryStatus,
  type SeriesUpdate,
} from "../lib/liveFeed";
import { theme, type ChartType } from "../stores/layout";
import { compareReading, formatPct, MAX_COMPARE, rebase } from "../lib/compare";
import { PALETTES, type Palette } from "../lib/theme";
import type { SeriesPayload } from "../types/responses";
import {
  barLegend,
  lastFiniteCloseIndex,
  pricePrecision,
  type BarLegend,
} from "../lib/format";
import { formatBarTime } from "../lib/timezone";
import {
  divlineSegments,
  fillColors,
  fillPolygons,
  hbarRects,
  hlineStrokes,
  payloadToDataIndex,
  toDataSpace,
  valueExtents,
  type Mapper,
  type SeriesIndicator,
} from "./geometry";
import { PaneHeader, type PaneHeaderLine } from "./PaneHeader";
import { PriceAxisControls } from "./PriceAxisControls";

const FONT_FAMILY = "Inter, system-ui, sans-serif";

// TradingView-aligned rotating palette for indicator line/bar series.
// Color assignment is deterministic per (indicator name, series index)
// so a refresh doesn't reshuffle colors.
const TV_SERIES_PALETTE = [
  "#2962ff",
  "#ff6d00",
  "#00bcd4",
  "#e91e63",
  "#4caf50",
  "#ffeb3b",
  "#9c27b0",
  "#ff5252",
  "#607d8b",
  "#795548",
] as const;

function hashName(name: string): number {
  let sum = 0;
  for (let i = 0; i < name.length; i++) sum += name.charCodeAt(i);
  return sum;
}

export function paletteColor(name: string, index: number): string {
  const base = hashName(name);
  return TV_SERIES_PALETTE[(base + index) % TV_SERIES_PALETTE.length]!;
}

/**
 * Figure-style mapping for user plot overrides (ADR-0017). The user's
 * `IndicatorStyle` rides on the klinecharts indicator instance as its
 * `extendData`, so a style change is one `overrideIndicator` (which
 * recalculates and repaints) and never touches the data or the server.
 */
function plotFigureStyle(
  name: string,
  kind: "line" | "hist",
  plotIndex: number,
  colorIdx: number,
  indicator: { extendData?: unknown },
): Record<string, unknown> {
  const ext = indicator.extendData as IndicatorStyle | undefined;
  return figureStyle(
    kind,
    resolveOverride(ext?.plots[`${kind}${plotIndex}`], paletteColor(name, colorIdx)),
  );
}

/**
 * klinecharts `extendData` of an indicator: the user's overrides keyed by
 * figure key for the plots of the current payload (labels already matched).
 */
function extendFor(
  name: string,
  ind: NonNullable<SeriesPayload["indicators"][string]> | undefined,
  style: IndicatorStyle | undefined,
): IndicatorStyle {
  return ind
    ? figureOverrides(plotsOf(ind, (n) => paletteColor(name, n)), style)
    : NO_STYLE;
}

/** Extend data of an indicator without overrides; klinecharts ignores an undefined override. */
const NO_STYLE: IndicatorStyle = { plots: {} };

/** Legend entries of an indicator with the user's overrides applied; hidden plots are omitted. */
function legendPlots(
  name: string,
  ind: NonNullable<SeriesPayload["indicators"][string]>,
  style: IndicatorStyle | undefined,
): Array<{ label: string; color: string; kind: "line" | "hist"; index: number }> {
  return plotsOf(ind, (n) => paletteColor(name, n)).flatMap((p) => {
    const r = resolvePlot(p, style);
    return r.visible
      ? [{ label: p.label, color: r.color, kind: p.kind, index: p.index }]
      : [];
  });
}

function indicatorRegName(name: string): string {
  return `chartgen_${name}`;
}

function lineStashKey(name: string, i: number): string {
  return `chartgen_${name}_line${i}`;
}

function histStashKey(name: string, i: number): string {
  return `chartgen_${name}_hist${i}`;
}

const registeredIndicators = new Set<string>();

interface IndicatorShape {
  lines: number;
  hist: number;
  /** True when the indicator emits fills, hbars, divlines or hlines. */
  geometry: boolean;
}

const isEmptyShape = (s: IndicatorShape): boolean =>
  s.lines === 0 && s.hist === 0 && !s.geometry;

/**
 * Geometry per registered indicator name, already re-indexed into data
 * space (`toDataSpace`). Written by the data effect before
 * `applyNewData`, whose recalc + repaint is what picks it up, so a new
 * payload costs no extra `overrideIndicator` pass. Module-level like
 * klinecharts's own indicator registry; the app mounts one Chart.
 */
const geometryStore = new Map<string, SeriesIndicator>();

/**
 * Put the geometry of `names` into `geometryStore`, in data space. When
 * every bar has a timestamp (`timed`; always so for merged live data)
 * payload and data indices coincide, and the stored entries share their
 * fill arrays with the series, which a tail merge updates in place; a
 * tail tick then only re-stores them (no per-bar copy) to pick up its
 * re-built divlines.
 */
function storeGeometry(
  series: SeriesPayload,
  names: string[],
  timed = series.bars.every((b) => b.t !== null),
): void {
  const remap = timed ? null : payloadToDataIndex(series.bars);
  for (const name of names) {
    const ind = series.indicators[name];
    if (!ind || isEmptyShape(shapeFromPayload(series, name))) continue;
    geometryStore.set(indicatorRegName(name), toDataSpace(ind, remap, name));
  }
}

/**
 * klinecharts `draw` callback painting the indicator's fills, hbars and
 * divlines from `geometryStore`. Runs on every frame, so the geometry
 * follows pan/zoom/rescale. Returns false so the default line/bar
 * figures still draw on top.
 */
function drawGeometry(name: string): IndicatorDrawCallback {
  const regName = indicatorRegName(name);
  return ({ ctx, visibleRange, bounding, xAxis, yAxis }) => {
    const ind = geometryStore.get(regName);
    if (!ind) return false;
    const map: Mapper = {
      x: (i) => xAxis.convertToPixel(i),
      y: (v) => yAxis.convertToPixel(v),
    };

    // Fills go behind the candles already on this canvas, as in
    // TradingView; the grid is painted afterwards on top. This relies on
    // nothing opaque (e.g. a background) being painted on the pane canvas
    // first. The e2e z-order test guards it. With destination-over each
    // paint lands beneath the previous one, so iterate in reverse to keep
    // the server's paint order. klinecharts restores the context after draw.
    ctx.globalCompositeOperation = "destination-over";
    const fills = ind.fills ?? [];
    for (let fi = fills.length - 1; fi >= 0; fi--) {
      const fill = fills[fi]!;
      const theme = fillColors(fill, name, fi);
      for (const poly of fillPolygons(fill.y1, fill.y2, visibleRange, map)) {
        ctx.fillStyle = poly.up ? theme.up : theme.down;
        ctx.beginPath();
        poly.points.forEach((p, k) =>
          k === 0 ? ctx.moveTo(p.x, p.y) : ctx.lineTo(p.x, p.y),
        );
        ctx.closePath();
        ctx.fill();
      }
    }
    ctx.globalCompositeOperation = "source-over";

    if (ind.hbars && ind.hbars.length > 0) {
      for (const r of hbarRects(
        ind.hbars,
        bounding.width,
        bounding.height,
        map.y,
      )) {
        ctx.fillStyle = ind.hbars[r.index]!.color!;
        ctx.fillRect(r.x, r.y, r.width, r.height);
      }
    }

    const segments = divlineSegments(
      ind.divlines ?? [],
      visibleRange,
      map,
      ind.is_overlay,
    );
    ctx.lineWidth = 2;
    for (const s of segments) {
      ctx.strokeStyle = s.color;
      ctx.setLineDash(s.dashed ? [4, 4] : []);
      ctx.beginPath();
      ctx.moveTo(s.from.x, s.from.y);
      ctx.lineTo(s.to.x, s.to.y);
      ctx.stroke();
    }
    drawHLines(ctx, ind, bounding, map.y);
    return false;
  };
}

/**
 * Full-width 1px horizontal reference lines (RSI 70/30, profile
 * POC/VAH/VAL) in the server's color, as the PNG renderer draws them.
 * Clipped to the pane: levels mapped outside it are skipped.
 */
function drawHLines(
  ctx: CanvasRenderingContext2D,
  ind: SeriesIndicator,
  bounding: { width: number; height: number },
  mapY: (value: number) => number,
): void {
  ctx.lineWidth = 1;
  ctx.setLineDash([]);
  for (const h of hlineStrokes(ind.hlines ?? [], bounding.height, mapY)) {
    ctx.strokeStyle = h.color;
    // Snap to the pixel center so a 1px line is not blurred over two rows.
    const y = Math.round(h.y) + 0.5;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(bounding.width, y);
    ctx.stroke();
  }
}

/**
 * Register (once) a generic chartgen indicator for `name` with the
 * given figure shape. Subsequent calls with the same name are no-ops —
 * klinecharts's registry is global and re-registering throws.
 *
 * Each figure's `calc` just reads the stashed value back off the bar;
 * the Rust server has already done the math.
 */
function ensureRegistered(name: string, shape: IndicatorShape): void {
  const regName = indicatorRegName(name);
  if (registeredIndicators.has(regName)) return;
  registeredIndicators.add(regName);

  type Row = Record<string, number | null>;
  const figures: Array<{
    key: string;
    title: string;
    type?: "line" | "bar";
    baseValue?: number;
    styles?: (data: unknown, indicator: { extendData?: unknown }) => Record<string, unknown>;
  }> = [
    // Typeless figures draw nothing but feed klinecharts's y-axis
    // auto-scale: fill edges and the payload y_range (`valueExtents`).
    // Used instead of minValue/maxValue, which klinecharts only applies
    // when figures already have values (geometry-only panes have none)
    // and whose override path sets maxValue via setMinValue.
    { key: "range_lo", title: "" },
    { key: "range_hi", title: "" },
  ];

  for (let i = 0; i < shape.lines; i++) {
    figures.push({
      key: `line${i}`,
      title: "",
      type: "line",
      styles: (_d, ind) => plotFigureStyle(name, "line", i, i, ind),
    });
  }
  for (let i = 0; i < shape.hist; i++) {
    // Histograms are placed after lines in the palette rotation so a
    // MACD-style (2 lines + 1 hist) keeps line0/line1/hist0 distinct.
    const colorIdx = shape.lines + i;
    figures.push({
      key: `hist${i}`,
      title: "",
      type: "bar",
      baseValue: 0,
      styles: (_d, ind) => plotFigureStyle(name, "hist", i, colorIdx, ind),
    });
  }

  registerIndicator<Row>({
    name: regName,
    shortName: name.toUpperCase(),
    precision: 2,
    figures: figures as never,
    draw: drawGeometry(name),
    calc: (dataList: KLineData[], indicator) => {
      const geo = geometryStore.get(regName);
      const extents = geo ? valueExtents(geo, dataList.length) : [];
      // A hidden plot's value is null, so nothing is drawn for it.
      const ext = indicator.extendData as IndicatorStyle | undefined;
      const shown = (key: string) => ext?.plots[key]?.visible !== false;
      const lineShown = Array.from({ length: shape.lines }, (_, i) => shown(`line${i}`));
      const histShown = Array.from({ length: shape.hist }, (_, i) => shown(`hist${i}`));
      return dataList.map((bar, di) => {
        const row: Row = {
          range_lo: extents[di]?.lo ?? null,
          range_hi: extents[di]?.hi ?? null,
        };
        for (let i = 0; i < shape.lines; i++) {
          const raw = bar[lineStashKey(name, i)];
          row[`line${i}`] =
            lineShown[i] &&
            typeof raw === "number" &&
            Number.isFinite(raw)
              ? raw
              : null;
        }
        for (let i = 0; i < shape.hist; i++) {
          const raw = bar[histStashKey(name, i)];
          row[`hist${i}`] =
            histShown[i] &&
            typeof raw === "number" &&
            Number.isFinite(raw)
              ? raw
              : null;
        }
        return row;
      });
    },
  });
}

// --- Compare lines (ADR-0022) ------------------------------------------
const COMPARE_REG_NAME = "chartgen_compare";
const compareStashKey = (slot: number) => `chartgen_cmp_${slot}`;

/** Extend data of the compare indicator: the first visible bar and the line colors. */
interface CompareExtend {
  from: number;
  colors: string[];
}

let compareRegistered = false;

/**
 * Register (once) the indicator drawing every compare line on the candle
 * pane. Each bar carries the symbol's as-of aligned close under
 * `chartgen_cmp_<slot>`; `calc` only rescales it into the main series' price
 * space (`rebase`) from the first visible bar, which is what makes the
 * klinecharts percent axis (normalised by the main close at that bar) read the
 * compare symbol's own percent change. Display normalisation, no indicator
 * maths (ADR-0003).
 */
function ensureCompareRegistered(): void {
  if (compareRegistered) return;
  compareRegistered = true;
  type Row = Record<string, number | null>;
  const figures = Array.from({ length: MAX_COMPARE }, (_, i) => ({
    key: `cmp${i}`,
    title: "",
    type: "line" as const,
    styles: (_d: unknown, ind: { extendData?: unknown }) => ({
      color: (ind.extendData as CompareExtend | undefined)?.colors[i],
      size: 2,
      style: "solid",
    }),
  }));
  registerIndicator<Row>({
    name: COMPARE_REG_NAME,
    shortName: "COMPARE",
    precision: 2,
    figures: figures as never,
    calc: (dataList: KLineData[], indicator) => {
      const ext = indicator.extendData as CompareExtend | undefined;
      const closes = dataList.map((d) => d.close);
      const lines = Array.from({ length: MAX_COMPARE }, (_, i) =>
        rebase(
          dataList.map((d) => d[compareStashKey(i)] as number | null | undefined),
          closes,
          ext?.from ?? 0,
        ),
      );
      return dataList.map((_, di) =>
        Object.fromEntries(lines.map((l, i) => [`cmp${i}`, l[di] ?? null])),
      );
    },
  });
}
// ------------------------------------------------------------------------

function shapeFromPayload(
  series: SeriesPayload,
  name: string,
): IndicatorShape {
  const ind = series.indicators[name];
  if (!ind) return { lines: 0, hist: 0, geometry: false };
  const lines = ind.lines?.length ?? 0;
  const hist = ind.histogram?.length ?? 0;
  const geometry =
    (ind.fills?.length ?? 0) > 0 ||
    (ind.hbars?.length ?? 0) > 0 ||
    (ind.divlines?.length ?? 0) > 0 ||
    (ind.hlines?.length ?? 0) > 0;
  return { lines, hist, geometry };
}

/** KLineData (with indicator stashes) for bars `from` up to `to`. */
function seriesToKLines(
  series: SeriesPayload,
  from = 0,
  to = series.bars.length,
): KLineData[] {
  const names = Object.keys(series.indicators);
  const klines: KLineData[] = [];
  for (let i = from; i < to; i++) {
    const bar = series.bars[i];
    if (!bar || bar.t === null) continue;
    const kline: KLineData = {
      timestamp: bar.t * 1000,
      open: bar.o ?? 0,
      high: bar.h ?? 0,
      low: bar.l ?? 0,
      close: bar.c ?? 0,
      volume: bar.v ?? 0,
    };
    for (const name of names) {
      const ind = series.indicators[name];
      if (!ind) continue;
      const lines = ind.lines ?? [];
      for (let li = 0; li < lines.length; li++) {
        const values = lines[li]?.values;
        if (!values) continue;
        kline[lineStashKey(name, li)] = values[i] ?? null;
      }
      const hist = ind.histogram ?? [];
      for (let hi = 0; hi < hist.length; hi++) {
        const values = hist[hi]?.values;
        if (!values) continue;
        kline[histStashKey(name, hi)] = values[i] ?? null;
      }
    }
    klines.push(kline);
  }
  return klines;
}

function lastFinite(values: Array<number | null>): number | null {
  for (let i = values.length - 1; i >= 0; i--) {
    const v = values[i];
    if (typeof v === "number" && Number.isFinite(v)) return v;
  }
  return null;
}

function valueAt(
  values: Array<number | null>,
  index: number,
): number | null {
  const v = values[index];
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

/**
 * Compact K/M/B formatter used for y-axis ticks and the right-axis
 * volume readout. Two decimals above 10k; unchanged below.
 */
function formatBigNumber(value: string | number): string {
  const n = typeof value === "string" ? Number(value) : value;
  if (!Number.isFinite(n)) return String(value);
  const abs = Math.abs(n);
  if (abs >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(2)}B`;
  if (abs >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (abs >= 10_000) return `${(n / 1_000).toFixed(2)}K`;
  return String(value);
}

/**
 * Format an indicator's parameter object in the TradingView style,
 * e.g. `{ length: 14 }` → `(14)`. Only scalar values are included
 * (numbers, strings, booleans) — objects/arrays are skipped. The
 * `name` key itself is filtered out.
 */
function formatIndicatorParams(entry: ActiveIndicator): string | undefined {
  if (typeof entry === "string") return undefined;
  const parts: string[] = [];
  for (const k of Object.keys(entry)) {
    if (k === "name") continue;
    const v = entry[k];
    if (typeof v === "number" || typeof v === "string" || typeof v === "boolean") {
      parts.push(String(v));
    }
  }
  return parts.length > 0 ? `(${parts.join(", ")})` : undefined;
}

/** TradingView-style setStyles payload for a palette. Kept as one function so
 *  future readers can see the whole styled surface in one place. */
function tvStyles(p: Palette) {
  return {
    grid: {
      show: true,
      horizontal: { show: true, color: p.grid, size: 1 },
      vertical: { show: true, color: p.grid, size: 1 },
    },
    candle: {
      bar: {
        upColor: p.bull,
        downColor: p.bear,
        noChangeColor: p.fgMuted,
        upBorderColor: p.bull,
        downBorderColor: p.bear,
        noChangeBorderColor: p.fgMuted,
        upWickColor: p.bull,
        downWickColor: p.bear,
        noChangeWickColor: p.fgMuted,
      },
      priceMark: {
        show: true,
        high: { show: false, color: p.fg, textFamily: FONT_FAMILY },
        low: { show: false, color: p.fg, textFamily: FONT_FAMILY },
        last: {
          show: true,
          upColor: p.bull,
          downColor: p.bear,
          noChangeColor: p.accent,
          line: {
            show: true,
            style: "dashed",
            dashedValue: [4, 4],
            size: 1,
          },
          text: {
            show: true,
            size: 12,
            paddingLeft: 4,
            paddingRight: 4,
            paddingTop: 2,
            paddingBottom: 2,
            color: "#ffffff",
            family: FONT_FAMILY,
            weight: "500",
            borderSize: 0,
            borderColor: "transparent",
            borderRadius: 2,
          },
        },
      },
      // Suppress the canvas-drawn candle tooltip — the DOM PaneHeader
      // renders the readouts in a styled, consistent format.
      tooltip: {
        showRule: "none",
        showType: "standard",
      },
    },
    indicator: {
      // Same story on the indicator side: the PaneHeader overlay is the
      // single source of truth, so the canvas tooltip is silenced.
      tooltip: {
        showRule: "none",
        showName: false,
        showParams: false,
      },
      lastValueMark: {
        show: false,
      },
    },
    xAxis: {
      axisLine: { show: true, color: p.border, size: 1 },
      tickLine: { show: true, color: p.border, size: 1, length: 3 },
      tickText: {
        show: true,
        color: p.fgMuted,
        family: FONT_FAMILY,
        weight: "400",
        size: 11,
      },
    },
    yAxis: {
      axisLine: { show: true, color: p.border, size: 1 },
      tickLine: { show: true, color: p.border, size: 1, length: 3 },
      tickText: {
        show: true,
        color: p.fgMuted,
        family: FONT_FAMILY,
        weight: "400",
        size: 11,
      },
    },
    crosshair: {
      show: true,
      horizontal: {
        show: true,
        line: {
          show: true,
          style: "dashed",
          dashedValue: [4, 2],
          size: 1,
          color: p.crosshair,
        },
        text: {
          show: true,
          color: p.labelFg,
          size: 11,
          family: FONT_FAMILY,
          weight: "400",
          backgroundColor: p.labelBg,
          borderColor: p.border,
        },
      },
      vertical: {
        show: true,
        line: {
          show: true,
          style: "dashed",
          dashedValue: [4, 2],
          size: 1,
          color: p.crosshair,
        },
        text: {
          show: true,
          color: p.labelFg,
          size: 11,
          family: FONT_FAMILY,
          weight: "400",
          backgroundColor: p.labelBg,
          borderColor: p.border,
        },
      },
    },
    separator: {
      size: 1,
      color: p.border,
      fill: true,
      activeBackgroundColor: p.border,
    },
  } as const;
}

const CANDLE_PANE_ID = "candle_pane";
// Matches the two-decimal compact volume in the legend.
const VOLUME_PRECISION = 2;
// New bars in one tail update up to which each is appended with its own
// updateData (one full recalc each); more is applied in one step.
const MAX_INCREMENTAL_BARS = 5;

interface HeaderState {
  /** Series key (indicator request name); absent for the price pane. */
  key?: string;
  name: string;
  params?: string;
  lines: PaneHeaderLine[];
  legend?: BarLegend;
  top: number;
  /** Compare symbol this legend row belongs to. */
  compare?: string;
}

/** klinecharts `candle` style overrides per toolbar chart type. */
const CHART_TYPE_STYLES: Record<ChartType, object> = {
  candles: { type: "candle_solid" },
  heikin_ashi: { type: "candle_solid" },
  hollow: { type: "candle_up_stroke" },
  bars: { type: "ohlc" },
  line: {
    type: "area",
    area: { backgroundColor: "rgba(0,0,0,0)", point: { show: false } },
  },
  area: {
    type: "area",
    area: {
      backgroundColor: [
        { offset: 0, color: "rgba(41,98,255,0.28)" },
        { offset: 1, color: "rgba(41,98,255,0.02)" },
      ],
      point: { show: false },
    },
  },
};

export interface CompareLine {
  symbol: string;
  color: string;
  values: Array<number | null>;
}

export interface ChartProps {
  /** Current series payload; `null` before the first load. */
  series: SeriesPayload | null;
  /**
   * The active indicator registry from the store. Used to source
   * per-indicator parameter tuples for the header; the server's
   * payload only carries the indicator's `label`, not the params the
   * client asked for.
   */
  activeIndicators: ActiveIndicator[];
  /** Client-side plot style overrides (ADR-0017), keyed by indicator name. */
  styles?: Record<string, IndicatorStyle | undefined>;
  /** Price rendering style chosen in the toolbar. */
  chartType: ChartType;
  /** IANA zone of the axis, crosshair and legend times. */
  timeZone: string;
  /** Legend controls, keyed by indicator name (series key). */
  onToggleHidden?: (name: string) => void;
  onSettings?: (name: string) => void;
  onRemove?: (name: string) => void;
  onChartReady?: (chart: KChart) => void;
  /**
   * Compare symbols (ADR-0022): one line per entry, `values` aligned to
   * `series.bars` (null where the symbol has no close yet).
   */
  compare?: CompareLine[];
  onRemoveCompare?: (symbol: string) => void;
  /**
   * What changed since the previous `series` (lib/liveFeed.ts). A
   * prepended page or a tail update that directly follows the last
   * applied snapshot is fed to klinecharts incrementally, so the view
   * keeps its position; anything else is a full `applyNewData`.
   */
  update?: SeriesUpdate | null;
  /** History paging state, shown at the left edge. */
  history?: HistoryStatus;
  /**
   * Called when the view reaches the oldest bar, and by the Retry
   * button. True while a page load is in flight; false if there is no
   * older history.
   */
  onLoadOlder?: () => boolean;
}

export function Chart(props: ChartProps): JSX.Element {
  let host!: HTMLDivElement;
  let container!: HTMLDivElement;
  const [chart, setChart] = createSignal<KChart | null>(null);
  // Map of indicator name → klinecharts pane id. Drives create/remove
  // decisions across effect runs.
  const [paneIds, setPaneIds] = createSignal<Record<string, string>>({});
  const [headers, setHeaders] = createSignal<HeaderState[]>([]);
  // Crosshair state drives the header readout. null = no crosshair
  // (resting state → show last finite values).
  const [crosshairIndex, setCrosshairIndex] = createSignal<number | null>(
    null,
  );
  // True while the oldest loaded bar is in view.
  const [atOldest, setAtOldest] = createSignal(false);
  // First visible bar index: the base of the percent axis and of the compare lines.
  const [firstVisible, setFirstVisible] = createSignal(0);
  // Computed once per series, not per crosshair frame. pricePrecision
  // samples the last 50 closes, so only those are mapped: this reruns on
  // every live tick.
  const pricePrecisionForSeries = createMemo(() =>
    pricePrecision(props.series?.bars.slice(-50).map((b) => b.c) ?? []),
  );
  let crosshairFrame: number | null = null;
  let layoutFrame: number | null = null;
  // Version of the last snapshot handed to klinecharts (null: none).
  let appliedVersion: number | null = null;
  // klinecharts's "load older data" callback while a page is on its way.
  // Holding it keeps klinecharts from asking again, which is what makes
  // page loads one at a time; an error keeps it held until Retry.
  let pendingOlder: ((data: KLineData[], more?: boolean) => void) | null =
    null;
  // True once the compare indicator sits on the candle pane.
  let compareCreated = false;
  // Names of the indicators currently requested (visible). Panes and legends
  // of anything else are dropped right away, before a refetch lands. A string
  // so param-only changes do not re-run the pane effect.
  const visibleKey = createMemo(() =>
    props.activeIndicators.map(indicatorName).join("\n"),
  );

  onMount(() => {
    const c = init(container, {
      // DeepPartial on purpose — we only override the leaves we care
      // about. klinecharts merges the rest from its default theme.
      styles: tvStyles(PALETTES[untrack(theme)]) as never,
      customApi: { formatBigNumber },
    });
    setChart(c);
    if (c) props.onChartReady?.(c);

    // klinecharts asks for more data when the view reaches either end.
    // "forward" is older bars (prepended); "backward" would be newer ones,
    // which only ever come from the live tail, so that one is answered
    // with "no more" at once (an unanswered request blocks the other).
    c?.setLoadDataCallback(({ type, callback }) => {
      if (type !== LoadDataType.Forward) {
        callback([], false);
        return;
      }
      pendingOlder = callback;
      if (!(props.onLoadOlder?.() ?? false)) {
        pendingOlder = null;
        callback([], false);
      }
    });

    // Wire the crosshair subscription. klinecharts fires an event on
    // every mouse move; we debounce into a single rAF tick so Solid
    // doesn't re-render 60+ times a second for a pixel-level change.
    //
    // klinecharts only executes the onCrosshairChange action when the
    // crosshair is over a pane (it checks `isString(crosshair.paneId)`),
    // which means we never see an event when the mouse leaves the
    // chart. A mouseleave listener on the host clears the state.
    const onCrosshair = (data?: unknown) => {
      if (crosshairFrame !== null) return;
      const payload = data as
        | { dataIndex?: number; paneId?: string }
        | undefined;
      crosshairFrame = requestAnimationFrame(() => {
        crosshairFrame = null;
        if (!payload || typeof payload.dataIndex !== "number") {
          setCrosshairIndex(null);
        } else {
          setCrosshairIndex(payload.dataIndex);
        }
      });
    };
    // Use the string form of the action type to avoid pulling in the
    // runtime enum (TS-only import-elision friendly).
    c!.subscribeAction("onCrosshairChange" as never, onCrosshair);
    // The history-limit notice shows only while the oldest bar is in view.
    const onRange = (range?: unknown) => {
      const from = (range as { from?: number } | undefined)?.from;
      setAtOldest(from === 0);
      if (typeof from === "number") setFirstVisible(from);
    };
    c!.subscribeAction("onVisibleRangeChange" as never, onRange);
    const onLeave = () => setCrosshairIndex(null);
    host.addEventListener("mouseleave", onLeave);
    onCleanup(() => {
      c!.unsubscribeAction("onCrosshairChange" as never, onCrosshair);
      c!.unsubscribeAction("onVisibleRangeChange" as never, onRange);
      host.removeEventListener("mouseleave", onLeave);
      if (crosshairFrame !== null) cancelAnimationFrame(crosshairFrame);
    });

    // Re-layout pane headers when the chart resizes. A ResizeObserver
    // on the host catches both viewport changes and indicator-pane
    // toggles.
    if (host && typeof ResizeObserver !== "undefined") {
      const ro = new ResizeObserver(() => {
        chart()?.resize();
        recomputeHeaders();
      });
      ro.observe(host);
      onCleanup(() => ro.disconnect());
    }
  });

  const recomputeHeaders = () => {
    const c = chart();
    const series = props.series;
    if (!c || !series) {
      setHeaders([]);
      return;
    }
    const next: HeaderState[] = [];
    const xi = crosshairIndex();

    // Price pane header: symbol, interval and the OHLC legend of the
    // crosshair bar, or of the last bar at rest.
    const bars = series.bars;
    const legendIndex =
      xi !== null && xi >= 0 && xi < bars.length
        ? xi
        : lastFiniteCloseIndex(bars);
    const priceLegend = barLegend(bars, legendIndex, pricePrecisionForSeries());
    const barTime = bars[legendIndex]?.t;
    next.push({
      name: series.symbol ?? "price",
      params: series.interval ? `· ${series.interval}` : undefined,
      lines: [],
      legend:
        priceLegend && barTime != null
          ? { ...priceLegend, time: formatBarTime(barTime * 1000, props.timeZone, series.interval) }
          : (priceLegend ?? undefined),
      top: 4,
    });

    // Per-indicator headers. Overlays stack below the price-pane row
    // on the candle pane; oscillators anchor at their pane's top.
    const paneMap = paneIds();
    const paramsByName = new Map<string, string | undefined>();
    for (const entry of props.activeIndicators) {
      paramsByName.set(indicatorName(entry), formatIndicatorParams(entry));
    }
    let overlayRow = 0;

    for (const name of Object.keys(series.indicators)) {
      const ind = series.indicators[name]!;
      if (!paneMap[name]) continue; // pane not yet created (or skipped)

      if ((ind.lines?.length ?? 0) + (ind.histogram?.length ?? 0) === 0) continue;
      const headerLines: PaneHeaderLine[] = legendPlots(
        name,
        ind,
        props.styles?.[name],
      ).map((p) => {
        const values =
          (p.kind === "line" ? ind.lines![p.index] : ind.histogram![p.index])!
            .values;
        return {
          label: p.label,
          value: xi !== null ? valueAt(values, xi) : lastFinite(values),
          color: p.color,
        };
      });

      const displayName = ind.label && ind.label.length > 0 ? ind.label : name;
      const params = paramsByName.get(name);

      if (ind.is_overlay) {
        overlayRow += 1;
        // 20px per row below the 4px price-header row — keeps overlays
        // readable without a DOM-measured layout pass.
        const top = 4 + 20 * overlayRow;
        next.push({ key: name, name: displayName, params, lines: headerLines, top });
      } else {
        const size = c.getSize(paneMap[name]!);
        const top = size ? size.top + 4 : 0;
        next.push({ key: name, name: displayName, params, lines: headerLines, top });
      }
    }

    // Compare legend rows stack below the overlay rows.
    (props.compare ?? []).forEach((line, i) => {
      const reading = compareReading(line.values, firstVisible(), xi);
      next.push({
        name: line.symbol,
        lines: [
          { label: "", value: reading?.value ?? null, color: line.color },
          {
            label: "",
            value: null,
            text: reading ? formatPct(reading.pct) : undefined,
            color: line.color,
          },
        ],
        top: 4 + 20 * (overlayRow + 1 + i),
        compare: line.symbol,
      });
    });

    setHeaders(next);
  };

  // Repaint the canvas chrome on theme change; data is untouched.
  createEffect(() => {
    chart()?.setStyles(tvStyles(PALETTES[theme()]) as never);
  });

  createEffect(() => {
    chart()?.setStyles({
      candle: CHART_TYPE_STYLES[props.chartType],
    } as never);
  });

  // The axis and crosshair labels are klinecharts' own; the legend's time
  // is ours.
  createEffect(() => {
    chart()?.setTimezone(props.timeZone);
    recomputeHeaders();
  });

  // React to crosshair changes by recomputing header values only.
  createEffect(() => {
    crosshairIndex();
    recomputeHeaders();
  });

  // Line labels per indicator: overrides are matched by label (ADR-0017), so
  // they are re-applied only when a payload changes the labels.
  const labelSignature = createMemo(() =>
    JSON.stringify(
      Object.entries(props.series?.indicators ?? {}).map(([n, i]) => [
        n,
        i.lines?.map((l) => l.label),
        i.histogram?.length ?? 0,
      ]),
    ),
  );

  // Apply plot style overrides to the live indicators: one recalculation and
  // repaint, no new data and no server round trip. Runs on style or label
  // changes only; panes and the payload are read untracked.
  createEffect(() => {
    const c = chart();
    const styles = props.styles;
    labelSignature();
    if (!c) return;
    const panes = untrack(paneIds);
    const indicators = untrack(() => props.series?.indicators);
    for (const name of Object.keys(panes)) {
      c.overrideIndicator(
        {
          name: indicatorRegName(name),
          extendData: extendFor(name, indicators?.[name], styles?.[name]),
        },
        panes[name],
      );
    }
  });

  /**
   * Feed a live-feed change to klinecharts without losing the scroll
   * position (a bare `applyNewData` snaps to the newest bar). False if
   * klinecharts does not hold what the change builds on; the caller then
   * does a full apply.
   */
  const applyIncremental = (
    c: KChart,
    series: SeriesPayload,
    update: SeriesUpdate,
    names: string[],
  ): boolean => {
    if (update.kind === "older") {
      // Prepend through the held "load older" callback: klinecharts keeps
      // the view anchored to the newest bar, so nothing on screen moves.
      // The prepended page's indices shift every geometry index, so the
      // store is refreshed first; the prepend recalcs and repaints.
      // An empty page (count 0) ends the history, whatever has_more says.
      const done = pendingOlder;
      if (!done) return false;
      pendingOlder = null;
      storeGeometry(series, names);
      done(
        seriesToKLines(series, 0, update.count),
        update.count > 0 && canPageBefore(series),
      );
      return true;
    }
    if (update.kind !== "tail") return false;
    const list = c.getDataList();
    const { from, prevLength } = update;
    const head = series.bars[from]?.t;
    if (list.length !== prevLength || head == null) return false;
    if (list[from]?.timestamp !== head * 1000) return false;
    // The tail merge updates fill arrays and divline lists in place, which
    // the stored geometry shares (storeGeometry); only a list the merge
    // had to create is not shared yet. Merged data is always timed.
    for (const name of names) {
      const ind = series.indicators[name];
      const stored = geometryStore.get(indicatorRegName(name));
      if (ind && stored && stored.divlines !== ind.divlines) {
        storeGeometry(series, [name], true);
      }
    }
    const klines = seriesToKLines(series, from);
    const kept = prevLength - from;
    const added = klines.length - kept;
    if (added > MAX_INCREMENTAL_BARS) {
      // A catch-up after missed bars: one updateData per bar would recalc
      // every indicator per bar. Apply the whole list once instead and
      // move the view back to where it was (applyNewData snaps it to the
      // newest bar): the bar at the left edge returns to the same pixel.
      // When the newest bar was in view the chart keeps following it.
      const range = c.getVisibleRange();
      const following = range.to >= prevLength;
      const anchor = list[Math.min(range.from, prevLength - 1)]!.timestamp;
      const xOf = () =>
        (
          c.convertToPixel({ timestamp: anchor }, {
            paneId: CANDLE_PANE_ID,
          }) as { x?: number }
        ).x;
      const x = xOf();
      // A held "load older" callback stays valid: it prepends to whatever
      // list klinecharts holds, which is still this series.
      c.applyNewData(seriesToKLines(series), canPageBefore(series));
      const x2 = xOf();
      if (!following && x !== undefined && x2 !== undefined) {
        c.scrollByDistance(x - x2);
      }
      return true;
    }
    // Bars we already have are updated in place; new ones go through
    // updateData, which appends and, when the view is scrolled back,
    // keeps it where it is. Each updateData recalcs every indicator over
    // the whole list (klinecharts has no partial recalc), so an unchanged
    // length still needs exactly one call to recalc and repaint.
    for (let i = 0; i < kept; i++) Object.assign(list[from + i]!, klines[i]);
    if (added === 0) c.updateData(list[prevLength - 1]!);
    for (const k of klines.slice(kept)) c.updateData(k);
    return true;
  };

  // Feed new data and manage the indicator pane lifecycle.
  createEffect(() => {
    const c = chart();
    const series = props.series;
    const update = props.update ?? null;
    if (!c) return;
    if (!series) {
      // Failed or cleared load: drop the stale candles and panes.
      appliedVersion = null;
      pendingOlder = null;
      c.clearData();
      const stale = untrack(paneIds);
      for (const name of Object.keys(stale)) {
        c.removeIndicator(stale[name]!, indicatorRegName(name));
        geometryStore.delete(indicatorRegName(name));
      }
      setPaneIds({});
      setHeaders([]);
      return;
    }

    const visible = new Set(visibleKey().split("\n"));
    const names = Object.keys(series.indicators).filter((n) => visible.has(n));
    // Axis ticks, the last-price badge and the legend share one precision.
    const precision = pricePrecisionForSeries();

    // A page or tail right after the last applied snapshot builds on what
    // klinecharts holds. Anything else (a reset, a skipped version, a
    // re-run for a visibility change) re-applies the whole series.
    const follows =
      update !== null &&
      appliedVersion !== null &&
      update.version === appliedVersion + 1;
    appliedVersion = update?.version ?? null;
    if (follows && applyIncremental(c, series, update, names)) {
      c.setPriceVolumePrecision(precision, VOLUME_PRECISION);
      return;
    }

    // 1. Register any indicators we haven't seen before. Must happen
    //    before applyNewData so klinecharts's calc pipeline has the
    //    figure config to draw against. The geometry store is filled here
    //    too, in data space, so applyNewData's recalc and repaint see it.
    for (const name of names) {
      const shape = shapeFromPayload(series, name);
      if (isEmptyShape(shape)) continue;
      ensureRegistered(name, shape);
    }
    storeGeometry(series, names);

    // 2. Push bar data with all per-indicator stashes applied. applyNewData
    //    drops any held "load older" request; `canPageBefore` (more history,
    //    under the cap) tells klinecharts whether to ask for older bars at
    //    the left edge.
    pendingOlder = null;
    const klines = seriesToKLines(series);
    c.applyNewData(klines, canPageBefore(series));
    c.setPriceVolumePrecision(precision, VOLUME_PRECISION);

    // 3. Pane lifecycle. Create for new indicators, remove for dropped.
    // untrack: this effect writes paneIds below, so reading it tracked
    // would re-run the effect forever (stack overflow).
    const currentPanes = untrack(paneIds);
    const nextPanes: Record<string, string> = {};
    const activeNames = new Set<string>();
    for (const name of names) {
      const shape = shapeFromPayload(series, name);
      if (isEmptyShape(shape)) continue; // skip empty
      activeNames.add(name);
      const regName = indicatorRegName(name);
      if (currentPanes[name]) {
        nextPanes[name] = currentPanes[name]!;
        continue;
      }
      const ind = series.indicators[name]!;
      // Overlays attach to the candle pane; oscillators get their own
      // pane (isStack=true stacks panes bottom-up).
      const paneOptions = ind.is_overlay
        ? { id: CANDLE_PANE_ID }
        : { height: 120 };
      const newPaneId = c.createIndicator(
        {
          name: regName,
          extendData: extendFor(
            name,
            ind,
            untrack(() => props.styles?.[name]),
          ),
        },
        true,
        paneOptions,
      );
      if (newPaneId !== null) nextPanes[name] = newPaneId;
    }
    for (const name of Object.keys(currentPanes)) {
      if (!activeNames.has(name)) {
        c.removeIndicator(currentPanes[name]!, indicatorRegName(name));
        geometryStore.delete(indicatorRegName(name));
      }
    }
    setPaneIds(nextPanes);

    // klinecharts lays out panes synchronously inside createIndicator,
    // but getSize reads through the renderer — one microtask is enough
    // for pane heights to settle before we position the DOM headers.
    queueMicrotask(recomputeHeaders);
    // Pane tops are not final yet in the microtask (the RSI legend landed
    // on the price row); re-measure once the layout has settled.
    layoutFrame = requestAnimationFrame(recomputeHeaders);
  });

  // Compare lines: stash the aligned closes on the bars and (re)calculate the
  // indicator from the first visible bar. Declared after the data effect so a
  // new series is in klinecharts before its values are stashed.
  createEffect(() => {
    const c = chart();
    const lines = props.compare ?? [];
    const series = props.series;
    const from = firstVisible();
    if (!c) return;
    if (lines.length === 0) {
      if (compareCreated) c.removeIndicator(CANDLE_PANE_ID, COMPARE_REG_NAME);
      compareCreated = false;
      return;
    }
    const list = c.getDataList();
    // Not in step with the series yet (a load in flight): a later run stashes.
    if (!series || list.length !== series.bars.length) return;
    for (let slot = 0; slot < MAX_COMPARE; slot++) {
      const values = lines[slot]?.values;
      const key = compareStashKey(slot);
      for (let i = 0; i < list.length; i++) {
        list[i]![key] = values?.[i] ?? null;
      }
    }
    ensureCompareRegistered();
    const extendData: CompareExtend = { from, colors: lines.map((l) => l.color) };
    if (compareCreated) {
      c.overrideIndicator({ name: COMPARE_REG_NAME, extendData }, CANDLE_PANE_ID);
    } else {
      c.createIndicator({ name: COMPARE_REG_NAME, extendData }, true, {
        id: CANDLE_PANE_ID,
      });
      compareCreated = true;
    }
  });

  onCleanup(() => {
    if (layoutFrame !== null) cancelAnimationFrame(layoutFrame);
    if (container) dispose(container);
  });

  return (
    <div ref={host} class="chart-host" tabindex="-1">
      <div ref={container} class="chart-container" />
      <Index each={headers()}>
        {(h) => (
          <PaneHeader
            name={h().name}
            params={h().params}
            lines={h().lines}
            legend={h().legend}
            top={h().top}
            testId={
              h().compare !== undefined
                ? `compare-legend-${h().compare}`
                : undefined
            }
            onToggleHidden={
              h().key !== undefined && props.onToggleHidden
                ? () => props.onToggleHidden!(h().key!)
                : undefined
            }
            onSettings={
              h().key !== undefined && props.onSettings
                ? () => props.onSettings!(h().key!)
                : undefined
            }
            onRemove={
              h().compare !== undefined && props.onRemoveCompare
                ? () => props.onRemoveCompare!(h().compare!)
                : h().key !== undefined && props.onRemove
                  ? () => props.onRemove!(h().key!)
                  : undefined
            }
          />
        )}
      </Index>
      <PriceAxisControls chart={chart()} series={props.series} />
      <Show
        when={
          props.history?.kind === "loading" ||
          props.history?.kind === "error" ||
          (props.history?.kind === "limit" && atOldest())
        }
      >
        <div
          class="history-status"
          classList={{ "is-error": props.history?.kind === "error" }}
          data-testid="history-status"
          data-state={props.history?.kind}
          title={
            props.history?.kind === "error" ? props.history.message : undefined
          }
        >
          <Switch>
            <Match when={props.history?.kind === "loading"}>
              <span class="history-spinner" aria-hidden="true" />
              Loading history…
            </Match>
            <Match when={props.history?.kind === "error"}>
              Older bars failed to load
              <button type="button" onClick={() => props.onLoadOlder?.()}>
                Retry
              </button>
            </Match>
            <Match when={props.history?.kind === "limit"}>
              History limit reached
            </Match>
          </Switch>
        </div>
      </Show>
    </div>
  );
}
