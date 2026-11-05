// Compare-symbol maths (ADR-0022). A compare line is another symbol's close
// series, so this is alignment and display normalisation only, never
// indicator maths (ADR-0003).
//
// klinecharts' percent axis (YAxisType.Percentage) maps every value drawn on
// the candle pane, indicators included, to (v - c0) / c0 * 100, where c0 is the
// MAIN series' close at the first visible bar. A compare close in its own price
// units would therefore be normalised by the wrong base. `rebase` puts the line
// in the main series' price space, scaled so that it starts at the main close
// of the first visible bar (0%); the axis then shows exactly the compare
// symbol's own percent change from where it starts.

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
 * As-of join: for each main bar, the close of the last compare bar that
 * opened before the main bar ends (`t < t_main + step`); null before the
 * compare series starts. Joining on the end, not the open, matters when the
 * two sources stamp the same session differently (Binance daily bars open
 * at 00:00 UTC, Yahoo's at the exchange open, hours later). `step` is the
 * main interval in seconds; 1 joins on "at or before". Both inputs are sorted.
 */
export function alignAsOf(
  times: ReadonlyArray<number | null>,
  bars: readonly CompareBar[],
  step = 1,
): Array<number | null> {
  const out: Array<number | null> = [];
  let j = -1;
  for (const t of times) {
    if (t === null) {
      out.push(null);
      continue;
    }
    while (j + 1 < bars.length && bars[j + 1]!.t < t + step) j++;
    out.push(j >= 0 ? bars[j]!.c : null);
  }
  return out;
}

const finite = (v: number | null | undefined): v is number =>
  typeof v === "number" && Number.isFinite(v);

/**
 * Factor that puts a compare line into the main series' price space for the
 * klinecharts percent axis: `closeAt(from) / rawAt(a)`, where `a` is the
 * first bar at or after `from` with a value. The axis reads the main close at
 * `from` as 0%, so the line starts at 0% at `a` and shows the compare symbol's
 * own change from there on, which is also what the legend reports. Null when
 * the line has no value from `from` on.
 */
export function rebaseScale(
  n: number,
  rawAt: (i: number) => number | null | undefined,
  closeAt: (i: number) => number | null | undefined,
  from: number,
): number | null {
  const f = Math.min(Math.max(0, from), n - 1);
  for (let a = f; a < n; a++) {
    const v = rawAt(a);
    if (!finite(v) || v === 0) continue;
    const base = closeAt(f);
    const close = finite(base) ? base : closeAt(a);
    return finite(close) ? close / v : null;
  }
  return null;
}

/** `raw` scaled by `rebaseScale`; all null when there is no scale. */
export function rebase(
  raw: ReadonlyArray<number | null | undefined>,
  closes: ReadonlyArray<number | null | undefined>,
  from: number,
): Array<number | null> {
  const scale = rebaseScale(raw.length, (i) => raw[i], (i) => closes[i], from);
  return raw.map((v) => (scale !== null && finite(v) ? v * scale : null));
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
