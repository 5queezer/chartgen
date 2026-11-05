export type Magnet = "normal" | "weak_magnet" | "strong_magnet";

/**
 * The magnet drawings actually use. Holding Ctrl/Cmd inverts the chosen one,
 * like TradingView: off becomes strong, weak or strong becomes off.
 */
export function effectiveMagnet(chosen: Magnet, inverted: boolean): Magnet {
  if (!inverted) return chosen;
  return chosen === "normal" ? "strong_magnet" : "normal";
}
