// "Lock price to bar ratio": while on, the vertical pixels per price unit
// stay proportional to the bar width, so the candle aspect survives a
// horizontal zoom. klinecharts has no public way to set the y range, so
// the range is steered through the candle pane's top/bottom gap, which
// widens or crops the auto-fitted range:
//   from = low - base * bottom,  to = high + base * top.

export interface Gap {
  top: number;
  bottom: number;
}

export const DEFAULT_GAP: Gap = { top: 0.2, bottom: 0.1 };
/** The range never crops below this share of the fitted data range. */
const MIN_SPAN = 0.1;
const MAX_GAP = 20;
/** Share of the window kept clear around the anchor price. */
const ANCHOR_MARGIN = 0.05;

/** Pixels per price unit, per pixel of bar width. */
export function lockRatio(range: number, height: number, barSpace: number): number {
  return height / range / barSpace;
}

/**
 * Gaps that make the axis span `height / (ratio * barSpace)` price units.
 * `vMin`/`vMax` are the axis bounds now, with `gap` applied. When the span
 * is smaller than the fitted data range (zoomed in) the window is centered
 * on the data but slides to keep `anchor` (the last visible close) in view.
 */
export function lockedGaps(args: {
  vMin: number;
  vMax: number;
  gap: Gap;
  height: number;
  barSpace: number;
  ratio: number;
  anchor?: number;
}): Gap {
  const { vMin, vMax, gap, height, barSpace, ratio, anchor } = args;
  const base = (vMax - vMin) / (1 + gap.top + gap.bottom);
  const low = vMin + base * gap.bottom;
  const high = vMax - base * gap.top;
  const wanted = Math.max(base * MIN_SPAN, height / (ratio * barSpace));
  if (wanted >= base) {
    // Room around the data, split like the default gap (more above).
    const share = DEFAULT_GAP.top / (DEFAULT_GAP.top + DEFAULT_GAP.bottom);
    const extra = (wanted - base) / base;
    return { top: Math.min(MAX_GAP, extra * share), bottom: Math.min(MAX_GAP, extra * (1 - share)) };
  }
  let start = (low + high) / 2 - wanted / 2;
  if (anchor !== undefined) {
    const margin = wanted * ANCHOR_MARGIN;
    if (anchor < start + margin) start = anchor - margin;
    else if (anchor > start + wanted - margin) start = anchor + margin - wanted;
  }
  const clamp = (g: number) => Math.min(MAX_GAP, g);
  return {
    top: clamp((start + wanted - high) / base),
    bottom: clamp((low - start) / base),
  };
}

/** klinecharts reads a gap of 1 or more as pixels, not a share of the range. */
export function toPaneGap(gap: Gap, height: number): Gap {
  const px = (g: number) => (g >= 1 ? g * height : g);
  return { top: px(gap.top), bottom: px(gap.bottom) };
}
