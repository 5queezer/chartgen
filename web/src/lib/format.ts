// Pure formatting for the price legend (O H L C, change, volume).
// No DOM or klinecharts imports so it stays trivially unit-testable.

import type { SeriesPayload } from "../types/responses";

type SeriesBar = SeriesPayload["bars"][number];

const MAX_PRECISION = 12;
const MIN_PRECISION = 2;
const SAMPLE_BARS = 50;

/**
 * Decimals needed to print `v` exactly, but at least enough for three
 * significant digits so tiny prices (1.234e-9) do not collapse to 0.00.
 * Capped at MAX_PRECISION.
 */
function decimalsOf(v: number): number {
  const s = v.toFixed(MAX_PRECISION).replace(/0+$/, "");
  const dot = s.indexOf(".");
  const fixed = dot === -1 ? 0 : s.length - dot - 1;
  const significant = v === 0 ? 0 : -Math.floor(Math.log10(Math.abs(v))) + 3;
  return Math.min(MAX_PRECISION, Math.max(fixed, significant));
}

/** Upper bound on useful decimals by magnitude, so float noise is not shown. */
function magnitudeCap(abs: number): number {
  if (abs >= 1000) return 2;
  if (abs >= 100) return 3;
  if (abs >= 1) return 5;
  if (abs >= 0.01) return 6;
  return MAX_PRECISION;
}

/**
 * Price precision derived from the data: the most decimals any of the
 * last bars' closes needs, at least 2 and capped by magnitude (2 for
 * >=1000, 3 for >=100, 5 for >=1, 6 for >=0.01, else 12).
 */
export function pricePrecision(values: ReadonlyArray<number | null>): number {
  const finite = values
    .slice(-SAMPLE_BARS)
    .filter((v): v is number => v !== null && Number.isFinite(v));
  if (finite.length === 0) return MIN_PRECISION;
  const cap = magnitudeCap(Math.max(...finite.map(Math.abs)));
  const needed = Math.max(...finite.map(decimalsOf));
  return Math.max(MIN_PRECISION, Math.min(needed, cap));
}

export function formatPrice(v: number | null, precision: number): string {
  return v === null || !Number.isFinite(v) ? "—" : v.toFixed(precision);
}

/** Compact volume: 1.23K / 4.56M / 7.89B, two decimals, plain below 1000. */
export function formatVolume(v: number | null): string {
  if (v === null || !Number.isFinite(v)) return "—";
  const abs = Math.abs(v);
  if (abs >= 1e9) return `${(v / 1e9).toFixed(2)}B`;
  if (abs >= 1e6) return `${(v / 1e6).toFixed(2)}M`;
  if (abs >= 1e3) return `${(v / 1e3).toFixed(2)}K`;
  return v.toFixed(2);
}

export type Direction = "up" | "down" | "flat";

export interface Change {
  /** Signed absolute change, e.g. "+1.25". */
  abs: string;
  /** Signed percent change, e.g. "+0.42%". */
  pct: string;
  direction: Direction;
}

/** Change of `close` versus `prevClose`; null when either is unusable. */
export function formatChange(
  close: number | null,
  prevClose: number | null,
  precision: number,
): Change | null {
  if (
    close === null ||
    prevClose === null ||
    !Number.isFinite(close) ||
    !Number.isFinite(prevClose)
  ) {
    return null;
  }
  const diff = close - prevClose;
  const direction: Direction = diff > 0 ? "up" : diff < 0 ? "down" : "flat";
  const sign = diff > 0 ? "+" : diff < 0 ? "-" : "";
  const pct = prevClose === 0 ? 0 : (Math.abs(diff) / Math.abs(prevClose)) * 100;
  return {
    abs: `${sign}${Math.abs(diff).toFixed(precision)}`,
    pct: `${sign}${pct.toFixed(2)}%`,
    direction,
  };
}

export interface BarLegend {
  o: string;
  h: string;
  l: string;
  c: string;
  v: string;
  /** Null for the first bar, which has no previous close. */
  change: Change | null;
  /** Bar open time in the chart's time zone; set by the chart, not `barLegend`. */
  time?: string;
}

function isFinitePrice(v: number | null): v is number {
  return v !== null && Number.isFinite(v);
}

/** Index of the last bar with a finite close, or -1. */
export function lastFiniteCloseIndex(bars: readonly SeriesBar[]): number {
  for (let i = bars.length - 1; i >= 0; i--) {
    if (isFinitePrice(bars[i]!.c)) return i;
  }
  return -1;
}

/**
 * Legend strings for `bars[index]`; null when the index is out of range.
 * The change is versus the most recent earlier bar with a finite close.
 */
export function barLegend(
  bars: readonly SeriesBar[],
  index: number,
  precision: number,
): BarLegend | null {
  const bar = bars[index];
  if (!bar) return null;
  let prevClose: number | null = null;
  for (let i = index - 1; i >= 0; i--) {
    if (isFinitePrice(bars[i]!.c)) {
      prevClose = bars[i]!.c;
      break;
    }
  }
  return {
    o: formatPrice(bar.o, precision),
    h: formatPrice(bar.h, precision),
    l: formatPrice(bar.l, precision),
    c: formatPrice(bar.c, precision),
    v: formatVolume(bar.v),
    change: formatChange(bar.c, prevClose, precision),
  };
}
