// Toolbar layout state — symbol, timeframe, chart type, price-scale
// mode and recent symbols. Solid signals backed by localStorage under the
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

/** Price-axis scale: plain, percent change from the first visible bar, or logarithmic. */
export type ScaleMode = "normal" | "percentage" | "log";

export const SCALE_MODES: readonly ScaleMode[] = [
  "normal",
  "percentage",
  "log",
];

export const MAX_RECENTS = 10;

const PREFIX = "chartgen.layout.";

const DEFAULT_SYMBOL = "BTCUSDT";
const DEFAULT_TIMEFRAME: Timeframe = "1h";
const DEFAULT_CHART_TYPE: ChartType = "candles";
const DEFAULT_SCALE_MODE: ScaleMode = "normal";

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

const MAX_SYMBOL_LENGTH = 32;
const SYMBOL_CHARSET = /^[A-Z0-9.^=\-_/]+$/;

/** Trim + uppercase; null if empty, too long, or has unexpected characters. */
export function normalizeSymbol(x: unknown): string | null {
  if (typeof x !== "string") return null;
  const v = x.trim().toUpperCase();
  if (v.length === 0 || v.length > MAX_SYMBOL_LENGTH) return null;
  return SYMBOL_CHARSET.test(v) ? v : null;
}

function isChartType(x: unknown): x is ChartType {
  return (CHART_TYPES as readonly unknown[]).includes(x);
}

function isScaleMode(x: unknown): x is ScaleMode {
  return (SCALE_MODES as readonly unknown[]).includes(x);
}

function loadRecents(): string[] {
  const raw = read("recents");
  if (!Array.isArray(raw)) return [];
  const normalized = raw
    .map(normalizeSymbol)
    .filter((s): s is string => s !== null);
  return [...new Set(normalized)].slice(0, MAX_RECENTS);
}

const storedSymbol = read("symbol");
const storedTimeframe = read("timeframe");
const storedChartType = read("chartType");
const storedScaleMode = read("scaleMode");
const storedAutoScale = read("autoScale");

const [symbol, setSymbolSignal] = createSignal(
  normalizeSymbol(storedSymbol) ?? DEFAULT_SYMBOL,
);
const [timeframe, setTimeframeSignal] = createSignal<Timeframe>(
  isTimeframe(storedTimeframe) ? storedTimeframe : DEFAULT_TIMEFRAME,
);
const [chartType, setChartTypeSignal] = createSignal<ChartType>(
  isChartType(storedChartType) ? storedChartType : DEFAULT_CHART_TYPE,
);
const [scaleMode, setScaleModeSignal] = createSignal<ScaleMode>(
  isScaleMode(storedScaleMode) ? storedScaleMode : DEFAULT_SCALE_MODE,
);
// Auto-fit locks the price axis to the visible bars; off lets the user
// drag the axis to scale it by hand.
const [autoScale, setAutoScaleSignal] = createSignal(
  typeof storedAutoScale === "boolean" ? storedAutoScale : true,
);
const [recentSymbols, setRecentSymbolsSignal] =
  createSignal<string[]>(loadRecents());

export { symbol, timeframe, chartType, scaleMode, autoScale, recentSymbols };

/** Select a symbol; invalid input is ignored. Recents are added separately. */
export function setSymbol(next: string): void {
  const value = normalizeSymbol(next);
  if (value === null) return;
  setSymbolSignal(value);
  persist("symbol", value);
}

/** Move a symbol that loaded successfully to the front of the recents. */
export function addRecent(value: string): void {
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

export function setScaleMode(next: ScaleMode): void {
  setScaleModeSignal(next);
  persist("scaleMode", next);
}

export function setAutoScale(next: boolean): void {
  setAutoScaleSignal(next);
  persist("autoScale", next);
}
