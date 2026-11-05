// Single source of truth for the dark and light palettes (ADR-0016).
// klinecharts draws to canvas and cannot resolve CSS variables, so Chart.tsx
// reads these values directly; everything else gets them as CSS custom
// properties via `applyTheme`. globals.css keeps the dark values as the
// pre-JS default, and theme.test.ts asserts the two agree.

/** App font, also used by the canvas text (chart axes, overlays). */
export const FONT_FAMILY = "Inter, system-ui, sans-serif";

export type ThemeName = "dark" | "light";

export const THEME_NAMES: readonly ThemeName[] = ["dark", "light"];

export interface Palette {
  bg: string;
  surface: string;
  border: string;
  fg: string;
  fgMuted: string;
  accent: string;
  bull: string;
  bear: string;
  grid: string;
  crosshair: string;
  /** Background and text of the crosshair axis labels. */
  labelBg: string;
  labelFg: string;
  /** Neutral overlay fill (WaveTrend VWAP area) that must contrast with `bg`. */
  neutralFill: string;
}

export const dark: Palette = {
  bg: "#131722",
  surface: "#1e222d",
  border: "#2a2e39",
  fg: "#d1d4dc",
  fgMuted: "#787b86",
  accent: "#2962ff",
  bull: "#26a69a",
  bear: "#ef5350",
  grid: "#2a2e39",
  crosshair: "#787b86",
  labelBg: "#1e222d",
  labelFg: "#ffffff",
  neutralFill: "rgba(255, 255, 255, 0.15)",
};

// TradingView's light theme. fgMuted is darkened from TV's #787b86 (4.2:1 on
// white) to reach WCAG AA for text.
export const light: Palette = {
  bg: "#ffffff",
  surface: "#f0f3fa",
  border: "#e0e3eb",
  fg: "#131722",
  fgMuted: "#6a6d78",
  accent: "#2962ff",
  bull: "#089981",
  bear: "#f23645",
  grid: "#f0f3fa",
  crosshair: "#9598a1",
  labelBg: "#131722",
  labelFg: "#ffffff",
  neutralFill: "rgba(19, 23, 34, 0.10)",
};

export const PALETTES: Record<ThemeName, Palette> = { dark, light };

/** `fgMuted` -> `--color-fg-muted`. */
export function cssVarName(key: keyof Palette): string {
  return "--color-" + key.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());
}

/** Push a palette into CSS custom properties on `root` and flag the scheme. */
export function applyTheme(
  name: ThemeName,
  root: HTMLElement = document.documentElement,
): void {
  const palette = PALETTES[name];
  for (const key of Object.keys(palette) as (keyof Palette)[]) {
    root.style.setProperty(cssVarName(key), palette[key]);
  }
  root.style.colorScheme = name;
  root.dataset.theme = name;
}
