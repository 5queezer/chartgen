// Compare-symbol maths (ADR-0022). A compare line is another symbol's close
// series, so this is alignment and display normalisation only, never
// indicator maths (ADR-0003).
//
// klinecharts' percent axis (YAxisType.Percentage) maps every value drawn on
// the candle pane, indicators included, to (v - c0) / c0 * 100, where c0 is the
// MAIN series' close at the first visible bar. A compare close in its own price
// units would therefore be normalised by the wrong base. `rebase` puts the line
// in the main series' price space, scaled so that its first visible value sits
// at the main close of that bar; the axis then shows exactly the compare
// symbol's own percent change since that bar.

import type { SeriesPayload } from "../types/responses";

export interface CompareBar {
  /** Unix seconds. */
  t: number;
  c: number;
}

/** Symbols one chart can compare at once. */
export const MAX_COMPARE = 5;

/** Line colors by slot. */
export const COMPARE_PALETTE = [
  "#f23645",
  "#089981",
  "#ff9800",
  "#9c27b0",
  "#00bcd4",
] as const;

/** Time-stamped, finite closes of a series payload, oldest first. */
export function toCompareBars(series: SeriesPayload): CompareBar[] {
  const out: CompareBar[] = [];
  for (const b of series.bars) {
    if (b.t !== null && b.c !== null && Number.isFinite(b.c)) {
      out.push({ t: b.t, c: b.c });
    }
  }
  return out;
}

/**
 * Union of two sorted bar lists by timestamp; on a shared timestamp the bar
 * of `b` wins (a tail replaces the forming bar it overlaps).
 */
export function mergeBars(
  a: readonly CompareBar[],
  b: readonly CompareBar[],
): CompareBar[] {
  const out: CompareBar[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length || j < b.length) {
    const x = a[i];
    const y = b[j];
    if (x && (!y || x.t < y.t)) {
      out.push(x);
      i++;
    } else if (y && (!x || y.t < x.t)) {
      out.push(y);
      j++;
    } else {
      out.push(y!);
      i++;
      j++;
    }
  }
  return out;
}

/**
 * As-of join: for each main bar time, the close of the last compare bar at or
 * before it; null before the compare series starts. Both inputs are sorted.
 */
export function alignAsOf(
  times: ReadonlyArray<number | null>,
  bars: readonly CompareBar[],
): Array<number | null> {
  const out: Array<number | null> = [];
  let j = -1;
  for (const t of times) {
    if (t === null) {
      out.push(null);
      continue;
    }
    while (j + 1 < bars.length && bars[j + 1]!.t <= t) j++;
    out.push(j >= 0 ? bars[j]!.c : null);
  }
  return out;
}

const finite = (v: number | null | undefined): v is number =>
  typeof v === "number" && Number.isFinite(v);

/**
 * Scale `raw` into the main series' price space so the klinecharts percent
 * axis reads the compare symbol's own change since bar `from` (the first
 * visible bar). Anchored at the first bar from `from` where the line and the
 * main close both have a value; all null when there is none.
 */
export function rebase(
  raw: ReadonlyArray<number | null | undefined>,
  closes: ReadonlyArray<number | null | undefined>,
  from: number,
): Array<number | null> {
  let a = -1;
  for (let i = Math.max(0, from); i < raw.length; i++) {
    if (finite(raw[i]) && raw[i] !== 0 && finite(closes[i])) {
      a = i;
      break;
    }
  }
  if (a < 0) return raw.map(() => null);
  const scale = closes[a]! / raw[a]!;
  return raw.map((v) => (finite(v) ? v * scale : null));
}

export interface CompareReading {
  value: number;
  /** Percent change from the first value at or after `from` to `value`. */
  pct: number;
}

/**
 * Legend reading of a line: its value at `at` (the last finite one when null
 * or without a value) and the percent change since the first value from `from`.
 */
export function compareReading(
  raw: ReadonlyArray<number | null | undefined>,
  from: number,
  at: number | null,
): CompareReading | null {
  let start = -1;
  for (let i = Math.max(0, from); i < raw.length; i++) {
    if (finite(raw[i]) && raw[i] !== 0) {
      start = i;
      break;
    }
  }
  if (start < 0) return null;
  let end = at !== null && finite(raw[at]) ? at : -1;
  if (end < 0) {
    for (let i = raw.length - 1; i >= 0; i--) {
      if (finite(raw[i])) {
        end = i;
        break;
      }
    }
  }
  if (end < 0) return null;
  const value = raw[end]!;
  return { value, pct: (value / raw[start]! - 1) * 100 };
}

/** `+1.23%` / `-0.50%`. */
export function formatPct(pct: number): string {
  return `${pct >= 0 ? "+" : ""}${pct.toFixed(2)}%`;
}
