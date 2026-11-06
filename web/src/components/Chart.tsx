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
// instance (the payload key: the name, or `rsi#2` for a repeat, ADR-0028).
// When a new instance shows up we `createIndicator`; when one drops out of
// the payload we `removeIndicator` and delete the entry.
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
  batch,
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
  FormatDateType,
  init,
  LoadDataType,
  registerIndicator,
  utils,
  type Chart as KChart,
  type IndicatorDrawCallback,
  type KLineData,
} from "klinecharts";

import {
  barColor,
  figureOverrides,
  DASH,
  figureStyle,
  paletteBarColor,
  plotColorAt,
  plotsOf,
  resolveOverride,
  resolvePlot,
  type PlotInfo,
} from "../lib/plotStyle";
import { keyName } from "../lib/instances";
import { withoutParamNumber } from "../lib/indicatorNames";
import { fitSubPanes } from "../lib/paneLayout";
import {
  applyFill,
  divColor,
  matchFill,
  matchLevel,
  shown,
  visibleFills,
} from "../lib/geometryStyle";
import {
  instanceKey,
  paramLabels,
  plotKey,
  type ActiveIndicator,
  type GeometryStyle,
  type IndicatorStyle,
} from "../stores/indicators";
import { paramDefaults } from "../stores/layouts";
import {
  canPageBefore,
  type HistoryStatus,
  type SeriesUpdate,
} from "../lib/liveFeed";
import { scaleMode, theme, type ChartType } from "../stores/layout";
import {
  compareReading,
  formatPct,
  MAX_COMPARE,
  rebaseScale,
} from "../lib/compare";
import { chartSettings } from "../stores/chartSettings";
import { composeStyles, resolveColors, settingsStyles } from "../lib/chartStyle";
import {
  ensureSeriesRegistered,
  hiddenCandleStyle,
  isSeriesType,
  SERIES_REG_NAME,
  type SeriesExtend,
  type SeriesType,
} from "../lib/chartTypes";
import { FONT_FAMILY, PALETTES, type Palette } from "../lib/theme";
import type { SeriesPayload } from "../types/responses";
import {
  abbreviate,
  barLegend,
  formatIndicatorValue,
  formatPrice,
  isEmptyVolumePane,
  indicatorValueKind,
  isVolumeIndicator,
  lastFiniteCloseIndex,
  pricePrecision,
  type BarLegend,
} from "../lib/format";
import { readableOn } from "../lib/color";
import { formatBarTime, isDateBar } from "../lib/timezone";
import {
  divlineSegments,
  fillColors,
  fillPolygons,
  hbarRects,
  hbarTiers,
  hlineStrokes,
  payloadToDataIndex,
  toDataSpace,
  valueExtents,
  type Mapper,
  type SeriesIndicator,
} from "./geometry";
import {
  FUTURE_LEGEND,
  fitProjection,
  futureFills,
  futureLines,
  futureTime,
  futureValue,
  inProjection,
  projectionBars,
  setShownProjection,
} from "../lib/projection";
import { PaneHeader, type PaneHeaderLine } from "./PaneHeader";
import { PriceAxisControls } from "./PriceAxisControls";

// klinecharts' own date format options, in UTC.
const UTC_FORMAT = new Intl.DateTimeFormat("en", {
  hour12: false,
  timeZone: "UTC",
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});

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
 * Figure-style mapping for user plot overrides (ADR-0017) over the server's
 * plot colors (ADR-0026). The user's `IndicatorStyle` and the plots'
 * defaults ride on the klinecharts indicator instance as its `extendData`,
 * so a style change is one `overrideIndicator` (which recalculates and
 * repaints) and never touches the data or the server. `bar` is the server
 * color of the histogram bar being drawn: it beats the plot default, not
 * the user's color.
 */
function plotFigureStyle(
  name: string,
  kind: "line" | "hist",
  plotIndex: number,
  colorIdx: number,
  indicator: { extendData?: unknown },
  bar?: string,
): Record<string, unknown> {
  const ext = indicator.extendData as FigureExtend | undefined;
  const key = plotKey(kind, plotIndex);
  const d = ext?.defaults[key];
  const o = ext?.plots[key];
  const server = bar === undefined ? undefined : paletteBarColor(bar, o?.palette);
  return figureStyle(
    kind,
    resolveOverride(o, server ?? d?.color ?? paletteColor(name, colorIdx), d?.width),
  );
}

/**
 * klinecharts `extendData` of an indicator: the user's overrides and the
 * plots' default color and width (server's, else the palette's), both keyed
 * by figure key for the plots of the current payload (labels already matched).
 */
interface FigureExtend extends IndicatorStyle {
  defaults: Record<string, { color: string; width: number }>;
}

