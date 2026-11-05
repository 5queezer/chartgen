// Per-plot style resolution (ADR-0017): maps a payload indicator plus the
// client-side overrides from the store to the effective color / width /
// line style / visibility of each plotted line and histogram, and to the
// klinecharts figure style that draws it.

import {
  labelKey,
  plotKey,
  type IndicatorStyle,
  type PlotLineStyle,
  type PlotStyle,
} from "../stores/indicators";

export interface PlotInfo {
  /**
   * Store key of the plot: `l:<label>` for a line whose label is unique in
   * the indicator, else the index key `line0`, `hist1`, ...
   */
  key: string;
  /** True when `key` is an index key, which is only valid for `label`. */
  indexKeyed: boolean;
  kind: "line" | "hist";
  /** Index within its kind. */
  index: number;
  /** Legend / Style tab label. */
  label: string;
  /**
   * Color used when the user has not overridden it: the server's (ADR-0026),
   * else the palette's. Per-bar histogram colors take precedence over it.
   */
  defaultColor: string;
  /** Line width used when the user has not overridden it. */
  defaultWidth: number;
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
  lines?: ReadonlyArray<{ label: string | null; color?: string; width?: number }>;
  histogram?: ReadonlyArray<PayloadHistogram>;
}

/** Label and per-bar colors of a payload histogram (ADR-0026). */
interface PayloadHistogram {
  label?: string;
  palette?: readonly string[];
  color_idx?: ReadonlyArray<number | null>;
}

/**
 * Plots of a payload indicator in drawing order (lines, then histograms).
 * `colorAt` gives the fallback color for the n-th plot of the indicator,
 * used where the server sends none.
 */
export function plotsOf(
  ind: PayloadPlots,
  colorAt: (n: number) => string,
): PlotInfo[] {
  const lines = ind.lines ?? [];
  const counts = new Map<string, number>();
  for (const line of lines) {
    if (line.label) counts.set(line.label, (counts.get(line.label) ?? 0) + 1);
  }
  const plots: PlotInfo[] = lines.map((line, i) => {
    const unique = !!line.label && counts.get(line.label) === 1;
    return {
      key: unique ? labelKey(line.label!) : plotKey("line", i),
      indexKeyed: !unique,
      kind: "line",
      index: i,
      label: line.label ?? `line${i}`,
      defaultColor: line.color ?? colorAt(i),
      defaultWidth: line.width ?? DEFAULT_WIDTH,
    };
  });
  (ind.histogram ?? []).forEach((h, i) =>
    plots.push({
      key: plotKey("hist", i),
      indexKeyed: true,
      kind: "hist",
      index: i,
      label: h.label ?? `hist${i}`,
      defaultColor: h.palette?.[0] ?? colorAt(lines.length + i),
      defaultWidth: DEFAULT_WIDTH,
    }),
  );
  return plots;
}

/**
 * The stored override that applies to a plot, or undefined. An entry saved
 * for another label is ignored. A legacy `line<i>` entry (saved before label
 * keys existed) still applies to a label-keyed line at that index.
 */
export function plotOverride(
  plot: PlotInfo,
  style: IndicatorStyle | undefined,
): PlotStyle | undefined {
  const plots = style?.plots;
  if (!plots) return undefined;
  const o =
    plots[plot.key] ??
    (plot.kind === "line" ? plots[plotKey("line", plot.index)] : undefined);
  return o?.label !== undefined && o.label !== plot.label ? undefined : o;
}

/**
 * Overrides re-keyed to the plots' current keys, recording the label on
 * index-keyed lines. Overrides that no longer apply are dropped. This
 * migrates legacy `line<i>` keys when the Style tab opens.
 */
export function rekeyPlots(
  plots: readonly PlotInfo[],
  style: IndicatorStyle | undefined,
): Record<string, PlotStyle> {
  const out: Record<string, PlotStyle> = {};
  for (const p of plots) {
    const o = plotOverride(p, style);
    if (o) {
      out[p.key] = p.kind === "line" && p.indexKeyed ? { ...o, label: p.label } : o;
    }
  }
  return out;
}

/**
 * Overrides keyed by figure key (`line0`, `hist1`) for the current payload,
 * with labels already matched. This is what the chart hands klinecharts.
 */
export function figureOverrides(
  plots: readonly PlotInfo[],
  style: IndicatorStyle | undefined,
): IndicatorStyle {
  const out: Record<string, PlotStyle> = {};
  for (const p of plots) {
    const o = plotOverride(p, style);
    if (o) out[plotKey(p.kind, p.index)] = o;
  }
  return { plots: out };
}

/** An override over the defaults. */
export function resolveOverride(
  o: PlotStyle | undefined,
  defaultColor: string,
  defaultWidth = DEFAULT_WIDTH,
): ResolvedPlot {
  return {
    color: o?.color ?? defaultColor,
    width: o?.width ?? defaultWidth,
    lineStyle: o?.lineStyle ?? "solid",
    visible: o?.visible ?? true,
  };
}

/** Effective style of a plot: the user's overrides over the defaults. */
export function resolvePlot(
  plot: PlotInfo,
  style: IndicatorStyle | undefined,
): ResolvedPlot {
  return resolveOverride(plotOverride(plot, style), plot.defaultColor, plot.defaultWidth);
}

/** Server color of bar `i` of a histogram, if the payload has one. */
export function barColor(
  h: PayloadHistogram | undefined,
  i: number,
): string | undefined {
  const k = h?.color_idx?.[i];
  return k == null ? undefined : h?.palette?.[k];
}

/**
 * Color of a plot at bar `at`: the user's color, else (histograms) the
 * server's color of that bar, else the plot's default.
 */
export function plotColorAt(
  plot: PlotInfo,
  ind: PayloadPlots,
  style: IndicatorStyle | undefined,
  at: number,
): string {
  const own = plotOverride(plot, style)?.color;
  if (own !== undefined) return own;
  const bar = plot.kind === "hist" ? barColor(ind.histogram?.[plot.index], at) : undefined;
  return bar ?? plot.defaultColor;
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
