// Client-side alert evaluation and delivery (ADR-0020). Prices come from
// two places and both end in `deliver`:
//   - the chart symbol: the live feed's last close, via `feedChartPrice`;
//   - every other symbol with an active alert: its own `get_quotes` poll
//     (`createAlertQuotes`), at the watchlist's 5 s cadence. The chart
//     symbol is left out of that poll so one price is never counted twice.
// Delivery is a toast, a short WebAudio beep and, if the user granted the
// permission, a browser Notification.

import { createEffect, createMemo, createSignal, type Accessor } from "solid-js";

import { CONDITION_LABELS, type Alert } from "../lib/alerts";
import { formatPrice } from "../lib/quote";
import type { McpSession } from "../mcp";
import { createQuotesQuery } from "../queries";
import { activeAlertSymbols, processPrice } from "../stores/alerts";
import type { SeriesPayload } from "../types/responses";

const TOAST_MS = 8_000;

export interface Toast {
  id: number;
  text: string;
}

const [toasts, setToasts] = createSignal<Toast[]>([]);
export { toasts };

let toastId = 0;

export function showToast(text: string): void {
  const id = ++toastId;
  setToasts((list) => [...list, { id, text }]);
  setTimeout(() => dismissToast(id), TOAST_MS);
}

export function dismissToast(id: number): void {
  setToasts((list) => list.filter((t) => t.id !== id));
}

/** "BTCUSDT crossing 65000.00" plus the user's message, if any. */
export function alertText(a: Alert): string {
  const head = `${a.symbol} ${CONDITION_LABELS[a.condition].toLowerCase()} ${formatPrice(a.price)}`;
  return a.message ? `${head} — ${a.message}` : head;
}

let audio: AudioContext | undefined;

function beep(): void {
  try {
    audio ??= new AudioContext();
    void audio.resume();
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
    // No audio device or autoplay policy: the toast still shows.
  }
}

/** Ask once, on the first alert the user creates (never on load). */
export function requestNotificationPermission(): void {
  if (typeof Notification !== "undefined" && Notification.permission === "default") {
    void Notification.requestPermission();
  }
}

function deliver(fired: Alert[]): void {
  if (fired.length === 0) return;
  for (const a of fired) {
    const text = alertText(a);
    showToast(`Alert: ${text}`);
    if (typeof Notification !== "undefined" && Notification.permission === "granted") {
      new Notification("chartgen alert", { body: text, tag: a.id });
    }
  }
  beep();
}

/** Evaluate and deliver for one live price. */
export function feedPrice(symbol: string, price: number | null | undefined): void {
  if (price == null) return;
  deliver(processPrice(symbol, price));
}

/** Last close of a live series, the chart symbol's current price. */
export function feedChartPrice(symbol: string, series: SeriesPayload): void {
  feedPrice(symbol, series.bars[series.bars.length - 1]?.c);
}

/**
 * Poll `get_quotes` for the symbols that have active alerts, except the
 * chart symbol. Call inside a component.
 */
export function createAlertQuotes(
  session: McpSession,
  enabled: Accessor<boolean>,
  chartSymbol: Accessor<string>,
): void {
  const symbols = createMemo(
    () => activeAlertSymbols().filter((s) => s !== chartSymbol()),
    [],
    { equals: (a, b) => a.join() === b.join() },
  );
  const query = createQuotesQuery(session, () => ({
    symbols: symbols(),
    enabled: enabled(),
  }));
  createEffect(() => {
    for (const q of query.data?.quotes ?? []) {
      if (!("error" in q) && symbols().includes(q.symbol)) feedPrice(q.symbol, q.last);
    }
  });
}
