// Active indicator set — Solid signal backed by localStorage.
//
// Wire-compatible with the chartgen `generate_chart` tool: each entry is
// either a bare indicator name (string) or an object `{ name, ...params }`.
// On first load the set is seeded with the v0.1 default so the chart looks
// the same before the user touches the picker.
//
// Client-only state: an entry may also carry `hidden: true` (ADR-0013). It is
// persisted with the entry but never sent to the server — `toWire` drops
// hidden entries and strips the field from the rest. Entries are unique by
// name because the server keys its response by indicator name.
//
// An entry may also carry a client-only `style` (ADR-0017): per-plot color,
// width, line style and visibility. It is validated on load and, like
// `hidden`, never sent to the server nor part of the query key.

import { createSignal } from "solid-js";

import { isColor } from "../lib/color";

/**
 * Wire shape for a single indicator request. Matches the `indicators` field
 * on `generate_chart` input: bare name or object with `name` plus arbitrary
 * numeric parameters.
 */
export type ActiveIndicator =
  | string
  | { name: string; [k: string]: unknown };

export type PlotLineStyle = "solid" | "dashed" | "dotted";

/** Client-side override of one plotted line or histogram. */
export interface PlotStyle {
  color?: string;
  /** Stroke width in px, 1-4. */
  width?: number;
  lineStyle?: PlotLineStyle;
  visible?: boolean;
  /**
   * Server label of the line this entry was saved for. Only set on
   * index-keyed entries (`line<i>`): the entry is ignored once the line at
   * that index has a different label.
   */
  label?: string;
}

/** Per-indicator style overrides, keyed by `plotKey`. */
export interface IndicatorStyle {
  plots: Record<string, PlotStyle>;
}

const STORAGE_KEY = "chartgen.indicators.active";

export const DEFAULT_SET: ActiveIndicator[] = [
  "ema_stack",
  { name: "rsi", length: 14 },
];

function load(): ActiveIndicator[] {
  if (typeof localStorage === "undefined") return [...DEFAULT_SET];
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === null) return [...DEFAULT_SET];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [...DEFAULT_SET];
    // Two cases:
    // 1. Stored array is explicitly empty → respect the user's choice
    //    (they deliberately cleared all indicators).
    // 2. Filter via isActiveIndicator drops every entry → treat the
    //    stored data as corrupted and fall back to DEFAULT_SET rather
    //    than presenting an empty chart silently.
    if (parsed.length === 0) return [];
    // Entries are unique by name (the server keys its response by name);
    // keep the first of any duplicates.
    const seen = new Set<string>();
    const valid = parsed.filter((e) => {
      if (!isActiveIndicator(e) || seen.has(indicatorName(e))) return false;
      seen.add(indicatorName(e));
      return true;
    });
    return valid.length > 0 ? valid.map(sanitizeEntry) : [...DEFAULT_SET];
  } catch {
    return [...DEFAULT_SET];
  }
}

function persist(value: ActiveIndicator[]): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(value));
  } catch {
    // Quota exceeded or private-mode storage: best-effort only.
  }
}

const MAX_LABEL = 64;
// `line<i>` / `hist<i>` (index-keyed) or `l:<label>` (label-keyed line).
const PLOT_KEY_RE = new RegExp(`^((line|hist)\\d+|l:.{1,${MAX_LABEL}})$`);
const LINE_STYLES: readonly string[] = ["solid", "dashed", "dotted"];

/**
 * Index key of a plot: its kind and index in the series payload (`line0`,
 * `hist1`). Used for histograms and for lines whose label is not unique.
 */
export function plotKey(kind: "line" | "hist", index: number): string {
  return `${kind}${index}`;
}

/** Key of a line identified by its (payload-unique) server label. */
export function labelKey(label: string): string {
  return `l:${label}`;
}

