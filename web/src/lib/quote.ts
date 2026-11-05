// Watchlist number formatting. Decimals follow the price magnitude
// (BTC 2, EURUSD 4, SHIB 8) so a row's Last and Chg use the same
// precision; signs use a typographic minus, like TradingView.

const MINUS = "−";
const EMPTY = "—";

/** Decimal places for a price of this magnitude. */
export function priceDecimals(price: number): number {
  const abs = Math.abs(price);
  if (abs >= 10) return 2;
  if (abs >= 1) return 4;
  if (abs >= 0.01) return 6;
  return 8;
}

function signed(value: number, decimals: number): string {
  const text = Math.abs(value).toFixed(decimals);
  // Rounds to zero: no sign, so "+0.00" never shows next to a flat row.
  if (Number(text) === 0) return text;
  return `${value > 0 ? "+" : MINUS}${text}`;
}

export function formatPrice(price: number | null): string {
  return price === null ? EMPTY : price.toFixed(priceDecimals(price));
}

/** Absolute change, at the precision of `last`. */
export function formatChange(change: number | null, last: number | null): string {
  if (change === null) return EMPTY;
  return signed(change, priceDecimals(last ?? change));
}

export function formatChangePct(pct: number | null): string {
  return pct === null ? EMPTY : `${signed(pct, 2)}%`;
}

/** Direction for bull/bear coloring. */
export function trend(change: number | null): "up" | "down" | "flat" {
  if (change === null || change === 0) return "flat";
  return change > 0 ? "up" : "down";
}
