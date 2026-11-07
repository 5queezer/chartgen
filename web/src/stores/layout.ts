// Toolbar layout state — symbol, timeframe, chart type, price-scale
// mode, color theme, time zone and recent symbols. Solid signals backed by localStorage under the
// `chartgen.layout.` prefix; every value is validated on load and falls
// back to its default if missing or corrupt.

import { batch, createSignal } from "solid-js";

import { activePane } from "./grid";
import {
  DEFAULT_CHART_PARAMS,
  isPriceBarType,
  sanitizeChartParams,
  type ChartParams,
  type PriceBarType,
} from "../lib/priceBars";
import { applyTheme, THEME_NAMES, type ThemeName } from "../lib/theme";
import { TIMEFRAME_OPTIONS, isTimeframe, type Timeframe } from "../lib/timeframe";
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
  | "line_markers"
  | "step_line"
  | "area"
  | "baseline"
  | "columns"
  | "high_low"
  | "hlc_area"
  | "heikin_ashi"
  | "volume_candles"
  | "footprint"
  | "tpo"
  | "renko"
  | "kagi"
  | "point_figure"
  | "line_break"
  | "range";

/** Menu order, as in TradingView. */
export const CHART_TYPES: readonly ChartType[] = [
  "candles",
  "hollow",
  "bars",
  "line",
  "line_markers",
  "step_line",
  "area",
  "baseline",
  "columns",
  "high_low",
  "hlc_area",
  "heikin_ashi",
  "volume_candles",
  "footprint",
  "tpo",
  "renko",
  "line_break",
  "kagi",
  "point_figure",
  "range",
];

/** Server price series style (`generate_chart.chart_style`) a chart type needs. */
export type ChartStyle = "candles" | "heikin_ashi" | PriceBarType;

/**
 * Heikin Ashi (ADR-0003) and the price-based types (ADR-0047) are computed
 * server-side; every other type draws raw bars.
 */
export function chartStyleOf(type: ChartType): ChartStyle {
  return type === "heikin_ashi" || isPriceBarType(type) ? type : "candles";
}

/** Price-axis scale: plain, percent change from the first visible bar, or logarithmic. */
export type ScaleMode = "normal" | "percentage" | "log";

export const SCALE_MODES: readonly ScaleMode[] = [
  "normal",
  "percentage",
  "log",
];

export const MAX_RECENTS = 10;
export const MAX_RECENT_TIMEFRAMES = 5;

const PREFIX = "chartgen.layout.";

const DEFAULT_SYMBOL = "BTCUSDT";
export const DEFAULT_TIMEFRAME: Timeframe = "1h";
export const DEFAULT_CHART_TYPE: ChartType = "candles";
export const DEFAULT_SCALE_MODE: ScaleMode = "normal";

function readAt(fullKey: string): unknown {
  if (typeof localStorage === "undefined") return undefined;
  try {
    const raw = localStorage.getItem(fullKey);
    return raw === null ? undefined : JSON.parse(raw);
  } catch {
    return undefined;
  }
}

function persistAt(fullKey: string, value: unknown): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(fullKey, JSON.stringify(value));
  } catch {
    // Quota exceeded or private-mode storage: best-effort only.
  }
}

const read = (key: string): unknown => readAt(PREFIX + key);
const persist = (key: string, value: unknown): void => persistAt(PREFIX + key, value);

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

