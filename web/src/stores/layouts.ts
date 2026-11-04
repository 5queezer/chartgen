// Named chart layouts (ADR-0015). A layout bundles symbol, timeframe, chart
// type, scale mode and the indicator set. The live state stays in
// the existing stores (layout, indicators); this store keeps the
// saved snapshots under `chartgen.layouts` and applies them back through the
// other stores' setters, so their persisted formats are untouched.
//
// Drawings are not part of a layout: they stay global per symbol, as in
// TradingView with "sync drawings to all layouts".

import { batch, createSignal } from "solid-js";

import { isTimeframe, type Timeframe } from "../lib/timeframe";
import {
  activeIndicators,
  activeIndicatorsKey,
  DEFAULT_SET,
  indicatorName,
  isActiveIndicator,
  setActiveIndicators,
  type ActiveIndicator,
} from "./indicators";
import {
  CHART_TYPES,
  chartType,
  DEFAULT_CHART_TYPE,
  DEFAULT_SCALE_MODE,
  normalizeSymbol,
  SCALE_MODES,
  scaleMode,
  setChartType,
  setScaleMode,
  setSymbol,
  setTimeframe,
  symbol,
  timeframe,
  type ChartType,
  type ScaleMode,
} from "./layout";

const STORAGE_KEY = "chartgen.layouts";

export const MAX_LAYOUTS = 50;
export const MAX_NAME_LENGTH = 64;
export const DEFAULT_LAYOUT_NAME = "Unnamed";

export interface LayoutState {
  symbol: string;
  timeframe: Timeframe;
  chartType: ChartType;
  scaleMode: ScaleMode;
  indicators: ActiveIndicator[];
}

export interface Layout {
  id: string;
  name: string;
  updatedAt: number;
  state: LayoutState;
}

export interface LayoutsFile {
  version: 1;
  active: string;
  items: Layout[];
}

function isPlainObject(x: unknown): x is Record<string, unknown> {
  return typeof x === "object" && x !== null && !Array.isArray(x);
}

/** Trimmed name, or null when empty. Over-long names are cut. */
export function cleanName(x: unknown): string | null {
  if (typeof x !== "string") return null;
  const v = x.trim().slice(0, MAX_NAME_LENGTH).trim();
  return v.length > 0 ? v : null;
}

/** `name`, or `name (2)`, `name (3)`... when another layout already has it (case-insensitive). */
export function uniqueName(
  items: readonly Layout[],
  name: string,
  exceptId?: string,
): string {
  const taken = new Set(
    items.filter((l) => l.id !== exceptId).map((l) => l.name.toLowerCase()),
  );
  let candidate = name;
  for (let n = 2; taken.has(candidate.toLowerCase()); n++) {
    const suffix = ` (${n})`;
    candidate = name.slice(0, MAX_NAME_LENGTH - suffix.length).trimEnd() + suffix;
  }
  return candidate;
}

/** Validate a stored/imported state; null if a required field is malformed. */
export function parseState(x: unknown): LayoutState | null {
  if (!isPlainObject(x)) return null;
  const sym = normalizeSymbol(x.symbol);
  if (
    sym === null ||
    !isTimeframe(x.timeframe) ||
    !(CHART_TYPES as readonly unknown[]).includes(x.chartType) ||
    !(SCALE_MODES as readonly unknown[]).includes(x.scaleMode) ||
    !Array.isArray(x.indicators)
  ) {
    return null;
  }
  // Same rule as the indicator store: valid entries, unique by name.
  const seen = new Set<string>();
  const indicators = x.indicators.filter((e): e is ActiveIndicator => {
    if (!isActiveIndicator(e) || seen.has(indicatorName(e))) return false;
    seen.add(indicatorName(e));
    return true;
  });
  // Any leftover `drawingsBySymbol` key is ignored.
  return {
    symbol: sym,
    timeframe: x.timeframe,
    chartType: x.chartType as ChartType,
    scaleMode: x.scaleMode as ScaleMode,
    indicators,
  };
}

