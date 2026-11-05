// Alerts (ADR-0020, sources ADR-0034): a Solid signal backed by localStorage
// under `chartgen.alerts.list`. Validated on load (bad entries dropped, ids
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
  drawingLevels,
  fires,
  indicatorReading,
  needsRearm,
  rearmedReading,
  sourceOf,
  type Alert,
  type AlertAgainst,
  type AlertSource,
  type ChartValues,
  type Reading,
} from "../lib/alerts";
import { loadDrawings, type DrawingRecord } from "./drawings";
import { normalizeSymbol, symbol as chartSymbol } from "./layout";

export const MAX_ALERTS = 200;
const MAX_MESSAGE = 200;
const MAX_NAME = 80;
const KEY = "chartgen.alerts.list";
/** The note of a drawing alert whose drawing was deleted. */
export const DRAWING_DELETED = "drawing deleted";

const isFinitePositive = (v: unknown): v is number =>
  typeof v === "number" && Number.isFinite(v) && v > 0;
const isText = (v: unknown, max: number): v is string =>
  typeof v === "string" && v !== "" && v.length <= max;
const isIndex = (v: unknown): v is number =>
  typeof v === "number" && Number.isInteger(v) && v >= 0 && v < 64;

function sanitizeAgainst(raw: unknown): AlertAgainst | null {
  if (typeof raw !== "object" || raw === null) return null;
  const r = raw as Record<string, unknown>;
  switch (r.kind) {
    case "price":
      return { kind: "price" };
    case "value":
      return typeof r.value === "number" && Number.isFinite(r.value)
        ? { kind: "value", value: r.value }
        : null;
    case "plot":
      return isIndex(r.line) && isText(r.name, MAX_NAME)
        ? { kind: "plot", line: r.line, name: r.name }
        : null;
    default:
      return null;
  }
}

/** A valid non-price source, `undefined` for a price alert, null if invalid. */
function sanitizeSource(raw: unknown): AlertSource | null | undefined {
  if (raw === undefined) return undefined;
  if (typeof raw !== "object" || raw === null) return null;
  const r = raw as Record<string, unknown>;
  switch (r.kind) {
    case "price":
      return undefined;
    case "drawing":
      return isText(r.drawingId, 64) && isText(r.label, MAX_NAME)
        ? { kind: "drawing", drawingId: r.drawingId, label: r.label }
        : null;
    case "indicator": {
      const against = sanitizeAgainst(r.against);
      return typeof r.interval === "string" &&
        r.interval.length <= 16 &&
        isText(r.instanceKey, 100) &&
        isIndex(r.line) &&
        isText(r.name, MAX_NAME) &&
        against !== null
        ? {
            kind: "indicator",
            interval: r.interval,
            instanceKey: r.instanceKey,
            line: r.line,
            name: r.name,
            against,
          }
        : null;
    }
    default:
      return null;
  }
}

function sanitizeAlert(raw: unknown): Alert | null {
  if (typeof raw !== "object" || raw === null) return null;
  const r = raw as Record<string, unknown>;
  const symbol = normalizeSymbol(r.symbol);
  const source = sanitizeSource(r.source);
  if (
    typeof r.id !== "string" ||
    r.id === "" ||
    symbol === null ||
    source === null ||
    !(CONDITIONS as readonly unknown[]).includes(r.condition) ||
    // Only a price alert has a level of its own.
    (source === undefined && !isFinitePositive(r.price)) ||
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
    price: source === undefined ? (r.price as number) : 0,
    trigger: r.trigger as Alert["trigger"],
    active: r.active,
    createdAt: r.createdAt,
  };
  if (source !== undefined) alert.source = source;
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
  if (isText(r.note, MAX_NAME)) alert.note = r.note;
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
  // Same content: keep the signal (and what renders from it) untouched.
  if (JSON.stringify(list) !== JSON.stringify(alerts())) setAlertsSignal(list);
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
// symbol (for prefilling a dialog). `baselines` is the "prev" of a crossing
// for price and drawing alerts: the previous price and the bar time it was
// for (a sloped line is read at that time). `readings` is the previous
// reading of each indicator alert. Both are forgotten whenever continuity
// is lost (see `forgetBaseline`).
const lastPrices = new Map<string, number>();
const baselines = new Map<string, { price: number; time: number }>();
const readings = new Map<string, { symbol: string; reading: Reading }>();

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
  if (symbol === undefined) {
    baselines.clear();
    readings.clear();
    return;
  }
  baselines.delete(symbol);
  for (const [id, r] of readings) if (r.symbol === symbol) readings.delete(id);
}

