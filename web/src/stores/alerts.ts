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
  eventTime,
  indicatorReading,
  isHorizontalTool,
  levelsAnchor,
  isVerticalLine,
  needsRearm,
  rearmedReading,
  sourceOf,
  type Alert,
  type AlertAgainst,
  type AlertSource,
  type ChartValues,
  type ChartView,
  type Reading,
} from "../lib/alerts";
import {
  DRAWINGS_PREFIX,
  INDEX_KEY,
  loadDrawings,
  setEvictionListener,
  type DrawingRecord,
  type TimeAxis,
} from "./drawings";
import { normalizeSymbol, symbol as chartSymbol } from "./layout";

export const MAX_ALERTS = 200;
const MAX_MESSAGE = 200;
const MAX_NAME = 80;
const KEY = "chartgen.alerts.list";
/** The note of a drawing alert whose drawing was deleted. */
export const DRAWING_DELETED = "drawing deleted";
/** Why a drawing alert's line has no value to read; stored as a note only if vertical by its points. */
export const LINE_VERTICAL = "line is vertical";

/** What the alerts need of a stored drawing to tell if it can be read. */
type DrawingRecordRef = Pick<DrawingRecord, "id" | "points"> & { name: string };

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
    case "event":
      return isText(r.key, 64) ? { kind: "event", key: r.key } : null;
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
      if (!isText(r.drawingId, 64) || !isText(r.label, MAX_NAME)) return null;
      if (r.interval !== undefined && !isText(r.interval, 16)) return null;
      return {
        kind: "drawing",
        drawingId: r.drawingId,
        label: r.label,
        ...(r.interval !== undefined ? { interval: r.interval } : {}),
      };
    case "indicator": {
      const against = sanitizeAgainst(r.against);
      return isText(r.interval, 16) &&
        isText(r.instanceKey, 100) &&
        isText(r.signature, 2000) &&
        isIndex(r.line) &&
        isText(r.name, MAX_NAME) &&
        against !== null
        ? {
            kind: "indicator",
            interval: r.interval,
            instanceKey: r.instanceKey,
            signature: r.signature,
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
  // A note explains why the app switched the alert off: only on inactive ones.
  if (isText(r.note, MAX_NAME) && !alert.active) alert.note = r.note;
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

// `inactiveReason` reads the stored drawings: bumped when they change, here or in another tab.
const [drawingsRevision, setDrawingsRevision] = createSignal(0);
const drawingsChanged = () => setDrawingsRevision((n) => n + 1);

if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key === KEY || e.key === null) setAlertsSignal(load());
    if (e.key === null || (e.key.startsWith(DRAWINGS_PREFIX) && e.key !== INDEX_KEY)) drawingsChanged();
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
const readings = new Map<string, { symbol: string; reading: Reading; anchor?: string }>();

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
  forgetReadings(symbol);
}

/**
 * Drop only the indicator readings of a symbol: after bar replay the feed
 * restarts from older values, while the price baseline (fed by the quotes
 * poll meanwhile) is still good.
 */
export function forgetReadings(symbol: string): void {
  for (const [id, r] of readings) if (r.symbol === symbol) readings.delete(id);
}

// --- What the chart shows (ADR-0034) ---------------------------------
// Indicator and sloped-drawing alerts are only evaluated on the chart's own
// series, so the Alerts tab needs to know which symbol, interval and
// indicators it holds. `indicatorSignatures` maps the chart's instance keys
// to their name and params (`activeIndicatorsKey` of the wire entry): an
// alert whose instance changed since its creation does not run.

const [indicatorSignatures, setIndicatorSignaturesSignal] = createSignal<
  Readonly<Record<string, string>>
>({}, { equals: (a, b) => JSON.stringify(a) === JSON.stringify(b) });
export { indicatorSignatures };
export const setIndicatorSignatures = (s: Readonly<Record<string, string>>): void =>
  void setIndicatorSignaturesSignal(s);

export interface ChartContext {
  symbol: string;
  interval: string;
  /** Instance keys of the indicators in the series. */
  keys: readonly string[];
  /** The series' bars; a sloped line's alert is read on them at its interval. */
  axis?: TimeAxis | null;
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
        a.keys.join("\n") === b.keys.join("\n") &&
        a.axis?.firstTs === b.axis?.firstTs &&
        a.axis?.lastTs === b.axis?.lastTs),
  },
);
export { chartContext };
export const setChartContext = (ctx: ChartContext | null): void =>
  void setChartContextSignal(ctx);

