// Merging of `generate_chart(format=series)` responses into one series:
// older history pages in front (`mergeOlder`), live tail polls at the end
// (`mergeTail`). The merged series is what the chart renders from. See
// ADR-0009 for the paging/tail contract this implements.
//
// Rules shared by both merges:
// - Per-bar channels (`lines`, `histogram`, `fills`) are joined by bar
//   timestamp; every array stays 1:1 aligned with `bars`. A histogram's
//   per-bar `color_idx` (ADR-0026) is joined the same way, its incoming
//   indices remapped into the current `palette`, which grows by any color
//   it lacks.
// - `signals` and `divlines` carry bar indices; the incoming side is
//   shifted into the merged index space.
// - `hbars`, `hlines`, `y_range`, `label` and `is_overlay` always come from
//   the current series. Profile hbars/hlines (vpvr, hvn_lvn, tpo) on a page
//   or tail cover only that slice, so ADR-0009 says to keep the full load's.
//   Every other indicator's hlines and y_range do not depend on the window.
// - `future`, the values past the last bar (ADR-0033), belongs to the newest
//   bar: a tail's replaces ours, an older page's is ignored.
// - `labels` (ADR-0046) belong to their bar, and merge like `signals`.
// - `patterns`, `levels` and `events` (ADR-0046) are re-detected as a whole on
//   the newest bars: a tail's replace ours, an older page's are ignored. Levels
//   rewrite their plot on every bar, as a swing moves the whole level.
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
type Histogram = NonNullable<Indicator["histogram"]>[number];
type BarLabel = NonNullable<Indicator["labels"]>[number];
type Pattern = NonNullable<Indicator["patterns"]>[number];
type Level = NonNullable<Indicator["levels"]>[number];

/**
 * The per-bar colors of `src` as indices into `palette`, which gains (in
 * place) every color of `src` it lacks. Bars without a color are null, all
 * of them when `src` has no per-bar colors (an older server).
 */
export function colorIdxInto(
  palette: string[],
  src: Histogram,
): Array<number | null> {
  const map = (src.palette ?? []).map((c) => {
    const k = palette.indexOf(c);
    return k >= 0 ? k : palette.push(c) - 1;
  });
  return src.color_idx
    ? src.color_idx.map((k) => (k === null ? null : (map[k] ?? null)))
    : src.values.map(() => null);
}

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
  /** Different `chart_style` or box: bars of different builds must not mix. */
  | "style"
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
 * True when `inc` has the indicators of `cur` with the same number of lines,
 * histograms and fills, and every per-bar array of `inc` has one value per bar
 * of `inc`. An indicator one side reports under `indicator_errors` may be
 * missing from that side: a script can fail on a full load and not on a tail,
 * or the reverse (ADR-0025), and a reload cannot change that.
 */
function sameShape(cur: SeriesPayload, inc: SeriesPayload): boolean {
  for (const name of Object.keys(inc.indicators)) {
    if (!cur.indicators[name] && !cur.indicator_errors?.[name]) return false;
  }
  const n = inc.bars.length;
  for (const [name, a] of Object.entries(cur.indicators)) {
    const b = inc.indicators[name];
    if (!b) {
      if (inc.indicator_errors?.[name]) continue;
      return false;
    }
    if (!fits(a, b, n)) return false;
  }
  return true;
}

/** `b` has the channels of `a`, with one value per bar of a window of `n` bars. */
function fits(a: Indicator, b: Indicator, n: number): boolean {
  const al = a.lines ?? [];
  const bl = b.lines ?? [];
  const ah = a.histogram ?? [];
  const bh = b.histogram ?? [];
  const af = a.fills ?? [];
  const bf = b.fills ?? [];
  if (al.length !== bl.length || ah.length !== bh.length) return false;
  if (af.length !== bf.length) return false;
  if (bl.some((l) => l.values.length !== n)) return false;
  if (bh.some((h) => h.values.length !== n || (h.color_idx && h.color_idx.length !== n)))
    return false;
  return !bf.some((f) => f.y1.length !== n || f.y2.length !== n);
}

/**
 * `tail` with only the indicators that fit `cur`; the others become errors of
 * their own. For when reloading is no longer an option (MAX_RELOADS): the bars and
 * the rest of the chart stay live, and only what does not fit stops updating.
 */
