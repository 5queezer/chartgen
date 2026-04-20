// Active indicator set — Solid signal backed by localStorage.
//
// Wire-compatible with the chartgen `generate_chart` tool: each entry is
// either a bare indicator name (string) or an object `{ name, ...params }`.
// On first load the set is seeded with the v0.1 default so the chart looks
// the same before the user touches the picker.

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
    const valid = parsed.filter(isActiveIndicator);
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
