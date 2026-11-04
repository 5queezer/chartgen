// Merging of `generate_chart(format=series)` responses into one series:
// older history pages in front (`mergeOlder`), live tail polls at the end
// (`mergeTail`). The merged series is what the chart renders from. See
// ADR-0009 for the paging/tail contract this implements.
//
// Rules shared by both merges:
// - Per-bar channels (`lines`, `histogram`, `fills`) are joined by bar
//   timestamp; every array stays 1:1 aligned with `bars`.
// - `signals` and `divlines` carry bar indices; the incoming side is
//   shifted into the merged index space.
// - `hbars`, `hlines`, `y_range`, `label` and `is_overlay` always come from
//   the current series. Profile hbars/hlines (vpvr, hvn_lvn, tpo) on a page
//   or tail cover only that slice, so ADR-0009 says to keep the full load's.
//   Every other indicator's hlines and y_range do not depend on the window.
// - Anything that does not line up (other symbol or interval, a bar
//   without a timestamp, unordered or overlapping bars, a different
//   indicator set or channel count) is rejected with a reason instead of
//   being merged, so the caller can fall back to a full reload.
//
// No DOM, no klinecharts: unit-tested in seriesMerge.test.ts.

import type { SeriesPayload } from "../types/responses";

type Indicator = SeriesPayload["indicators"][string];
type DivLine = NonNullable<Indicator["divlines"]>[number];
type Signal = NonNullable<Indicator["signals"]>[number];

export type MergeFailure =
  /** Different `symbol` or `interval` than the current series. */
  | "symbol"
  | "interval"
  /** A bar has `t === null` (no-ticker demo data cannot be merged). */
  | "untimed"
  /** Bars are not strictly increasing in `t`. */
  | "order"
  /** An older page reaches into bars we already have. */
  | "overlap"
  /** A tail starts after our newest bar: bars in between are missing. */
  | "gap"
  /** A tail ends before our newest bar: an outdated response. */
  | "stale"
  /** A tail's timestamps do not line up with ours. */
  | "mismatch"
  /** Different indicators or channel counts, or misaligned arrays. */
  | "shape";

export type MergeResult<T> =
  | ({ ok: true; series: SeriesPayload } & T)
  | { ok: false; reason: MergeFailure };

const fail = (reason: MergeFailure) => ({ ok: false, reason }) as const;

/** Why a series' bars cannot be merged, or null if they can. */
function barsProblem(s: SeriesPayload): MergeFailure | null {
  let prev = -Infinity;
  for (const b of s.bars) {
    if (b.t === null) return "untimed";
    if (b.t <= prev) return "order";
    prev = b.t;
  }
  return null;
}

/**
 * True when `inc` has exactly the indicators of `cur` with the same number
 * of lines, histograms and fills, and every per-bar array of `inc` has one
 * value per bar of `inc`.
 */
function sameShape(cur: SeriesPayload, inc: SeriesPayload): boolean {
  const names = Object.keys(cur.indicators);
  if (names.length !== Object.keys(inc.indicators).length) return false;
  const n = inc.bars.length;
  for (const name of names) {
    const a = cur.indicators[name]!;
    const b = inc.indicators[name];
    if (!b) return false;
    const al = a.lines ?? [];
    const bl = b.lines ?? [];
    const ah = a.histogram ?? [];
    const bh = b.histogram ?? [];
    const af = a.fills ?? [];
    const bf = b.fills ?? [];
    if (al.length !== bl.length || ah.length !== bh.length) return false;
    if (af.length !== bf.length) return false;
    if (bl.some((l) => l.values.length !== n)) return false;
    if (bh.some((h) => h.values.length !== n)) return false;
    if (bf.some((f) => f.y1.length !== n || f.y2.length !== n)) return false;
  }
  return true;
}

/**
 * Checks shared by both merges. `cur`'s own bars are only scanned when
 * `checkCurrent` is set; a tail poll checks just the newest one, so a
 * poll does not walk the whole history.
 */
