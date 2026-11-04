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

import { createSignal } from "solid-js";

/**
 * Wire shape for a single indicator request. Matches the `indicators` field
 * on `generate_chart` input: bare name or object with `name` plus arbitrary
 * numeric parameters.
 */
export type ActiveIndicator =
  | string
  | { name: string; [k: string]: unknown };

const STORAGE_KEY = "chartgen.indicators.active";

const DEFAULT_SET: ActiveIndicator[] = [
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
    return valid.length > 0 ? valid : [...DEFAULT_SET];
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

function isActiveIndicator(x: unknown): x is ActiveIndicator {
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

/** Parameters of an entry, without `name` and the client-only `hidden`. */
export function indicatorParams(entry: ActiveIndicator): Record<string, unknown> {
  if (typeof entry === "string") return {};
  const { name: _name, hidden: _hidden, ...params } = entry;
  return params;
}

/** Build an entry from a name and params; a param-less entry stays a bare name. */
function makeEntry(
  name: string,
  params: Record<string, unknown>,
  hidden: boolean,
): ActiveIndicator {
  const hasParams = Object.keys(params).length > 0;
  if (!hasParams && !hidden) return name;
  return hidden ? { name, ...params, hidden: true } : { name, ...params };
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
    i === index ? makeEntry(indicatorName(e), params, isHidden(e)) : e,
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
    i === index ? makeEntry(indicatorName(e), indicatorParams(e), hidden) : e,
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
      const sorted: Record<string, unknown> = {};
      for (const k of Object.keys(e).sort()) sorted[k] = e[k];
      return sorted;
    }),
  );
}
