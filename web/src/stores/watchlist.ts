// Watchlist state (ADR-0031) — named lists with sections, per-symbol color
// flags and whether the panel is collapsed. Solid signals backed by
// localStorage under the `chartgen.watchlist.` prefix: everything but the
// collapsed flag is one versioned JSON document under `v2`. Each list is
// validated on load: symbols are normalized like the toolbar symbol (trim +
// uppercase), deduplicated and capped at MAX_WATCHLIST, the server's
// per-call `get_quotes` limit. The pre-v2 single list (`symbols`) is
// migrated into a list named "Watchlist" and left untouched in storage.

import { createSignal } from "solid-js";

import { normalizeSymbol } from "./layout";

export const MAX_WATCHLIST = 50;
export const MAX_LISTS = 20;
export const MAX_SECTIONS = 20;
export const MAX_NAME_LENGTH = 40;
export const DEFAULT_LIST_NAME = "Watchlist";
/** Id of the virtual, read-only list of every flagged symbol. */
export const FLAGGED_ID = "flagged";

export const FLAG_COLORS = [
  "red",
  "orange",
  "yellow",
  "green",
  "cyan",
  "blue",
  "purple",
] as const;
export type FlagColor = (typeof FLAG_COLORS)[number];

export type WatchItem =
  | { kind: "symbol"; symbol: string }
  | { kind: "section"; id: string; name: string; collapsed: boolean };

export interface WatchList {
  id: string;
  name: string;
  items: WatchItem[];
}

interface State {
  lists: WatchList[];
  /** A list id, or FLAGGED_ID. */
  active: string;
  flags: Record<string, FlagColor>;
}

/** First-run list: a few crypto and stock tickers from `lib/symbols.ts`. */
export const DEFAULT_WATCHLIST: readonly string[] = [
  "BTCUSDT",
  "ETHUSDT",
  "SOLUSDT",
  "AAPL",
  "MSFT",
  "NVDA",
  "SPY",
];

const PREFIX = "chartgen.watchlist.";

function read(key: string): unknown {
  if (typeof localStorage === "undefined") return undefined;
  try {
    const raw = localStorage.getItem(PREFIX + key);
    return raw === null ? undefined : JSON.parse(raw);
  } catch {
    return undefined;
  }
}

function persist(key: string, value: unknown): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(PREFIX + key, JSON.stringify(value));
  } catch {
    // Quota exceeded or private-mode storage: best-effort only.
  }
}

/**
 * Normalize, drop invalid entries, dedupe (first wins) and cap. `null`
 * when `raw` is not an array.
 */
export function sanitizeWatchlist(raw: unknown): string[] | null {
  if (!Array.isArray(raw)) return null;
  const normalized = raw
    .map(normalizeSymbol)
    .filter((s): s is string => s !== null);
  return [...new Set(normalized)].slice(0, MAX_WATCHLIST);
}

/** The pre-v2 single list. */
function loadSymbols(): string[] {
  const raw = read("symbols");
  const list = sanitizeWatchlist(raw);
  if (list === null) return [...DEFAULT_WATCHLIST];
  // An explicitly empty list is the user's choice; a non-empty list with
  // nothing valid left is corrupt and falls back to the defaults.
  if (list.length === 0 && (raw as unknown[]).length > 0) {
    return [...DEFAULT_WATCHLIST];
  }
  return list;
}

let idCounter = 0;
const newId = (prefix: string) =>
  `${prefix}${Date.now().toString(36)}${(idCounter++).toString(36)}${Math.random().toString(36).slice(2, 5)}`;

const symbolItems = (symbols: readonly string[]): WatchItem[] =>
  symbols.map((symbol) => ({ kind: "symbol", symbol }));

const cleanName = (x: unknown): string | null => {
  if (typeof x !== "string") return null;
  const v = x.trim().slice(0, MAX_NAME_LENGTH);
  return v.length > 0 ? v : null;
};