function headerProblem(
  cur: SeriesPayload,
  inc: SeriesPayload,
  checkCurrent: boolean,
): MergeFailure | null {
  if (cur.symbol !== inc.symbol) return "symbol";
  if (cur.interval !== inc.interval) return "interval";
  const own = checkCurrent
    ? barsProblem(cur)
    : cur.bars.at(-1)?.t === null
      ? "untimed"
      : null;
  return own ?? barsProblem(inc) ?? (sameShape(cur, inc) ? null : "shape");
}

const shiftSignal = (s: Signal, by: number): Signal => ({ ...s, x: s.x + by });

const shiftDivline = (d: DivLine, by: number): DivLine => ({
  ...d,
  x1: d.x1 + by,
  x2: d.x2 + by,
});

const startOf = (d: DivLine) => Math.min(d.x1, d.x2);
const endOf = (d: DivLine) => Math.max(d.x1, d.x2);

/**
 * Identity of a divergence line across responses: the bar it ends on and
 * its style. The server clips a line that crosses into a response's
 * window at that window's first bar, so the same line can arrive with
 * different starts but always with the same end.
 */
function divKey(d: DivLine, bars: SeriesPayload["bars"]): string {
  return `${bars[endOf(d)]?.t ?? endOf(d)}|${d.dashed}`;
}

/**
 * One line per `divKey`, keeping the longest (earliest start); on a tie
 * the later entry wins. Input indices must be in `bars`' index space.
 */
export function dedupeDivlines(
  divlines: DivLine[],
  bars: SeriesPayload["bars"],
): DivLine[] {
  const byKey = new Map<string, DivLine>();
  for (const d of divlines) {
    const key = divKey(d, bars);
    const seen = byKey.get(key);
    if (!seen || startOf(d) <= startOf(seen)) byKey.set(key, d);
  }
  return [...byKey.values()].sort((a, b) => startOf(a) - startOf(b));
}

/**
 * Prepend an older history page (`end_time` = our oldest bar). The page
 * must end strictly before `current` starts. Returns a new series; the
 * inputs are not modified. `count` is the number of bars prepended.
 * `has_more` is taken from the page.
 */
export function mergeOlder(
  current: SeriesPayload,
  page: SeriesPayload,
): MergeResult<{ count: number }> {
  const problem = headerProblem(current, page, true);
  if (problem) return fail(problem);
  const first = current.bars[0]?.t;
  const last = page.bars.at(-1)?.t;
  if (first != null && last != null && last >= first) return fail("overlap");

  const offset = page.bars.length;
  const bars = [...page.bars, ...current.bars];
  const indicators: SeriesPayload["indicators"] = {};
  for (const [name, cur] of Object.entries(current.indicators)) {
    const pg = page.indicators[name]!;
    const merged: Indicator = { ...cur };
    if (cur.lines) {
      merged.lines = cur.lines.map((l, i) => ({
        ...l,
        values: [...pg.lines![i]!.values, ...l.values],
      }));
    }
    if (cur.histogram) {
      merged.histogram = cur.histogram.map((h, i) => ({
        ...h,
        values: [...pg.histogram![i]!.values, ...h.values],
      }));
    }
    if (cur.fills) {
      merged.fills = cur.fills.map((f, i) => ({
        y1: [...pg.fills![i]!.y1, ...f.y1],
        y2: [...pg.fills![i]!.y2, ...f.y2],
      }));
    }
    if (cur.signals || pg.signals) {
      merged.signals = [
        ...(pg.signals ?? []),
        ...(cur.signals ?? []).map((s) => shiftSignal(s, offset)),
      ];
    }
    if (cur.divlines || pg.divlines) {
      merged.divlines = dedupeDivlines(
        [
          ...(pg.divlines ?? []),
          ...(cur.divlines ?? []).map((d) => shiftDivline(d, offset)),
        ],
        bars,
      );
    }
    indicators[name] = merged;
  }
  return {
    ok: true,
    count: offset,
    series: { ...current, bars, indicators, has_more: page.has_more },
  };
}

