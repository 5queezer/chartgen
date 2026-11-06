// Named chart layouts (ADR-0015). A layout bundles symbol, timeframe, chart
// type, scale mode, time zone and the indicator set. The live state stays in
// the existing stores (layout, indicators); this store keeps the
// saved snapshots under `chartgen.layouts` and applies them back through the
// other stores' setters, so their persisted formats are untouched.
//
// Drawings are not part of a layout: they stay global per symbol, as in
// TradingView with "sync drawings to all layouts".

import { batch, createSignal } from "solid-js";

import { isTimeframe, type Timeframe } from "../lib/timeframe";
import {
  isTimeZoneChoice,
  type TimeZoneChoice,
} from "../lib/timezone";
import {
  compareRestore,
  compareSymbols,
  parseCompare,
  setCompareSymbols,
} from "./compare";
import {
  activeIndicators,
  DEFAULT_SET,
  indicatorName,
  indicatorParams,
  indicatorStyle,
  instanceKey,
  isHidden,
  isActiveIndicator,
  normalizeInstances,
  sanitizeStyle,
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
  invertScale,
  lockRatio,
  setChartType,
  setInvertScale,
  setLockRatio,
  setScaleMode,
  setSymbol,
  setTimeframe,
  setTimeZone,
  symbol,
  timeframe,
  timeZone,
  type ChartType,
  type ScaleMode,
} from "./layout";

const STORAGE_KEY = "chartgen.layouts";

export const MAX_LAYOUTS = 50;
export const MAX_NAME_LENGTH = 64;
export const MAX_INDICATORS = 50;
/** Largest layout file `parseExport` looks at. */
export const MAX_IMPORT_BYTES = 256 * 1024;
const MAX_PARAM_STRING = 200;
const BACKUP_KEY = STORAGE_KEY + ".bak";
const FORBIDDEN_KEYS = new Set(["__proto__", "constructor", "prototype"]);
export const DEFAULT_LAYOUT_NAME = "Unnamed";

