// Toolbar layout state — symbol, timeframe, chart type, price-scale
// mode, color theme, time zone and recent symbols. Solid signals backed by localStorage under the
// `chartgen.layout.` prefix; every value is validated on load and falls
// back to its default if missing or corrupt.

import { createSignal } from "solid-js";

import { applyTheme, THEME_NAMES, type ThemeName } from "../lib/theme";
import { isTimeframe, type Timeframe } from "../lib/timeframe";
import {
  DEFAULT_TIME_ZONE,
  isTimeZoneChoice,
  type TimeZoneChoice,
} from "../lib/timezone";

export type ChartType =
  | "candles"
  | "hollow"
  | "bars"
  | "line"
  | "area"
  | "heikin_ashi";

export const CHART_TYPES: readonly ChartType[] = [
  "candles",
  "hollow",
  "bars",
  "line",
  "area",
  "heikin_ashi",
];

/** Server price series style (`generate_chart.chart_style`) a chart type needs. */
export type ChartStyle = "candles" | "heikin_ashi";

/** Heikin Ashi is computed server-side (ADR-0003); every other type draws raw bars. */
export function chartStyleOf(type: ChartType): ChartStyle {
  return type === "heikin_ashi" ? "heikin_ashi" : "candles";
}

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
export const DEFAULT_CHART_TYPE: ChartType = "candles";
export const DEFAULT_SCALE_MODE: ScaleMode = "normal";

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
const SYMBOL_CHARSET = /^[A-Z0-9.^=\-_]+$/;

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

function isThemeName(x: unknown): x is ThemeName {
  return (THEME_NAMES as readonly unknown[]).includes(x);
}

/** Stored theme, else the OS preference on first load. */
function initialTheme(stored: unknown): ThemeName {
  if (isThemeName(stored)) return stored;
  const prefersLight =
    typeof matchMedia !== "undefined" &&
    matchMedia("(prefers-color-scheme: light)").matches;
  return prefersLight ? "light" : "dark";
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
const storedTheme = read("theme");
const storedTimeZone = read("timeZone");

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
const [theme, setThemeSignal] = createSignal<ThemeName>(
  initialTheme(storedTheme),
);
if (typeof document !== "undefined") applyTheme(theme());
const [timeZone, setTimeZoneSignal] = createSignal<TimeZoneChoice>(
  isTimeZoneChoice(storedTimeZone) ? storedTimeZone : DEFAULT_TIME_ZONE,
);
// Auto-fit keeps the price axis fitted to the visible bars; it turns off
// when the user drags the axis. Session-only: it starts on at every load
// because a manual scale does not survive a reload.
const [autoScale, setAutoScale] = createSignal(true);
const [recentSymbols, setRecentSymbolsSignal] =
  createSignal<string[]>(loadRecents());

export {
  symbol,
  timeframe,
  chartType,
  scaleMode,
  theme,
  timeZone,
  autoScale,
  setAutoScale,
  recentSymbols,
};

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

export function setTheme(next: ThemeName): void {
  setThemeSignal(next);
  if (typeof document !== "undefined") applyTheme(next);
  persist("theme", next);
}

export function setTimeZone(next: TimeZoneChoice): void {
  if (!isTimeZoneChoice(next)) return;
  setTimeZoneSignal(next);
  persist("timeZone", next);
}