function sanitizePlot(x: unknown): PlotStyle | undefined {
  if (!x || typeof x !== "object" || Array.isArray(x)) return undefined;
  const { color, width, lineStyle, visible, label } = x as Record<
    string,
    unknown
  >;
  const out: PlotStyle = {};
  if (isColor(color)) out.color = color;
  if (
    typeof width === "number" &&
    Number.isInteger(width) &&
    width >= 1 &&
    width <= 4
  ) {
    out.width = width;
  }
  if (typeof lineStyle === "string" && LINE_STYLES.includes(lineStyle)) {
    out.lineStyle = lineStyle as PlotLineStyle;
  }
  if (typeof visible === "boolean") out.visible = visible;
  if (typeof label === "string" && label.length > 0 && label.length <= MAX_LABEL) {
    out.label = label;
  }
  return Object.keys(out).length > 0 ? out : undefined;
}

/**
 * Validate an untrusted style (e.g. from localStorage): invalid fields and
 * plots are dropped. Returns undefined when nothing valid remains.
 */
export function sanitizeStyle(x: unknown): IndicatorStyle | undefined {
  if (!x || typeof x !== "object" || Array.isArray(x)) return undefined;
  const raw = (x as { plots?: unknown }).plots;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return undefined;
  const plots: Record<string, PlotStyle> = {};
  for (const [key, value] of Object.entries(raw)) {
    const plot = PLOT_KEY_RE.test(key) ? sanitizePlot(value) : undefined;
    if (plot) plots[key] = plot;
  }
  return Object.keys(plots).length > 0 ? { plots } : undefined;
}

/** Entry with a validated `style`; the field is dropped when nothing valid remains. */
function sanitizeEntry(entry: ActiveIndicator): ActiveIndicator {
  if (typeof entry === "string" || !("style" in entry)) return entry;
  return makeEntry(
    entry.name,
    indicatorParams(entry),
    isHidden(entry),
    sanitizeStyle(entry.style),
  );
}

export function isActiveIndicator(x: unknown): x is ActiveIndicator {
  if (typeof x === "string") return x.length > 0;
  if (x && typeof x === "object" && !Array.isArray(x)) {
    const name = (x as { name?: unknown }).name;
    return typeof name === "string" && name.length > 0;
  }
  return false;
}

/** True when the entry is hidden on the client (never sent to the server). */
export function isHidden(entry: ActiveIndicator): boolean {
  return typeof entry !== "string" && entry.hidden === true;
}

/** The client-only style of an entry, if any. */
export function indicatorStyle(
  entry: ActiveIndicator,
): IndicatorStyle | undefined {
  return typeof entry === "string"
    ? undefined
    : (entry.style as IndicatorStyle | undefined);
}

/** Parameters of an entry, without `name` and the client-only `hidden` and `style`. */
export function indicatorParams(entry: ActiveIndicator): Record<string, unknown> {
  if (typeof entry === "string") return {};
  const { name: _name, hidden: _hidden, style: _style, ...params } = entry;
  return params;
}

/** Build an entry from a name and params; a param-less entry stays a bare name. */
function makeEntry(
  name: string,
  params: Record<string, unknown>,
  hidden: boolean,
  style?: IndicatorStyle,
): ActiveIndicator {
  const extra = {
    ...(hidden ? { hidden: true } : {}),
    ...(style ? { style } : {}),
  };
  if (Object.keys(params).length === 0 && Object.keys(extra).length === 0) {
    return name;
  }
  return { name, ...params, ...extra };
}

/** Replace the parameters of the entry at `index`, keeping name and visibility. */
export function withParams(
  entries: ActiveIndicator[],
  index: number,
  params: Record<string, unknown>,
): ActiveIndicator[] {
  const entry = entries[index];
  if (entry === undefined) return entries;
  return entries.map((e, i) =>
    i === index
      ? makeEntry(indicatorName(e), params, isHidden(e), indicatorStyle(e))
      : e,
  );
}

