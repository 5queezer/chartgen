export type Magnet = "normal" | "weak_magnet" | "strong_magnet";

/**
 * The magnet drawings actually use. Holding Ctrl/Cmd inverts the chosen one,
 * like TradingView: off becomes strong, weak or strong becomes off.
 */
export function effectiveMagnet(chosen: Magnet, inverted: boolean): Magnet {
  if (!inverted) return chosen;
  return chosen === "normal" ? "strong_magnet" : "normal";
}

/**
 * The key that inverts the magnet is held: Cmd on Mac, where Ctrl+click is the
 * context menu gesture, and Ctrl elsewhere, where Meta is the system's key.
 */
export function magnetKeyHeld(e: { ctrlKey: boolean; metaKey: boolean }, mac: boolean): boolean {
  return mac ? e.metaKey : e.ctrlKey;
}

/**
 * `value` pulled to `bar` by the magnet, for a point that klinecharts' own magnet
 * read the wrong bar for: strong takes the nearest of OHLC, weak only when that
 * is within `sensitivity` pixels (`overlay.modeSensitivity`). `toY` is the axis.
 */
export function magnetValue(
  value: number,
  bar: { open: number; high: number; low: number; close: number },
  mode: Magnet,
  sensitivity: number,
  toY: (value: number) => number,
): number {
  if (mode === "normal") return value;
  const nearest = [bar.open, bar.high, bar.low, bar.close].reduce((best, v) =>
    Math.abs(v - value) < Math.abs(best - value) ? v : best,
  );
  if (mode === "weak_magnet" && !(Math.abs(toY(nearest) - toY(value)) <= sensitivity)) return value;
  return nearest;
}