/**
 * What the tab that evaluates alerts shows, as "BTCUSDT 1h", when that is
 * another tab and it said so. Only it evaluates (`leader` in alertEngine).
 */
const [evaluatingChart, setEvaluatingChartSignal] = createSignal<string | null>(null);
export const setEvaluatingChart = (chart: string | null): void =>
  void setEvaluatingChartSignal(chart);

/**
 * Why an alert cannot fire right now, for the Alerts tab and Object Tree:
 * its note ("drawing deleted"), "open BTCUSDT 1h" (an indicator or sloped
 * drawing alert while the chart shows something else), "open BTCUSDT 1h in the
 * other tab" (the same for the tab that evaluates), "line is vertical" (both
 * points on one bar of these bars, which only holds in this tab, so it is never
 * stored as a note), "indicator not on the chart" or "indicator changed" (other
 * name or params under its key). Null when it is evaluated, or simply paused or
 * triggered.
 */
export function inactiveReason(
  a: Alert,
  ctx: ChartContext | null = chartContext(),
  signatures: Readonly<Record<string, string>> = indicatorSignatures(),
): string | null {
  if (a.note) return a.note;
  const src = a.source;
  if (!a.active || src === undefined || src.kind === "price") return null;
  const interval = src.interval;
  if (interval === undefined) return null;
  const evaluating = evaluatingChart();
  if (evaluating !== null && evaluating !== `${a.symbol} ${interval}`) {
    return `open ${a.symbol} ${interval} in the other tab`;
  }
  if (ctx?.symbol !== a.symbol || ctx.interval !== interval) {
    return `open ${a.symbol} ${interval}`;
  }
  if (src.kind === "drawing") {
    void drawingsRevision();
    const rec = loadDrawings(a.symbol, false).find((d) => d.id === src.drawingId);
    return rec && !isHorizontalTool(rec.name) && isVerticalLine(rec, ctx.axis) ? LINE_VERTICAL : null;
  }
  if (!ctx.keys.includes(src.instanceKey)) return "indicator not on the chart";
  return signatures[src.instanceKey] === src.signature ? null : "indicator changed";
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
      const next: Alert = switchOn({ ...a, ...rest });
      if (value !== undefined && src?.kind === "indicator" && src.against.kind === "value") {
        next.source = { ...src, against: { kind: "value", value } };
      }
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
 * Why the drawing `rec` cannot be read for alert `a` in any tab, or null: it is gone, or
 * vertical by its points. The note is stored and all tabs share it, so a line falling on
 * one bar of the bars on screen does not count here (`inactiveReason` says that one).
 * A horizontal tool's level needs no second point: a ray stored from when it took two
 * clicks can have them on one time, and it is still readable.
 */
function drawingNote(a: Alert, rec: DrawingRecordRef | undefined): string | null {
  if (a.source?.kind !== "drawing") return null;
  if (!rec) return DRAWING_DELETED;
  return !isHorizontalTool(rec.name) && isVerticalLine(rec) ? LINE_VERTICAL : null;
}

/** Why a drawing alert's stored drawing cannot be read, or null when it can. */
function deadDrawingNote(a: Alert): string | null {
  const src = a.source;
  if (src?.kind !== "drawing") return null;
  return drawingNote(a, loadDrawings(a.symbol, false).find((d) => d.id === src.drawingId));
}

/**
 * `a` made active and re-armed; a drawing alert whose drawing is gone or
 * vertical stays off, with the note, instead (it could never fire).
 */
function switchOn(a: Alert): Alert {
  const next: Alert = { ...a, active: true };
  delete next.disarmed;
  delete next.note;
  const note = deadDrawingNote(a);
  return note ? { ...next, active: false, note } : next;
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
      if (active) {
        baselines.delete(a.symbol);
        readings.delete(a.id);
        return switchOn(a);
      }
      const next = { ...a, active };
      delete next.disarmed;
      delete next.note;
      delete next.lastTriggeredAt;
      return next;
    }),
  );
}

