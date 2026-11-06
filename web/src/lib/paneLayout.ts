// Keep the price pane usable when many sub-panes are stacked: shrink the
// sub-panes (down to MIN_SUB_PANE each) before the price pane drops below
// ~40% of the pane stack or 200px (floor on the sub-panes: 50px), whichever is larger.

export const MIN_SUB_PANE = 50;
// On a short compact screen (a phone on its side) the price pane takes most of
// the height, and the sub-panes get what is left, down to 30px each.
const SHORT_STACK = 420;
const SHORT_CANDLE_FRACTION = 0.65;
const SHORT_MIN_SUB_PANE = 30;
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

/**
 * Sub-pane heights on the compact shell, for a stack of `stack` pixels. A pure
 * function of the screen and of the heights the panes had before shrinking
 * (`natural`), so a rotation there and back returns to those heights.
 */
export function compactPaneHeights(stack: number, natural: number[]): number[] {
  const sum = natural.reduce((a, b) => a + b, 0);
  if (stack >= SHORT_STACK) return fitSubPanes(stack - sum, natural) ?? natural;
  const room = Math.max(
    stack * (1 - SHORT_CANDLE_FRACTION),
    SHORT_MIN_SUB_PANE * natural.length,
  );
  if (sum <= room) return natural;
  return natural.map((h) => Math.max(SHORT_MIN_SUB_PANE, Math.floor((h * room) / sum)));
}