// --- What the chart shows (ADR-0034) ---------------------------------
// Indicator alerts are only evaluated on the chart's own series, so the
// Alerts tab needs to know which symbol, interval and indicators it holds.

export interface ChartContext {
  symbol: string;
  interval: string;
  /** Instance keys of the indicators in the series. */
  keys: readonly string[];
}

const [chartContext, setChartContextSignal] = createSignal<ChartContext | null>(
  null,
  {
    equals: (a, b) =>
      a === b ||
      (a !== null &&
        b !== null &&
        a.symbol === b.symbol &&
        a.interval === b.interval &&
        a.keys.join("\n") === b.keys.join("\n")),
  },
);
export { chartContext };
export const setChartContext = (ctx: ChartContext | null): void =>
  void setChartContextSignal(ctx);

/**
 * Why an alert cannot fire right now, for the Alerts tab: "drawing deleted",
 * "open BTCUSDT 1h" (an indicator alert off the chart) or "indicator
 * removed". Null when it is evaluated (or simply paused).
 */
export function inactiveReason(
  a: Alert,
  ctx: ChartContext | null = chartContext(),
): string | null {
  if (a.note) return a.note;
  const src = a.source;
  if (!a.active || src?.kind !== "indicator") return null;
  if (ctx?.symbol !== a.symbol || ctx.interval !== src.interval) {
    return `open ${a.symbol} ${src.interval}`.trimEnd();
  }
  return ctx.keys.includes(src.instanceKey) ? null : "indicator not on the chart";
}
// ---------------------------------------------------------------------

export type NewAlert = Pick<Alert, "symbol" | "condition" | "trigger"> &
  Partial<Pick<Alert, "price" | "message" | "source">>;

const newId = () =>
  typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `${Date.now().toString(36)}${Math.random().toString(36).slice(2)}`;

/** Create an active alert; null when invalid or at the cap. */
export function addAlert(input: NewAlert, now = Date.now()): Alert | null {
  const list = fresh();
  if (list.length >= MAX_ALERTS) return null;
  const alert = sanitizeAlert({
    ...input,
    id: newId(),
    message: input.message?.trim(),
    price: input.price ?? 0,
    active: true,
    createdAt: now,
  });
  if (alert === null) return null;
  // The chart feed keeps the chart symbol's baseline; any other symbol is
  // fed by a poll that only now starts, so its old price is stale.
  if (alert.symbol !== chartSymbol()) baselines.delete(alert.symbol);
  commit([...list, alert]);
  return alert;
}

/**
 * Change an alert's definition; it is re-armed and active again. `value`
 * is the level of an indicator alert compared with a value.
 */
export function updateAlert(
  id: string,
  patch: Partial<Pick<Alert, "condition" | "price" | "trigger" | "message">> & {
    value?: number;
  },
): void {
  if (patch.price !== undefined && !isFinitePositive(patch.price)) return;
  if (patch.value !== undefined && !Number.isFinite(patch.value)) return;
  readings.delete(id);
  commit(
    fresh().map((a) => {
      if (a.id !== id) return a;
      const { message, value, ...rest } = patch;
      const src = a.source;
      const next: Alert = { ...a, ...rest, active: true };
      if (value !== undefined && src?.kind === "indicator" && src.against.kind === "value") {
        next.source = { ...src, against: { kind: "value", value } };
      }
      delete next.disarmed;
      delete next.note;
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
      if (a.id !== id || a.source !== undefined) return a;
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
      delete next.note;
      if (!active) delete next.lastTriggeredAt;
      else {
        baselines.delete(a.symbol);
        readings.delete(a.id);
      }
      return next;
    }),
  );
}

