// Timeframes the chartgen backend accepts for both data sources
// (Binance and Yahoo; see `yahoo_interval_range` in src/fetch.rs), plus
// the pure parser behind the TradingView-style "type a number" shortcut.

export const TIMEFRAME_OPTIONS = [
  "1m",
  "3m",
  "5m",
  "15m",
  "30m",
  "1h",
  "2h",
  "4h",
  "6h",
  "12h",
  "1d",
  "3d",
  "1wk",
  "1mo",
] as const;
export type Timeframe = (typeof TIMEFRAME_OPTIONS)[number];

/** Shown as one-click buttons in the toolbar; the rest go in the overflow menu. */
export const FAVOURITE_TIMEFRAMES: readonly Timeframe[] = [
  "1m",
  "5m",
  "15m",
  "1h",
  "4h",
  "1d",
  "1wk",
];

const LABELS: Record<Timeframe, string> = {
  "1m": "1m",
  "3m": "3m",
  "5m": "5m",
  "15m": "15m",
  "30m": "30m",
  "1h": "1h",
  "2h": "2h",
  "4h": "4h",
  "6h": "6h",
  "12h": "12h",
  "1d": "1D",
  "3d": "3D",
  "1wk": "1W",
  "1mo": "1M",
};

export function timeframeLabel(tf: Timeframe): string {
  return LABELS[tf];
}

export type TimeframeGroup = "Minutes" | "Hours" | "Days";

/** Menu section of a timeframe, as TradingView groups its interval list. */
export function timeframeGroup(tf: Timeframe): TimeframeGroup {
  if (tf.endsWith("m")) return "Minutes";
  if (tf.endsWith("h")) return "Hours";
  return "Days";
}

export function isTimeframe(x: unknown): x is Timeframe {
  return (TIMEFRAME_OPTIONS as readonly unknown[]).includes(x);
}

const MINUTES_PER_UNIT = {
  m: 1,
  h: 60,
  d: 1440,
  w: 10080,
  M: 43200,
} as const;

const BY_MINUTES = new Map<number, Timeframe>([
  [1, "1m"],
  [3, "3m"],
  [5, "5m"],
  [15, "15m"],
  [30, "30m"],
  [60, "1h"],
  [120, "2h"],
  [240, "4h"],
  [360, "6h"],
  [720, "12h"],
  [1440, "1d"],
  [4320, "3d"],
  [10080, "1wk"],
  [43200, "1mo"],
]);

/**
 * Parse what the user typed into the timeframe-entry popup. A bare
 * number means minutes (TradingView convention: `60` = 1h, `120` = 2h);
 * an optional unit m/h/d/w follows (case-insensitive), except that an
 * uppercase `M` (or `mo`) means month, as in TradingView. A lone `D`,
 * `W` or `M` means one day / week / month. Returns null for anything
 * the backend cannot serve.
 */
export function parseTimeframe(input: string): Timeframe | null {
  const m = /^(\d*)\s*(mo|[mhdw])?$/i.exec(input.trim());
  if (!m) return null;
  const digits = m[1] ?? "";
  const raw = m[2] ?? "m";
  const unit = (
    raw === "M" || raw.toLowerCase() === "mo" ? "M" : raw.toLowerCase()
  ) as keyof typeof MINUTES_PER_UNIT;
  if (digits === "" && unit !== "d" && unit !== "w" && unit !== "M") {
    return null;
  }
  const n = digits === "" ? 1 : Number(digits);
  return BY_MINUTES.get(n * MINUTES_PER_UNIT[unit]) ?? null;
}
