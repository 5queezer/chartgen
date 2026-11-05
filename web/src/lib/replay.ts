// Bar replay (ADR-0023): the chart as it looked at a past bar, the cursor.
//
// Indicator maths stays on the server (ADR-0003). Most outputs are causal,
// so cutting the loaded series at the cursor (`sliceSeries`) shows exactly
// what they showed then. The exceptions repaint: volume/time profiles
// aggregate over the whole window, ZigZag's last leg and the linear
// regression channel move with every new bar, and Ichimoku's lagging span
// plots future closes. For those (`isRepainting`) the replay asks the server
// for a recompute that ends at the cursor (`generate_chart` with `end_time`)
// and `replayFrame` lays it over the slice. Markers drawn some bars before
// they are confirmed (Cipher B's fractals, a moving average shifted back)
// are held back by their lag (`replayLags`).
//
// No Solid, no klinecharts: services/replay.ts drives these, and
// replay.test.ts covers them.

import type { ActiveIndicator } from "../stores/indicators";
import type { SeriesPayload } from "../types/responses";
import { barCloseMs } from "./countdown";
import { keyName } from "./instances";
import { INITIAL_BARS, type SeriesUpdate } from "./liveFeed";
import { sliceFuture } from "./projection";
import { colorIdxInto } from "./seriesMerge";

type Indicator = SeriesPayload["indicators"][string];
type DivLine = NonNullable<Indicator["divlines"]>[number];
type Values = Array<number | null>;

/** Playback speeds in bars per second, as TradingView offers them. */
export const REPLAY_SPEEDS = [1, 3, 10] as const;
export type ReplaySpeed = (typeof REPLAY_SPEEDS)[number];

/** Most one recompute per this long while the cursor moves (playing). */
export const RECOMPUTE_THROTTLE_MS = 500;
/** Bars a recompute covers, ending at the cursor: the size of a full load. */
export const RECOMPUTE_BARS = INITIAL_BARS;
/**
 * First version of replay frames. Frames count up from here, far below the
 * live feed's versions (which start at 1), so the chart never takes a frame
 * for the successor of a live snapshot or the other way round.
 */
export const REPLAY_VERSION_BASE = -(2 ** 40);

/** Indicator request names (and aliases) whose output repaints with new bars. */
const REPAINTING = new Set([
  "volume_profile",
  "vp",
  "vpvr",
  "session_vp",
  "svp",
  "session_volume_profile",
  "hvn_lvn",
  "vp_nodes",
  "volume_nodes",
  "naked_poc",
  "npoc",
  "tpo",
  "market_profile",
  "zigzag",
  "linreg_channel",
  "ichimoku",
]);

/**
 * True when indicator `name`'s output at a bar depends on later bars. Takes
 * a payload key too: `zigzag#2` is a ZigZag (ADR-0028).
 */
export const isRepainting = (name: string): boolean =>
  REPAINTING.has(keyName(name).toLowerCase());

/**
 * Bars after its own bar at which an indicator's signals and divergence
 * ends are known. Cipher B confirms a fractal two bars after its center
 * (src/indicators/cipher_b.rs, `center = i - 2`); the Kalman volume filter
 * marks a cross on the bar before the one that confirms it (`x: i - 1`).
 */
const MARKER_LAG: Record<string, number> = { cipher_b: 2, kalman: 1, kalman_volume: 1, kvf: 1 };
/** Moving averages with an `offset` param; a negative one plots later bars' values. */
const OFFSET_MAS = new Set(["ma", "vwma", "hma"]);

/** How far an indicator's output runs ahead of its bars. */
export interface Lag {
  /** Bars until a signal or divergence end is confirmed. */
  markers?: number;
  /** Newest bars whose per-bar values come from later bars. */
  tail?: number;
}
export type Lags = Record<string, Lag>;

/** `Lag` per payload key (the request name, or the instance id) of the requested indicators. */
export function replayLags(entries: readonly ActiveIndicator[]): Lags {
  const out: Lags = {};
  for (const e of entries) {
    const name = typeof e === "string" ? e : e.name;
    const id = typeof e !== "string" && typeof e.id === "string" ? e.id : undefined;
    const key = name.toLowerCase();
    const offset = typeof e === "string" ? undefined : e.offset;
    const lag: Lag = {};
    if (MARKER_LAG[key]) lag.markers = MARKER_LAG[key];
    if (OFFSET_MAS.has(key) && typeof offset === "number" && offset < 0) lag.tail = -offset;
    if (lag.markers || lag.tail) out[id ?? name] = lag;
  }
  return out;
}

