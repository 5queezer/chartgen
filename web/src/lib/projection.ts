// Values plotted past the last bar (ADR-0033): a series indicator's `future`
// block, e.g. the Ichimoku cloud's leading spans. Pure helpers for the
// chart's right margin and drawing, the legend / Data Window readouts and
// bar replay. No DOM, no klinecharts: unit-tested in projection.test.ts.

import type { SeriesPayload } from "../types/responses";
import type { BarLegend } from "./format";

type Indicator = SeriesPayload["indicators"][string];
export type SeriesFuture = NonNullable<Indicator["future"]>;
type Values = Array<number | null>;

/** klinecharts' default space right of the last bar, in pixels. */
export const DEFAULT_RIGHT_OFFSET = 80;
/** Empty bars kept right of the projection, as TradingView leaves a few. */
const MARGIN_BARS = 3;

/** Price legend of a future bar: it has no OHLC yet. */
export const FUTURE_LEGEND: BarLegend = { o: "—", h: "—", l: "—", c: "—", v: "—", change: null };

/** Most bars any of the indicators `names` of `series` projects. */
export function projectionBars(
  series: SeriesPayload,
  names: readonly string[],
): number {
  let most = 0;
  for (const n of names) most = Math.max(most, series.indicators[n]?.future?.bars ?? 0);
  return most;
}

/**
 * Right offset (px) that shows `bars` projected bars plus a small margin at
 * `barSpace` px per bar, at most half of a `paneWidth` px pane (a long
 * projection at a wide zoom would push every bar off screen); the default
 * when nothing is projected.
 */
export function rightOffset(bars: number, barSpace: number, paneWidth: number): number {
  if (bars <= 0) return DEFAULT_RIGHT_OFFSET;
  const want = Math.max(DEFAULT_RIGHT_OFFSET, (bars + MARGIN_BARS) * barSpace);
  return paneWidth > 0 ? Math.min(want, Math.max(DEFAULT_RIGHT_OFFSET, paneWidth / 2)) : want;
}

/** The klinecharts calls `fitProjection` makes (a structural subset of its Chart). */
export interface OffsetChart {
  getBarSpace: () => number;
  getSize: (paneId?: string, position?: never) => { width: number } | null;
  setOffsetRightDistance: (distance: number) => void;
}

// Bars the chart's shown indicators project: set by Chart.tsx per full load.
let shownProjection = 0;

/** Record how many bars the shown indicators project (0: none). */
export function setShownProjection(bars: number): void {
  shownProjection = bars;
}

/**
 * Bar width that fits `count` bars plus the shown projection's margin into
 * `width` px (the margin depends on the bar width); null when nothing is
 * projected.
 */
export function projectionBarSpace(count: number, width: number): number | null {
  if (shownProjection <= 0 || count <= 0) return null;
  const free = width / (count + shownProjection + MARGIN_BARS);
  const offset = rightOffset(shownProjection, free, width);
  return (width - offset) / count;
}

/**
 * Size the chart's right margin for the shown projection at its current bar
 * width. klinecharts stores the margin in pixels and returns to it on
 * scrollToRealTime, so call this before every scrollToRealTime and full
 * data load; a wheel zoom keeps the margin in bars and needs nothing.
 */
export function fitProjection(chart: OffsetChart): void {
  const width = chart.getSize("candle_pane", "main" as never)?.width ?? 0;
  chart.setOffsetRightDistance(rightOffset(shownProjection, chart.getBarSpace(), width));
}

/** Future slot (0 = the bar after the last) of bar `index`, or null. */
function slot(future: SeriesFuture | undefined, length: number, index: number): number | null {
  const k = index - length;
  return future && k >= 0 && k < future.bars ? k : null;
}

/** True when bar `index` lies in the projection of an indicator of `names`. */
export function inProjection(
  series: SeriesPayload,
  names: readonly string[],
  index: number,
): boolean {
  return index >= series.bars.length && index < series.bars.length + projectionBars(series, names);
}

/**
 * Value of line `line` of `ind` at bar `index` past the last of `length`
 * bars; null where it projects nothing.
 */
export function futureValue(
  ind: Indicator,
  line: number,
  length: number,
  index: number,
): number | null {
  const k = slot(ind.future, length, index);
  if (k === null) return null;
  const v = ind.future!.lines.find((l) => l.index === line)?.values[k];
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

/** Open time (unix s) of bar `index` past the last bar, from any projection of `names`. */
export function futureTime(
  series: SeriesPayload,
  names: readonly string[],
  index: number,
): number | null {
  for (const n of names) {
    const f = series.indicators[n]?.future;
    const k = slot(f, series.bars.length, index);
    const t = k === null ? undefined : f!.t?.[k];
    if (t !== undefined) return t;
  }
  return null;
}

export interface FutureRun<T> {
  /** Index of the line / fill it continues. */
  index: number;
  /** Data index of the first value: the last bar, so the run joins the plot. */
  from: number;
  values: T;
}

/** Projected lines of `ind`, each led by its value at the last of `length` bars. */
export function futureLines(ind: Indicator, length: number): FutureRun<Values>[] {
  if (!ind.future || length === 0) return [];
  return ind.future.lines.map((l) => ({
    index: l.index,
    from: length - 1,
    values: [ind.lines?.[l.index]?.values[length - 1] ?? null, ...l.values],
  }));
}

/** Projected fills of `ind`, each led by its edges at the last of `length` bars. */
export function futureFills(
  ind: Indicator,
  length: number,
): FutureRun<{ y1: Values; y2: Values }>[] {
  if (!ind.future || length === 0) return [];
  return ind.future.fills.map((f) => {
    const fill = ind.fills?.[f.index];
    return {
      index: f.index,
      from: length - 1,
      values: {
        y1: [fill?.y1[length - 1] ?? null, ...f.y1],
        y2: [fill?.y2[length - 1] ?? null, ...f.y2],
      },
    };
  });
}

/**
 * The projection of `ind` seen from bar `n - 1` (bar replay): the plotted
 * values after it, then the loaded projection, `bars` long. A leading span
 * at a later bar was computed `bars` or more bars earlier, so it is known.
 * `t` is dropped when a bar has no time.
 */
export function sliceFuture(
  ind: Indicator,
  n: number,
  bars: SeriesPayload["bars"],
): SeriesFuture | undefined {
  const f = ind.future;
  if (!f) return undefined;
  const cut = <T>(after: readonly T[], future: readonly T[]): T[] =>
    [...after.slice(n), ...future].slice(0, f.bars);
  const times = cut(
    bars.map((b) => b.t),
    f.t ?? [],
  );
  return {
    bars: f.bars,
    ...(f.t && times.every((t) => t !== null) && { t: times as number[] }),
    lines: f.lines.map((l) => ({
      index: l.index,
      values: cut(ind.lines?.[l.index]?.values ?? [], l.values),
    })),
    fills: f.fills.map((x) => ({
      index: x.index,
      y1: cut(ind.fills?.[x.index]?.y1 ?? [], x.y1),
      y2: cut(ind.fills?.[x.index]?.y2 ?? [], x.y2),
    })),
  };
}
