// Bar replay controller (ADR-0023): the replay state and the frames the
// chart shows while replaying. Built on the pure helpers in lib/replay.ts.
//
// Modes: "off" (live chart), "select" (the full series with a cut line
// under the mouse; a click picks the start) and "active" (the series cut at
// the cursor, stepped by hand or played). While not "off" the live feed is
// suspended and `liveDoc` tells it, and the compare feed, that the page is
// hidden, so neither polls nor subscribes until replay ends; history paging
// keeps working. Exiting restarts the feed from the full series.
//
// The cursor is a replay clock: the close time of the cursor bar. A frame
// holds every bar closed by then, so a history page (indices shift) keeps
// the cursor, and a symbol, timeframe or indicator change (new data) is cut
// again at the same clock; data that starts after it is paged back first.
// Frames come from the live snapshot (full series), the clock and the latest
// server recompute of the repainting indicators, which runs at most every
// RECOMPUTE_THROTTLE_MS while the cursor moves, and once more on pause.
//
// Price alerts never see a frame: they are fed from the live feed's own
// snapshots (App.tsx), which carry no tail while replaying.

import { batch, createEffect, createMemo, createSignal, onCleanup, untrack, type Accessor } from "solid-js";

import { WARMUP, type FetchArgs, type LiveFeed, type LiveRequest, type LiveSnapshot } from "../lib/liveFeed";
import {
  RECOMPUTE_BARS,
  RECOMPUTE_THROTTLE_MS,
  REPLAY_VERSION_BASE,
  closeOf,
  createThrottle,
  cursorIndexAt,
  frameShape,
  frameUpdate,
  indexClosedBy,
  isRepainting,
  playDelayMs,
  replayFrame,
  stepCursor,
  type FrameKey,
  type ReplaySpeed,
} from "../lib/replay";
import { indicatorName } from "../stores/indicators";
import type { SeriesPayload } from "../types/responses";

export type ReplayMode = "off" | "select" | "active";

/** The page visibility as the live feeds see it. */
export type ReplayDoc = Pick<Document, "visibilityState" | "addEventListener" | "removeEventListener">;

export interface ReplayOptions {
  /** The live feed's latest snapshot: the full series. */
  live: Accessor<LiveSnapshot | null>;
  /** The request the live series was loaded with; null before the first load. */
  request: () => LiveRequest | null;
  /** End the live feed's generation (polls, pushes, pages in flight). */
  suspendLive: () => void;
  /** Restart the live feed from its full series. */
  resumeLive: () => void;
  loadUntil: LiveFeed["loadUntil"];
  fetch: (request: LiveRequest, args: FetchArgs) => Promise<SeriesPayload>;
}

export interface Replay {
  mode: Accessor<ReplayMode>;
  playing: Accessor<boolean>;
  speed: Accessor<ReplaySpeed>;
  /** Open time (seconds) of the frame's last bar; null outside "active". */
  cursorTime: Accessor<number | null>;
  /** What the chart shows: the live snapshot, or the replay frame. */
  snapshot: Accessor<LiveSnapshot | null>;
  /** Pass to the live feeds as their `doc`. */
  liveDoc: ReplayDoc;
  /** Enter replay in "select" mode; false when the chart cannot replay. */
  enter: () => boolean;
  /** Back to "select" to pick a new start. */
  selectStart: () => void;
  /** Start (or restart) replay at bar `index` of the full series. */
  startAt: (index: number) => void;
  /** Start at the bar open at `ms`, paging history back to it first. */
  startAtDate: (ms: number) => Promise<void>;
  /** One bar on; false at the last bar. */
  step: () => boolean;
  togglePlay: () => void;
  pause: () => void;
  setSpeed: (speed: ReplaySpeed) => void;
  /** Back to the live chart ("Jump to real time"). */
  exit: () => void;
}