/** Delay between two steps at `speed` bars per second. */
export const playDelayMs = (speed: ReplaySpeed): number => 1000 / speed;

/** Index of the last bar opening at or before `t` (seconds), -1 if none. Bars are sorted. */
export function cursorIndexAt(bars: ReadonlyArray<{ t: number | null }>, t: number): number {
  let lo = 0;
  let hi = bars.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if ((bars[mid]!.t ?? -Infinity) <= t) {
      found = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return found;
}

/**
 * Close (seconds) of the bar opening at `t`: the replay clock once that bar
 * is the cursor. An unknown interval closes a bar at its open.
 */
export const closeOf = (t: number, interval: string | null): number =>
  ((interval ? barCloseMs(t, interval) : null) ?? t * 1000) / 1000;

/**
 * Index of the last bar closed by the replay clock `clock` (seconds), -1 if
 * none. Bars are sorted. The clock rather than a bar index or open time is
 * the cursor, so another timeframe shows exactly the bars complete then.
 */
export function indexClosedBy(
  bars: ReadonlyArray<{ t: number | null }>,
  clock: number,
  interval: string | null,
): number {
  let lo = 0;
  let hi = bars.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const t = bars[mid]!.t;
    if (t !== null && closeOf(t, interval) <= clock) {
      found = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return found;
}

/** The cursor `by` bars on, or null at the last of `length` bars. -1 steps to bar 0. */
export function stepCursor(cursor: number, length: number, by = 1): number | null {
  const next = Math.min(length - 1, Math.max(0, cursor + by));
  return next > cursor ? next : null;
}

/** First bar of the recompute window ending at `cursor`. */
export const windowStart = (cursor: number): number =>
  Math.max(0, cursor - RECOMPUTE_BARS + 1);

const startOf = (d: { x1: number; x2: number }) => Math.min(d.x1, d.x2);
const endOf = (d: { x1: number; x2: number }) => Math.max(d.x1, d.x2);

function sliceIndicator(
  ind: Indicator,
  n: number,
  bars: SeriesPayload["bars"],
  lag: Lag = {},
): Indicator {
  // Values from later bars (a negative MA offset) are not known yet.
  const cut = (v: Values) => {
    const out = v.slice(0, n);
    for (let i = Math.max(0, n - (lag.tail ?? 0)); i < n; i++) out[i] = null;
    return out;
  };
  const out: Indicator = { ...ind };
  if (ind.lines) out.lines = ind.lines.map((l) => ({ ...l, values: cut(l.values) }));
  if (ind.histogram) {
    // Per-bar colors (ADR-0026) are cut with the values; the palette is
    // copied because `replayFrame` may add recompute colors to it.
    out.histogram = ind.histogram.map((h) => ({
      ...h,
      values: cut(h.values),
      ...(h.color_idx && { color_idx: cut(h.color_idx), palette: [...(h.palette ?? [])] }),
    }));
  }
  if (ind.fills) {
    out.fills = ind.fills.map((f) => ({ ...f, y1: cut(f.y1), y2: cut(f.y2) }));
  }
  const markers = lag.markers ?? 0;
  if (ind.signals) out.signals = ind.signals.filter((s) => s.x + markers < n);
  // A divergence is known once its end bar printed (and was confirmed): one
  // ending later is not drawn yet, not even in part.
  if (ind.divlines) out.divlines = ind.divlines.filter((d) => endOf(d) + markers < n);
  // New arrays per frame even where nothing changes: the chart re-stores an
  // indicator's geometry when these arrays change identity.
  if (ind.hlines) out.hlines = ind.hlines.slice();
  if (ind.hbars) out.hbars = ind.hbars.slice();
  // The cloud past the cursor (ADR-0033).
  if (ind.future) out.future = sliceFuture(ind, n, bars);
  return out;
}

/**
 * `series` up to and including bar `cursorIndex` (clamped to the bars; -1
 * leaves none), in every channel: bars and every per-bar array (lines,
 * histogram, fills) are cut, signals and divlines ending after the cursor
 * are dropped, both with each indicator's `lags` applied. Indices
 * stay those of `series` (a prefix keeps them), so the result has the
 * seriesMerge/geometry shape of a live series. hbars, hlines and y_range
 * are window aggregates and pass through; `replayFrame` replaces them for
 * repainting indicators. Returns new arrays throughout; `series` is untouched.
 */
export function sliceSeries(
  series: SeriesPayload,
  cursorIndex: number,
  lags: Lags = {},
): SeriesPayload {
  const n = Math.max(0, Math.min(cursorIndex, series.bars.length - 1) + 1);
  const indicators: SeriesPayload["indicators"] = {};
  for (const [name, ind] of Object.entries(series.indicators)) {
    indicators[name] = sliceIndicator(ind, n, series.bars, lags[name]);
  }
  return { ...series, bars: series.bars.slice(0, n), indicators };
}

/** Index into `into` of each bar of `from` with the same open time, -1 if none. Both sorted. */
function indexByTime(
  from: SeriesPayload["bars"],
  into: SeriesPayload["bars"],
): number[] {
  const out: number[] = [];
  let j = 0;
  for (const b of from) {
    while (j < into.length && (into[j]!.t ?? -Infinity) < (b.t ?? -Infinity)) j++;
    out.push(j < into.length && into[j]!.t === b.t && b.t !== null ? j : -1);
  }
  return out;
}

/** Same channel counts, and every per-bar array of `r` has one value per bar. */
function fits(ind: Indicator, r: Indicator, bars: number): boolean {
  const len = (a?: unknown[]) => a?.length ?? 0;
  return (
    len(ind.lines) === len(r.lines) &&
    len(ind.histogram) === len(r.histogram) &&
    len(ind.fills) === len(r.fills) &&
    (r.lines ?? []).every((l) => l.values.length === bars) &&
    (r.histogram ?? []).every((h) => h.values.length === bars) &&
    (r.fills ?? []).every((f) => f.y1.length === bars && f.y2.length === bars)
  );
}

/**
 * The replay view at bar `cursor`: `sliceSeries`, then for every repainting
 * indicator the recompute window (the RECOMPUTE_BARS bars up to the cursor)
 * is taken from `recompute`, a server response for the same request that
 * ends at or before the cursor (null: none yet). Bars of the window the
 * recompute does not cover (it ended earlier, or there is none) have no
 * value: a repainting value from the full series would show the future.
 * Bars before the window keep the full series' values. hbars and hlines
 * come from the recompute only, signals and divlines from the recompute in
 * the window and from the full series before it; a full-series line that
 * crosses into the window (ZigZag's leg) is clipped at the window start.
 */
export function replayFrame(
  series: SeriesPayload,
  cursor: number,
  recompute: SeriesPayload | null,
  lags: Lags = {},
): SeriesPayload {
  const slice = sliceSeries(series, cursor, lags);
  const names = Object.keys(slice.indicators).filter(isRepainting);
  if (names.length === 0) return slice;
  const n = slice.bars.length;
  const from = windowStart(n - 1);
  // Slice index of each recompute bar, and the reverse for the window.
  const toSlice = recompute ? indexByTime(recompute.bars, slice.bars) : [];
  const toRc = new Array<number>(Math.max(0, n - from)).fill(-1);
  toSlice.forEach((si, ri) => {
    if (si >= from) toRc[si - from] = ri;
  });

  for (const name of names) {
    const ind = slice.indicators[name]!;
    const found = recompute?.indicators[name];
    // An empty frame shows no profile either.
    const r = n > 0 && found && fits(ind, found, recompute!.bars.length) ? found : null;
    const pick = (dst: Values, src: Values | undefined) => {
      for (let i = from; i < n; i++) {
        const ri = toRc[i - from]!;
        dst[i] = r && ri >= 0 ? (src?.[ri] ?? null) : null;
      }
    };
    ind.lines?.forEach((l, k) => pick(l.values, r?.lines?.[k]?.values));
    ind.histogram?.forEach((h, k) => {
      const rh = r?.histogram?.[k];
      pick(h.values, rh?.values);
      if (h.color_idx) pick(h.color_idx, rh && colorIdxInto(h.palette!, rh));
    });
    ind.fills?.forEach((f, k) => {
      pick(f.y1, r?.fills?.[k]?.y1);
      pick(f.y2, r?.fills?.[k]?.y2);
    });
    const at = (x: number) => toSlice[x] ?? -1;
    if (ind.signals || r?.signals) {
      ind.signals = [
        ...(ind.signals ?? []).filter((s) => s.x < from),
        ...(r?.signals ?? []).flatMap((s) => (at(s.x) >= from ? [{ ...s, x: at(s.x) }] : [])),
      ];
    }
    if (ind.divlines || r?.divlines) {
      ind.divlines = [
        ...(series.indicators[name]!.divlines ?? []).flatMap((d) =>
          startOf(d) >= from ? [] : endOf(d) < from ? [d] : [clipEnd(d, from, slice.bars)],
        ),
        ...(r?.divlines ?? []).flatMap((d) => {
          const x1 = at(d.x1);
          const x2 = at(d.x2);
          return x1 >= 0 && x2 >= 0 && Math.max(x1, x2) >= from ? [{ ...d, x1, x2 }] : [];
        }),
      ];
    }
    ind.hbars = r?.hbars ? r.hbars.slice() : [];
    ind.hlines = r?.hlines ? r.hlines.slice() : [];
    if (r?.y_range) ind.y_range = r.y_range;
  }
  return slice;
}

/** `d` cut off at bar `at` (between its ends), on the same straight line. */
function clipEnd(d: DivLine, at: number, bars: SeriesPayload["bars"]): DivLine {
  const fromLeft = d.x1 <= d.x2;
  const [xs, ys, xe, ye] = fromLeft ? [d.x1, d.y1, d.x2, d.y2] : [d.x2, d.y2, d.x1, d.y1];
  const y = ys + ((ye - ys) * (at - xs)) / (xe - xs);
  const t = bars[at]?.t ?? null;
  return fromLeft ? { ...d, x2: at, y2: y, t2: t } : { ...d, x1: at, y1: y, t1: t };
}

/** What a replay frame was cut from: the live snapshot, the cursor and the recompute. */
export interface FrameKey {
  /** Version of the live snapshot (the full series). */
  baseVersion: number;
  cursor: number;
  /** Revision of the recompute applied (0: none). */
  recompute: number;
  /** `frameShape` of the frame. */
  shape: string;
}

/**
 * The indicators of `series` with any output, the ones the chart gives a
 * pane (an indicator with none gets none). Only a full apply creates panes,
 * so a frame that changes this set is a reset: a profile has no output
 * until its first recompute lands.
 */
export function frameShape(series: SeriesPayload): string {
  const some = (a?: unknown[]) => (a?.length ?? 0) > 0;
  return Object.entries(series.indicators)
    .filter(
      ([, i]) =>
        some(i.lines) || some(i.histogram) || some(i.fills) || some(i.hbars) || some(i.divlines) || some(i.hlines),
    )
    .map(([name]) => name)
    .join("\n");
}

/**
 * How the chart gets from frame `prev` to frame `next`, numbered `version`:
 * a cursor that only moved on (or a new recompute) is a `tail` from the
 * first bar that can differ, a history page under an unchanged view is
 * `older`, anything else (first frame, cursor moved back, new data, another
 * `frameShape`) a `reset`. Null when nothing changed. `repainting`: some
 * indicator repaints, so the recompute window's values change as well.
 */
export function frameUpdate(
  prev: FrameKey | null,
  next: FrameKey,
  base: SeriesUpdate,
  repainting: boolean,
  version: number,
): SeriesUpdate | null {
  const reset: SeriesUpdate = { version, kind: "reset" };
  if (!prev || next.shape !== prev.shape) return reset;
  if (next.baseVersion !== prev.baseVersion) {
    const paged =
      base.kind === "older" &&
      base.version === next.baseVersion &&
      next.baseVersion === prev.baseVersion + 1 &&
      next.cursor === prev.cursor + base.count &&
      next.recompute === prev.recompute;
    return paged ? { version, kind: "older", count: base.count } : reset;
  }
  // From an empty frame (no bar closed by the clock) the chart holds nothing to build on.
  if (next.cursor < prev.cursor || prev.cursor < 0) return reset;
  if (next.cursor === prev.cursor && next.recompute === prev.recompute) return null;
  return {
    version,
    kind: "tail",
    from: repainting ? windowStart(prev.cursor) : prev.cursor,
    prevLength: prev.cursor + 1,
  };
}

export interface Throttle {
  /** Run now if the last run is `ms` ago, else once when it is. */
  call: () => void;
  /** Run a pending call now. */
  flush: () => void;
  /** Drop a pending call. */
  cancel: () => void;
}

/** Leading and trailing throttle: `fn` runs at most once per `ms`, and after the last call. */
export function createThrottle(fn: () => void, ms: number): Throttle {
  let last = -Infinity;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const run = () => {
    timer = null;
    last = Date.now();
    fn();
  };
  const cancel = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };
  return {
    call() {
      if (timer !== null) return;
      const wait = last + ms - Date.now();
      if (wait <= 0) run();
      else timer = setTimeout(run, wait);
    },
    flush() {
      if (timer === null) return;
      cancel();
      run();
    },
    cancel,
  };
}
