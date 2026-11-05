// Keep the price pane usable when many sub-panes are stacked: shrink the
// sub-panes (down to MIN_SUB_PANE each) before the price pane drops below
// ~40% of the pane stack or 200px (floor on the sub-panes: 50px), whichever is larger.

export const MIN_SUB_PANE = 50;
const MIN_CANDLE = 210;
const MIN_CANDLE_FRACTION = 0.4;

/**
 * New sub-pane heights for a stack of `candle` + `subs` pixels, or null when
 * the price pane is already tall enough (or nothing can shrink).
 */
export function fitSubPanes(candle: number, subs: number[]): number[] | null {
  if (subs.length === 0) return null;
  const sum = subs.reduce((a, b) => a + b, 0);
  const total = candle + sum;
  const want = Math.min(
    Math.max(MIN_CANDLE, total * MIN_CANDLE_FRACTION),
    total - MIN_SUB_PANE * subs.length,
  );
  if (candle >= want) return null;
  const avail = total - want;
  return subs.map((h) => Math.max(MIN_SUB_PANE, Math.floor((h * avail) / sum)));
}