function sanitizeItems(raw: unknown): WatchItem[] {
  if (!Array.isArray(raw)) return [];
  const items: WatchItem[] = [];
  const symbols = new Set<string>();
  const sections = new Set<string>();
  for (const x of raw as Record<string, unknown>[]) {
    if (x === null || typeof x !== "object") continue;
    if (x.kind === "symbol") {
      const symbol = normalizeSymbol(x.symbol);
      if (symbol === null || symbols.has(symbol)) continue;
      if (symbols.size >= MAX_WATCHLIST) continue;
      symbols.add(symbol);
      items.push({ kind: "symbol", symbol });
    } else if (x.kind === "section") {
      const name = cleanName(x.name);
      const id = typeof x.id === "string" ? x.id : "";
      if (name === null || id === "" || sections.has(id)) continue;
      if (sections.size >= MAX_SECTIONS) continue;
      sections.add(id);
      items.push({ kind: "section", id, name, collapsed: x.collapsed === true });
    }
  }
  return items;
}

/** Drop flags of symbols that no list holds any more. */
function prune(s: State): State {
  const held = new Set(
    s.lists.flatMap((l) =>
      l.items.flatMap((i) => (i.kind === "symbol" ? [i.symbol] : [])),
    ),
  );
  const flags = Object.fromEntries(
    Object.entries(s.flags).filter(([sym]) => held.has(sym)),
  ) as Record<string, FlagColor>;
  return { ...s, flags };
}

function sanitizeState(raw: unknown): State | null {
  if (raw === null || typeof raw !== "object") return null;
  const r = raw as Record<string, unknown>;
  if (r.version !== 2 || !Array.isArray(r.lists)) return null;
  const lists: WatchList[] = [];
  const ids = new Set<string>();
  for (const l of r.lists as Record<string, unknown>[]) {
    if (l === null || typeof l !== "object") continue;
    const name = cleanName(l.name);
    const id = typeof l.id === "string" ? l.id : "";
    if (name === null || id === "" || id === FLAGGED_ID || ids.has(id)) continue;
    if (lists.length >= MAX_LISTS) break;
    ids.add(id);
    lists.push({ id, name, items: sanitizeItems(l.items) });
  }
  if (lists.length === 0) return null;
  const flags: Record<string, FlagColor> = {};
  if (r.flags !== null && typeof r.flags === "object") {
    for (const [k, v] of Object.entries(r.flags)) {
      const sym = normalizeSymbol(k);
      if (sym !== null && (FLAG_COLORS as readonly unknown[]).includes(v)) {
        flags[sym] = v as FlagColor;
      }
    }
  }
  const active =
    r.active === FLAGGED_ID || ids.has(r.active as string)
      ? (r.active as string)
      : lists[0]!.id;
  return prune({ lists, active, flags });
}

function loadState(): State {
  const v2 = sanitizeState(read("v2"));
  if (v2 !== null) return v2;
  const id = newId("l");
  return {
    lists: [{ id, name: DEFAULT_LIST_NAME, items: symbolItems(loadSymbols()) }],
    active: id,
    flags: {},
  };
}

const [state, setStateSignal] = createSignal<State>(loadState());
const [watchlistCollapsed, setCollapsedSignal] = createSignal(
  read("collapsed") === true,
);

export { watchlistCollapsed };

function commit(next: State): void {
  const pruned = prune(next);
  setStateSignal(pruned);
  persist("v2", { version: 2, ...pruned });
}

export const watchlists = (): readonly WatchList[] => state().lists;
export const flags = (): Readonly<Record<string, FlagColor>> => state().flags;
export const activeListId = (): string => state().active;
export const isFlaggedView = (): boolean => state().active === FLAGGED_ID;

/** The list being shown (the first one while the Flagged view is active). */
export function activeList(): WatchList {
  const s = state();
  return s.lists.find((l) => l.id === s.active) ?? s.lists[0]!;
}