function parseLayout(x: unknown): Layout | null {
  if (!isPlainObject(x)) return null;
  const name = cleanName(x.name);
  const state = parseState(x.state);
  if (typeof x.id !== "string" || x.id.length === 0 || !name || !state) {
    return null;
  }
  const updatedAt =
    typeof x.updatedAt === "number" && Number.isFinite(x.updatedAt)
      ? x.updatedAt
      : 0;
  return { id: x.id, name, updatedAt, state };
}

/**
 * Validate the stored file, dropping malformed items, duplicate ids and
 * anything past the cap. A missing or unusable file (fresh install,
 * corruption) becomes one default layout built from `current`.
 */
export function parseLayouts(
  raw: unknown,
  current: LayoutState,
  id: string,
  now: number,
): LayoutsFile {
  if (isPlainObject(raw) && raw.version === 1 && Array.isArray(raw.items)) {
    const ids = new Set<string>();
    const items: Layout[] = [];
    for (const entry of raw.items) {
      const layout = parseLayout(entry);
      if (!layout || ids.has(layout.id) || items.length >= MAX_LAYOUTS) continue;
      ids.add(layout.id);
      items.push({ ...layout, name: uniqueName(items, layout.name) });
    }
    if (items.length > 0) {
      const active =
        typeof raw.active === "string" && ids.has(raw.active)
          ? raw.active
          : items[0]!.id;
      return { version: 1, active, items };
    }
  }
  return {
    version: 1,
    active: id,
    items: [{ id, name: DEFAULT_LAYOUT_NAME, updatedAt: now, state: current }],
  };
}

function comparable(s: LayoutState): string {
  return JSON.stringify([
    s.symbol,
    s.timeframe,
    s.chartType,
    s.scaleMode,
    activeIndicatorsKey(s.indicators),
  ]);
}

/** True when `live` differs from `saved` in anything a layout holds . */
export function isDirty(saved: LayoutState, live: LayoutState): boolean {
  return comparable(saved) !== comparable(live);
}

export function withSaved(
  file: LayoutsFile,
  id: string,
  live: LayoutState,
  now: number,
): LayoutsFile {
  return {
    ...file,
    items: file.items.map((l) =>
      l.id === id
        ? { ...l, updatedAt: now, state: live }
        : l,
    ),
  };
}

/** Add a layout and make it active; null when the cap is reached or the name is empty. */
export function withAdded(
  file: LayoutsFile,
  name: string,
  state: LayoutState,
  id: string,
  now: number,
): LayoutsFile | null {
  const clean = cleanName(name);
  if (clean === null || file.items.length >= MAX_LAYOUTS) return null;
  const layout: Layout = {
    id,
    name: uniqueName(file.items, clean),
    updatedAt: now,
    state,
  };
  return { ...file, active: id, items: [...file.items, layout] };
}

/** Rename; a clash with another layout gets a numeric suffix. Empty names are ignored. */
export function withRenamed(
  file: LayoutsFile,
  id: string,
  name: string,
): LayoutsFile {
  const clean = cleanName(name);
  if (clean === null) return file;
  return {
    ...file,
    items: file.items.map((l) =>
      l.id === id ? { ...l, name: uniqueName(file.items, clean, id) } : l,
    ),
  };
}

/** Remove a layout; the active one falls to the first remaining. Null if none would be left. */
export function withoutLayout(
  file: LayoutsFile,
  id: string,
): LayoutsFile | null {
  const items = file.items.filter((l) => l.id !== id);
  if (items.length === 0) return null;
  const active = file.active === id ? items[0]!.id : file.active;
  return { ...file, active, items };
}

/** Parse an exported layout file: `{ name, state }`. */
export function parseExport(
  text: string,
): { name: string; state: LayoutState } | null {
  try {
    const raw: unknown = JSON.parse(text);
    if (!isPlainObject(raw)) return null;
    const name = cleanName(raw.name);
    const state = parseState(raw.state);
    return name && state ? { name, state } : null;
  } catch {
    return null;
  }
}

