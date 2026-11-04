// Per-plot style resolution (ADR-0017): maps a payload indicator plus the
// client-side overrides from the store to the effective color / width /
// line style / visibility of each plotted line and histogram, and to the
// klinecharts figure style that draws it.

import { plotKey, type IndicatorStyle, type PlotLineStyle } from "../stores/indicators";

export interface PlotInfo {
  /** `plotKey` of the plot: `line0`, `hist1`, ... */
  key: string;
  kind: "line" | "hist";
  /** Index within its kind. */
  index: number;
  /** Legend / Style tab label. */
  label: string;
  /** Color used when the user has not overridden it. */
  defaultColor: string;
}

export interface ResolvedPlot {
  color: string;
  width: number;
  lineStyle: PlotLineStyle;
  visible: boolean;
}

export const DEFAULT_WIDTH = 1;

/** The subset of a series indicator the Style tab and legend need. */
interface PayloadPlots {
  lines?: ReadonlyArray<{ label: string | null }>;
  histogram?: ReadonlyArray<unknown>;
}

/**
 * Plots of a payload indicator in drawing order (lines, then histograms).
 * `colorAt` gives the default color for the n-th plot of the indicator.
 */
export function plotsOf(
  ind: PayloadPlots,
  colorAt: (n: number) => string,
): PlotInfo[] {
  const lines = ind.lines ?? [];
  const plots: PlotInfo[] = lines.map((line, i) => ({
    key: plotKey("line", i),
    kind: "line",
    index: i,
    label: line.label ?? `line${i}`,
    defaultColor: colorAt(i),
  }));
  (ind.histogram ?? []).forEach((_, i) =>
    plots.push({
      key: plotKey("hist", i),
      kind: "hist",
      index: i,
      label: `hist${i}`,
      defaultColor: colorAt(lines.length + i),
    }),
  );
  return plots;
}

/** Effective style of a plot: the user's overrides over the defaults. */
export function resolvePlot(
  plot: Pick<PlotInfo, "key" | "defaultColor">,
  style: IndicatorStyle | undefined,
): ResolvedPlot {
  const o = style?.plots[plot.key];
  return {
    color: o?.color ?? plot.defaultColor,
    width: o?.width ?? DEFAULT_WIDTH,
    lineStyle: o?.lineStyle ?? "solid",
    visible: o?.visible ?? true,
  };
}

const DASH: Record<Exclude<PlotLineStyle, "solid">, number[]> = {
  dashed: [6, 4],
  dotted: [2, 3],
};

/** klinecharts figure `styles` for a plot; histograms only take a color. */
export function figureStyle(
  kind: PlotInfo["kind"],
  plot: ResolvedPlot,
): Record<string, unknown> {
  if (kind === "hist") return { color: plot.color };
  return plot.lineStyle === "solid"
    ? { color: plot.color, size: plot.width, style: "solid" }
    : {
        color: plot.color,
        size: plot.width,
        style: "dashed",
        dashedValue: DASH[plot.lineStyle],
      };
}
