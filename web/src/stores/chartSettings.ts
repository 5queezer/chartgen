// Chart settings dialog state (ADR-0019). Colors and the candle part toggles
// are overrides of the active theme palette, stored per theme and sparse (a
// missing key means "use the palette"). Everything else is a plain option
// shared by both themes. Persisted under `chartgen.chartSettings`, validated
// on load; unknown or malformed values fall back to the defaults.
//
// `previewChartSettings` changes the live value only, so the dialog can
// preview edits and revert them on Cancel; `saveChartSettings` persists.

import { createSignal } from "solid-js";

import { isColor } from "../lib/color";
import { THEME_NAMES, type ThemeName } from "../lib/theme";

export const COLOR_KEYS = [
  "bodyUp",
  "bodyDown",
  "borderUp",
  "borderDown",
  "wickUp",
  "wickDown",
  "line",
  "background",
  "grid",
  "crosshair",
] as const;
export type ColorKey = (typeof COLOR_KEYS)[number];

export const PART_KEYS = ["bodyOn", "borderOn", "wickOn"] as const;
export type PartKey = (typeof PART_KEYS)[number];

export type ThemeOverrides = { [K in ColorKey]?: string } & {
  [K in PartKey]?: boolean;
};

export const CROSSHAIR_STYLES = ["dashed", "dotted", "solid"] as const;
export type CrosshairStyle = (typeof CROSSHAIR_STYLES)[number];

export const SCALE_PLACEMENTS = ["right", "left"] as const;
export type ScalePlacement = (typeof SCALE_PLACEMENTS)[number];

const BOOLEAN_OPTIONS = [
  "gridVertical",
  "gridHorizontal",
  "lastPriceLine",
  "lastPriceLabel",
  "highLowMarks",
  "legendOhlc",
  "legendChange",
  "legendVolume",
  "legendIndicators",
] as const;
type BooleanOption = (typeof BOOLEAN_OPTIONS)[number];

export type ChartOptions = { [K in BooleanOption]: boolean } & {
  crosshairStyle: CrosshairStyle;
  scalePlacement: ScalePlacement;
};

export interface ChartSettings {
  version: 1;
  themes: Record<ThemeName, ThemeOverrides>;
  options: ChartOptions;
}

export const DEFAULT_OPTIONS: ChartOptions = {
  gridVertical: true,
  gridHorizontal: true,
  lastPriceLine: true,
  lastPriceLabel: true,
  highLowMarks: false,
  legendOhlc: true,
  legendChange: true,
  legendVolume: true,
  legendIndicators: true,
  crosshairStyle: "dashed",
  scalePlacement: "right",
};

export function defaultSettings(): ChartSettings {
  return {
    version: 1,
    themes: { dark: {}, light: {} },
    options: { ...DEFAULT_OPTIONS },
  };
}

const KEY = "chartgen.chartSettings";

function isPlainObject(x: unknown): x is Record<string, unknown> {
  return typeof x === "object" && x !== null && !Array.isArray(x);
}

function sanitizeOverrides(x: unknown): ThemeOverrides {
  const out: ThemeOverrides = {};
  if (!isPlainObject(x)) return out;
  for (const k of COLOR_KEYS) {
    const v = x[k];
    if (isColor(v)) out[k] = v;
  }
  for (const k of PART_KEYS) {
    const v = x[k];
    if (typeof v === "boolean") out[k] = v;
  }
  return out;
}

function sanitizeOptions(x: unknown): ChartOptions {
  const out: ChartOptions = { ...DEFAULT_OPTIONS };
  if (!isPlainObject(x)) return out;
  for (const k of BOOLEAN_OPTIONS) {
    const v = x[k];
    if (typeof v === "boolean") out[k] = v;
  }
  if ((CROSSHAIR_STYLES as readonly unknown[]).includes(x.crosshairStyle)) {
    out.crosshairStyle = x.crosshairStyle as CrosshairStyle;
  }
  if ((SCALE_PLACEMENTS as readonly unknown[]).includes(x.scalePlacement)) {
    out.scalePlacement = x.scalePlacement as ScalePlacement;
  }
  return out;
}

/** Validate stored or edited settings; anything invalid falls back to its default. */
export function sanitizeSettings(x: unknown): ChartSettings {
  if (!isPlainObject(x)) return defaultSettings();
  const themes = isPlainObject(x.themes) ? x.themes : {};
  return {
    version: 1,
    themes: Object.fromEntries(
      THEME_NAMES.map((t) => [t, sanitizeOverrides(themes[t])]),
    ) as Record<ThemeName, ThemeOverrides>,
    options: sanitizeOptions(x.options),
  };
}

function load(): ChartSettings {
  if (typeof localStorage === "undefined") return defaultSettings();
  try {
    const raw = localStorage.getItem(KEY);
    return raw === null ? defaultSettings() : sanitizeSettings(JSON.parse(raw));
  } catch {
    return defaultSettings();
  }
}

function persist(value: ChartSettings): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(KEY, JSON.stringify(value));
  } catch {
    // Quota exceeded or private-mode storage: best-effort only.
  }
}

const [chartSettings, setChartSettings] = createSignal<ChartSettings>(load());

export { chartSettings };

/** Change the live settings without persisting (dialog preview and revert). */
export function previewChartSettings(next: ChartSettings): void {
  setChartSettings(sanitizeSettings(next));
}

/** Change the live settings and persist them. */
export function saveChartSettings(next: ChartSettings): void {
  const value = sanitizeSettings(next);
  setChartSettings(value);
  persist(value);
}
