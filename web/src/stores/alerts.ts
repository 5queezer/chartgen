// Price alerts (ADR-0020): a Solid signal backed by localStorage under
// `chartgen.alerts.list`. Validated on load (bad entries dropped, ids
// deduplicated) and capped at MAX_ALERTS. `processPrice` is the single
// entry point for live prices: the chart's live feed and the quotes poll
// both call it, and it fires, re-arms and persists.
//
// Several tabs share the key: every mutation and `processPrice` re-read
// storage first, and a `storage` event refreshes the signal, so a tab
// never writes back a stale list. Which tab evaluates is decided in
// services/alertEngine.

import { createSignal } from "solid-js";

import {
  CONDITIONS,
  TRIGGERS,
  evaluate,
  needsRearm,
  rearmed,
  type Alert,
} from "../lib/alerts";
import { normalizeSymbol, symbol as chartSymbol } from "./layout";

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
  if (r.disarmed === true) alert.disarmed = true;
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

/** The stored list, which another tab may have changed since we last looked. */
function fresh(): Alert[] {
  if (typeof localStorage === "undefined") return alerts();
  const list = load();
  setAlertsSignal(list);
  return list;
}

if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key === KEY || e.key === null) setAlertsSignal(load());
  });
}

function commit(next: Alert[]): void {
  setAlertsSignal(next);
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    // Quota exceeded or private-mode storage: best-effort only.
  }
}

// Runtime state, not persisted. `lastPrices` is the latest price seen per
// symbol (for prefilling a dialog); `baselines` is the "prev" of a crossing
// and is forgotten whenever continuity is lost (see `forgetBaseline`).
const lastPrices = new Map<string, number>();
const baselines = new Map<string, number>();

/** Last live price seen for a symbol, or null. */
export function lastPrice(symbol: string): number | null {
  return lastPrices.get(symbol) ?? null;
}

/** Remember a price for display without evaluating alerts. */
export function observePrice(symbol: string, price: number): void {
  if (Number.isFinite(price)) lastPrices.set(symbol, price);
}

/**
 * Drop the crossing baseline of one symbol, or of all. The next price only
 * becomes the new baseline: a close replayed from a cache, a tab that just
 * became the evaluating one, or a resumed alert must not be compared with
 * a price from before the gap.
 */
export function forgetBaseline(symbol?: string): void {
  if (symbol === undefined) baselines.clear();
  else baselines.delete(symbol);
}

export type NewAlert = Pick<Alert, "symbol" | "condition" | "price" | "trigger"> &
  Partial<Pick<Alert, "message">>;

const newId = () =>
  typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `${Date.now().toString(36)}${Math.random().toString(36).slice(2)}`;

/** Create an active alert; null when invalid or at the cap. */
export function addAlert(input: NewAlert, now = Date.now()): Alert | null {
  const list = fresh();
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
  // The chart feed keeps the chart symbol's baseline; any other symbol is
  // fed by a poll that only now starts, so its old price is stale.
  if (symbol !== chartSymbol()) baselines.delete(symbol);
  commit([...list, alert]);
  return alert;
}

/** Change an alert's definition; it is re-armed and active again. */
export function updateAlert(
  id: string,
  patch: Partial<Pick<Alert, "condition" | "price" | "trigger" | "message">>,
): void {
  if (patch.price !== undefined && !isFinitePositive(patch.price)) return;
  commit(
    fresh().map((a) => {
      if (a.id !== id) return a;
      const { message, ...rest } = patch;
      const next: Alert = { ...a, ...rest, active: true };
      delete next.disarmed;
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
  commit(
    fresh().map((a) => {
      if (a.id !== id) return a;
      const next = { ...a, price };
      delete next.disarmed;
      return next;
    }),
  );
}

/** Pause or resume. Resuming re-arms; pausing forgets a past trigger. */
export function setAlertActive(id: string, active: boolean): void {
  commit(
    fresh().map((a) => {
      if (a.id !== id) return a;
      const next = { ...a, active };
      delete next.disarmed;
      if (!active) delete next.lastTriggeredAt;
      else baselines.delete(a.symbol);
      return next;
    }),
  );
}

export function removeAlert(id: string): void {
  commit(fresh().filter((a) => a.id !== id));
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
  const prev = baselines.get(symbol) ?? null;
  baselines.set(symbol, price);
  lastPrices.set(symbol, price);

  const list = fresh();
  const out: Alert[] = [];
  let changed = false;
  const next = list.map((a) => {
    if (a.symbol !== symbol || !a.active) return a;
    let cur = a;
    if (cur.disarmed) {
      if (!rearmed(cur, price)) return cur;
      const { disarmed: _, ...rest } = cur;
      cur = rest;
      changed = true;
    }
    if (!evaluate(cur, prev, price)) return cur;
    changed = true;
    const fired: Alert = {
      ...cur,
      active: cur.trigger === "every",
      lastTriggeredAt: now,
    };
    if (cur.trigger === "every" && needsRearm(cur.condition)) {
      fired.disarmed = true;
    }
    out.push(fired);
    return fired;
  });
  if (changed) commit(next);
  return out;
}
