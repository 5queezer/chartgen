// Row model of the symbol search dialog: category tabs and the merge of
// recents, the curated catalogue and server results (`search_symbols`).

import type { SearchResult } from "../types/responses";
import {
  SYMBOLS,
  searchSymbols,
  type SymbolEntry,
  type SymbolKind,
} from "./symbols";

type ResultType = SearchResult["type"];

export interface Category {
  id: string;
  label: string;
  /** `search_symbols` types the tab selects; empty means all. */
  types: readonly ResultType[];
}

export const CATEGORIES: readonly Category[] = [
  { id: "all", label: "All", types: [] },
  { id: "crypto", label: "Crypto", types: ["crypto"] },
  { id: "stocks", label: "Stocks", types: ["stock"] },
  { id: "funds", label: "Funds", types: ["etf", "fund"] },
  { id: "indices", label: "Indices", types: ["index"] },
  { id: "forex", label: "Forex", types: ["forex"] },
  { id: "futures", label: "Futures", types: ["future"] },
];

const KIND_TYPE: Record<SymbolKind, ResultType> = {
  crypto: "crypto",
  stock: "stock",
  etf: "etf",
  index: "index",
  fx: "forex",
  futures: "future",
};

export interface Row {
  section: string;
  ticker: string;
  name: string;
  exchange: string;
  /** Instrument type badge; empty for rows without a known type. */
  badge: string;
}

const inCategory = (c: Category, t: ResultType): boolean =>
  c.types.length === 0 || c.types.includes(t);

const curatedRow = (section: string, s: SymbolEntry): Row => ({
  section,
  ticker: s.ticker,
  name: s.name,
  exchange: s.exchange,
  badge: KIND_TYPE[s.kind],
});

const resultRow = (r: SearchResult): Row => ({
  section: "Symbols",
  ticker: r.symbol,
  name: r.name,
  exchange: r.exchange,
  badge: r.type,
});

export interface RowsInput {
  query: string;
  category: Category;
  /** Recent tickers, newest first. */
  recents: readonly string[];
  /**
   * Server results for exactly this query and category, or null while they
   * are loading or failed. Then the curated catalogue answers instantly.
   */
  remote: readonly SearchResult[] | null;
}

/**
 * Rows in display order. Empty query: recents, then the curated list.
 * Otherwise the server results (or the curated matches as a stand-in),
 * followed by an "as typed" row unless the typed ticker is already listed.
 */
export function buildRows(input: RowsInput): Row[] {
  const { category } = input;
  const q = input.query.trim();
  if (q === "") {
    const recent = input.recents.flatMap((t): Row[] => {
      const known = SYMBOLS.find((s) => s.ticker === t);
      if (!known) {
        return category.types.length === 0
          ? [{ section: "Recent", ticker: t, name: "", exchange: "", badge: "" }]
          : [];
      }
      return inCategory(category, KIND_TYPE[known.kind])
        ? [curatedRow("Recent", known)]
        : [];
    });
    const popular = SYMBOLS.filter((s) =>
      inCategory(category, KIND_TYPE[s.kind]),
    ).map((s) => curatedRow("Popular", s));
    return [...recent, ...popular];
  }
  const matches = input.remote
    ? input.remote.map(resultRow)
    : searchSymbols(q)
        .filter((s) => inCategory(category, KIND_TYPE[s.kind]))
        .map((s) => curatedRow("Symbols", s));
  const upper = q.toUpperCase();
  if (matches.some((r) => r.ticker === upper)) return matches;
  return [
    ...matches,
    {
      section: "Other",
      ticker: upper,
      name: `Use "${upper}" as typed`,
      exchange: "",
      badge: "custom",
    },
  ];
}
