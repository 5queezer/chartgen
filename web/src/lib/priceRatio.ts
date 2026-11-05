// "Lock price to bar ratio": while on, the vertical pixels per price unit
// stay proportional to the bar width, so the candle aspect survives a
// horizontal zoom. klinecharts has no public way to set the y range, so
// the range is steered through the candle pane's top/bottom gap, which
// scales the auto-fitted range: range = base * (1 + top + bottom).

export const DEFAULT_GAP = { top: 0.2, bottom: 0.1 };
/** Negative gaps crop the fitted range; stay clear of -0.5 where it vanishes. */
const MIN_GAP = -0.45;
const MAX_GAP = 20;

/** Pixels per price unit, per pixel of bar width. */
export function lockRatio(range: number, height: number, barSpace: number): number {
  return height / range / barSpace;
}

/**
 * Symmetric gap that makes the axis span `height / (ratio * barSpace)` price
 * units. `range` is the span the axis shows now with `gap` applied.
 */
export function lockedGap(args: {
  range: number;
  gap: { top: number; bottom: number };
  height: number;
  barSpace: number;
  ratio: number;
}): number {
  const base = args.range / (1 + args.gap.top + args.gap.bottom);
  const wanted = args.height / (args.ratio * args.barSpace);
  const g = (wanted / base - 1) / 2;
  return Math.min(MAX_GAP, Math.max(MIN_GAP, g));
}
