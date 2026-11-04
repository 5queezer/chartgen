// Price alerts (ADR-0020): a Solid signal backed by localStorage under
// `chartgen.alerts.list`. Validated on load (bad entries dropped, ids
// deduplicated) and capped at MAX_ALERTS. `processPrice` is the single
// entry point for live prices: the chart's live feed and the quotes poll
// both call it, and it fires, re-arms and persists.

import { createSignal } from "solid-js";

import {
  CONDITIONS,
  TRIGGERS,
  evaluate,
  needsRearm,
  rearmed,
  type Alert,
} from "../lib/alerts";
import { normalizeSymbol } from "./layout";

export const MAX_ALERTS = 200;
const MAX_MESSAGE = 200;
const KEY = "chartgen.alerts.list";

const isFinitePositive = (v: unknown): v is number =>
  typeof v === "number" && Number.isFinite(v) && v > 0;

function sanitizeAlert(raw: unknown): Alert | null {
  if (typeof raw !== "object" || raw === null) return null;
  const r = raw as Record<string, unknown>;
  const symbol = normalizeSymbol(r.symbol);
  if (
    typeof r.id !== "string" ||
    r.id === "" ||
    symbol === null ||
    !(CONDITIONS as readonly unknown[]).includes(r.condition) ||
    !isFinitePositive(r.price) ||
    !(TRIGGERS as readonly unknown[]).includes(r.trigger) ||
    typeof r.active !== "boolean" ||
    typeof r.createdAt !== "number" ||
    !Number.isFinite(r.createdAt)
  ) {
    return null;
  }
  const alert: Alert = {
    id: r.id,
    symbol,
    condition: r.condition as Alert["condition"],
    price: r.price,
    trigger: r.trigger as Alert["trigger"],
    active: r.active,
    createdAt: r.createdAt,
  };
  if (typeof r.message === "string" && r.message !== "") {
    alert.message = r.message.slice(0, MAX_MESSAGE);
  }
  if (
    typeof r.lastTriggeredAt === "number" &&
    Number.isFinite(r.lastTriggeredAt)
  ) {
    alert.lastTriggeredAt = r.lastTriggeredAt;
  }
  return alert;
}

/** Drop invalid entries and duplicate ids, cap at MAX_ALERTS. `null` if not an array. */
export function sanitizeAlerts(raw: unknown): Alert[] | null {
  if (!Array.isArray(raw)) return null;
  const seen = new Set<string>();
  const out: Alert[] = [];
  for (const item of raw) {
    const a = sanitizeAlert(item);
    if (a === null || seen.has(a.id)) continue;
    seen.add(a.id);
    out.push(a);
    if (out.length >= MAX_ALERTS) break;
  }
  return out;
}

function load(): Alert[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(KEY);
    return (raw === null ? null : sanitizeAlerts(JSON.parse(raw))) ?? [];
  } catch {
    return [];
  }
}

const [alerts, setAlertsSignal] = createSignal<Alert[]>(load());
export { alerts };

function commit(next: Alert[]): void {
  setAlertsSignal(next);
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    // Quota exceeded or private-mode storage: best-effort only.
  }
}

// Runtime state, not persisted: the last price seen per symbol (the "prev"
// of a crossing) and the fired "every" state alerts waiting to re-arm.
const lastPrices = new Map<string, number>();
const disarmed = new Set<string>();

/** Last live price seen for a symbol, or null. */
export function lastPrice(symbol: string): number | null {
  return lastPrices.get(symbol) ?? null;
}

export type NewAlert = Pick<Alert, "symbol" | "condition" | "price" | "trigger"> &
  Partial<Pick<Alert, "message">>;

const newId = () =>
  typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `${Date.now().toString(36)}${Math.random().toString(36).slice(2)}`;

/** Create an active alert; null when invalid or at the cap. */
export function addAlert(input: NewAlert, now = Date.now()): Alert | null {
  const list = alerts();
  const symbol = normalizeSymbol(input.symbol);
  const message = input.message?.trim().slice(0, MAX_MESSAGE);
  if (
    list.length >= MAX_ALERTS ||
    symbol === null ||
    !isFinitePositive(input.price)
  ) {
    return null;
  }
  const alert: Alert = {
    id: newId(),
    symbol,
    condition: input.condition,
    price: input.price,
    trigger: input.trigger,
    active: true,
    createdAt: now,
    ...(message ? { message } : {}),
  };
  commit([...list, alert]);
  return alert;
}

/** Change an alert's definition; it is re-armed and active again. */
export function updateAlert(
  id: string,
  patch: Partial<Pick<Alert, "condition" | "price" | "trigger" | "message">>,
): void {
  if (patch.price !== undefined && !isFinitePositive(patch.price)) return;
  disarmed.delete(id);
  commit(
    alerts().map((a) => {
      if (a.id !== id) return a;
      const { message, ...rest } = patch;
      const next: Alert = { ...a, ...rest, active: true };
      if (message !== undefined) {
        const m = message.trim().slice(0, MAX_MESSAGE);
        if (m === "") delete next.message;
        else next.message = m;
      }
      delete next.lastTriggeredAt;
      return next;
    }),
  );
}

/**
 * Move an alert's level only (a chart drag). Unlike `updateAlert` it keeps
 * the alert's state: a paused alert stays paused.
 */
export function moveAlert(id: string, price: number): void {
  if (!isFinitePositive(price)) return;
  disarmed.delete(id);
  commit(alerts().map((a) => (a.id === id ? { ...a, price } : a)));
}

/** Pause or resume. Resuming re-arms; pausing forgets a past trigger. */
export function setAlertActive(id: string, active: boolean): void {
  disarmed.delete(id);
  commit(
    alerts().map((a) => {
      if (a.id !== id) return a;
      const next = { ...a, active };
      if (!active) delete next.lastTriggeredAt;
      return next;
    }),
  );
}

export function removeAlert(id: string): void {
  disarmed.delete(id);
  commit(alerts().filter((a) => a.id !== id));
}

/** Symbols that have at least one active alert, in first-seen order. */
export function activeAlertSymbols(): string[] {
  return [...new Set(alerts().filter((a) => a.active).map((a) => a.symbol))];
}

/**
 * Feed a live price for `symbol`. Evaluates its active alerts against the
 * previous price seen, applies the trigger mode ("once" deactivates, "every"
 * stays active, state conditions disarm until `rearmed`) and returns the
 * alerts that fired.
 */
export function processPrice(
  symbol: string,
  price: number,
  now = Date.now(),
): Alert[] {
  if (!Number.isFinite(price)) return [];
  const prev = lastPrices.get(symbol) ?? null;
  lastPrices.set(symbol, price);

  const fired = new Set<string>();
  for (const a of alerts()) {
    if (a.symbol !== symbol || !a.active) continue;
    if (disarmed.has(a.id)) {
      if (!rearmed(a, price)) continue;
      disarmed.delete(a.id);
    }
    if (!evaluate(a, prev, price)) continue;
    fired.add(a.id);
    if (a.trigger === "every" && needsRearm(a.condition)) disarmed.add(a.id);
  }
  if (fired.size === 0) return [];

  const out: Alert[] = [];
  commit(
    alerts().map((a) => {
      if (!fired.has(a.id)) return a;
      const next = {
        ...a,
        active: a.trigger === "every",
        lastTriggeredAt: now,
      };
      out.push(next);
      return next;
    }),
  );
  return out;
}