export function removeAlert(id: string): void {
  readings.delete(id);
  commit(fresh().filter((a) => a.id !== id));
}

/**
 * The drawings of `symbol` are now `ids` (called when they are saved):
 * alerts on a drawing that is gone are switched off with a note, not
 * deleted, so the user sees what happened.
 */
export function syncDrawingAlerts(
  symbol: string,
  ids: readonly (string | undefined)[],
): void {
  const have = new Set(ids);
  const list = fresh();
  let changed = false;
  const next = list.map((a) => {
    const src = a.source;
    if (a.symbol !== symbol || src?.kind !== "drawing" || a.note || have.has(src.drawingId)) {
      return a;
    }
    changed = true;
    const off: Alert = { ...a, active: false, note: DRAWING_DELETED };
    delete off.disarmed;
    return off;
  });
  if (changed) commit(next);
}

/**
 * Symbols with an active alert a price poll can evaluate, in first-seen
 * order. Indicator alerts only run on the chart's series, so they are left out.
 */
export function activeAlertSymbols(): string[] {
  return [
    ...new Set(
      alerts()
        .filter((a) => a.active && a.source?.kind !== "indicator")
        .map((a) => a.symbol),
    ),
  ];
}

/** Extra context of a price: what a drawing or indicator alert reads besides it. */
export interface Tick {
  /** Epoch ms of the bar the price belongs to; defaults to `now`. */
  time?: number;
  /** The chart's series, when the price is its last close (indicator alerts). */
  chart?: ChartValues;
}

/**
 * Feed a live price for `symbol`. Evaluates its active alerts against the
 * previous reading, applies the trigger mode ("once" deactivates, "every"
 * stays active, state conditions disarm until re-armed) and returns the
 * alerts that fired.
 */
export function processPrice(
  symbol: string,
  price: number,
  now = Date.now(),
  tick: Tick = {},
): Alert[] {
  if (!Number.isFinite(price)) return [];
  const time = tick.time ?? now;
  const prev = baselines.get(symbol) ?? null;
  baselines.set(symbol, { price, time });
  lastPrices.set(symbol, price);

  // Drawings are read from storage, once, only if an alert needs them.
  let drawings: DrawingRecord[] | undefined;
  const readingOf = (a: Alert): { prev: Reading | null; cur: Reading } | null => {
    const src = sourceOf(a);
    switch (src.kind) {
      case "price":
        return {
          prev: prev && [prev.price - a.price],
          cur: [price - a.price],
        };
      case "drawing": {
        drawings ??= loadDrawings(symbol, false);
        const rec = drawings.find((d) => d.id === src.drawingId);
        if (!rec) return null;
        // The line as it is now, at the previous and the current bar time:
        // moving a drawing over the price does not fire it.
        return {
          prev: prev && drawingLevels(rec, prev.time).map((l) => prev.price - l),
          cur: drawingLevels(rec, time).map((l) => price - l),
        };
      }
      case "indicator": {
        const cur = indicatorReading(src, price, tick.chart);
        const before = readings.get(a.id)?.reading ?? null;
        if (cur === null) readings.delete(a.id);
        else readings.set(a.id, { symbol, reading: cur });
        return cur && { prev: before, cur };
      }
    }
  };

  const list = fresh();
  const out: Alert[] = [];
  let changed = false;
  const next = list.map((a) => {
    if (a.symbol !== symbol || !a.active) return a;
    const r = readingOf(a);
    if (r === null) return a;
    let cur = a;
    if (cur.disarmed) {
      if (!rearmedReading(cur.condition, r.cur)) return cur;
      const { disarmed: _, ...rest } = cur;
      cur = rest;
      changed = true;
    }
    if (!fires(cur.condition, r.prev, r.cur)) return cur;
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