/** Rows to show: the active list, or every flagged symbol across lists. */
export function activeItems(): readonly WatchItem[] {
  const s = state();
  if (s.active !== FLAGGED_ID) return activeList().items;
  const seen = new Set<string>();
  for (const l of s.lists) {
    for (const i of l.items) {
      if (i.kind === "symbol" && s.flags[i.symbol]) seen.add(i.symbol);
    }
  }
  return symbolItems([...seen].slice(0, MAX_WATCHLIST));
}

/** Symbols of the active view, the set that is polled for quotes. */
export const watchlist = (): string[] =>
  activeItems().flatMap((i) => (i.kind === "symbol" ? [i.symbol] : []));

/** Replace the active list's items; `fn` returns null for "no change". */
function editActive(fn: (items: WatchItem[]) => WatchItem[] | null): void {
  const s = state();
  if (s.active === FLAGGED_ID) return;
  const list = activeList();
  const items = fn(list.items);
  if (items === null) return;
  commit({
    ...s,
    lists: s.lists.map((l) => (l.id === list.id ? { ...l, items } : l)),
  });
}

export function setActiveList(id: string): void {
  const s = state();
  if (id === s.active) return;
  if (id !== FLAGGED_ID && !s.lists.some((l) => l.id === id)) return;
  commit({ ...s, active: id });
}

/** Create an empty list and switch to it; null when the name is blank or at the cap. */
export function createList(name: string): string | null {
  const s = state();
  const n = cleanName(name);
  if (n === null || s.lists.length >= MAX_LISTS) return null;
  const id = newId("l");
  commit({ ...s, lists: [...s.lists, { id, name: n, items: [] }], active: id });
  return id;
}

export function renameList(id: string, name: string): void {
  const n = cleanName(name);
  if (n === null) return;
  const s = state();
  commit({
    ...s,
    lists: s.lists.map((l) => (l.id === id ? { ...l, name: n } : l)),
  });
}

/** Copy a list under "<name> copy" and switch to it; null at the cap. */
export function duplicateList(id: string): string | null {
  const s = state();
  const src = s.lists.find((l) => l.id === id);
  if (!src || s.lists.length >= MAX_LISTS) return null;
  const copy: WatchList = {
    id: newId("l"),
    name: `${src.name} copy`.slice(0, MAX_NAME_LENGTH),
    items: src.items.map((i) => ({ ...i })),
  };
  commit({ ...s, lists: [...s.lists, copy], active: copy.id });
  return copy.id;
}

/** Delete a list; the last remaining list cannot be deleted. */
export function deleteList(id: string): boolean {
  const s = state();
  if (s.lists.length <= 1 || !s.lists.some((l) => l.id === id)) return false;
  const lists = s.lists.filter((l) => l.id !== id);
  commit({ ...s, lists, active: s.active === id ? lists[0]!.id : s.active });
  return true;
}

/** Append a symbol; ignored when invalid, already listed, or at the cap. */
export function addToWatchlist(next: string): void {
  const value = normalizeSymbol(next);
  if (value === null) return;
  editActive((items) => {
    const count = items.filter((i) => i.kind === "symbol").length;
    const dup = items.some((i) => i.kind === "symbol" && i.symbol === value);
    if (dup || count >= MAX_WATCHLIST) return null;
    return [...items, { kind: "symbol", symbol: value }];
  });
}

export function removeFromWatchlist(value: string): void {
  editActive((items) =>
    items.filter((i) => !(i.kind === "symbol" && i.symbol === value)),
  );
}

/** Append a named section divider; null when blank, at the cap or in the Flagged view. */
export function addSection(name: string): string | null {
  const n = cleanName(name);
  if (n === null) return null;
  const id = newId("s");
  let added = false;
  editActive((items) => {
    if (items.filter((i) => i.kind === "section").length >= MAX_SECTIONS) {
      return null;
    }
    added = true;
    return [...items, { kind: "section", id, name: n, collapsed: false }];
  });
  return added ? id : null;
}

type Section = Extract<WatchItem, { kind: "section" }>;