export function withoutMisfits(cur: SeriesPayload, tail: SeriesPayload): SeriesPayload {
  const misfit = {
    step: "shape",
    line: null,
    message: "Its plots keep changing between updates; reload the page to start over.",
  };
  const errors = { ...tail.indicator_errors };
  const indicators: SeriesPayload["indicators"] = {};
  for (const [name, b] of Object.entries(tail.indicators)) {
    const a = cur.indicators[name];
    if (a && fits(a, b, tail.bars.length)) indicators[name] = b;
    else errors[name] = cur.indicator_errors?.[name] ?? misfit;
  }
  for (const name of Object.keys(cur.indicators)) {
    if (!indicators[name]) errors[name] ??= misfit;
  }
  return withErrors({ ...tail, indicators }, errors);
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
  if ((cur.chart_style ?? "candles") !== (inc.chart_style ?? "candles"))
    return "style";
  if (cur.chart_params?.box_size !== inc.chart_params?.box_size) return "style";
  const own = checkCurrent
    ? barsProblem(cur)
    : cur.bars.at(-1)?.t === null
      ? "untimed"
      : null;
  return own ?? barsProblem(inc) ?? (sameShape(cur, inc) ? null : "shape");
}

/**
 * The errors after merging `inc`: its own, and ours for an indicator we still
 * hold no values of. One that has values and did not fail in `inc` recovered.
 */
function mergedErrors(cur: SeriesPayload, inc: SeriesPayload): SeriesPayload["indicator_errors"] {
  const kept = Object.entries(cur.indicator_errors ?? {}).filter(([k]) => !cur.indicators[k]);
  const errors = { ...Object.fromEntries(kept), ...inc.indicator_errors };
  return Object.keys(errors).length > 0 ? errors : undefined;
}

/** `series` with `errors` as its `indicator_errors`, or none. */
function withErrors(
  series: SeriesPayload,
  errors: SeriesPayload["indicator_errors"],
): SeriesPayload {
  const { indicator_errors: _old, ...rest } = series;
  return errors ? { ...rest, indicator_errors: errors } : rest;
}

const nulls = (n: number): null[] => new Array<null>(n).fill(null);

const shiftSignal = (s: Signal, by: number): Signal => ({ ...s, x: s.x + by });

const shiftDivline = (d: DivLine, by: number): DivLine => ({
  ...d,
  x1: d.x1 + by,
  x2: d.x2 + by,
});

const shiftLabel = (l: BarLabel, by: number): BarLabel => ({ ...l, x: l.x + by });

export const shiftPattern = (p: Pattern, by: number): Pattern => ({
  ...p,
  strokes: p.strokes.map((s) => ({ ...s, x1: s.x1 + by, x2: s.x2 + by })),
  label: { ...p.label, x: p.label.x + by },
});

export const shiftLevel = (l: Level, by: number): Level => ({ ...l, x: l.x + by });

/**
 * Write `levels` over the plots of `lines` on all `length` bars: each level's plot
 * is flat from its bar on and null before, and a plot without a level is null.
 * Only for indicators that have levels: their plots hold nothing else.
 */
export function writeLevels(
  lines: Indicator["lines"],
  levels: readonly Level[],
  length: number,
): void {
  lines?.forEach((l, line) => {
    const lv = levels.find((v) => v.line === line);
    l.values.length = length;
    for (let i = 0; i < length; i++) l.values[i] = lv && i >= lv.x ? lv.y : null;
  });
}

/** True when both lists draw the same levels on the same bars. */
const sameLevels = (a: readonly Level[], b: readonly Level[]): boolean =>
  a.length === b.length &&
  a.every((v, i) => v.line === b[i]!.line && v.x === b[i]!.x && v.y === b[i]!.y);

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

/** Sort order of signals and labels: by bar. Sorting is stable: one bar's order stays. */
const bySignal = (a: { x: number }, b: { x: number }) => a.x - b.x;

/** Sort order of divlines: by end bar, then start bar. */
const byDivline = (a: DivLine, b: DivLine) =>
  endOf(a) - endOf(b) || startOf(a) - startOf(b);

