// Pure parts of the order-flow chart types (ADR-0048): what fits on screen, what a
// cell shows, and what the summary tables read. The numbers come from the server;
// this only derives what depends on the bars on screen.

import { barCloseMs } from "./countdown";
import type { FootprintBar, FootprintMeta, TpoMeta, TpoProfile } from "../types/responses";

/** Narrower than this a bar has no room for cells: it draws as a candle. */
export const CELL_MIN_BAR = 22;
/** Bars at least this wide show numbers in their cells and an info box under them. */
export const TEXT_MIN_BAR = 34;
/** Rows at least this tall show numbers. */
export const TEXT_MIN_ROW = 9;
/** Rows thinner than this are not worth drawing: the row size is too fine here. */
export const CELL_MIN_ROW = 2;

export type CellMode = "cells" | "cells_text" | "candles";

export function cellMode(barPx: number, rowPx: number): CellMode {
  if (barPx < CELL_MIN_BAR || rowPx < CELL_MIN_ROW) return "candles";
  return barPx >= TEXT_MIN_BAR && rowPx >= TEXT_MIN_ROW ? "cells_text" : "cells";
}

/** Which of the 4 gradient colours a share of the bar's largest row takes. */
export function gradientIndex(ratio: number): 0 | 1 | 2 | 3 {
  if (ratio < 0.25) return 0;
  if (ratio < 0.5) return 1;
  return ratio < 0.75 ? 2 : 3;
}

/** A short number for a cell: 950, 1.2K, 3.4M. */
export function shortNumber(n: number): string {
  const a = Math.abs(n);
  const trim = (x: number) => (x >= 100 ? x.toFixed(0) : x.toFixed(1).replace(/\.0$/, ""));
  const sign = n < 0 ? "-" : "";
  if (a >= 1e9) return `${sign}${trim(a / 1e9)}B`;
  if (a >= 1e6) return `${sign}${trim(a / 1e6)}M`;
  if (a >= 1e3) return `${sign}${trim(a / 1e3)}K`;
  if (a >= 10 || a === 0) return `${sign}${a.toFixed(0)}`;
  return `${sign}${a.toPrecision(2)}`;
}

/**
 * Where a stacked imbalance of the bar at `start` ends: the first later bar whose
 * range touches it, or null while price has not come back.
 */
export function stackedEnd(
  zone: { low: number; high: number },
  start: number,
  bars: readonly { high: number; low: number }[],
): number | null {
  for (let i = start + 1; i < bars.length; i++) {
    const b = bars[i]!;
    if (b.low <= zone.high && b.high >= zone.low) return i;
  }
  return null;
}

/**
 * Running delta per bar, restarting on each UTC day below daily bars, keyed by open
 * time. It runs from the first loaded bar of the day, so with a day only partly
 * loaded it is not the day's whole CVD.
 */
export function sessionCvd(
  bars: readonly Pick<FootprintBar, "t" | "buy" | "sell">[],
  intraday: boolean,
): Map<number, number> {
  const out = new Map<number, number>();
  let sum = 0;
  let day: number | null = null;
  for (const b of bars) {
    const d = Math.floor(b.t / 86_400);
    if (intraday && d !== day) sum = 0;
    day = d;
    sum += b.buy - b.sell;
    out.set(b.t, sum);
  }
  return out;
}

const percent = (part: number, whole: number) =>
  whole > 0 ? `${((part / whole) * 100).toFixed(1)}%` : "-";

/** The rows of the footprint summary table, in order, for one bar. */
export function footprintSummary(
  fp: FootprintBar,
  prev: FootprintBar | undefined,
  cvd: number | undefined,
  rowSize: number,
  formatPrice: (p: number | null | undefined) => string,
): [string, string][] {
  const volume = fp.buy + fp.sell;
  const delta = fp.buy - fp.sell;
  const before = prev ? prev.buy + prev.sell : 0;
  const high = fp.rows.at(-1)?.[0];
  const low = fp.rows[0]?.[0];
  return [
    ["Volume", shortNumber(volume)],
    ["Vol chg", prev && before > 0 ? `${(((volume - before) / before) * 100).toFixed(0)}%` : "-"],
    ["Buy", shortNumber(fp.buy)],
    ["Buy %", percent(fp.buy, volume)],
    ["Sell", shortNumber(fp.sell)],
    ["Sell %", percent(fp.sell, volume)],
    ["Delta", shortNumber(delta)],
    ["Delta %", percent(delta, volume)],
    ["Min Δ", shortNumber(fp.min_delta)],
    ["Max Δ", shortNumber(fp.max_delta)],
    ["CVD", cvd === undefined ? "-" : shortNumber(cvd)],
    ["POC", formatPrice(fp.poc)],
    ["HL range", high === undefined || low === undefined ? "-" : formatPrice(high + rowSize - low)],
  ];
}

/** The rows of the TPO summary table, in order, for one profile. */
export function tpoSummary(
  p: TpoProfile,
  formatPrice: (n: number) => string,
): [string, string][] {
  return [
    ["HL range", formatPrice(p.high - p.low)],
    ["VA range", formatPrice(p.vah - p.val)],
    ["VAH", formatPrice(p.vah)],
    ["VAL", formatPrice(p.val)],
    ["POC", formatPrice(p.poc)],
    ["Volume", shortNumber(p.volume)],
    ["TPO blocks", String(p.total_blocks)],
    ["Above POC", String(p.blocks_above_poc)],
    ["Below POC", String(p.blocks_below_poc)],
    ["Rotation", String(p.rotation_factor)],
    ["IB high", formatPrice(p.ib_high)],
    ["IB low", formatPrice(p.ib_low)],
    ["IB range", formatPrice(p.ib_high - p.ib_low)],
  ];
}

