// Viewport-width breakpoints for the header: how much of it collapses into
// menus. Wider than BP_FULL shows everything; each narrower step moves more
// into the "⋯" menu.

import { createSignal, onCleanup } from "solid-js";

export const BP_FULL = 1280; // below: screenshot/share/fullscreen/settings -> ⋯
export const BP_TIMEFRAMES = 1200; // below: timeframe buttons -> dropdown
export const BP_NARROW = 900; // below: compare/reset/theme -> ⋯
export const BP_TINY = 640; // below: replay -> ⋯, no wordmark

/** Reactive `window.innerWidth`. */
export function createViewportWidth(): () => number {
  const [width, setWidth] = createSignal(window.innerWidth);
  const onResize = () => setWidth(window.innerWidth);
  window.addEventListener("resize", onResize);
  onCleanup(() => window.removeEventListener("resize", onResize));
  return width;
}