/**
 * One line per `divKey`, keeping the longest (earliest start); on a tie
 * the later entry wins. Input indices must be in `bars`' index space.
 * The result is sorted by end bar (`byDivline`).
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
  return [...byKey.values()].sort(byDivline);
}

/**
 * Sort every indicator's signals by bar and divlines by end bar, in place,
 * and return `series`. `mergeTail` relies on this order to touch only the
 * newest entries; `mergeOlder` keeps it. Run once on a full load.
 */
export function sortChannels(series: SeriesPayload): SeriesPayload {
  for (const ind of Object.values(series.indicators)) {
    ind.signals?.sort(bySignal);
    ind.labels?.sort(bySignal);
    ind.divlines?.sort(byDivline);
  }
  return series;
}

/**
 * Prepend an older history page (`end_time` = our oldest bar). The page
 * must end strictly before `current` starts. Returns a new series; the
 * inputs are not modified. `count` is the number of bars prepended.
 * `has_more` is taken from the page, except that an empty page always
 * ends the history (asking again would return it again).
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
    const pg = page.indicators[name];
    if (!pg) {
      indicators[name] = padOlder(cur, offset);
      continue;
    }
    const merged: Indicator = { ...cur };
    if (cur.lines) {
      merged.lines = cur.lines.map((l, i) => ({
        ...l,
        values: [...pg.lines![i]!.values, ...l.values],
      }));
    }
    if (cur.histogram) {
      merged.histogram = cur.histogram.map((h, i) => {
        const ph = pg.histogram![i]!;
        const out: Histogram = { ...h, values: [...ph.values, ...h.values] };
        if (h.color_idx) {
          const palette = [...(h.palette ?? [])];
          out.color_idx = [...colorIdxInto(palette, ph), ...h.color_idx];
          out.palette = palette;
        }
        return out;
      });
    }
    if (cur.fills) {
      merged.fills = cur.fills.map((f, i) => ({
        ...f,
        y1: [...pg.fills![i]!.y1, ...f.y1],
        y2: [...pg.fills![i]!.y2, ...f.y2],
      }));
    }
    if (cur.signals || pg.signals) {
      merged.signals = [
        ...(pg.signals ?? []),
        ...(cur.signals ?? []).map((s) => shiftSignal(s, offset)),
      ].sort(bySignal);
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
    if (cur.labels || pg.labels) {
      merged.labels = [
        ...(pg.labels ?? []),
        ...(cur.labels ?? []).map((l) => shiftLabel(l, offset)),
      ].sort(bySignal);
    }
    shiftWhole(merged, cur, offset);
    if (merged.levels) writeLevels(merged.lines, merged.levels, bars.length);
    indicators[name] = merged;
  }
  return {
    ok: true,
    count: offset,
    series: withErrors(
      {
        ...current,
        bars,
        indicators,
        has_more: offset > 0 && page.has_more,
      },
      mergedErrors(current, page),
    ),
  };
}

/** `out`'s whole-window channels: those of `cur`, moved `by` bars. */
function shiftWhole(out: Indicator, cur: Indicator, by: number): void {
  if (cur.patterns) out.patterns = cur.patterns.map((p) => shiftPattern(p, by));
  if (cur.levels) out.levels = cur.levels.map((l) => shiftLevel(l, by));
}

/** An indicator that failed on an older page: no values for those `count` bars. */
function padOlder(cur: Indicator, count: number): Indicator {
  const merged: Indicator = { ...cur };
  if (cur.lines) {
    merged.lines = cur.lines.map((l) => ({ ...l, values: [...nulls(count), ...l.values] }));
  }
  if (cur.histogram) {
    merged.histogram = cur.histogram.map((h) => ({
      ...h,
      values: [...nulls(count), ...h.values],
      ...(h.color_idx ? { color_idx: [...new Array<number>(count).fill(0), ...h.color_idx] } : {}),
    }));
  }
  if (cur.fills) {
    merged.fills = cur.fills.map((f) => ({
      ...f,
      y1: [...nulls(count), ...f.y1],
      y2: [...nulls(count), ...f.y2],
    }));
  }
  if (cur.signals) merged.signals = cur.signals.map((s) => shiftSignal(s, count));
  if (cur.divlines) merged.divlines = cur.divlines.map((d) => shiftDivline(d, count));
  if (cur.labels) merged.labels = cur.labels.map((l) => shiftLabel(l, count));
  shiftWhole(merged, cur, count);
  return merged;
}