/** Set the hidden flag of the entry at `index`. */
export function withHidden(
  entries: ActiveIndicator[],
  index: number,
  hidden: boolean,
): ActiveIndicator[] {
  const entry = entries[index];
  if (entry === undefined) return entries;
  return entries.map((e, i) =>
    i === index
      ? makeEntry(indicatorName(e), indicatorParams(e), hidden, indicatorStyle(e))
      : e,
  );
}

/** Replace the style of the entry at `index`; undefined clears it. */
export function withStyle(
  entries: ActiveIndicator[],
  index: number,
  style: IndicatorStyle | undefined,
): ActiveIndicator[] {
  const entry = entries[index];
  if (entry === undefined) return entries;
  return entries.map((e, i) =>
    i === index
      ? makeEntry(
          indicatorName(e),
          indicatorParams(e),
          isHidden(e),
          sanitizeStyle(style),
        )
      : e,
  );
}

/** Request payload for `generate_chart`: visible entries, client-only fields stripped. */
export function toWire(entries: ActiveIndicator[]): ActiveIndicator[] {
  return entries
    .filter((e) => !isHidden(e))
    .map((e) => makeEntry(indicatorName(e), indicatorParams(e), false));
}

/** Extract the indicator name from an entry regardless of its form. */
export function indicatorName(entry: ActiveIndicator): string {
  return typeof entry === "string" ? entry : entry.name;
}

const [activeIndicators, setActiveIndicatorsSignal] =
  createSignal<ActiveIndicator[]>(load());

export { activeIndicators };

/** Replace the entire set, persisting to localStorage. */
export function setActiveIndicators(next: ActiveIndicator[]): void {
  setActiveIndicatorsSignal(next);
  persist(next);
}

/** Append an indicator. No-op if an entry with the same name is already active. */
export function addIndicator(entry: ActiveIndicator): void {
  const current = activeIndicators();
  const name = indicatorName(entry);
  if (current.some((e) => indicatorName(e) === name)) return;
  setActiveIndicators([...current, entry]);
}

/** Replace the parameters of the entry at `index`. */
export function updateIndicatorParams(
  index: number,
  params: Record<string, unknown>,
): void {
  setActiveIndicators(withParams(activeIndicators(), index, params));
}

/**
 * Apply a settings dialog result to the entry at `index` in one write:
 * `params` (undefined keeps them) and the plot `style`. The style is only
 * written when it differs from the stored one, and nothing is persisted
 * when nothing changes.
 */
export function updateIndicatorSettings(
  index: number,
  params: Record<string, unknown> | undefined,
  style: IndicatorStyle | undefined,
): void {
  const entries = activeIndicators();
  const entry = entries[index];
  if (entry === undefined) return;
  let next = params === undefined ? entries : withParams(entries, index, params);
  const clean = sanitizeStyle(style);
  if (JSON.stringify(clean) !== JSON.stringify(indicatorStyle(entry))) {
    next = withStyle(next, index, clean);
  }
  if (next !== entries) setActiveIndicators(next);
}

/** Toggle the client-side visibility of the entry at `index`. */
export function toggleIndicatorHidden(index: number): void {
  const entries = activeIndicators();
  const entry = entries[index];
  if (entry === undefined) return;
  setActiveIndicators(withHidden(entries, index, !isHidden(entry)));
}

/** Remove every entry whose name matches. */
export function removeIndicator(name: string): void {
  setActiveIndicators(
    activeIndicators().filter((e) => indicatorName(e) !== name),
  );
}

/**
 * Stable-stringify the active set for use in a TanStack Query key. Object
 * entries have their keys sorted so `{ name: "rsi", length: 14 }` and
 * `{ length: 14, name: "rsi" }` hash the same. Array order is preserved —
 * rendering order is semantic, so a reorder should invalidate the cache.
 */
export function activeIndicatorsKey(entries: ActiveIndicator[]): string {
  return JSON.stringify(
    entries.map((e) => {
      if (typeof e === "string") return e;
      // fromEntries defines own properties, so a `__proto__` key stays data.
      return Object.fromEntries(Object.keys(e).sort().map((k) => [k, e[k]]));
    }),
  );
}