export function removeAlert(id: string): void {
  readings.delete(id);
  commit(fresh().filter((a) => a.id !== id));
}

/**
 * The drawings of `symbol` are now `records` (called when they are saved, and with
 * none when the symbol's drawings are evicted). Active alerts on a drawing that is
 * gone, or on a line with no value to read (`drawingNote`), are switched off with a
 * note, not deleted, so the user sees what happened; when the drawing comes back
 * (undo) or stops being vertical they are switched on again.
 */
export function syncDrawingAlerts(symbol: string, records: readonly DrawingRecordRef[]): void {
  drawingsChanged();
  const list = fresh();
  let changed = false;
  const next = list.map((a) => {
    const src = a.source;
    if (a.symbol !== symbol || src?.kind !== "drawing") return a;
    const note = drawingNote(a, records.find((r) => r.id === src.drawingId));
    const ours = a.note === DRAWING_DELETED || a.note === LINE_VERTICAL;
    if (note !== null && (a.active || (ours && a.note !== note))) {
      changed = true;
      const off: Alert = { ...a, active: false, note };
      // A line passing through vertical comes back as it was: a fired "every" alert
      // still has to re-arm.
      if (note === DRAWING_DELETED) delete off.disarmed;
      return off;
    }
    if (note === null && ours) {
      changed = true;
      baselines.delete(symbol);
      const on: Alert = { ...a, active: true };
      delete on.note;
      return on;
    }
    return a;
  });
  if (changed) commit(next);
}
// Evicted drawings (the per-symbol LRU in stores/drawings) are deleted too.
setEvictionListener((symbol) => syncDrawingAlerts(symbol, []));

/** Needs the chart's series: indicator alerts and sloped-drawing alerts. */
const needsChart = (a: Alert) =>
  a.source !== undefined && a.source.kind !== "price" && a.source.interval !== undefined;

/**
 * Symbols with an active alert a price poll can evaluate, in first-seen
 * order. Alerts that need the chart's series are left out.
 */
export function activeAlertSymbols(): string[] {
  return [
    ...new Set(
      alerts()
        .filter((a) => a.active && !needsChart(a))
        .map((a) => a.symbol),
    ),
  ];
}

/** Extra context of a price: what a drawing or indicator alert reads besides it. */
export interface Tick {
  /** Epoch ms of the bar the price belongs to; defaults to `now`. */
  time?: number;
  /**
   * The chart's series, when the price is its last close: its interval and
   * indicators (indicator alerts), its bars and scale (sloped drawings).
   */
  chart?: ChartValues & { view?: ChartView };
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
        // A sloped line needs the bars of the interval it was drawn on.
        const view = src.interval === undefined ? undefined : tick.chart?.view;
        if (src.interval !== undefined && (!view || tick.chart?.interval !== src.interval)) {
          return null;
        }
        // Both points on one bar of these bars: no value, and no re-arming from having none.
        if (view && !isHorizontalTool(rec.name) && isVerticalLine(rec, view.axis)) return null;
        // The line as it is now, at the previous and the current bar time:
        // moving a drawing over the price does not fire it.
        return {
          prev: prev && drawingLevels(rec, prev.time, view).map((l) => prev.price - l),
          cur: drawingLevels(rec, time, view).map((l) => price - l),
        };
      }
      case "indicator": {
        const same = indicatorSignatures()[src.instanceKey] === src.signature;
        if (src.against.kind === "event") {
          // A newer bar than last seen is a crossing of "has happened". The first
          // reading only sets the baseline: an old detection does not fire.
          const at = same ? eventTime(src, tick.chart) : null;
          const seen = readings.get(a.id)?.reading[0];
          if (at === null) readings.delete(a.id);
          else readings.set(a.id, { symbol, reading: [at] });
          if (at === null) return null;
          return { prev: seen === undefined ? null : [-1], cur: [seen !== undefined && at > seen ? 1 : -1] };
        }
        const cur = same ? indicatorReading(src, price, tick.chart) : null;
        const anchor = levelsAnchor(src, tick.chart);
        const was = readings.get(a.id);
        const before = was && was.anchor === anchor ? was.reading : null;
        if (cur === null) readings.delete(a.id);
        else readings.set(a.id, { symbol, reading: cur, anchor });
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
