// Pure formatting for the price legend (O H L C, change, volume).
// No DOM or klinecharts imports so it stays trivially unit-testable.

import type { SeriesPayload } from "../types/responses";
import { keyName } from "./instances";

type SeriesBar = SeriesPayload["bars"][number];

const MAX_PRECISION = 12;
const MIN_PRECISION = 2;
const SAMPLE_BARS = 50;

/** Relative error below which a decimal expansion counts as exact (float32 noise is ~6e-8). */
const REL_TOLERANCE = 2e-7;

/**
 * Decimals needed to print `v`, ignoring float noise (Yahoo sends float32,
 * so 385.19 arrives as 385.19000244), but at least enough for three
 * significant digits so tiny prices (1.234e-9) do not collapse to 0.00.
 * Capped at MAX_PRECISION.
 */
function decimalsOf(v: number): number {
  let fixed = MAX_PRECISION;
  for (let d = 0; d <= MAX_PRECISION; d++) {
    if (Math.abs(Number(v.toFixed(d)) - v) <= Math.abs(v) * REL_TOLERANCE) {
      fixed = d;
      break;
    }
  }
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

/**
 * Compact K/M/B number: at most two decimals, no trailing zeros, so
 * 180000000 reads "180M" and 4200 reads "4.2K". The value is rounded before
 * the unit is picked (999995 reads "1M", not "1000K"). Plain below 1000;
 * values under 0.005 keep their significant digits instead of reading "0".
 */
export function abbreviate(v: number): string {
  const abs = Math.abs(v);
  const units: ReadonlyArray<readonly [number, string]> = [[1, ""], [1e3, "K"], [1e6, "M"], [1e9, "B"]];
  let i = abs >= 1e9 ? 3 : abs >= 1e6 ? 2 : abs >= 1e3 ? 1 : 0;
  const hundredths = (u: number) => Math.round(abs / (units[u]![0] / 100));
  let cents = hundredths(i);
  if (cents >= 100_000 && i < 3) cents = hundredths(++i);
  if (abs > 0 && abs < 0.005) {
    const decimals = Math.min(MAX_PRECISION, -Math.floor(Math.log10(abs)) + 1);
    return String(Number(v.toFixed(decimals)));
  }
  return `${v < 0 && cents > 0 ? "-" : ""}${cents / 100}${units[i]![1]}`;
}

/** Compact volume: 1.23K / 4.56M / 7.89B, at most two decimals. */
export function formatVolume(v: number | null): string {
  return v === null || !Number.isFinite(v) ? "—" : abbreviate(v);
}

/** True when `bars` carry no traded volume at all (Yahoo FX): all 0 or null. */
export function hasNoVolume(bars: ReadonlyArray<{ v: number | null }>): boolean {
  return bars.every((b) => b.v === null || b.v === 0 || !Number.isFinite(b.v));
}

/** Indicators whose values are traded quantities, not prices or bounded oscillators. */
const VOLUME_INDICATORS = new Set(["volume", "obv", "ad", "cvd", "oi", "elder_force_index"]);

/** Panel indicators measured in price units (differences of prices, ranges). */
const PRICE_UNIT_INDICATORS = new Set(["atr", "macd", "momentum", "dpo", "awesome_oscillator"]);

export function isVolumeIndicator(name: string): boolean {
  return VOLUME_INDICATORS.has(name.toLowerCase());
}

/** True when the default Volume pane would be empty: a "volume" key and bars with no volume. */
export function isEmptyVolumePane(bars: ReadonlyArray<{ v: number | null }>, key: string): boolean {
  return keyName(key) === "volume" && hasNoVolume(bars);
}

export type IndicatorValueKind = "overlay" | "priceUnit" | "volume" | "oscillator";

/** Value kind of the indicator with instance key `key`. */
export function indicatorValueKind(key: string, isOverlay: boolean): IndicatorValueKind {
  const name = keyName(key).toLowerCase();
  if (VOLUME_INDICATORS.has(name)) return "volume";
  if (isOverlay) return "overlay";
  return PRICE_UNIT_INDICATORS.has(name) ? "priceUnit" : "oscillator";
}

/**
 * Legend text of one indicator value. Overlays are in price units and share
 * the symbol's price precision; volume-type indicators use K/M/B; the rest
 * (oscillators) keep their own precision.
 */
export function formatIndicatorValue(
  v: number | null,
  kind: IndicatorValueKind,
  pricePrec: number,
): string {
  if (kind === "overlay") return formatPrice(v, pricePrec);
  if (kind === "priceUnit") {
    // Price precision, widened to two significant digits for values far
    // below the price (ATR of a 0.00001 coin).
    if (v === null || !Number.isFinite(v) || v === 0) return formatPrice(v, pricePrec);
    const significant = Math.min(MAX_PRECISION, -Math.floor(Math.log10(Math.abs(v))) + 1);
    return formatPrice(v, Math.max(pricePrec, significant));
  }
  if (kind === "volume") return formatVolume(v);
  return formatValue(v);
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

export function formatValue(v: number | null): string {
  if (v === null || !Number.isFinite(v)) return "—";
  const abs = Math.abs(v);
  // Four decimals under 1, two decimals between 1-10_000, K/M above.
  if (abs >= 1_000_000) return `${(v / 1_000_000).toFixed(2)}M`;
  if (abs >= 10_000) return `${(v / 1_000).toFixed(2)}K`;
  if (abs >= 1) return v.toFixed(2);
  return v.toFixed(4);
}
