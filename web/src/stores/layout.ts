// Toolbar layout state — symbol, timeframe, chart type and recent
// symbols. Solid signals backed by localStorage under the
// `chartgen.layout.` prefix; every value is validated on load and falls
// back to its default if missing or corrupt.

import { createSignal } from "solid-js";

import { isTimeframe, type Timeframe } from "../lib/timeframe";

export type ChartType = "candles" | "hollow" | "bars" | "line" | "area";

export const CHART_TYPES: readonly ChartType[] = [
  "candles",
  "hollow",
  "bars",
  "line",
  "area",
];

export const MAX_RECENTS = 10;

const PREFIX = "chartgen.layout.";

const DEFAULT_SYMBOL = "BTCUSDT";
const DEFAULT_TIMEFRAME: Timeframe = "1h";
const DEFAULT_CHART_TYPE: ChartType = "candles";

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

function isSymbol(x: unknown): x is string {
  return typeof x === "string" && x.trim().length > 0;
}

function isChartType(x: unknown): x is ChartType {
  return (CHART_TYPES as readonly unknown[]).includes(x);
}

function loadRecents(): string[] {
  const raw = read("recents");
  if (!Array.isArray(raw)) return [];
  return [...new Set(raw.filter(isSymbol))].slice(0, MAX_RECENTS);
}

const storedSymbol = read("symbol");
const storedTimeframe = read("timeframe");
const storedChartType = read("chartType");

const [symbol, setSymbolSignal] = createSignal(
  isSymbol(storedSymbol) ? storedSymbol : DEFAULT_SYMBOL,
);
const [timeframe, setTimeframeSignal] = createSignal<Timeframe>(
  isTimeframe(storedTimeframe) ? storedTimeframe : DEFAULT_TIMEFRAME,
);
const [chartType, setChartTypeSignal] = createSignal<ChartType>(
  isChartType(storedChartType) ? storedChartType : DEFAULT_CHART_TYPE,
);
const [recentSymbols, setRecentSymbolsSignal] =
  createSignal<string[]>(loadRecents());

export { symbol, timeframe, chartType, recentSymbols };

/** Select a symbol (uppercased) and move it to the front of the recents. */
export function setSymbol(next: string): void {
  const value = next.trim().toUpperCase();
  if (value === "") return;
  setSymbolSignal(value);
  persist("symbol", value);
  const recents = [
    value,
    ...recentSymbols().filter((s) => s !== value),
  ].slice(0, MAX_RECENTS);
  setRecentSymbolsSignal(recents);
  persist("recents", recents);
}

export function setTimeframe(next: Timeframe): void {
  setTimeframeSignal(next);
  persist("timeframe", next);
}

export function setChartType(next: ChartType): void {
  setChartTypeSignal(next);
  persist("chartType", next);
}