interface Recompute {
  /** JSON of the request it was made for. */
  key: string;
  /** Replay clock it ends at. */
  clock: number;
  payload: SeriesPayload;
  rev: number;
}

export function createReplay(opts: ReplayOptions): Replay {
  const [mode, setModeSignal] = createSignal<ReplayMode>("off");
  // Replay clock (seconds): close of the cursor bar; null outside "active".
  const [clock, setClock] = createSignal<number | null>(null);
  const [playing, setPlaying] = createSignal(false);
  const [speed, setSpeedSignal] = createSignal<ReplaySpeed>(1);
  const [recomputed, setRecomputed] = createSignal<Recompute | null>(null);
  // Mirrors `mode` for `liveDoc`, which the feeds read from timers and from
  // inside a batch, where a signal read could lag behind.
  let replaying = false;
  const setMode = (m: ReplayMode) => {
    replaying = m !== "off";
    setModeSignal(m);
  };

  const liveDoc: ReplayDoc = {
    get visibilityState() {
      return replaying ? "hidden" : document.visibilityState;
    },
    addEventListener: document.addEventListener.bind(document),
    removeEventListener: document.removeEventListener.bind(document),
  };

  // Identity of the live series' request; re-evaluated per snapshot, equal
  // across pages, so only a new load changes it.
  const requestKey = createMemo(() => {
    opts.live();
    const r = opts.request();
    return r ? JSON.stringify(r) : null;
  });

  // A recompute is used only for its own request and up to its own clock:
  // one from a later clock (after the cursor moved back) shows the future.
  const usable = createMemo(() => {
    const rc = recomputed();
    const c = clock();
    return rc && c !== null && rc.key === requestKey() && rc.clock <= c ? rc : null;
  });

  let prevKey: FrameKey | null = null;
  let version = REPLAY_VERSION_BASE;
  const snapshot = createMemo<LiveSnapshot | null>((prev) => {
    const live = opts.live();
    const c = clock();
    const rc = usable();
    if (mode() !== "active" || !live || c === null) {
      prevKey = null;
      return live;
    }
    const base = live.series;
    const cursor = Math.max(0, indexClosedBy(base.bars, c, base.interval));
    const series = replayFrame(base, cursor, rc?.payload ?? null);
    const key: FrameKey = {
      baseVersion: live.update.version,
      cursor,
      recompute: rc?.rev ?? 0,
      shape: frameShape(series),
    };
    const repainting = Object.keys(base.indicators).some(isRepainting);
    const update = frameUpdate(prevKey, key, live.update, repainting, version + 1);
    if (update === null && prev) return prev;
    version++;
    prevKey = key;
    return { series, update: update ?? { version, kind: "reset" } };
  }, null);

  const cursorTime = createMemo(() =>
    mode() === "active" ? (snapshot()?.series.bars.at(-1)?.t ?? null) : null,
  );

  // New data (another timeframe or symbol) can start after the clock: page
  // history back to it. Until then the frame holds the first bar.
  let pagingTo: number | null = null;
  createEffect(() => {
    const c = clock();
    const series = opts.live()?.series;
    const first = series?.bars[0]?.t;
    if (mode() !== "active" || c === null || first == null || pagingTo === c) return;
    if (closeOf(first, series!.interval) <= c) return;
    pagingTo = c;
    const isCurrent = () => untrack(mode) === "active" && untrack(clock) === c;
    // The bar closing at the clock opens one bar length before it.
    void opts.loadUntil(c - (closeOf(first, series!.interval) - first), isCurrent).finally(() => {
      if (pagingTo === c) pagingTo = null;
    });
  });

  // --- Server recompute of the repainting indicators -------------------
  let abort: AbortController | null = null;
  let seq = 0;
  let rev = 0;
  const recompute = () => {
    abort?.abort();
    abort = null;
    const req = opts.request();
    const c = untrack(clock);
    if (!req || c === null || untrack(mode) !== "active") return;
    const indicators = req.indicators.filter((e) => isRepainting(indicatorName(e)));
    if (indicators.length === 0) return;
    const my = ++seq;
    const ctrl = (abort = new AbortController());
    const key = JSON.stringify(req);
    // end_time is exclusive on the bar open: bars opening before the clock
    // are exactly those closed by it.
    opts
      .fetch(
        { ...req, indicators },
        { bars: RECOMPUTE_BARS, warmup: WARMUP, end_time: c * 1000, signal: ctrl.signal },
      )
      .then(
        (payload) => {
          if (my === seq) setRecomputed({ key, clock: c, payload, rev: ++rev });
        },
        // Keep the last one; the next cursor move asks again.
        () => {},
      );
  };
  const throttle = createThrottle(recompute, RECOMPUTE_THROTTLE_MS);
  createEffect(() => {
    if (mode() !== "active") {
      throttle.cancel();
      abort?.abort();
      abort = null;
      setRecomputed(null);
      return;
    }
    clock();
    requestKey();
    throttle.call();
  });
  // ---------------------------------------------------------------------

  let playTimer: ReturnType<typeof setTimeout> | null = null;
  const stopTimer = () => {
    if (playTimer !== null) clearTimeout(playTimer);
    playTimer = null;
  };
  const schedule = () => {
    stopTimer();
    playTimer = setTimeout(tick, playDelayMs(untrack(speed)));
  };
  const tick = () => {
    playTimer = null;
    if (step()) schedule();
    else pause();
  };

  const pause = () => {
    stopTimer();
    if (!untrack(playing)) return;
    setPlaying(false);
    throttle.flush();
  };

  const step = (): boolean => {
    const series = untrack(opts.live)?.series;
    const c = untrack(clock);
    if (untrack(mode) !== "active" || !series || c === null) return false;
    const next = stepCursor(indexClosedBy(series.bars, c, series.interval), series.bars.length);
    if (next === null) return false;
    setClock(closeOf(series.bars[next]!.t!, series.interval));
    return true;
  };

  const startAt = (index: number) => {
    const series = untrack(opts.live)?.series;
    const t = series?.bars[Math.max(0, Math.min(index, series.bars.length - 1))]?.t;
    if (t == null) return;
    pause();
    batch(() => {
      setClock(closeOf(t, series!.interval));
      setMode("active");
    });
  };

  const exit = () => {
    if (untrack(mode) === "off") return;
    pause();
    batch(() => {
      setMode("off");
      setClock(null);
      opts.resumeLive();
    });
  };

  onCleanup(() => {
    stopTimer();
    throttle.cancel();
    abort?.abort();
  });

  return {
    mode,
    playing,
    speed,
    cursorTime,
    snapshot,
    liveDoc,
    enter() {
      const bars = untrack(opts.live)?.series.bars;
      // Untimed (no-ticker demo) bars have no times to replay by.
      if (untrack(mode) !== "off" || !bars?.length || bars.some((b) => b.t === null)) return false;
      opts.suspendLive();
      setMode("select");
      return true;
    },
    selectStart() {
      if (untrack(mode) === "off") return;
      pause();
      setMode("select");
    },
    startAt,
    async startAtDate(ms) {
      const ts = ms / 1000;
      const isCurrent = () => untrack(mode) !== "off";
      const first = untrack(opts.live)?.series.bars[0]?.t;
      if (first != null && ts < first) await opts.loadUntil(ts, isCurrent);
      const bars = untrack(opts.live)?.series.bars;
      if (!isCurrent() || !bars) return;
      // The bar the date falls in.
      startAt(Math.max(0, cursorIndexAt(bars, ts)));
    },
    step,
    togglePlay() {
      if (untrack(playing)) {
        pause();
      } else if (untrack(mode) === "active") {
        setPlaying(true);
        schedule();
      }
    },
    pause,
    setSpeed(s) {
      setSpeedSignal(s);
      if (untrack(playing)) schedule();
    },
    exit,
  };
}
