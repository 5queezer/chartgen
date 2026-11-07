// Timeframes: the menu's presets, and the parser behind the TradingView-style
// "type a number" shortcut. The server's `src/interval.rs` owns the grammar
// and rejects what it cannot serve (ADR-0038). `parseTimeframe` mirrors it so
// the popup can answer as you type; tests/fixtures/interval_grammar.json is
// run against both, so they cannot drift apart.

export const TIMEFRAME_OPTIONS = [
  "1s",
  "5s",
  "15s",
  "30s",
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
/** A canonical interval: `5s`, `7m`, `8h`, `2d`, `2wk`, `3mo`. See `isTimeframe`. */
export type Timeframe = string;

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

const UNIT_LABELS: Record<string, string> = { d: "D", wk: "W", mo: "M" };

/** As TradingView writes it: minutes and hours as they are, then `2D`, `2W`, `3M`. */
export function timeframeLabel(tf: Timeframe): string {
  return tf.replace(/(d|wk|mo)$/, (unit) => UNIT_LABELS[unit] ?? unit);
}

export type TimeframeGroup = "Seconds" | "Minutes" | "Hours" | "Days";

/** Menu section of a timeframe, as TradingView groups its interval list. */
export function timeframeGroup(tf: Timeframe): TimeframeGroup {
  if (tf.endsWith("s")) return "Seconds";
  if (tf.endsWith("m")) return "Minutes";
  if (tf.endsWith("h")) return "Hours";
  return "Days";
}

/** A stored or imported value: a timeframe in its canonical form. */
export function isTimeframe(x: unknown): x is Timeframe {
  return typeof x === "string" && canonicalTimeframe(x) === x;
}

type Unit = "s" | "m" | "h" | "d" | "wk" | "mo";

/** The longest of each unit, as in `Interval::parse`. */
const MAX_BY_UNIT: Record<Unit, number> = { s: 59, m: 1439, h: 168, d: 365, wk: 52, mo: 12 };

/** Binance's intraday klines in minutes, coarsest first (`binance_base`). */
const BINANCE_MINUTES = [720, 480, 360, 240, 120, 60, 30, 15, 5, 3, 1];

/** Most native bars one bar may take (`MAX_BASE_PER_BAR`). */
const MAX_BASE_PER_BAR = 60;

/** The canonical form of `<number><unit>` with this unit, or null if out of bounds. */
function normalize(count: number, unit: Unit): Timeframe | null {
  if (!Number.isSafeInteger(count) || count < 1) return null;
  let n = count;
  let u = unit;
  if (u === "s" && n % 60 === 0) [n, u] = [n / 60, "m"];
  if (u === "m" && n % 60 === 0) [n, u] = [n / 60, "h"];
  if (u === "h" && n % 24 === 0) [n, u] = [n / 24, "d"];
  if (u === "d" && n % 7 === 0) [n, u] = [n / 7, "wk"];
  if (n > MAX_BY_UNIT[u]) return null;
  if (u === "m" || u === "h") {
    const minutes = u === "h" ? n * 60 : n;
    const base = BINANCE_MINUTES.find((b) => minutes % b === 0) ?? 1;
    if (minutes / base > MAX_BASE_PER_BAR) return null;
  }
  return `${n}${u}`;
}

/** The server's grammar: digits, then `m`, `M`, or `s`/`h`/`d`/`w`/`wk`/`mo` in any case. */
function canonicalTimeframe(text: string): Timeframe | null {
  const m = /^(\d+)(m|M|[sS]|[hH]|[dD]|[wW]|[wW][kK]|[mM][oO])$/.exec(text);
  if (!m) return null;
  const raw = m[2]!;
  const lower = raw.toLowerCase();
  const unit: Unit =
    raw === "m" ? "m"
    : raw === "M" || lower === "mo" ? "mo"
    : lower === "s" ? "s"
    : lower === "h" ? "h"
    : lower === "d" ? "d"
    : "wk";
  return normalize(Number(m[1]), unit);
}

/**
 * Parse what the user typed into the timeframe-entry popup: the server's
 * grammar, plus two TradingView shortcuts. A bare number means minutes
 * (`60` = 1h, `45` = 45m), and a lone `D`, `W` or `M` means one day, week
 * or month. Spaces may come before a unit (`2 h`), never between digits.
 * Returns null for anything the backend cannot serve.
 */
export function parseTimeframe(input: string): Timeframe | null {
  const text = input.trim().replace(/^(\d+)\s+(?=[a-zA-Z])/, "$1");
  if (/^\d+$/.test(text)) return canonicalTimeframe(`${text}m`);
  if (/^[dwDW]$/.test(text) || text === "M") return canonicalTimeframe(`1${text}`);
  return canonicalTimeframe(text);
}