/** An indicator that failed on a tail: it keeps its values, and new bars have none. */
function padTail(cur: Indicator, length: number): void {
  const pad = (values: Array<number | null>, fill: number | null = null) => {
    while (values.length < length) values.push(fill);
  };
  cur.lines?.forEach((l) => pad(l.values));
  cur.histogram?.forEach((h) => {
    pad(h.values);
    if (h.color_idx) pad(h.color_idx, 0);
  });
  cur.fills?.forEach((f) => {
    pad(f.y1);
    pad(f.y2);
  });
}

/** Write `src` (from index `skip` on) over `dst` from index `from`. */
function overwrite<T>(
  dst: T[],
  from: number,
  src: readonly T[],
  skip: number,
): void {
  dst.length = from;
  for (let k = skip; k < src.length; k++) dst.push(src[k]!);
}

/**
 * Merge a live tail poll (the newest bars: the last closed bar and the
 * forming one, or more after a gap) into `current`. The tail must overlap
 * our newest bars: from its first bar we have (tail bars older than our
 * oldest one are skipped) the tail replaces our bars and extends them when
 * the interval rolled over.
 *
 * Takes ownership of `current`: its per-bar arrays are updated in place
 * (only the last few entries change), so a poll costs no O(bars) copy.
 * Validation runs before any write, so a rejected tail leaves `current`
 * untouched. The returned series is a new top-level object sharing those
 * arrays. `from` is the first bar index that changed; bars from there to
 * the end are new or updated.
 */
/** True when a line or histogram of `ind` has a finite value, looking from the newest bar back. */
function hasValue(ind: Indicator): boolean {
  const plots = [...(ind.lines ?? []), ...(ind.histogram ?? [])];
  return plots.some((p) => {
    for (let i = p.values.length - 1; i >= 0; i--) if (Number.isFinite(p.values[i])) return true;
    return false;
  });
}

/**
 * `recoverEmpty`: a panel we hold with no value at all, that the tail has one for,
 * is a full load that came back empty (a futures series during a Binance cooldown).
 * The tail would fill only its newest bars, so it fails as "shape" and the caller
 * reloads, which also brings the label of the full load. Off once reloads run out.
 */