export function serializeExport(layout: Layout): string {
  return JSON.stringify(
    { version: 1, name: layout.name, state: layout.state },
    null,
    2,
  );
}

// --- live bindings -------------------------------------------------------

function newId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

/** The live state as the other stores hold it right now. */
function liveState(): LayoutState {
  return {
    symbol: symbol(),
    timeframe: timeframe(),
    chartType: chartType(),
    scaleMode: scaleMode(),
    indicators: activeIndicators(),
  };
}

function load(): LayoutsFile {
  let raw: unknown;
  try {
    const text =
      typeof localStorage === "undefined"
        ? null
        : localStorage.getItem(STORAGE_KEY);
    raw = text === null ? undefined : JSON.parse(text);
  } catch {
    raw = undefined;
  }
  return parseLayouts(raw, liveState(), newId(), Date.now());
}

function persist(file: LayoutsFile): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(file));
  } catch {
    // Quota exceeded or private-mode storage: best-effort only.
  }
}

const [file, setFile] = createSignal<LayoutsFile>(load());
persist(file());

export const layouts = () => file().items;
export const activeLayout = (): Layout =>
  file().items.find((l) => l.id === file().active) ?? file().items[0]!;

/** True when the live state differs from the active layout's saved snapshot. */
export const layoutDirty = (): boolean => {
  return isDirty(activeLayout().state, liveState());
};

function update(next: LayoutsFile): void {
  setFile(next);
  persist(next);
}

/** Write a state through the existing stores' setters. */
function applyState(state: LayoutState): void {
  batch(() => {
    setSymbol(state.symbol);
    setTimeframe(state.timeframe);
    setChartType(state.chartType);
    setScaleMode(state.scaleMode);
    setActiveIndicators(state.indicators);
  });
}

/** Save the live state into the active layout. */
export function saveLayout(): void {
  update(withSaved(file(), activeLayout().id, liveState(), Date.now()));
}

/** Copy the live state into a new layout and switch to it. False when the cap is reached. */
export function saveLayoutAs(name: string): boolean {
  const next = withAdded(file(), name, liveState(), newId(), Date.now());
  if (next === null) return false;
  update(next);
  return true;
}

/** New layout on the current symbol and timeframe, everything else at its default; switches to it. */
export function createLayout(name: string): boolean {
  const state: LayoutState = {
    symbol: symbol(),
    timeframe: timeframe(),
    chartType: DEFAULT_CHART_TYPE,
    scaleMode: DEFAULT_SCALE_MODE,
    indicators: [...DEFAULT_SET],
  };
  const next = withAdded(file(), name, state, newId(), Date.now());
  if (next === null) return false;
  update(next);
  applyState(state);
  return true;
}

export function switchLayout(id: string): void {
  const target = file().items.find((l) => l.id === id);
  if (!target) return;
  update({ ...file(), active: id });
  applyState(target.state);
}

export function renameLayout(id: string, name: string): void {
  update(withRenamed(file(), id, name));
}

/** Delete a layout; deleting the last one resets to a fresh "Unnamed" layout of the live state. */
export function deleteLayout(id: string): void {
  const wasActive = file().active === id;
  const next = withoutLayout(file(), id);
  if (next === null) {
    update(parseLayouts(undefined, liveState(), newId(), Date.now()));
    return;
  }
  update(next);
  if (wasActive) applyState(activeLayout().state);
}

/** Import an exported layout as a new layout and switch to it. False on invalid input or the cap. */
export function importLayout(text: string): boolean {
  const parsed = parseExport(text);
  if (parsed === null) return false;
  const next = withAdded(file(), parsed.name, parsed.state, newId(), Date.now());
  if (next === null) return false;
  update(next);
  applyState(parsed.state);
  return true;
}

/** JSON of the saved layout `id` for download, or null if unknown. */
export function exportLayout(id: string): string | null {
  const layout = file().items.find((l) => l.id === id);
  return layout ? serializeExport(layout) : null;
}
