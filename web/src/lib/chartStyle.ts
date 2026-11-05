// Merges the theme palette with the chart-settings overrides (ADR-0019).
// `tvStyles` in Chart.tsx paints the palette; `settingsStyles` is applied on
// top of it with klinecharts' deep merge, so a key removed from the overrides
// falls back to the palette on the next apply. Pure, so it can be unit tested.

import { formatColor, parseColor } from "./color";
import type { Palette } from "./theme";
import type {
  ChartOptions,
  ColorKey,
  CrosshairStyle,
  ThemeOverrides,
} from "../stores/chartSettings";
import type { ChartType } from "../stores/layout";

/** klinecharts' own default for the line and area chart types. */
export const DEFAULT_LINE_COLOR = "#2196f3";

const TRANSPARENT = "rgba(0, 0, 0, 0)";

const CROSSHAIR_LINE: Record<
  CrosshairStyle,
  { style: "solid" | "dashed"; dashedValue: number[] }
> = {
  solid: { style: "solid", dashedValue: [2, 2] },
  dashed: { style: "dashed", dashedValue: [4, 2] },
  dotted: { style: "dashed", dashedValue: [1, 3] },
};

/**
 * Colors the chart actually draws: the override, else the palette. Border
 * and wick follow the body color until they get an override of their own.
 */
export function resolveColors(
  p: Palette,
  o: ThemeOverrides,
): Record<ColorKey, string> {
  const bodyUp = o.bodyUp ?? p.bull;
  const bodyDown = o.bodyDown ?? p.bear;
  return {
    bodyUp,
    bodyDown,
    borderUp: o.borderUp ?? bodyUp,
    borderDown: o.borderDown ?? bodyDown,
    wickUp: o.wickUp ?? bodyUp,
    wickDown: o.wickDown ?? bodyDown,
    line: o.line ?? DEFAULT_LINE_COLOR,
    background: o.background ?? p.bg,
    grid: o.grid ?? p.grid,
    crosshair: o.crosshair ?? p.crosshair,
  };
}

type Tree = { [k: string]: unknown };

const isTree = (x: unknown): x is Tree =>
  typeof x === "object" && x !== null && !Array.isArray(x);

/** Deep-merge style layers, later ones winning; arrays are replaced whole. */
export function composeStyles(...layers: Tree[]): Tree {
  const out: Tree = {};
  for (const layer of layers) {
    for (const [k, v] of Object.entries(layer)) {
      const prev = out[k];
      out[k] = isTree(v) && isTree(prev) ? composeStyles(prev, v) : v;
    }
  }
  return out;
}

/** klinecharts `setStyles` payload for the settings, to apply after the palette styles. */
export function settingsStyles(
  p: Palette,
  o: ThemeOverrides,
  opts: ChartOptions,
  chartType: ChartType,
) {
  const c = resolveColors(p, o);
  const part = (on: boolean | undefined, color: string) =>
    on === false ? TRANSPARENT : color;
  // klinecharts draws Bars (`ohlc`) with the body color only.
  const bodyOn = chartType === "bars" ? undefined : o.bodyOn;
  const crosshairLine = {
    color: c.crosshair,
    ...CROSSHAIR_LINE[opts.crosshairStyle],
  };
  const { hex, alpha } = parseColor(c.line);
  return {
    grid: {
      horizontal: { show: opts.gridHorizontal, color: c.grid },
      vertical: { show: opts.gridVertical, color: c.grid },
    },
    candle: {
      bar: {
        upColor: part(bodyOn, c.bodyUp),
        downColor: part(bodyOn, c.bodyDown),
        upBorderColor: part(o.borderOn, c.borderUp),
        downBorderColor: part(o.borderOn, c.borderDown),
        upWickColor: part(o.wickOn, c.wickUp),
        downWickColor: part(o.wickOn, c.wickDown),
      },
      area: {
        lineColor: c.line,
        // The default fill is the accent gradient; follow the line color
        // only once the user picked one.
        ...(chartType === "area" && o.line !== undefined
          ? {
              backgroundColor: [
                { offset: 0, color: formatColor(hex, alpha * 0.28) },
                { offset: 1, color: formatColor(hex, alpha * 0.02) },
              ],
            }
          : {}),
      },
      priceMark: {
        high: { show: opts.highLowMarks },
        low: { show: opts.highLowMarks },
        last: {
          line: { show: opts.lastPriceLine },
          text: { show: opts.lastPriceLabel },
        },
      },
    },
    crosshair: {
      horizontal: { line: crosshairLine },
      vertical: { line: crosshairLine },
    },
    yAxis: { position: opts.scalePlacement },
  };
}
