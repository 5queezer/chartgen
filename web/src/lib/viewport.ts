// Viewport-width breakpoints for the header: how much of it collapses into
// menus. Wider than BP_FULL shows everything; each narrower step moves more
// into the "⋯" menu.

import { createSignal, onCleanup } from "solid-js";

export const BP_FULL = 1280; // below: screenshot/share/fullscreen/settings -> ⋯
export const BP_TIMEFRAMES = 1200; // below: timeframe buttons -> dropdown
export const BP_NARROW = 900; // below: compare/reset/theme -> ⋯
export const BP_TINY = 640; // below: replay -> ⋯, no wordmark

// The compact shell: phones and portrait tablets, or any very narrow window.
// `html.compact` scopes its CSS (globals.css); `compact()` is for the little
// behaviour that has to branch. Crossing it only moves the layout: nothing is
// reloaded and the chart stays mounted (ADR-0039).
const COMPACT_QUERY =
  "(max-width: 767px), (pointer: coarse) and (max-width: 1023px), (pointer: coarse) and (max-height: 500px)";
const media =
  typeof window !== "undefined" && window.matchMedia
    ? window.matchMedia(COMPACT_QUERY)
    : null;
const [compact, setCompact] = createSignal(media?.matches ?? false);
export { compact };

if (media) {
  const root = document.documentElement;
  root.classList.toggle("compact", media.matches);
  media.addEventListener("change", (e) => {
    root.classList.toggle("compact", e.matches);
    setCompact(e.matches);
  });
  // dvh ignores the on-screen keyboard on iOS and by default on Android; the
  // visual viewport does not, so dialogs size to it (--vvh, --vvt).
  const visible = window.visualViewport;
  if (visible) {
    const sync = () => {
      root.style.setProperty("--vvh", `${visible.height}px`);
      root.style.setProperty("--vvt", `${visible.offsetTop}px`);
    };
    sync();
    visible.addEventListener("resize", sync);
    visible.addEventListener("scroll", sync);
  }
}

/** Reactive `window.innerWidth`. */
export function createViewportWidth(): () => number {
  const [width, setWidth] = createSignal(window.innerWidth);
  const onResize = () => setWidth(window.innerWidth);
  window.addEventListener("resize", onResize);
  onCleanup(() => window.removeEventListener("resize", onResize));
  return width;
}
