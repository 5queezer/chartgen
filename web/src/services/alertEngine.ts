// Client-side alert evaluation and delivery (ADR-0020). Prices come from
// two places and both end in `deliver`:
//   - the chart symbol: the live feed's tail polls, via `feedChartSnapshot`,
//     which also carry the indicator values and bars that indicator and
//     sloped-drawing alerts read (ADR-0034: those only run while the chart
//     shows their symbol and interval);
//   - every other symbol with an active alert: its own `get_quotes` poll
//     (`createAlertQuotes`), at the watchlist's 5 s cadence. The chart
//     symbol is left out of that poll so one price is never counted twice.
// Delivery is a toast, a short WebAudio beep and, if the user granted the
// permission, a browser Notification. Only one open tab evaluates and
// delivers (see `leader`).

import {
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  untrack,
  type Accessor,
} from "solid-js";

import { CONDITION_LABELS, describeAlert, type Alert } from "../lib/alerts";
import type { LiveSnapshot } from "../lib/liveFeed";
import { alertInterval } from "../lib/priceBars";
import { formatPrice } from "../lib/quote";
import type { McpSession } from "../mcp";
import { createQuotesQuery } from "../queries";
import {
  refreshServerAlerts,
  serverAlertText,
  serverAlertsEnabled,
  setServerAlertSession,
  takeNewFires,
} from "../stores/serverAlerts";
import { installPushClicks, pushOnHere, syncPushDevice } from "../stores/pushDevice";
import { timeAxisOf } from "../stores/drawings";
import { scaleMode } from "../stores/layout";
import type { SeriesPayload } from "../types/responses";
import {
  activeAlertSymbols,
  chartContext,
  forgetBaseline,
  observePrice,
  processPrice,
  setChartContext,
  setEvaluatingChart,
  type Tick,
} from "../stores/alerts";

const TOAST_MS = 5_000;
const SERVER_ALERT_POLL_MS = 5_000;
/** How often a server without them is asked again, and how soon a focus may do so. */
const SERVER_ALERT_RECHECK_MS = 300_000;
const SERVER_ALERT_FOCUS_GAP_MS = 30_000;
// A triggered alert is worth reading; it stays longer than a confirmation.
const ALERT_TOAST_MS = 8_000;
/** Toasts visible at once; a new one pushes the oldest out. */
const MAX_TOASTS = 3;

export interface Toast {
  id: number;
  text: string;
}

const [toasts, setToasts] = createSignal<Toast[]>([]);
export { toasts };

let toastId = 0;

export function showToast(text: string, ms = TOAST_MS): void {
  const id = ++toastId;
  setToasts((list) => [...list, { id, text }].slice(-MAX_TOASTS));
  setTimeout(() => dismissToast(id), ms);
}

export function dismissToast(id: number): void {
  setToasts((list) => list.filter((t) => t.id !== id));
}

/**
 * "BTCUSDT crossing 65000.00" (or "BTCUSDT RSI (14) crossing up 70") plus
 * the user's message, if any.
 */
export function alertText(a: Alert): string {
  const head = a.source
    ? `${a.symbol} ${describeAlert(a)}`
    : `${a.symbol} ${CONDITION_LABELS[a.condition].toLowerCase()} ${formatPrice(a.price)}`;
  return a.message ? `${head} — ${a.message}` : head;
}

let audio: AudioContext | undefined;

/**
 * Create and resume the AudioContext. Browsers only allow that after a user
 * gesture, so this runs from the dialog submit and the first pointerdown /
 * keydown (`installAudioUnlock`); `beep` only uses the context that exists.
 */
export function unlockAudio(): void {
  try {
    audio ??= new AudioContext();
    void audio.resume();
  } catch {
    // No audio support.
  }
}

/** Unlock audio on the first user gesture. Returns the uninstall function. */
function installAudioUnlock(): () => void {
  const off = () => {
    window.removeEventListener("pointerdown", unlock, true);
    window.removeEventListener("keydown", unlock, true);
  };
  const unlock = () => {
    unlockAudio();
    off();
  };
  window.addEventListener("pointerdown", unlock, true);
  window.addEventListener("keydown", unlock, true);
  return off;
}

function beep(): void {
  if (!audio || audio.state !== "running") return;
  try {
    const osc = audio.createOscillator();
    const gain = audio.createGain();
    const t = audio.currentTime;
    osc.frequency.value = 880;
    gain.gain.setValueAtTime(0.15, t);
    gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.4);
    osc.connect(gain).connect(audio.destination);
    osc.start(t);
    osc.stop(t + 0.4);
  } catch {
    // The toast still shows.
  }
}

