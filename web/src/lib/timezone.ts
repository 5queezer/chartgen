// Time zone handling for the chart (axis, crosshair, legend, bottom-bar
// clock, Go to date). Bar timestamps are UTC; everything here converts to
// and from wall-clock time in an IANA zone with Intl.DateTimeFormat, so
// there is no DOM and no klinecharts import and it stays unit-testable.

import { isContinuousMarket } from "./countdown";

/** "utc", "exchange" and "local" are resolved per symbol; anything else is an IANA zone. */
export type TimeZoneChoice = string;

/** Common IANA zones offered next to UTC / Exchange / Local. */
export const COMMON_TIME_ZONES = [
  "America/New_York",
  "America/Chicago",
  "America/Los_Angeles",
  "America/Toronto",
  "America/Sao_Paulo",
  "Europe/London",
  "Europe/Berlin",
  "Europe/Moscow",
  "Asia/Dubai",
  "Asia/Kolkata",
  "Asia/Shanghai",
  "Asia/Hong_Kong",
  "Asia/Tokyo",
  "Australia/Sydney",
] as const;

export const DEFAULT_TIME_ZONE: TimeZoneChoice = "local";

export function isTimeZoneChoice(x: unknown): x is TimeZoneChoice {
  return (
    x === "utc" ||
    x === "exchange" ||
    x === "local" ||
    (COMMON_TIME_ZONES as readonly unknown[]).includes(x)
  );
}

const SUFFIX_ZONES: ReadonlyArray<readonly [string, string]> = [
  [".DE", "Europe/Berlin"],
  [".L", "Europe/London"],
  [".TO", "America/Toronto"],
  ["=X", "UTC"],
];

/**
 * Exchange time zone of a symbol. A HEURISTIC: the series payload carries
 * no exchange, so crypto (Binance) is UTC, a few Yahoo suffixes map to
 * their exchange's zone, and every other symbol is assumed to be a US
 * listing (America/New_York).
 */
export function exchangeTimeZone(symbol: string): string {
  const s = symbol.toUpperCase();
  if (isContinuousMarket(s)) return "UTC";
  return SUFFIX_ZONES.find(([suffix]) => s.endsWith(suffix))?.[1] ?? "America/New_York";
}

/** The browser's own IANA zone. */
export function localTimeZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
}

/** IANA zone behind a choice, for `symbol`. */
export function resolveTimeZone(choice: TimeZoneChoice, symbol: string): string {
  switch (choice) {
    case "utc":
      return "UTC";
    case "exchange":
      return exchangeTimeZone(symbol);
    case "local":
      return localTimeZone();
    default:
      return choice;
  }
}

export interface Wall {
  year: number;
  /** 1-12 */
  month: number;
  day: number;
  hour: number;
  minute: number;
  second: number;
}

const formatters = new Map<string, Intl.DateTimeFormat>();

function formatterFor(tz: string): Intl.DateTimeFormat {
  let f = formatters.get(tz);
  if (!f) {
    f = new Intl.DateTimeFormat("en-US", {
      timeZone: tz,
      hourCycle: "h23",
      year: "numeric",
      month: "numeric",
      day: "numeric",
      hour: "numeric",
      minute: "numeric",
      second: "numeric",
    });
    formatters.set(tz, f);
  }
  return f;
}

/** Wall-clock fields of the instant `ms` in `tz`. */
export function wallClock(ms: number, tz: string): Wall {
  const out: Record<string, number> = {};
  for (const p of formatterFor(tz).formatToParts(ms)) {
    if (p.type !== "literal") out[p.type] = Number(p.value);
  }
  return {
    year: out.year!,
    month: out.month!,
    day: out.day!,
    hour: out.hour!,
    minute: out.minute!,
    second: out.second!,
  };
}

const DAY_MS = 86_400_000;

const utcOf = (w: Wall) =>
  Date.UTC(w.year, w.month - 1, w.day, w.hour, w.minute, w.second);

/** Offset of `tz` from UTC at the instant `ms`, in ms. */
export function offsetMs(ms: number, tz: string): number {
  return utcOf(wallClock(ms, tz)) - Math.floor(ms / 1000) * 1000;
}

/**
 * The instant at which `tz`'s wall clock reads `wall`. A wall time inside a
 * DST gap does not exist and lands after it (02:30 in the hour skipped at
 * 02:00 -> 03:00 reads 03:30); in an overlap the first occurrence is used.
 */
export function zonedToUtc(wall: Wall, tz: string): number {
  const guess = utcOf(wall);
  const before = guess - offsetMs(guess - DAY_MS, tz);
  const after = guess - offsetMs(guess + DAY_MS, tz);
  const valid = [before, after].filter((t) => utcOf(wallClock(t, tz)) === guess);
  return valid.length > 0 ? Math.min(...valid) : before;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** "YYYY-MM-DD HH:mm", the crosshair label's format. */
export function formatDateTime(ms: number, tz: string): string {
  const w = wallClock(ms, tz);
  return `${w.year}-${pad(w.month)}-${pad(w.day)} ${pad(w.hour)}:${pad(w.minute)}`;
}

/** "HH:mm:ss", the bottom-bar clock. */
export function formatClock(ms: number, tz: string): string {
  const w = wallClock(ms, tz);
  return `${pad(w.hour)}:${pad(w.minute)}:${pad(w.second)}`;
}

/** "UTC", "UTC-5", "UTC+5:30": the zone's offset at `ms`. */
export function offsetLabel(ms: number, tz: string): string {
  const minutes = Math.round(offsetMs(ms, tz) / 60_000);
  if (minutes === 0) return "UTC";
  const abs = Math.abs(minutes);
  const m = abs % 60;
  return `UTC${minutes < 0 ? "-" : "+"}${Math.floor(abs / 60)}${m ? `:${pad(m)}` : ""}`;
}

/** Daily, weekly and monthly bars stand for a date, not a time of day. */
export function isDateBar(interval: string | null | undefined): boolean {
  return /^\d+(d|wk|mo)$/.test(interval ?? "");
}

/**
 * Legend time of a bar: the date alone for date bars (their open time is a
 * session or UTC midnight, so it is read as a UTC date in every zone),
 * otherwise date and time in `tz`.
 */
export function formatBarTime(ms: number, tz: string, interval: string | null): string {
  return isDateBar(interval) ? new Date(ms).toISOString().slice(0, 10) : formatDateTime(ms, tz);
}