export interface LayoutState {
  symbol: string;
  timeframe: Timeframe;
  chartType: ChartType;
  scaleMode: ScaleMode;
  /** Only present when on; layouts saved before they existed are off. */
  invertScale?: boolean;
  lockRatio?: boolean;
  timeZone: TimeZoneChoice;
  indicators: ActiveIndicator[];
  /** Compare symbols (ADR-0022); layouts saved before it have none. */
  compare: string[];
  /**
   * Scale mode to return to when the last compare symbol goes (ADR-0040). Not
   * something the user set, so it never makes a layout dirty.
   */
  compareRestore?: ScaleMode;
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

/**
 * Keep an indicator entry's name and its primitive params (number, string,
 * boolean), the instance id (ADR-0028), the hidden flag and the validated
 * plot style (ADR-0017); prototype-polluting keys are dropped.
 */
function sanitizeEntry(e: ActiveIndicator): ActiveIndicator {
  if (typeof e === "string") return e;
  const params = Object.entries(e).filter(
    ([k, v]) =>
      k !== "name" &&
      !FORBIDDEN_KEYS.has(k) &&
      (typeof v === "boolean" ||
        (typeof v === "number" && Number.isFinite(v)) ||
        (typeof v === "string" && v.length <= MAX_PARAM_STRING)),
  );
  const style = sanitizeStyle(e.style);
  return Object.fromEntries([
    ["name", e.name],
    ...params,
    ...(style ? [["style", style]] : []),
  ]) as ActiveIndicator;
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
  // Same rule as the indicator store: valid entries, each with a unique
  // instance key (ADR-0028; older layouts have none and load unchanged).
  const indicators = normalizeInstances(
    x.indicators
      .filter((e): e is ActiveIndicator => isActiveIndicator(e))
      .slice(0, MAX_INDICATORS)
      .map(sanitizeEntry),
  );
  // Any leftover `drawingsBySymbol` key is ignored.
  return {
    symbol: sym,
    timeframe: x.timeframe,
    chartType: x.chartType as ChartType,
    scaleMode: x.scaleMode as ScaleMode,
    ...(x.invertScale === true ? { invertScale: true } : {}),
    ...(x.lockRatio === true ? { lockRatio: true } : {}),
    // Layouts saved before time zones existed keep the current zone.
    timeZone: isTimeZoneChoice(x.timeZone) ? x.timeZone : timeZone(),
    indicators,
    compare: parseCompare(x.compare),
    ...((SCALE_MODES as readonly unknown[]).includes(x.compareRestore)
      ? { compareRestore: x.compareRestore as ScaleMode }
      : {}),
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
 * anything past the cap. Null when nothing usable is left.
 */
export function parseStoredFile(raw: unknown): LayoutsFile | null {
  if (!isPlainObject(raw) || raw.version !== 1 || !Array.isArray(raw.items)) {
    return null;
  }
  const ids = new Set<string>();
  const items: Layout[] = [];
  for (const entry of raw.items) {
    const layout = parseLayout(entry);
    if (!layout || ids.has(layout.id) || items.length >= MAX_LAYOUTS) continue;
    ids.add(layout.id);
    items.push({ ...layout, name: uniqueName(items, layout.name) });
  }
  if (items.length === 0) return null;
  const active =
    typeof raw.active === "string" && ids.has(raw.active)
      ? raw.active
      : items[0]!.id;
  return { version: 1, active, items };
}

/** The stored file, or one default layout built from `current` (fresh install, corruption). */
export function parseLayouts(
  raw: unknown,
  current: LayoutState,
  id: string,
  now: number,
): LayoutsFile {
  return (
    parseStoredFile(raw) ?? {
      version: 1,
      active: id,
      items: [
        { id, name: DEFAULT_LAYOUT_NAME, updatedAt: now, state: current },
      ],
    }
  );
}

/** Param defaults per indicator name (and alias), as the registry reports them. */
export type ParamDefaults = Record<string, Record<string, unknown>>;

/** Object keys sorted at every level, so equal styles compare equal. */
function sortedKeys(x: unknown): unknown {
  if (Array.isArray(x)) return x.map(sortedKeys);
  if (x && typeof x === "object") {
    return Object.fromEntries(
      Object.entries(x)
        .sort(([a], [b]) => (a < b ? -1 : 1))
        .map(([k, v]) => [k, sortedKeys(v)]),
    );
  }
  return x;
}

/** A plot style (plots and geometry) in sorted order, or undefined when empty. */
function canonicalStyle(style: ReturnType<typeof indicatorStyle>): unknown {
  const { plots = {}, geometry } = style ?? {};
  if (Object.keys(plots).length === 0 && !geometry) return undefined;
  return sortedKeys({
    ...plots,
    ...(geometry ? { geometry } : {}),
  });
}

/**
 * Entries in a form that ignores how a value was entered: params equal to
 * the schema default are dropped (the settings dialog does the same), keys
 * are sorted. The hidden flag and the plot style are kept, so hiding an
 * indicator or restyling a plot is a change, as in TradingView.
 */
export function normalizeIndicators(
  entries: readonly ActiveIndicator[],
  defaults: ParamDefaults,
): unknown[] {
  return entries.map((e) => {
    const name = indicatorName(e);
    const params = Object.entries(indicatorParams(e))
      .filter(([k, v]) => defaults[name]?.[k] !== v)
      .sort(([a], [b]) => (a < b ? -1 : 1));
    const key = instanceKey(e);
    const id = key === name ? [] : [["id", key]];
    const hidden = isHidden(e) ? [["hidden", true]] : [];
    const style = canonicalStyle(indicatorStyle(e));
    const extra = [...id, ...hidden, ...(style ? [["style", style]] : [])];
    return params.length + extra.length === 0
      ? name
      : Object.fromEntries([["name", name], ...params, ...extra]);
  });
}

function comparable(s: LayoutState, defaults: ParamDefaults): string {
  return JSON.stringify([
    s.symbol,
    s.timeframe,
    s.chartType,
    s.scaleMode,
    !!s.invertScale,
    !!s.lockRatio,
    s.timeZone,
    normalizeIndicators(s.indicators, defaults),
    s.compare,
  ]);
}

/** True when `live` differs from `saved` in anything a layout holds . */
export function isDirty(
  saved: LayoutState,
  live: LayoutState,
  defaults: ParamDefaults = {},
): boolean {
  return comparable(saved, defaults) !== comparable(live, defaults);
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

/** Remove a layout. The active layout cannot be removed: the file is returned unchanged. */
export function withoutLayout(file: LayoutsFile, id: string): LayoutsFile {
  if (id === file.active) return file;
  return { ...file, items: file.items.filter((l) => l.id !== id) };
}

/** Parse an exported layout file: `{ name, state }`. */
export function parseExport(
  text: string,
): { name: string; state: LayoutState } | null {
  if (text.length > MAX_IMPORT_BYTES) return null;
  try {
    const raw: unknown = JSON.parse(text);
    if (!isPlainObject(raw) || raw.version !== 1) return null;
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
  const restoreTo = compareRestore();
  return {
    symbol: symbol(),
    timeframe: timeframe(),
    chartType: chartType(),
    scaleMode: scaleMode(),
    ...(invertScale() ? { invertScale: true } : {}),
    ...(lockRatio() ? { lockRatio: true } : {}),
    timeZone: timeZone(),
    indicators: activeIndicators(),
    compare: compareSymbols(),
    ...(restoreTo ? { compareRestore: restoreTo } : {}),
  };
}

type Stored =
  | { kind: "absent" }
  | { kind: "ok"; file: LayoutsFile }
  | { kind: "unusable"; text: string };

function readStored(): Stored {
  if (typeof localStorage === "undefined") return { kind: "absent" };
  let text: string | null;
  try {
    text = localStorage.getItem(STORAGE_KEY);
  } catch {
    return { kind: "absent" };
  }
  if (text === null) return { kind: "absent" };
  try {
    const parsed = parseStoredFile(JSON.parse(text));
    if (parsed) return { kind: "ok", file: parsed };
  } catch {
    // Falls through to unusable.
  }
  return { kind: "unusable", text };
}

/** False when the browser refused the write (quota exceeded, private-mode storage). */
function persist(file: LayoutsFile): boolean {
  if (typeof localStorage === "undefined") return true;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(file));
    return true;
  } catch {
    return false;
  }
}

/** Stored value that cannot be used is copied aside once and never overwritten by a fallback. */
function keepBackup(text: string): void {
  try {
    if (localStorage.getItem(BACKUP_KEY) === null) {
      localStorage.setItem(BACKUP_KEY, text);
    }
  } catch {
    // Best-effort only.
  }
}

function load(): LayoutsFile {
  const stored = readStored();
  if (stored.kind === "ok") return stored.file;
  const fallback = parseLayouts(undefined, liveState(), newId(), Date.now());
  if (stored.kind === "unusable") keepBackup(stored.text);
  else persist(fallback);
  return fallback;
}

const [file, setFile] = createSignal<LayoutsFile>(load());

/** `fresh` from storage, but this tab keeps its own active layout while it still exists. */
function mergeActive(fresh: LayoutsFile): LayoutsFile {
  const mine = file().active;
  return fresh.items.some((l) => l.id === mine)
    ? { ...fresh, active: mine }
    : fresh;
}

// Another tab wrote the layouts: pick up its list.
if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key !== STORAGE_KEY && e.key !== null) return;
    const stored = readStored();
    if (stored.kind === "ok") setFile(mergeActive(stored.file));
  });
}