function editSection(id: string, fn: (s: Section) => Section | null): void {
  editActive((items) =>
    items.flatMap((i) => {
      if (i.kind !== "section" || i.id !== id) return [i];
      const next = fn(i);
      return next ? [next] : [];
    }),
  );
}

export function renameSection(id: string, name: string): void {
  const n = cleanName(name);
  if (n !== null) editSection(id, (s) => ({ ...s, name: n }));
}

export function toggleSection(id: string): void {
  editSection(id, (s) => ({ ...s, collapsed: !s.collapsed }));
}

/** Remove the divider only; its symbols fall into the section above. */
export function removeSection(id: string): void {
  editSection(id, () => null);
}

/** [start, end) of the block (a section header and the symbols under it) containing `index`. */
function blockOf(items: readonly WatchItem[], index: number): [number, number] {
  let start = index;
  while (start > 0 && items[start]!.kind !== "section") start--;
  let end = index + 1;
  while (end < items.length && items[end]!.kind !== "section") end++;
  return [start, end];
}

/**
 * Move the item at `from` so it sits before the item at `before` (an index
 * into the current order; `items.length` means the end). A section moves
 * together with its symbols and snaps to a block boundary.
 */
export function moveItem(from: number, before: number): void {
  editActive((items) => {
    if (from < 0 || from >= items.length) return null;
    const target = Math.max(0, Math.min(before, items.length));
    if (items[from]!.kind === "symbol") {
      if (target === from || target === from + 1) return null;
      const next = items.filter((_, i) => i !== from);
      next.splice(target > from ? target - 1 : target, 0, items[from]!);
      return next;
    }
    const [start, end] = blockOf(items, from);
    const t = target < items.length ? blockOf(items, target)[0] : target;
    // A drop inside the block (or on its own header) changes nothing.
    if (t >= start && t <= end) return null;
    const block = items.slice(start, end);
    const rest = [...items.slice(0, start), ...items.slice(end)];
    rest.splice(t > start ? t - block.length : t, 0, ...block);
    return rest;
  });
}

/**
 * Keyboard move: one visible row up or down. Symbols skip rows hidden in a
 * collapsed section; a section jumps over whole neighbouring blocks.
 */
export function moveItemBy(index: number, delta: -1 | 1): void {
  if (isFlaggedView()) return;
  const items = activeList().items;
  if (index < 0 || index >= items.length) return;
  if (items[index]!.kind === "section") {
    const [start, end] = blockOf(items, index);
    if (delta < 0) {
      if (start > 0) moveItem(index, blockOf(items, start - 1)[0]);
    } else if (end < items.length) {
      moveItem(index, blockOf(items, end)[1]);
    }
    return;
  }
  const visible: number[] = [];
  let hidden = false;
  items.forEach((i, n) => {
    if (i.kind === "section") {
      hidden = i.collapsed;
      visible.push(n);
    } else if (!hidden) visible.push(n);
  });
  const neighbor = visible[visible.indexOf(index) + delta];
  if (neighbor === undefined) return;
  if (delta < 0) {
    moveItem(index, neighbor);
    return;
  }
  const n = items[neighbor]!;
  if (n.kind === "section" && n.collapsed) {
    // Step over the collapsed section and its hidden symbols, to the top of
    // the next one; stay put when it is the last (the symbol would vanish).
    const end = blockOf(items, neighbor)[1];
    if (end < items.length) moveItem(index, end + 1);
    return;
  }
  moveItem(index, neighbor + 1);
}

/** Set (or with null clear) a symbol's flag; flags are shared by all lists. */
export function setFlag(symbol: string, color: FlagColor | null): void {
  const s = state();
  const next = { ...s.flags };
  if (color === null) delete next[symbol];
  else next[symbol] = color;
  commit({ ...s, flags: next });
}

export function setWatchlistCollapsed(next: boolean): void {
  setCollapsedSignal(next);
  persist("collapsed", next);
}