/** Write `src` over `dst` from index `from`, growing `dst` as needed. */
function overwrite<T>(dst: T[], from: number, src: readonly T[]): void {
  dst.length = from;
  for (const v of src) dst.push(v);
}

/**
 * Merge a live tail poll (the newest bars: the last closed bar and the
 * forming one, or more after a gap) into `current`. The tail's first bar
 * must be one we already have; from there the tail replaces our bars and
 * extends them when the interval rolled over.
 *
 * Takes ownership of `current`: its per-bar arrays are updated in place
 * (only the last few entries change), so a poll costs no O(bars) copy.
 * Validation runs before any write, so a rejected tail leaves `current`
 * untouched. The returned series is a new top-level object sharing those
 * arrays. `from` is the first bar index that changed; bars from there to
 * the end are new or updated.
 */
export function mergeTail(
  current: SeriesPayload,
  tail: SeriesPayload,
): MergeResult<{ from: number }> {
  const problem = headerProblem(current, tail, false);
  if (problem) return fail(problem);
  const bars = current.bars;
  const head = tail.bars[0];
  const newest = bars.at(-1);
  if (!head || !newest) return fail("mismatch");
  if (tail.bars.at(-1)!.t! < newest.t!) return fail("stale");
  if (head.t! > newest.t!) return fail("gap");

  // Our index of the tail's first bar; the tail must match every bar of
  // ours it covers. Searched from the end: the tail overlaps the newest bars.
  let from = bars.length - 1;
  while (from >= 0 && bars[from]!.t! > head.t!) from--;
  if (from < 0 || bars[from]!.t !== head.t) return fail("mismatch");
  for (let i = from; i < bars.length; i++) {
    if (tail.bars[i - from]!.t !== bars[i]!.t) return fail("mismatch");
  }

  overwrite(bars, from, tail.bars);
  for (const [name, cur] of Object.entries(current.indicators)) {
    const tl = tail.indicators[name]!;
    cur.lines?.forEach((l, i) => overwrite(l.values, from, tl.lines![i]!.values));
    cur.histogram?.forEach((h, i) =>
      overwrite(h.values, from, tl.histogram![i]!.values),
    );
    cur.fills?.forEach((f, i) => {
      overwrite(f.y1, from, tl.fills![i]!.y1);
      overwrite(f.y2, from, tl.fills![i]!.y2);
    });
    // Inside the tail window the tail is authoritative (a signal on the
    // forming bar can appear or vanish); before it ours are final.
    if (cur.signals || tl.signals) {
      cur.signals = [
        ...(cur.signals ?? []).filter((s) => s.x < from),
        ...(tl.signals ?? []).map((s) => shiftSignal(s, from)),
      ];
    }
    if (cur.divlines || tl.divlines) {
      const shifted = (tl.divlines ?? []).map((d) => shiftDivline(d, from));
      const confirmed = new Set(shifted.map((d) => divKey(d, bars)));
      // A line of ours that reaches into the tail window survives only if
      // the tail still has it; then ours wins, being unclipped.
      cur.divlines = dedupeDivlines(
        [
          ...(cur.divlines ?? []).filter(
            (d) => endOf(d) < from || confirmed.has(divKey(d, bars)),
          ),
          ...shifted,
        ],
        bars,
      );
    }
  }
  return { ok: true, from, series: { ...current } };
}

/**
 * Bars missing between our newest bar and a tail that starts after it
 * (`mergeTail` → "gap"), from the interval length. An upper bound for
 * session markets, whose closed hours hold no bars.
 */
export function missingBars(
  newestT: number,
  tailFirstT: number,
  intervalSec: number,
): number {
  return Math.max(0, Math.ceil((tailFirstT - newestT) / intervalSec) - 1);
}
