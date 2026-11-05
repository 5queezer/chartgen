// Watchlist state — the symbols in the right-side panel and whether the
// panel is collapsed. Solid signals backed by localStorage under the
// `chartgen.watchlist.` prefix. The list is validated on load: entries are
// normalized like the toolbar symbol (trim + uppercase), deduplicated and
// capped at MAX_WATCHLIST, the server's per-call `get_quotes` limit.

import { createSignal } from "solid-js";

import { normalizeSymbol } from "./layout";

export const MAX_WATCHLIST = 50;

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

const [watchlist, setWatchlistSignal] = createSignal<string[]>(loadSymbols());
const [watchlistCollapsed, setCollapsedSignal] = createSignal(
  read("collapsed") === true,
);

export { watchlist, watchlistCollapsed };

/** Append a symbol; ignored when invalid, already listed, or at the cap. */
export function addToWatchlist(next: string): void {
  const value = normalizeSymbol(next);
  const list = watchlist();
  if (value === null || list.includes(value) || list.length >= MAX_WATCHLIST) {
    return;
  }
  const updated = [...list, value];
  setWatchlistSignal(updated);
  persist("symbols", updated);
}

export function removeFromWatchlist(value: string): void {
  const updated = watchlist().filter((s) => s !== value);
  setWatchlistSignal(updated);
  persist("symbols", updated);
}

export function setWatchlistCollapsed(next: boolean): void {
  setCollapsedSignal(next);
  persist("collapsed", next);
}