/** Width of a volume candle: from 1px to the bar's room, by its share of the most. */
export function volumeCandleWidth(volume: number, most: number, room: number): number {
  if (!(most > 0) || !(volume > 0)) return 1;
  return Math.max(1, Math.min(1, volume / most) * room);
}

/** Width of a block in a profile `width` px wide: its longest row fits, up to a letter's. */
export function tpoCellWidth(width: number, longestRow: number, letter = 9): number {
  if (longestRow <= 0) return letter;
  return Math.max(1, Math.min(letter, width / longestRow));
}

/** Block `k` of `total` in the hue range of its letter case: A-Z, then a-z. */
export function tpoHue(block: number): number {
  const lower = block % 52 >= 26;
  const within = (block % 26) / 25;
  return lower ? 280 + within * 60 : 190 + within * 80;
}

/** Index of the first of `times` (ascending) at or after `t`. */
export function firstAtOrAfter(times: readonly { timestamp: number }[], t: number): number {
  let lo = 0;
  let hi = times.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (times[mid]!.timestamp < t) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/** Where a profile starts on the chart: its first block. `t` is its period's start. */
export function profileStart(p: Pick<TpoProfile, "t" | "block_times">): number {
  return p.block_times[0] ?? p.t;
}

/**
 * Which of the first `count` of `opens` (ascending, unix seconds; one more may follow,
 * for the end of the last) no profile overlaps. A bar reaches to the next one's open:
 * a daily bar is stamped at midnight and its profile starts at the session's open.
 */
export function barsWithoutProfile(
  opens: readonly number[],
  profiles: Iterable<Pick<TpoProfile, "t" | "end" | "block_times">>,
  count: number,
): number[] {
  const spans = [...profiles]
    .map((p) => [profileStart(p), p.end] as const)
    .sort((a, b) => a[0] - b[0]);
  const out: number[] = [];
  let k = 0;
  for (let i = 0; i < count; i++) {
    const open = opens[i]!;
    const close = opens[i + 1] ?? Infinity;
    while (k < spans.length && spans[k]![1] <= open) k++;
    const span = spans[k];
    if (!span || span[0] >= close) out.push(i);
  }
  return out;
}

/** What the status chip says about how a footprint was made. */
export function footprintMethod(meta: {
  intrabar_interval: string | null;
  side: string;
  row_size: number;
  row_auto: boolean;
  row_widened: boolean;
  bars_without_data: number;
  coarser_older?: boolean;
}): string {
  const parts = [
    meta.intrabar_interval ? `from ${meta.intrabar_interval} bars` : "from the bars themselves",
    meta.side === "taker_volume" ? "taker volume" : "close vs open",
    `row ${meta.row_size}${meta.row_auto ? " auto" : ""}${meta.row_widened ? " (widened)" : ""}`,
  ];
  if (meta.bars_without_data > 0) parts.push(`${meta.bars_without_data} bars without data`);
  if (meta.coarser_older) parts.push("older bars from coarser data");
  return parts.join(" · ");
}

/** Most bars one `get_footprint` returns, and the periods of a TPO load. */
export const FOOTPRINT_MAX_TAIL = 200;
export const TPO_MAX_TAIL = 10;

/** Bars a footprint poll asks for `gap` seconds on: the last closed, the forming, and any missed. */
export function footprintTail(gap: number, timeframe: string): number {
  const close = barCloseMs(0, timeframe);
  return close ? Math.ceil(gap / (close / 1000)) + 2 : 3;
}

const TPO_PERIOD_SECS: Record<string, number> = {
  "1D": 86_400,
  "2D": 2 * 86_400,
  "1W": 7 * 86_400,
  "1M": 31 * 86_400,
};

/** Periods a TPO poll asks for: the current one and the one before, whose end it may have missed. */
export function tpoTail(gap: number, period: string): number {
  return Math.ceil(gap / (TPO_PERIOD_SECS[period] ?? 86_400)) + 1;
}

/**
 * A footprint poll or page over its load. Only the load chose the row size: a later
 * reply's bars carry theirs, and that it had to widen them still gets said.
 */
export function mergeFootprintMeta(load: FootprintMeta, later: FootprintMeta): FootprintMeta {
  return {
    ...later,
    row_size: load.row_size,
    row_auto: load.row_auto,
    row_widened: load.row_widened || later.row_widened,
    intrabar_interval: load.intrabar_interval,
    bars_without_data: Math.max(load.bars_without_data, later.bars_without_data),
    coarser_older:
      !!load.coarser_older || later.intrabar_interval !== load.intrabar_interval,
  };
}

/** A TPO poll or page over its load: as a footprint's (`mergeFootprintMeta`). */
export function mergeTpoMeta(load: TpoMeta, later: TpoMeta): TpoMeta {
  return {
    ...later,
    row_size: load.row_size,
    row_auto: load.row_auto,
    row_widened: load.row_widened || later.row_widened,
    periods_cut: load.periods_cut,
  };
}

/** What the status chip says about a TPO chart. */
export function tpoMethod(meta: {
  period: string;
  block: string;
  block_widened?: boolean;
  row_size: number;
  row_auto: boolean;
  row_widened: boolean;
  periods_cut: boolean;
}): string {
  const parts = [
    `${meta.period} from ${meta.block} blocks${meta.block_widened ? " (shorter ones cannot cover it)" : ""}`,
    `row ${meta.row_size}${meta.row_auto ? " auto" : ""}${meta.row_widened ? " (widened)" : ""}`,
  ];
  if (meta.periods_cut) parts.push("fewer periods: too many blocks");
  return parts.join(" · ");
}