export const layouts = () => file().items;
export const activeLayout = (): Layout =>
  file().items.find((l) => l.id === file().active) ?? file().items[0]!;

// Registry param defaults; also fill the legends of repeated indicators (ADR-0028).
const [paramDefaults, setParamDefaults] = createSignal<ParamDefaults>({});
export { paramDefaults, setParamDefaults };

/** True when the live state differs from the active layout's saved snapshot. */
export const layoutDirty = (): boolean =>
  isDirty(activeLayout().state, liveState(), paramDefaults());

/**
 * Apply `reducer` to the layouts as they are in storage right now (another
 * tab may have changed them since this tab loaded) and persist the result.
 * Returns false when the reducer refuses (null). If the stored value is
 * unusable, this tab's in-memory file is the base; the backup is kept. Storage
 * is best-effort unless `mustStore` is set: then a refused write leaves the
 * in-memory file as it was and returns false too.
 */
function update(
  reducer: (f: LayoutsFile) => LayoutsFile | null,
  mustStore = false,
): boolean {
  const stored = readStored();
  const base = stored.kind === "ok" ? mergeActive(stored.file) : file();
  const next = reducer(base);
  if (next === null) return false;
  if (!persist(next) && mustStore) return false;
  setFile(next);
  return true;
}

/** Write a state through the existing stores' setters. */
function applyState(state: LayoutState): void {
  batch(() => {
    setSymbol(state.symbol);
    setTimeframe(state.timeframe);
    setChartType(state.chartType);
    setScaleMode(state.scaleMode);
    setInvertScale(!!state.invertScale);
    setLockRatio(!!state.lockRatio);
    setTimeZone(state.timeZone);
    setActiveIndicators(state.indicators);
    setCompareSymbols(state.compare, state.compareRestore);
  });
}