export function mergeTail(
  current: SeriesPayload,
  tail: SeriesPayload,
  recoverEmpty = true,
): MergeResult<{ from: number }> {
  const problem = headerProblem(current, tail, false);
  if (problem) return fail(problem);
  // Values for a key we only hold an error for: it failed on the full load and
  // works now. Only a full load can bring it back (MAX_RELOADS bounds the attempts).
  if (Object.keys(tail.indicators).some((k) => !current.indicators[k])) return fail("shape");
  if (
    recoverEmpty &&
    Object.entries(current.indicators).some(([k, cur]) => {
      const tl = tail.indicators[k];
      return tl && hasValue(tl) && !hasValue(cur);
    })
  )
    return fail("shape");
  const bars = current.bars;
  const oldest = bars[0];
  const newest = bars.at(-1);
  if (!oldest || !newest || tail.bars.length === 0) return fail("mismatch");
  const rebuilt = (current.anchor ?? null) !== (tail.anchor ?? null);
  // A new build (ADR-0047): its bars need not line up with ours, and it may end
  // before our newest bar. An older one than ours is an outdated response.
  if (rebuilt && (tail.anchor ?? Infinity) < (current.anchor ?? -Infinity)) return fail("stale");
  if (!rebuilt && tail.bars.at(-1)!.t! < newest.t!) return fail("stale");
  if (tail.bars[0]!.t! > newest.t!) return fail("gap");
  if (tail.bars.at(-1)!.t! < oldest.t!) return fail("mismatch");

  // A long catch-up tail can reach back past our oldest bar; those `skip`
  // bars are ignored, the rest must line up with ours.
  let skip = 0;
  while (tail.bars[skip]!.t! < oldest.t!) skip++;
  const head = tail.bars[skip]!;
  // Our index of the tail's first kept bar; the tail must match every bar
  // of ours it covers. Searched from the end: the tail overlaps the newest.
  let from = bars.length - 1;
  while (from >= 0 && bars[from]!.t! > head.t!) from--;
  if (rebuilt) {
    // From its first bar on the new build replaces ours, whatever their times;
    // what is older stays, so paged history survives.
    if (from < 0 || bars[from]!.t !== head.t) from++;
  } else {
    if (from < 0 || bars[from]!.t !== head.t) return fail("mismatch");
    for (let i = from; i < bars.length; i++) {
      if (tail.bars[i - from + skip]!.t !== bars[i]!.t) return fail("mismatch");
    }
  }

  // Tail index k lands on our index k + shift.
  const shift = from - skip;
  overwrite(bars, from, tail.bars, skip);
  if (rebuilt) current.anchor = tail.anchor;
  // A swing that moved rewrites its levels on older bars too.
  let rewritten = from;
  for (const [name, cur] of Object.entries(current.indicators)) {
    const tl = tail.indicators[name];
    if (!tl) {
      padTail(cur, bars.length);
      continue;
    }
    cur.lines?.forEach((l, i) =>
      overwrite(l.values, from, tl.lines![i]!.values, skip),
    );
    cur.histogram?.forEach((h, i) => {
      const th = tl.histogram![i]!;
      overwrite(h.values, from, th.values, skip);
      if (h.color_idx) overwrite(h.color_idx, from, colorIdxInto((h.palette ??= []), th), skip);
    });
    cur.fills?.forEach((f, i) => {
      overwrite(f.y1, from, tl.fills![i]!.y1, skip);
      overwrite(f.y2, from, tl.fills![i]!.y2, skip);
    });
    // Inside the tail window the tail is authoritative (a signal on the
    // forming bar can appear or vanish); before it ours are final.
    // Both lists are kept sorted (sortChannels), so only the entries that
    // reach into the tail window are taken off the end and rebuilt, in
    // place: a poll never walks the whole history.
    if (cur.signals || tl.signals?.length) {
      const sig = (cur.signals ??= []);
      while (sig.length > 0 && sig[sig.length - 1]!.x >= from) sig.pop();
      const fresh = (tl.signals ?? [])
        .filter((s) => s.x >= skip)
        .map((s) => shiftSignal(s, shift));
      for (const s of fresh.sort(bySignal)) sig.push(s);
    }
    if (cur.divlines || tl.divlines?.length) {
      const div = (cur.divlines ??= []);
      const shifted = (tl.divlines ?? [])
        .filter((d) => endOf(d) >= skip)
        .map((d) => shiftDivline(d, shift));
      const confirmed = new Set(shifted.map((d) => divKey(d, bars)));
      // A line of ours that reaches into the tail window survives only if
      // the tail still has it; then ours wins, being unclipped. A tail line
      // starting in the skipped bars only confirms ours.
      const touching: DivLine[] = [];
      while (div.length > 0 && endOf(div[div.length - 1]!) >= from) {
        const d = div.pop()!;
        if (confirmed.has(divKey(d, bars))) touching.push(d);
      }
      const added = shifted.filter((d) => startOf(d) >= 0);
      for (const d of dedupeDivlines([...touching, ...added], bars)) div.push(d);
    }
    if (cur.labels || tl.labels?.length) {
      const own = (cur.labels ??= []);
      while (own.length > 0 && own[own.length - 1]!.x >= from) own.pop();
      for (const l of tl.labels ?? []) {
        if (l.x >= skip) own.push(shiftLabel(l, shift));
      }
    }
    if (cur.patterns || tl.patterns) {
      cur.patterns = (tl.patterns ?? []).map((p) => shiftPattern(p, shift));
    }
    if (cur.events || tl.events) cur.events = tl.events ?? [];
    if (cur.levels || tl.levels) {
      const old = cur.levels ?? [];
      const next = (tl.levels ?? []).map((l) => shiftLevel(l, shift));
      if (!sameLevels(old, next)) {
        writeLevels(cur.lines, next, bars.length);
        for (const l of [...old, ...next]) rewritten = Math.min(rewritten, Math.max(0, l.x));
      }
      cur.levels = next;
    }
    if (tl.future) cur.future = tl.future;
    else delete cur.future;
  }
  return {
    ok: true,
    from: rewritten,
    series: withErrors(current, mergedErrors(current, tail)),
  };
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