export function isScaleMode(x: unknown): x is ScaleMode {
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

/** Custom intervals offered again in the timeframe menu, newest first. */
function sanitizeRecentTimeframes(x: unknown): Timeframe[] {
  if (!Array.isArray(x)) return [];
  const valid = x.filter(
    (tf): tf is Timeframe =>
      isTimeframe(tf) && !TIMEFRAME_OPTIONS.includes(tf as (typeof TIMEFRAME_OPTIONS)[number]),
  );
  return [...new Set(valid)].slice(0, MAX_RECENT_TIMEFRAMES);
}

const storedTheme = read("theme");
const storedTimeZone = read("timeZone");

const [recentTimeframes, setRecentTimeframesSignal] = createSignal<Timeframe[]>(
  sanitizeRecentTimeframes(read("recentTimeframes")),
);
const [theme, setThemeSignal] = createSignal<ThemeName>(
  initialTheme(storedTheme),
);
if (typeof document !== "undefined") applyTheme(theme());
const [timeZone, setTimeZoneSignal] = createSignal<TimeZoneChoice>(
  isTimeZoneChoice(storedTimeZone) ? storedTimeZone : DEFAULT_TIME_ZONE,
);
const [recentSymbols, setRecentSymbolsSignal] =
  createSignal<string[]>(loadRecents());

/** What one chart of the grid holds (ADR-0043), persisted under `prefix`. */
function createChartStore(prefix: string) {
  const readKey = (key: string): unknown => readAt(prefix + key);
  const persistKey = (key: string, value: unknown): void =>
    persistAt(prefix + key, value);

  const storedSymbol = readKey("symbol");
  const storedTimeframe = readKey("timeframe");
  const storedChartType = readKey("chartType");
  const storedScaleMode = readKey("scaleMode");

  const [symbol, setSymbolSignal] = createSignal(
    normalizeSymbol(storedSymbol) ?? DEFAULT_SYMBOL,
  );
  const [timeframe, setTimeframeSignal] = createSignal<Timeframe>(
    isTimeframe(storedTimeframe) ? storedTimeframe : DEFAULT_TIMEFRAME,
  );
  const [chartType, setChartTypeSignal] = createSignal<ChartType>(
    isChartType(storedChartType) ? storedChartType : DEFAULT_CHART_TYPE,
  );
  const storedChartParams = readKey("chartParams");
  const [chartParams, setChartParamsSignal] = createSignal<ChartParams>(
    storedChartParams === undefined ? DEFAULT_CHART_PARAMS : sanitizeChartParams(storedChartParams),
  );
  const [scaleMode, setScaleModeSignal] = createSignal<ScaleMode>(
    isScaleMode(storedScaleMode) ? storedScaleMode : DEFAULT_SCALE_MODE,
  );
  // Auto-fit keeps the price axis fitted to the visible bars; it turns off
  // when the user drags the axis. Session-only: it starts on at every load
  // because a manual scale does not survive a reload.
  // A locked price-to-bar ratio (persisted) implies auto-fit is off.
  const storedLockRatio = readKey("lockRatio") === true;
  const [lockRatio, setLockRatioSignal] = createSignal(storedLockRatio);
  const [invertScale, setInvertScaleSignal] = createSignal(
    readKey("invertScale") === true,
  );
  const [autoScale, setAutoScaleSignal] = createSignal(!storedLockRatio);

  return {
    read: readKey,
    persist: persistKey,
    symbol,
    timeframe,
    chartType,
    chartParams,
    scaleMode,
    autoScale,
    lockRatio,
    invertScale,

    /** Auto-fit on or off; turning it on releases the price-to-bar ratio lock. */
    setAutoScale(next: boolean): void {
      batch(() => {
        setAutoScaleSignal(next);
        if (next && lockRatio()) {
          setLockRatioSignal(false);
          persistKey("lockRatio", false);
        }
      });
    },

    /** Lock the price-to-bar ratio; locking turns auto-fit off, releasing it turns it back on. */
    setLockRatio(next: boolean): void {
      const was = lockRatio();
      batch(() => {
        setLockRatioSignal(next);
        persistKey("lockRatio", next);
        if (next) setAutoScaleSignal(false);
        else if (was) setAutoScaleSignal(true);
      });
    },

    /** Flip the price axis so higher prices are at the bottom. */
    setInvertScale(next: boolean): void {
      setInvertScaleSignal(next);
      persistKey("invertScale", next);
    },

    /** Select a symbol; invalid input is ignored. Recents are added separately. */
    setSymbol(next: string): void {
      const value = normalizeSymbol(next);
      if (value === null) return;
      setSymbolSignal(value);
      persistKey("symbol", value);
    },

    setTimeframe(next: Timeframe): void {
      setTimeframeSignal(next);
      persistKey("timeframe", next);
      if (!TIMEFRAME_OPTIONS.includes(next as (typeof TIMEFRAME_OPTIONS)[number])) {
        const recents = [next, ...recentTimeframes().filter((tf) => tf !== next)].slice(
          0,
          MAX_RECENT_TIMEFRAMES,
        );
        setRecentTimeframesSignal(recents);
        persist("recentTimeframes", recents);
      }
    },

    setChartType(next: ChartType): void {
      setChartTypeSignal(next);
      persistKey("chartType", next);
    },

    setChartParams(next: ChartParams): void {
      setChartParamsSignal(next);
      persistKey("chartParams", next);
    },

    setScaleMode(next: ScaleMode): void {
      setScaleModeSignal(next);
      persistKey("scaleMode", next);
    },
  };
}

export type ChartStore = ReturnType<typeof createChartStore>;

const charts = new Map<number, ChartStore>();

/** The chart store of a pane. Pane 0 has the keys the single-chart app always had. */
export function chartStoreOf(pane: number): ChartStore {
  let store = charts.get(pane);
  if (!store) {
    store = createChartStore(pane === 0 ? PREFIX : `chartgen.pane.${pane}.`);
    charts.set(pane, store);
  }
  return store;
}

chartStoreOf(0);
const active = (): ChartStore => chartStoreOf(activePane());

// The active chart: what the toolbar, the panels and the shortcuts act on.
export const symbol = () => active().symbol();
export const timeframe = () => active().timeframe();
export const chartType = () => active().chartType();
export const scaleMode = () => active().scaleMode();
export const autoScale = () => active().autoScale();
export const lockRatio = () => active().lockRatio();
export const invertScale = () => active().invertScale();
export const setAutoScale = (next: boolean): void => active().setAutoScale(next);
export const setLockRatio = (next: boolean): void => active().setLockRatio(next);
export const setInvertScale = (next: boolean): void => active().setInvertScale(next);
export const setSymbol = (next: string): void => active().setSymbol(next);
export const setTimeframe = (next: Timeframe): void => active().setTimeframe(next);
export const setChartType = (next: ChartType): void => active().setChartType(next);
export const chartParams = () => active().chartParams();
export const setChartParams = (next: ChartParams): void => active().setChartParams(next);
export const setScaleMode = (next: ScaleMode): void => active().setScaleMode(next);

export { theme, timeZone, recentSymbols, recentTimeframes };

/** Move a symbol that loaded successfully to the front of the recents. */
export function addRecent(value: string): void {
  const recents = [
    value,
    ...recentSymbols().filter((s) => s !== value),
  ].slice(0, MAX_RECENTS);
  setRecentSymbolsSignal(recents);
  persist("recents", recents);
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