/** Save the live state into the active layout. False when the browser refused the write. */
export function saveLayout(): boolean {
  return update((f) => withSaved(f, f.active, liveState(), Date.now()), true);
}

/** Copy the live state into a new layout and switch to it. False when the cap is reached. */
export function saveLayoutAs(name: string): boolean {
  return update((f) => withAdded(f, name, liveState(), newId(), Date.now()));
}

/** New layout on the current symbol and timeframe, everything else at its default; switches to it. */
export function createLayout(name: string): boolean {
  const state: LayoutState = {
    symbol: symbol(),
    timeframe: timeframe(),
    chartType: DEFAULT_CHART_TYPE,
    scaleMode: DEFAULT_SCALE_MODE,
    timeZone: timeZone(),
    indicators: [...DEFAULT_SET],
    compare: [],
  };
  return batch(() => {
    const ok = update((f) => withAdded(f, name, state, newId(), Date.now()));
    if (ok) applyState(state);
    return ok;
  });
}

export function switchLayout(id: string): void {
  batch(() => {
    let target: Layout | undefined;
    update((f) => {
      target = f.items.find((l) => l.id === id);
      return target ? { ...f, active: id } : null;
    });
    if (target) applyState(target.state);
  });
}

export function renameLayout(id: string, name: string): void {
  update((f) => withRenamed(f, id, name));
}

/** Delete a layout other than the active one. */
export function deleteLayout(id: string): void {
  update((f) => withoutLayout(f, id));
}

/** Import an exported layout as a new layout and switch to it. False on invalid input or the cap. */
export function importLayout(text: string): boolean {
  const parsed = parseExport(text);
  if (parsed === null) return false;
  return batch(() => {
    const ok = update((f) =>
      withAdded(f, parsed.name, parsed.state, newId(), Date.now()),
    );
    if (ok) applyState(parsed.state);
    return ok;
  });
}

/** JSON of the saved layout `id` for download, or null if unknown. */
export function exportLayout(id: string): string | null {
  const layout = file().items.find((l) => l.id === id);
  return layout ? serializeExport(layout) : null;
}