function extendFor(
  name: string,
  ind: NonNullable<SeriesPayload["indicators"][string]> | undefined,
  style: IndicatorStyle | undefined,
): FigureExtend {
  if (!ind) return NO_STYLE;
  const plots = plotsOf(ind, (n) => paletteColor(name, n), name);
  const defaults = Object.fromEntries(
    plots.map((p) => [
      plotKey(p.kind, p.index),
      { color: p.defaultColor, width: p.defaultWidth },
    ]),
  );
  return { ...figureOverrides(plots, style), defaults };
}

/** Extend data of an indicator without overrides; klinecharts ignores an undefined override. */
const NO_STYLE: FigureExtend = { plots: {}, defaults: {} };

/** Plots of an indicator the legend shows: hidden plots are omitted. */
function legendPlots(
  name: string,
  ind: NonNullable<SeriesPayload["indicators"][string]>,
  style: IndicatorStyle | undefined,
): PlotInfo[] {
  return plotsOf(ind, (n) => paletteColor(name, n), name).filter(
    (p) => resolvePlot(p, style).visible,
  );
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

/** Stash key of a histogram bar's server color (ADR-0026). */
function histColorStashKey(name: string, i: number): string {
  return `chartgen_${name}_hist${i}_color`;
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
  return ({ ctx, kLineDataList, indicator, visibleRange, bounding, xAxis, yAxis }) => {
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
    const geo = (indicator.extendData as FigureExtend | undefined)?.geometry;
    const paint = (polys: ReturnType<typeof fillPolygons>, fi: number) => {
      const o = matchFill(geo?.fills?.[fi], fills[fi]!);
      if (!shown(o)) return;
      const theme = applyFill(fillColors(fills[fi]!, name, fi), o);
      for (const poly of polys) {
        ctx.fillStyle = poly.up ? theme.up : theme.down;
        ctx.beginPath();
        poly.points.forEach((p, k) =>
          k === 0 ? ctx.moveTo(p.x, p.y) : ctx.lineTo(p.x, p.y),
        );
        ctx.closePath();
        ctx.fill();
      }
    };
    for (let fi = fills.length - 1; fi >= 0; fi--) {
      paint(fillPolygons(fills[fi]!.y1, fills[fi]!.y2, visibleRange, map), fi);
    }
    // The cloud past the last bar (ADR-0033), joined to it at the last bar.
    const length = kLineDataList.length;
    for (const run of futureFills(ind, length)) {
      if (!fills[run.index]) continue;
      const at: Mapper = { x: (i) => map.x(run.from + i), y: map.y };
      const all = { from: 0, to: run.values.y1.length };
      paint(fillPolygons(run.values.y1, run.values.y2, all, at), run.index);
    }
    ctx.globalCompositeOperation = "source-over";
    drawFutureLines(ctx, name, ind, length, indicator, map);

    if (ind.hbars && ind.hbars.length > 0 && shown(geo?.hbars)) {
      const tiers = hbarTiers(ind.hbars, ind.hlines ?? []);
      for (const r of hbarRects(
        ind.hbars,
        bounding.width,
        bounding.height,
        map.y,
      )) {
        const tier = tiers[r.index];
        const own =
          tier === "poc"
            ? geo?.hbars?.pocColor
            : tier === "va"
              ? geo?.hbars?.vaColor
              : geo?.hbars?.color;
        ctx.fillStyle = own ?? ind.hbars[r.index]!.color!;
        ctx.fillRect(r.x, r.y, r.width, r.height);
      }
    }

    const segments = shown(geo?.divlines)
      ? divlineSegments(ind.divlines ?? [], visibleRange, map, ind.is_overlay)
      : [];
    ctx.lineWidth = 2;
    for (const s of segments) {
      ctx.strokeStyle = divColor(s.kind, s.color, geo?.divlines);
      ctx.setLineDash(s.dashed ? [4, 4] : []);
      ctx.beginPath();
      ctx.moveTo(s.from.x, s.from.y);
      ctx.lineTo(s.to.x, s.to.y);
      ctx.stroke();
    }
    drawHLines(ctx, ind, bounding, map.y, geo?.hlines);
    return false;
  };
}

/**
 * Projected lines past the last bar (ADR-0033) in their plot's style; the
 * line figures klinecharts draws stop at the last bar. Hidden plots are
 * skipped.
 */
function drawFutureLines(
  ctx: CanvasRenderingContext2D,
  name: string,
  ind: SeriesIndicator,
  length: number,
  indicator: { extendData?: unknown },
  map: Mapper,
): void {
  const ext = indicator.extendData as FigureExtend | undefined;
  for (const run of futureLines(ind, length)) {
    if (ext?.plots[plotKey("line", run.index)]?.visible === false) continue;
    const style = plotFigureStyle(name, "line", run.index, run.index, indicator) as {
      color: string;
      size: number;
      dashedValue?: number[];
    };
    ctx.strokeStyle = style.color;
    ctx.lineWidth = style.size;
    ctx.setLineDash(style.dashedValue ?? []);
    ctx.beginPath();
    let pen = false;
    run.values.forEach((v, i) => {
      if (v === null) {
        pen = false;
        return;
      }
      const x = map.x(run.from + i);
      const y = map.y(v);
      if (pen) ctx.lineTo(x, y);
      else ctx.moveTo(x, y);
      pen = true;
    });
    ctx.stroke();
  }
  ctx.setLineDash([]);
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
  overrides: GeometryStyle["hlines"],
): void {
  ctx.lineWidth = 1;
  for (const h of hlineStrokes(ind.hlines ?? [], bounding.height, mapY)) {
    const o = matchLevel(
      overrides?.[h.index],
      ind.hlines![h.index]!,
      (ind.hbars?.length ?? 0) > 0,
    );
    if (!shown(o)) continue;
    ctx.strokeStyle = o?.color ?? h.color;
    ctx.setLineDash(o?.lineStyle && o.lineStyle !== "solid" ? DASH[o.lineStyle] : []);
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
    styles?: (
      data: { current: { kLineData?: KLineData } },
      indicator: { extendData?: unknown },
    ) => Record<string, unknown>;
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
    // Each bar takes its server color (stashed on its KLineData) unless the
    // user set the histogram's color.
    figures.push({
      key: `hist${i}`,
      title: "",
      type: "bar",
      baseValue: 0,
      styles: (d, ind) =>
        plotFigureStyle(
          name,
          "hist",
          i,
          colorIdx,
          ind,
          d.current.kLineData?.[histColorStashKey(name, i)] as string | undefined,
        ),
    });
  }

  registerIndicator<Row>({
    name: regName,
    shortName: name.toUpperCase(),
    precision: 2,
    shouldFormatBigNumber: isVolumeIndicator(keyName(name)),
    figures: figures as never,
    draw: drawGeometry(name),
    calc: (dataList: KLineData[], indicator) => {
      const geo = geometryStore.get(regName);
      // A hidden plot's value is null, so nothing is drawn for it.
      const ext = indicator.extendData as IndicatorStyle | undefined;
      // Hidden fills do not scale the axis.
      const extents = geo
        ? valueExtents(visibleFills(geo, ext?.geometry?.fills), dataList.length)
        : [];
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
// Double underscore: server indicators register as `chartgen_<name>` and
// their names start with a letter, so this can never be one of them.
const COMPARE_REG_NAME = "chartgen__compare";
const compareStashKey = (slot: number) => `chartgen_cmp_${slot}`;

/**
 * Extend data of the compare indicator: the bar the lines start at (0%), the
 * bar window `calc` fills (the visible bars plus a margin, so a scroll or a
 * new bar needs no recalculation of its own) and the line colors.
 */
interface CompareExtend {
  from: number;
  lo: number;
  hi: number;
  colors: string[];
}

let compareRegistered = false;

/**
 * Register (once) the indicator drawing every compare line on the candle
 * pane. Each bar carries the symbol's as-of aligned close under
 * `chartgen_cmp_<slot>`; `calc` only rescales it into the main series' price
 * space (`rebaseScale`) from the first visible bar, which is what makes the
 * klinecharts percent axis (normalised by the main close at that bar) read the
 * compare symbol's own percent change. Display normalisation, no indicator
 * maths (ADR-0003). Only the window of `extendData` is filled; klinecharts
 * reads a missing row as no value.
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
      const n = dataList.length;
      const out: Row[] = new Array(n);
      if (!ext || n === 0) return out;
      const keys = ext.colors.map((_, k) => compareStashKey(k));
      const scales = keys.map((key) =>
        rebaseScale(
          n,
          (i) => dataList[i]![key] as number | null | undefined,
          (i) => dataList[i]!.close,
          ext.from,
        ),
      );
      const hi = Math.min(n - 1, ext.hi);
      for (let i = Math.max(0, ext.lo); i <= hi; i++) {
        const row: Row = {};
        keys.forEach((key, k) => {
          const v = dataList[i]![key];
          row[`cmp${k}`] =
            scales[k] !== null && typeof v === "number" && Number.isFinite(v)
              ? v * scales[k]!
              : null;
        });
        out[i] = row;
      }
      return out;
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
  compare: readonly CompareLine[] = [],
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
        const color = barColor(hist[hi], i);
        if (color !== undefined) kline[histColorStashKey(name, hi)] = color;
      }
    }
    compare.forEach((line, k) => {
      kline[compareStashKey(k)] = line.values[i] ?? null;
    });
    klines.push(kline);
  }
  return klines;
}

/** Index of the last finite value, or -1. */
function lastFiniteIndex(values: Array<number | null>): number {
  for (let i = values.length - 1; i >= 0; i--) {
    const v = values[i];
    if (typeof v === "number" && Number.isFinite(v)) return i;
  }
  return -1;
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
 * volume readout of volume-type panes: at most two decimals, no `.00`.
 */
function formatBigNumber(value: string | number): string {
  const n = typeof value === "string" ? Number(value) : value;
  return Number.isFinite(n) ? abbreviate(n) : String(value);
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
    // klinecharts' default overlay text family is "Helvetica Neue", which
    // falls back to a serif face where it is not installed.
    overlay: {
      text: { family: FONT_FAMILY },
      rectText: { family: FONT_FAMILY },
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

/** klinecharts `candle` style overrides per toolbar chart type it draws itself. */
const CHART_TYPE_STYLES: Record<Exclude<ChartType, SeriesType>, object> = {
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
  /** Client-side plot style overrides (ADR-0017), keyed by instance key. */
  styles?: Record<string, IndicatorStyle | undefined>;
  /** Price rendering style chosen in the toolbar. */
  chartType: ChartType;
  /** IANA zone of the axis, crosshair and legend times. */
  timeZone: string;
  /** Legend controls, keyed by instance key (series key, ADR-0028). */
  onToggleHidden?: (name: string) => void;
  onSettings?: (name: string) => void;
  onRemove?: (name: string) => void;
  /** Alert on an indicator's plots (ADR-0034); offered on rows with lines. */
  onAddAlert?: (name: string) => void;
  onChartReady?: (chart: KChart) => void;
  /** Bar index under the crosshair (one update per frame); null when it is off the chart. */
  onCrosshair?: (index: number | null) => void;
  /** Bar the compare percent is measured from (the view start on a percent axis, else 0). */
  onCompareFrom?: (index: number) => void;
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
  // Map of instance key → klinecharts pane id. Drives create/remove
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
  // Visible bars, one animation frame behind klinecharts: the base of the
  // percent axis and the window of the compare lines.
  const [firstVisible, setFirstVisible] = createSignal(0);
  const [lastVisible, setLastVisible] = createSignal(0);
  // Compare lines already stashed on klinecharts's bars, and the extend data
  // the indicator was last calculated with.
  let appliedCompare: readonly CompareLine[] | null = null;
  let compareExt: CompareExtend | null = null;
  let rangeFrame: number | null = null;
  // Computed once per series, not per crosshair frame. pricePrecision
  // samples the last 50 closes, so only those are mapped: this reruns on
  // every live tick.
  const pricePrecisionForSeries = createMemo(() =>
    pricePrecision(props.series?.bars.slice(-50).map((b) => b.c) ?? []),
  );
  // True while the series is daily or longer: the crosshair label then shows
  // the date alone. Read by klinecharts' formatDate below.
  let dateBars = false;
  // Oscillator pane keys, top to bottom, as klinecharts stacks them.
  let paneStack: string[] = [];
  // Shown indicator keys of the last applied snapshot.
  let appliedNames = "";
  // Indicator keys of the payload that are shown, in the stored order
  // (ADR-0028) rather than the payload's.
  const shownNames = (series: SeriesPayload): string[] => {
    const visible = visibleKey().split("\n");
    return visible.filter((k) => k in series.indicators && !isEmptyVolumePane(series.bars, k));
  };
  let crosshairFrame: number | null = null;
  let layoutFrame: number | null = null;
  let layoutTimer: ReturnType<typeof setTimeout> | null = null;
  let styleFrame: number | null = null;
  let pendingStyles: object = {};
  // Width of the price axis when it sits on the left; pane headers start
  // after it so they do not cover its labels.
  const [axisInset, setAxisInset] = createSignal(0);
  // Version of the last snapshot handed to klinecharts (null: none).
  let appliedVersion: number | null = null;
  // klinecharts's "load older data" callback while a page is on its way.
  // Holding it keeps klinecharts from asking again, which is what makes
  // page loads one at a time; an error keeps it held until Retry.
  let pendingOlder: ((data: KLineData[], more?: boolean) => void) | null =
    null;
  // True once the compare indicator sits on the candle pane.
  let compareCreated = false;
  // True once the series indicator of a client-drawn chart type is on it.
  let seriesCreated = false;
  // Instance keys of the indicators currently requested (visible). Panes and
  // legends of anything else are dropped right away, before a refetch lands.
  // A string so param-only changes do not re-run the pane effect.
  const visibleKey = createMemo(() =>
    props.activeIndicators.map(instanceKey).join("\n"),
  );

  onMount(() => {
    const c = init(container, {
      // DeepPartial on purpose — we only override the leaves we care
      // about. klinecharts merges the rest from its default theme.
      styles: tvStyles(PALETTES[untrack(theme)]) as never,
      customApi: {
        formatBigNumber,
        // Date bars stand for a UTC date: crosshair and axis ticks read it
        // in UTC whatever the selected zone, so they agree with the legend.
        formatDate: (dtf, ts, fmt, type) =>
          dateBars
            ? utils.formatDate(UTC_FORMAT, ts, type === FormatDateType.XAxis ? fmt : "YYYY-MM-DD")
            : utils.formatDate(dtf, ts, fmt),
      },
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
        | { dataIndex?: number; realDataIndex?: number; paneId?: string }
        | undefined;
      crosshairFrame = requestAnimationFrame(() => {
        crosshairFrame = null;
        if (!payload || typeof payload.dataIndex !== "number") {
          setCrosshairIndex(null);
          return;
        }
        // Right of the last bar klinecharts clamps `dataIndex` to it; over
        // a projection (ADR-0033) the real index reads the future values.
        const real = payload.realDataIndex;
        const series = props.series;
        const shown = Object.keys(untrack(paneIds));
        setCrosshairIndex(
          typeof real === "number" && series && inProjection(series, shown, real)
            ? real
            : payload.dataIndex,
        );
      });
    };
    // Use the string form of the action type to avoid pulling in the
    // runtime enum (TS-only import-elision friendly).
    c!.subscribeAction("onCrosshairChange" as never, onCrosshair);
    // The history-limit notice shows only while the oldest bar is in view.
    const onRange = (range?: unknown) => {
      const r = range as { from?: number; to?: number } | undefined;
      setAtOldest(r?.from === 0);
      if (typeof r?.from !== "number" || typeof r.to !== "number") return;
      const { from, to } = r;
      // One update per frame however many range events a drag fires.
      if (rangeFrame !== null) cancelAnimationFrame(rangeFrame);
      rangeFrame = requestAnimationFrame(() => {
        rangeFrame = null;
        batch(() => {
          setFirstVisible(from);
          setLastVisible(to);
        });
      });
    };
    c!.subscribeAction("onVisibleRangeChange" as never, onRange);
    const onLeave = () => setCrosshairIndex(null);
    host.addEventListener("mouseleave", onLeave);
    // A touch away from the crosshair hides it, without an action or a
    // mouseleave. This runs after klinecharts' own listener, on the bubble.
    const onTouch = () => {
      const store = (
        c as unknown as {
          getChartStore: () => { getTooltipStore: () => { getCrosshair: () => { x?: number } } };
        }
      ).getChartStore();
      if (store.getTooltipStore().getCrosshair().x === undefined) setCrosshairIndex(null);
    };
    host.addEventListener("touchstart", onTouch);
    host.addEventListener("touchend", onTouch);
    onCleanup(() => {
      c!.unsubscribeAction("onCrosshairChange" as never, onCrosshair);
      c!.unsubscribeAction("onVisibleRangeChange" as never, onRange);
      host.removeEventListener("mouseleave", onLeave);
      host.removeEventListener("touchstart", onTouch);
      host.removeEventListener("touchend", onTouch);
      if (crosshairFrame !== null) cancelAnimationFrame(crosshairFrame);
      if (rangeFrame !== null) cancelAnimationFrame(rangeFrame);
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

  // Legend text must stay readable on the chart background: light plot
  // colors are darkened on a light background (the swatches keep theirs).
  const legendTextColor = (color: string) =>
    readableOn(color, chartSettings().themes[theme()].background ?? PALETTES[theme()].bg);

  const recomputeHeaders = () => {
    const c = chart();
    setAxisInset(
      c && chartSettings().options.scalePlacement === "left"
        ? (c.getSize(CANDLE_PANE_ID, "yAxis" as never)?.width ?? 0)
        : 0,
    );
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
    const paneMap = paneIds();
    // Over the projection past the last bar (ADR-0033) the legend reads the
    // projected values at the future bar's time; there is no OHLC there.
    const shown = Object.keys(paneMap);
    const future = xi !== null && inProjection(series, shown, xi) ? xi : null;
    const legendIndex =
      xi !== null && xi >= 0 && xi < bars.length
        ? xi
        : lastFiniteCloseIndex(bars);
    const priceLegend =
      future !== null
        ? FUTURE_LEGEND
        : barLegend(bars, legendIndex, pricePrecisionForSeries());
    const barTime =
      future !== null ? futureTime(series, shown, future) : bars[legendIndex]?.t;
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
    // TradingView-style params, e.g. `(14)`; repeated indicators show their
    // defaults too, so the instances read apart (ADR-0028).
    const paramsByKey = paramLabels(props.activeIndicators, paramDefaults());
    let overlayRow = 0;

    for (const name of shownNames(series)) {
      const ind = series.indicators[name]!;
      if (!paneMap[name]) continue; // pane not yet created (or skipped)

      if ((ind.lines?.length ?? 0) + (ind.histogram?.length ?? 0) === 0) continue;
      const style = props.styles?.[name];
      const headerLines: PaneHeaderLine[] = legendPlots(name, ind, style).map((p) => {
        const plot =
          p.kind === "line" ? ind.lines![p.index]! : ind.histogram![p.index]!;
        const values = plot.values;
        const kind = indicatorValueKind(name, ind.is_overlay);
        const text = (value: number | null) =>
          formatIndicatorValue(value, kind, pricePrecisionForSeries());
        // Over the projection (ADR-0033) a line reads its projected value.
        if (future !== null) {
          const value = p.kind === "line" ? futureValue(ind, p.index, bars.length, future) : null;
          return {
            label: p.label,
            value,
            text: text(value),
            color: legendTextColor(plotColorAt(p, ind, style, legendIndex)),
          };
        }
        // At rest a plot reads its last finite value; a line that breaks at
        // trend flips (Supertrend, SAR) reads the bar the price legend reads,
        // so its inactive side shows none rather than a stale one. A
        // histogram's legend takes the color of the bar it reads.
        const breaks = p.kind === "line" && "breaks" in plot && plot.breaks === true;
        const at =
          xi !== null ? xi : breaks ? legendIndex : lastFiniteIndex(values);
        const value = valueAt(values, at);
        return {
          label: p.label,
          value,
          text: text(value),
          color: legendTextColor(plotColorAt(p, ind, style, at)),
        };
      });

      const params = paramsByKey.get(name);
      const displayName = withoutParamNumber(
        ind.label && ind.label.length > 0 ? ind.label : keyName(name),
        params,
      );

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
      const reading = compareReading(
        line.values,
        scaleMode() === "percentage" ? firstVisible() : 0,
        xi,
      );
      next.push({
        name: line.symbol,
        lines: [
          {
            label: "",
            value: reading?.value ?? null,
            text: formatPrice(reading?.value ?? null, pricePrecision(line.values)),
            color: legendTextColor(line.color),
          },
          {
            label: "",
            value: null,
            text: reading ? formatPct(reading.pct) : undefined,
            color: legendTextColor(line.color),
          },
        ],
        top: 4 + 20 * (overlayRow + 1 + i),
        compare: line.symbol,
      });
    });

    setHeaders(next);
  };

  // Repaint the canvas chrome on theme, chart type or settings change; data
  // is untouched. The settings (ADR-0019) merge over the palette styles.
  createEffect(() => {
    const c = chart();
    if (!c) return;
    const palette = PALETTES[theme()];
    const settings = chartSettings();
    // One setStyles call (one relayout), coalesced to a frame: a color drag
    // fires this on every input event.
    const t = props.chartType;
    pendingStyles = composeStyles(
      tvStyles(palette),
      { candle: isSeriesType(t) ? {} : CHART_TYPE_STYLES[t] },
      settingsStyles(palette, settings.themes[theme()], settings.options, t),
      // Last, so no setting brings back the series a custom type hides.
      { candle: isSeriesType(t) ? hiddenCandleStyle(t) : {} },
    );
    styleFrame ??= requestAnimationFrame(() => {
      styleFrame = null;
      chart()?.setStyles(pendingStyles as never);
      recomputeHeaders();
    });
  });

  // The axis and crosshair labels are klinecharts' own; the legend's time
  // is ours.
  createEffect(() => {
    const tz = props.timeZone;
    const c = chart();
    if (!c) return;
    c.setTimezone(tz);
    untrack(recomputeHeaders);
  });

  createEffect(() => props.onCrosshair?.(crosshairIndex()));
  createEffect(() =>
    props.onCompareFrom?.(scaleMode() === "percentage" ? firstVisible() : 0),
  );

  // React to crosshair changes by recomputing header values only.
  createEffect(() => {
    crosshairIndex();
    recomputeHeaders();
  });

  // Line labels per indicator: overrides are matched by label (ADR-0017), so
  // they are re-applied only when a payload changes the labels. The server's
  // line colors and widths (ADR-0026) ride along in `extendData` too.
  const labelSignature = createMemo(() =>
    JSON.stringify(
      Object.entries(props.series?.indicators ?? {}).map(([n, i]) => [
        n,
        i.lines?.map((l) => [l.label, l.color, l.width]),
        i.histogram?.map((h) => [h.label, h.palette]) ?? 0,
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
    compare: readonly CompareLine[],
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
        seriesToKLines(series, 0, update.count, compare),
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
      // Bar replay (ADR-0023): a replay frame (lib/replay.ts) is a new slice
      // per step, so its fills and hlines are new arrays too and re-store.
      // A tail brings a new projection past the last bar too (ADR-0033).
      const replaced =
        ind !== undefined &&
        (stored?.fills !== ind.fills ||
          stored?.hlines !== ind.hlines ||
          stored?.future !== ind.future);
      if (ind && stored && (stored.divlines !== ind.divlines || replaced)) {
        storeGeometry(series, [name], true);
      }
    }
    const klines = seriesToKLines(series, from, series.bars.length, compare);
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
      c.applyNewData(
        seriesToKLines(series, 0, series.bars.length, compare),
        canPageBefore(series),
      );
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
    // Compare values ride on the bars, so a data change needs no second
    // calculation of the compare indicator. Read untracked: a compare-only
    // change is the compare effect's.
    const compare = untrack(() => props.compare ?? []);
    if (!c) return;
    if (!series) {
      // Failed or cleared load: drop the stale candles and panes.
      appliedVersion = null;
      appliedCompare = null;
      pendingOlder = null;
      c.clearData();
      setShownProjection(0);
      const stale = untrack(paneIds);
      for (const name of Object.keys(stale)) {
        c.removeIndicator(stale[name]!, indicatorRegName(name));
        geometryStore.delete(indicatorRegName(name));
      }
      setPaneIds({});
      paneStack = [];
      appliedNames = "";
      setHeaders([]);
      return;
    }

    appliedCompare = compare;
    dateBars = isDateBar(series.interval);
    const names = shownNames(series);
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
    // A pane that appears or goes away with the data (Volume once volume
    // arrives) needs the full path.
    const namesKey = names.join("\n");
    const sameNames = namesKey === appliedNames;
    appliedNames = namesKey;
    if (follows && sameNames && applyIncremental(c, series, update, names, compare)) {
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
    const klines = seriesToKLines(series, 0, series.bars.length, compare);
    // Room right of the last bar for any projection (Ichimoku's cloud), as
    // TradingView extends its right margin; applyNewData scrolls to it.
    setShownProjection(projectionBars(series, names));
    fitProjection(c);
    c.applyNewData(klines, canPageBefore(series));
    c.setPriceVolumePrecision(precision, VOLUME_PRECISION);

    // 3. Pane lifecycle. klinecharts stacks new panes at the bottom and
    // cannot move one, so the oscillator panes are kept in the stored order
    // by keeping the longest matching prefix of the stack and recreating
    // the panes after it.
    // untrack: this effect writes paneIds below, so reading it tracked
    // would re-run the effect forever (stack overflow).
    const currentPanes = untrack(paneIds);
    const nextPanes: Record<string, string> = {};
    const wanted: string[] = [];
    for (const name of names) {
      if (!isEmptyShape(shapeFromPayload(series, name))) wanted.push(name);
    }
    const isOscillator = (n: string) => !series.indicators[n]!.is_overlay;
    const wantedOsc = wanted.filter(isOscillator);
    const stack = paneStack.filter((n) => wantedOsc.includes(n));
    let keep = 0;
    while (keep < stack.length && stack[keep] === wantedOsc[keep]) keep++;
    const rebuilt = new Set([...stack.slice(keep)]);
    // A rebuilt pane keeps the height it had.
    const heights: Record<string, number> = {};
    for (const name of rebuilt) {
      const h = c.getSize(currentPanes[name]!)?.height;
      if (h) heights[name] = h;
    }
    for (const name of Object.keys(currentPanes)) {
      if (!wanted.includes(name) || rebuilt.has(name)) {
        c.removeIndicator(currentPanes[name]!, indicatorRegName(name));
        if (!wanted.includes(name)) geometryStore.delete(indicatorRegName(name));
      }
    }
    paneStack = wantedOsc.slice(0, keep);
    for (const name of wanted) {
      const regName = indicatorRegName(name);
      if (currentPanes[name] && !rebuilt.has(name)) {
        nextPanes[name] = currentPanes[name]!;
        continue;
      }
      const ind = series.indicators[name]!;
      // Overlays attach to the candle pane; oscillators get their own
      // pane (isStack=true stacks panes bottom-up).
      const paneOptions = ind.is_overlay
        ? { id: CANDLE_PANE_ID }
        : { height: heights[name] ?? 120 };
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
      if (newPaneId !== null) {
        nextPanes[name] = newPaneId;
        if (!ind.is_overlay) paneStack.push(name);
      }
    }
    setPaneIds(nextPanes);

    // Keep the price pane usable: shrink the sub-panes first.
    const fitPanes = () => {
      const subIds = wantedOsc.map((n) => nextPanes[n]).filter((id) => !!id);
      const candle = c.getSize(CANDLE_PANE_ID)?.height;
      if (candle === undefined) return;
      const subs = subIds.map((id) => c.getSize(id)?.height ?? 0);
      const fit = fitSubPanes(candle, subs);
      if (fit) subIds.forEach((id, i) => c.setPaneOptions({ id, height: fit[i]! }));
    };
    fitPanes();

    // klinecharts lays out panes synchronously inside createIndicator,
    // but getSize reads through the renderer — one microtask is enough
    // for pane heights to settle before we position the DOM headers.
    queueMicrotask(recomputeHeaders);
    // Pane tops are not final yet in the microtask (the RSI legend landed
    // on the price row); re-measure once the layout has settled.
    if (layoutFrame !== null) cancelAnimationFrame(layoutFrame);
    if (layoutTimer !== null) clearTimeout(layoutTimer);
    layoutFrame = requestAnimationFrame(() => {
      layoutFrame = null;
      fitPanes();
      recomputeHeaders();
    });
    // At first load the container is still settling; fit once more after.
    layoutTimer = setTimeout(() => {
      layoutTimer = null;
      if (untrack(chart) !== c) return;
      fitPanes();
      recomputeHeaders();
    }, 300);
  });

  // Compare lines. A change of the compare data stashes the aligned closes on
  // the bars (the series effect does that itself for a data change); a change
  // of the first visible bar re-anchors the indicator. Outside percent mode
  // the lines are anchored at the first loaded bar and nothing here runs per
  // frame. Declared after the series effect, so a new series is in
  // klinecharts before its values are stashed.
  createEffect(() => {
    const c = chart();
    const lines = props.compare ?? [];
    const percent = scaleMode() === "percentage";
    const from = percent ? firstVisible() : 0;
    const to = percent ? lastVisible() : Infinity;
    if (!c) return;
    if (lines.length === 0) {
      if (compareCreated) c.removeIndicator(CANDLE_PANE_ID, COMPARE_REG_NAME);
      compareCreated = false;
      appliedCompare = null;
      compareExt = null;
      return;
    }
    const series = untrack(() => props.series);
    const list = c.getDataList();
    // Not in step with the series yet (a load in flight): a later run stashes.
    if (!series || list.length !== series.bars.length) return;
    const stash = lines !== appliedCompare;
    if (stash) {
      lines.forEach((line, k) => {
        const key = compareStashKey(k);
        for (let i = 0; i < list.length; i++) {
          list[i]![key] = line.values[i] ?? null;
        }
      });
      appliedCompare = lines;
    }
    const colors = lines.map((l) => l.color);
    const prev = compareExt;
    if (
      !stash &&
      compareCreated &&
      prev &&
      prev.from === from &&
      prev.lo <= from &&
      prev.hi >= to + 1 &&
      prev.colors.join() === colors.join()
    ) {
      return;
    }
    // The visible bars plus one screen on each side.
    const span = Math.max(1, to - from);
    const extendData: CompareExtend = {
      from,
      lo: percent ? Math.max(0, from - span) : 0,
      hi: percent ? to + span : Infinity,
      colors,
    };
    compareExt = extendData;
    ensureCompareRegistered();
    if (compareCreated) {
      c.overrideIndicator({ name: COMPARE_REG_NAME, extendData }, CANDLE_PANE_ID);
    } else {
      c.createIndicator({ name: COMPARE_REG_NAME, extendData }, true, {
        id: CANDLE_PANE_ID,
      });
      compareCreated = true;
    }
  });

  // Chart types klinecharts does not draw (ADR-0030): one indicator on the
  // candle pane paints the series over klinecharts' hidden one. Colors follow
  // the theme and the chart settings like the candles do.
  createEffect(() => {
    const c = chart();
    const t = props.chartType;
    if (!c) return;
    if (!isSeriesType(t)) {
      if (seriesCreated) c.removeIndicator(CANDLE_PANE_ID, SERIES_REG_NAME);
      seriesCreated = false;
      return;
    }
    const colors = resolveColors(PALETTES[theme()], chartSettings().themes[theme()]);
    const extendData: SeriesExtend = {
      type: t,
      up: colors.bodyUp,
      down: colors.bodyDown,
      line: colors.line,
    };
    ensureSeriesRegistered();
    if (seriesCreated) {
      c.overrideIndicator({ name: SERIES_REG_NAME, extendData }, CANDLE_PANE_ID);
    } else {
      c.createIndicator({ name: SERIES_REG_NAME, extendData }, true, {
        id: CANDLE_PANE_ID,
      });
      seriesCreated = true;
    }
  });

  onCleanup(() => {
    if (layoutFrame !== null) cancelAnimationFrame(layoutFrame);
    if (layoutTimer !== null) clearTimeout(layoutTimer);
    if (styleFrame !== null) cancelAnimationFrame(styleFrame);
    if (container) dispose(container);
  });

  return (
    <div
      ref={host}
      class="chart-host"
      tabindex="-1"
      style={{
        "background-color": chartSettings().themes[theme()].background,
        "--axis-inset": `${axisInset()}px`,
        "--legend-bg": chartSettings().themes[theme()].background ?? PALETTES[theme()].bg,
      }}
    >
      <div ref={container} class="chart-container" />
      <Index each={headers()}>
        {(h) => (
          <PaneHeader
            name={h().name}
            params={h().params}
            lines={chartSettings().options.legendIndicators ? h().lines : []}
            legend={h().legend}
            legendShow={{
              ohlc: chartSettings().options.legendOhlc,
              change: chartSettings().options.legendChange,
              volume: chartSettings().options.legendVolume,
            }}
            top={h().top}
            testId={
              h().compare !== undefined
                ? `compare-legend-${h().compare}`
                : h().key !== undefined
                  ? `indicator-legend-${h().key}`
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
            onAddAlert={
              h().key !== undefined &&
              props.onAddAlert &&
              (props.series?.indicators[h().key!]?.lines?.length ?? 0) > 0
                ? () => props.onAddAlert!(h().key!)
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
