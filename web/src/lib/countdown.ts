// Time left until the current bar closes, shown under the last-price
// badge. Pure math so rollover and interval edge cases are unit-tested.

const UNIT_SECONDS = { m: 60, h: 3600, d: 86_400, wk: 604_800 } as const;

const CRYPTO_QUOTES = ["USDT", "BUSD", "BTC", "ETH", "BNB", "USDC", "FDUSD"];

/**
 * True for symbols that trade around the clock, where bars close on the
 * interval grid. Mirrors the backend's `detect_source` Binance rule
 * (`src/fetch.rs`); everything else is a Yahoo symbol with trading
 * sessions, whose bar closes we cannot derive from the interval.
 */
export function isContinuousMarket(symbol: string | null | undefined): boolean {
  const s = (symbol ?? "").toUpperCase();
  return CRYPTO_QUOTES.some((q) => s.endsWith(q) && s.length > q.length);
}

/**
 * Close time (ms) of the bar that opens at `openSec`. Months follow the
 * calendar; every other interval is a fixed length. Null for an
 * interval string we do not know.
 */
export function barCloseMs(openSec: number, interval: string): number | null {
  const m = /^(\d+)(m|h|d|wk|mo)$/.exec(interval);
  if (!m) return null;
  const n = Number(m[1]);
  const unit = m[2] as keyof typeof UNIT_SECONDS | "mo";
  if (unit === "mo") {
    const d = new Date(openSec * 1000);
    d.setUTCMonth(d.getUTCMonth() + n);
    return d.getTime();
  }
  return (openSec + n * UNIT_SECONDS[unit]) * 1000;
}

const pad = (n: number) => String(n).padStart(2, "0");

/**
 * Remaining time as `MM:SS` (under an hour), `HH:MM:SS` (under a day) or
 * `Nd HH:MM`. Past the close it stays at `00:00`.
 */
export function formatCountdown(closeMs: number, nowMs: number): string {
  const total = Math.max(0, Math.ceil((closeMs - nowMs) / 1000));
  const days = Math.floor(total / 86_400);
  const hours = Math.floor((total % 86_400) / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  if (days > 0) return `${days}d ${pad(hours)}:${pad(minutes)}`;
  if (hours > 0) return `${pad(hours)}:${pad(minutes)}:${pad(seconds)}`;
  return `${pad(minutes)}:${pad(seconds)}`;
}

/**
 * Countdown text for the bar opening at `openSec`; null if the interval
 * is unknown or the bar has already closed (the next live bar is not
 * delivered yet, so there is nothing to count down to).
 */
export function barCountdown(
  openSec: number,
  interval: string,
  nowMs: number,
): string | null {
  const close = barCloseMs(openSec, interval);
  return close === null || nowMs >= close ? null : formatCountdown(close, nowMs);
}