/** Ask once, on the first alert the user creates (never on load). */
export function requestNotificationPermission(): void {
  if (typeof Notification !== "undefined" && Notification.permission === "default") {
    void Notification.requestPermission();
  }
}

function deliver(fired: readonly { id: string; text: string }[]): void {
  if (fired.length === 0) return;
  for (const { id, text } of fired) {
    showToast(`Alert: ${text}`, ALERT_TOAST_MS);
    try {
      if (typeof Notification !== "undefined" && Notification.permission === "granted") {
        new Notification("chartgen alert", { body: text, tag: id });
      }
    } catch {
      // Some browsers throw (e.g. Android Chrome wants a service worker):
      // the toast and the beep still happen.
    }
  }
  beep();
}

// Only one tab evaluates and delivers, or every open tab would fire the
// same alert. Tabs hold the Web Lock "chartgen-alerts" while visible; the
// others just mirror the store through its `storage` listener. A hidden tab
// does not poll, so it gives the lock up. Without Web Locks every tab
// evaluates (the store re-reads storage, which limits but does not rule
// out duplicates).
const LOCK = "chartgen-alerts";
const hasLocks = typeof navigator !== "undefined" && "locks" in navigator;
const [leader, setLeader] = createSignal(!hasLocks);
export { leader };

/** Take part in the election while the page is visible. Returns the stop function. */
function startLeaderElection(): () => void {
  if (!hasLocks) return () => {};
  let release: (() => void) | undefined;
  let abort: AbortController | undefined;

  const acquire = () => {
    if (abort) return;
    const mine = (abort = new AbortController());
    navigator.locks
      .request(LOCK, { signal: mine.signal }, () => {
        // Prices seen while not leading are not a baseline.
        forgetBaseline();
        setLeader(true);
        return new Promise<void>((resolve) => (release = resolve));
      })
      .catch(() => {
        // Aborted while waiting.
      });
  };
  const relinquish = () => {
    abort?.abort();
    abort = undefined;
    release?.();
    release = undefined;
    setLeader(false);
  };
  const onVisibility = () =>
    document.visibilityState === "hidden" ? relinquish() : acquire();

  if (document.visibilityState !== "hidden") acquire();
  document.addEventListener("visibilitychange", onVisibility);
  return () => {
    document.removeEventListener("visibilitychange", onVisibility);
    relinquish();
  };
}

// An alert bound to a chart only runs while the leading tab shows that chart. The leading
// tab says which one it shows, so that another tab does not read such an alert as running.
const LEADING_CHART_KEY = "chartgen.alerts.leadingChart";

function readLeadingChart(): string | null {
  try {
    return localStorage.getItem(LEADING_CHART_KEY);
  } catch {
    return null;
  }
}

/** Call inside a component. */
function shareLeadingChart(): void {
  createEffect(() => {
    if (!leader()) {
      setEvaluatingChart(readLeadingChart());
      return;
    }
    setEvaluatingChart(null);
    const ctx = chartContext();
    try {
      if (ctx) localStorage.setItem(LEADING_CHART_KEY, `${ctx.symbol} ${ctx.interval}`);
      else localStorage.removeItem(LEADING_CHART_KEY);
    } catch {
      // Not shared: other tabs go by their own chart, as without a leader.
    }
  });
  const onStorage = (e: StorageEvent) => {
    if ((e.key === LEADING_CHART_KEY || e.key === null) && !leader()) {
      setEvaluatingChart(readLeadingChart());
    }
  };
  window.addEventListener("storage", onStorage);
  onCleanup(() => window.removeEventListener("storage", onStorage));
}

/** Evaluate and deliver for one live price (in the leading tab). */
export function feedPrice(
  symbol: string,
  price: number | null | undefined,
  tick?: Tick,
): void {
  if (price == null) return;
  if (!leader()) {
    observePrice(symbol, price);
    return;
  }
  deliver(
    processPrice(symbol, price, Date.now(), tick).map((a) => ({ id: a.id, text: alertText(a) })),
  );
}

/**
 * What the chart's live feed emitted for the chart symbol. Only a `tail`
 * poll is a new price; it also carries the indicators' forming-bar values
 * (ADR-0034) and the bar time a drawing's line is read at. A `reset` (also
 * a cached load replayed when switching back to a symbol) may hold a close
 * minutes old: it only restarts the crossing baselines. `older` pages leave
 * the last close untouched.
 */
