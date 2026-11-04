// Bottom-bar date ranges (TradingView's 1D 5D 1M 3M 6M YTD 1Y 5Y All):
// the interval each range uses and where it starts. Pure.

import type { Timeframe } from "./timeframe";
import { wallClock, zonedToUtc } from "./timezone";

export const RANGES = ["1D", "5D", "1M", "3M", "6M", "YTD", "1Y", "5Y", "All"] as const;
export type RangeId = (typeof RANGES)[number];

/**
 * TradingView's interval per range. 6M is 4h here because TradingView's
 * 2h is not a backend timeframe.
 */
export const RANGE_TIMEFRAME: Record<RangeId, Timeframe> = {
  "1D": "1m",
  "5D": "5m",
  "1M": "30m",
  "3M": "1h",
  "6M": "4h",
  YTD: "1d",
  "1Y": "1d",
  "5Y": "1wk",
  All: "1mo",
};

const DAY_MS = 86_400_000;
const MONTHS: Partial<Record<RangeId, number>> = {
  "1M": 1,
  "3M": 3,
  "6M": 6,
  "1Y": 12,
  "5Y": 60,
};

const daysIn = (year: number, month: number) =>
  new Date(Date.UTC(year, month, 0)).getUTCDate();

/**
 * Start (ms) of `range` ending at `endMs`, in time zone `tz`. 1D and 5D are
 * 24 h multiples; month ranges are calendar months back at the same wall
 * clock time (a day past the target month's end is clamped: 31 Mar - 1M =
 * 28/29 Feb); YTD is 1 Jan 00:00 of `endMs`'s year in `tz`. Null for "All":
 * no start.
 */
export function rangeStart(range: RangeId, endMs: number, tz: string): number | null {
  if (range === "All") return null;
  if (range === "1D") return endMs - DAY_MS;
  if (range === "5D") return endMs - 5 * DAY_MS;
  const w = wallClock(endMs, tz);
  if (range === "YTD") {
    return zonedToUtc({ ...w, month: 1, day: 1, hour: 0, minute: 0, second: 0 }, tz);
  }
  const back = w.year * 12 + (w.month - 1) - MONTHS[range]!;
  const year = Math.floor(back / 12);
  const month = (back % 12) + 1;
  return zonedToUtc({ ...w, year, month, day: Math.min(w.day, daysIn(year, month)) }, tz);
}
