// Data Window values (sidebar tab): everything the legend shows for one bar,
// as plain rows. Pure: no Solid, no klinecharts. The caller supplies the bar
// index under the crosshair (null = resting state, the last bar) and the
// default plot color so this stays testable without the chart.

import type { SeriesPayload } from "../types/responses";
import { compareReading, formatPct } from "./compare";
import {
  barLegend,
  formatPrice,
  formatValue,
  lastFiniteCloseIndex,
  pricePrecision,
  type BarLegend,
} from "./format";
import { plotsOf, resolvePlot } from "./plotStyle";
import type { IndicatorStyle } from "../stores/indicators";
import { formatBarTime } from "./timezone";

export interface DataWindowRow {
  label: string;
  value: string;
  /** Plot color. */
  color: string;
}

export interface DataWindowSection {
  /** Series key (indicator request name). */
  key: string;
  title: string;
  rows: DataWindowRow[];
}

export interface DataWindowData {
  symbol: string;
  interval: string | null;
  /** Bar open time in the chart's zone; the date alone for D/W/M bars. */
  time: string | null;
  /** O H L C, change and volume of the bar; null while there are no bars. */
  bar: BarLegend | null;
  indicators: DataWindowSection[];
  compare: DataWindowRow[];
}

export interface CompareInput {
  symbol: string;
  color: string;
  values: ReadonlyArray<number | null>;
}

export interface DataWindowOptions {
  timeZone: string;
  /** Default color of the n-th plot of indicator `name`. */
  colorAt: (name: string, n: number) => string;
  /** Series keys of the visible indicators; every payload indicator when omitted. */
  visible?: ReadonlySet<string>;
  /** Bar the compare percent is measured from, as in the legend; 0 when omitted. */
  compareFrom?: number;
}

export function dataWindowRows(
  series: SeriesPayload,
  index: number | null,
  styles: Record<string, IndicatorStyle | undefined>,
  compare: readonly CompareInput[],
  opts: DataWindowOptions,
): DataWindowData {
  const bars = series.bars;
  const at =
    index !== null && index >= 0 && index < bars.length
      ? index
      : lastFiniteCloseIndex(bars);
  const precision = pricePrecision(bars.slice(-50).map((b) => b.c));
  const barTime = bars[at]?.t;

  const indicators: DataWindowSection[] = [];
  for (const [key, ind] of Object.entries(series.indicators)) {
    if (opts.visible && !opts.visible.has(key)) continue;
    const rows = plotsOf(ind, (n) => opts.colorAt(key, n)).flatMap((p) => {
      const r = resolvePlot(p, styles[key]);
      if (!r.visible) return [];
      const plot = p.kind === "line" ? ind.lines?.[p.index] : ind.histogram?.[p.index];
      return [
        {
          label: p.label,
          value: formatValue(plot?.values[at] ?? null),
          color: r.color,
        },
      ];
    });
    if (rows.length > 0) indicators.push({ key, title: ind.label || key, rows });
  }

  return {
    symbol: series.symbol ?? "price",
    interval: series.interval,
    time:
      barTime != null
        ? formatBarTime(barTime * 1000, opts.timeZone, series.interval)
        : null,
    bar: barLegend(bars, at, precision),
    indicators,
    compare: compare.map((line) => {
      const reading = compareReading(
        line.values,
        opts.compareFrom ?? 0,
        at === index ? at : null,
      );
      return {
        label: line.symbol,
        value: reading
          ? `${formatPrice(reading.value, pricePrecision(line.values))} (${formatPct(reading.pct)})`
          : "—",
        color: line.color,
      };
    }),
  };
}
