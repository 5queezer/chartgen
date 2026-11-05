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
 * `barSpace` px per bar; the default when nothing is projected.
 */
export function rightOffset(bars: number, barSpace: number): number {
  return bars > 0
    ? Math.max(DEFAULT_RIGHT_OFFSET, (bars + MARGIN_BARS) * barSpace)
    : DEFAULT_RIGHT_OFFSET;
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