export function feedChartSnapshot(symbol: string, snapshot: LiveSnapshot): void {
  const { bars, indicators } = snapshot.series;
  const close = bars[bars.length - 1]?.c;
  const tick = chartTick(snapshot.series);
  setChartContext({
    symbol,
    interval: alertInterval(snapshot.series),
    keys: Object.keys(indicators),
    axis: tick.chart?.view?.axis,
  });
  if (snapshot.update.kind === "reset") {
    forgetBaseline(symbol);
    if (close != null) observePrice(symbol, close);
  } else if (snapshot.update.kind === "tail") {
    feedPrice(symbol, close, tick);
  }
}

/**
 * What a price of the chart's series is evaluated with besides itself: the
 * last bar's time, the interval and indicators, and how the chart draws
 * (bars and log scale) for sloped drawing alerts.
 */
export function chartTick(series: SeriesPayload): Tick {
  const times = series.bars.map((b) => (b.t === null ? NaN : b.t * 1000));
  const axis = times.some(Number.isNaN) ? null : timeAxisOf(times);
  const last = times.at(-1);
  return {
    time: last === undefined || Number.isNaN(last) ? undefined : last,
    chart: {
      interval: alertInterval(series),
      indicators: series.indicators,
      view: axis ? { axis, log: scaleMode() === "log" } : undefined,
    },
  };
}

/**
 * Keep the server's alerts (ADR-0042) up to date and tell the user what fired. They
 * never enter the browser's own list, so no tab evaluates them; the server fires them.
 * Only the leading tab asks and reports, and the others read the list it shares. While
 * the server does not run them, this asks `tools/list` once, then on focus or every few
 * minutes, in case the operator turned them on.
 */
export function createServerAlertPoll(session: McpSession, enabled: Accessor<boolean>): void {
  createEffect(() => {
    if (enabled()) setServerAlertSession(session);
  });
  onCleanup(installPushClicks());
  // Outlives the leadership, so that switching tabs back and forth does not each ask.
  let lastAsked = 0;
  createEffect(() => {
    if (!enabled() || !leader()) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;
    let asking = false;
    const every = () => (serverAlertsEnabled() ? SERVER_ALERT_POLL_MS : SERVER_ALERT_RECHECK_MS);
    const ask = async () => {
      if (asking) return;
      clearTimeout(timer);
      // Without Web Locks a hidden tab leads as well, and nobody is looking at it.
      if (document.visibilityState === "hidden") {
        timer = setTimeout(() => void ask(), every());
        return;
      }
      asking = true;
      lastAsked = Date.now();
      const on = await refreshServerAlerts();
      asking = false;
      if (stopped) return;
      if (on) {
        void syncPushDevice();
        deliver(
          takeNewFires(pushOnHere()).map((a) => ({ id: a.id, text: serverAlertText(a) })),
        );
      }
      timer = setTimeout(() => void ask(), every());
    };
    /** Asked recently enough that a focus or a new leadership need not ask again? */
    const askedLately = () =>
      Date.now() - lastAsked <
      (serverAlertsEnabled() ? SERVER_ALERT_POLL_MS : SERVER_ALERT_FOCUS_GAP_MS);
    const onFocus = () => {
      if (!serverAlertsEnabled() && !askedLately()) void ask();
    };
    window.addEventListener("focus", onFocus);
    // Untracked: what it asks changes the store it reads, which must not restart this.
    untrack(() => {
      if (askedLately()) timer = setTimeout(() => void ask(), every() - (Date.now() - lastAsked));
      else void ask();
    });
    onCleanup(() => {
      stopped = true;
      clearTimeout(timer);
      window.removeEventListener("focus", onFocus);
    });
  });
}

/**
 * Poll `get_quotes` for the symbols that have active alerts, except the
 * chart symbol, in the leading tab. Also starts the leader election and the
 * audio unlock. Call inside a component.
 */
export function createAlertQuotes(
  session: McpSession,
  enabled: Accessor<boolean>,
  chartSymbol: Accessor<string>,
): void {
  onCleanup(startLeaderElection());
  onCleanup(installAudioUnlock());
  shareLeadingChart();

  const symbols = createMemo(
    () => activeAlertSymbols().filter((s) => s !== chartSymbol()),
    [],
    { equals: (a, b) => a.join() === b.join() },
  );
  const query = createQuotesQuery(session, () => ({
    symbols: symbols(),
    enabled: enabled() && leader(),
  }));
  // Each fetched response is fed once. keepPreviousData serves the previous
  // symbol set's quotes as a placeholder while a new set loads: not a price.
  let fed = 0;
  createEffect(() => {
    if (query.isPlaceholderData || query.dataUpdatedAt === fed) return;
    fed = query.dataUpdatedAt;
    for (const q of query.data?.quotes ?? []) {
      if (!("error" in q) && symbols().includes(q.symbol)) feedPrice(q.symbol, q.last);
    }
  });
}
