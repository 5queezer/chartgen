// Geometry colors of an indicator (fills, hlines, volume-profile bars,
// divergence lines): what the Style tab lists and how the user's overrides
// (GeometryStyle) apply over the server / theme colors at render time.

import {
  BIAS_COLORS,
  DIVLINE_COLORS,
  fillColors,
  hbarColors,
  hbarTiers,
  wireColor,
  type DivergenceKind,
  type FillTheme,
  type SeriesIndicator,
} from "../components/geometry";
import type { GeoItemStyle, GeometryStyle } from "../stores/indicators";

const HLINE_FALLBACK = "rgba(120, 123, 134, 0.80)";

/** A geometry group the Style tab can edit, with its default colors. */
export interface GeometryInfo {
  fills: Array<{
    label: string;
    up: string;
    down: string;
    twoTone: boolean;
    /** The wire color, which guards the override (see `GeoItemStyle.ref`). */
    ref?: string;
  }>;
  hlines: Array<{ label: string; color: string; y: number; ref?: string }>;
  /** `va` is null when the indicator has no value area. */
  hbars: { color: string; va: string | null; pocColor: string } | null;
  divlines: { bull: string; bear: string } | null;
  /** Pattern labels and outlines (ADR-0046). */
  marks: { bullish: string; bearish: string; neutral: string } | null;
}

const fmtLevel = (y: number) => String(Math.round(y * 100) / 100);

/** The geometry channels of a payload indicator with their default colors. */
export function geometryOf(name: string, ind: SeriesIndicator): GeometryInfo {
  const hbars = ind.hbars ?? [];
  let hbarInfo: GeometryInfo["hbars"] = null;
  if (hbars.length > 0) {
    const tiers = hbarTiers(hbars, ind.hlines ?? []);
    const colors = hbarColors(hbars, ind.hlines ?? [], name);
    const poc = tiers.indexOf("poc");
    const out = tiers.indexOf("out");
    const va = tiers.indexOf("va");
    hbarInfo = {
      color: colors[out >= 0 ? out : 0]!,
      va: va >= 0 ? colors[va]! : null,
      pocColor: colors[poc >= 0 ? poc : 0]!,
    };
  }
  return {
    fills: (ind.fills ?? []).map((f, i) => {
      const t = fillColors(f, name, i);
      return {
        label: `Fill ${i + 1}`,
        up: t.up,
        down: t.down,
        twoTone: t.up !== t.down,
        ref: f.color,
      };
    }),
    hlines: (ind.hlines ?? []).map((h) => ({
      label: fmtLevel(h.y),
      color: wireColor(h.color) ?? HLINE_FALLBACK,
      y: h.y,
      ref: h.color,
    })),
    hbars: hbarInfo,
    divlines:
      (ind.divlines ?? []).length > 0
        ? { bull: DIVLINE_COLORS.bull, bear: DIVLINE_COLORS.bear }
        : null,
    marks:
      (ind.labels?.length ?? 0) > 0 ||
      (ind.events?.length ?? 0) > 0 ||
      (ind.patterns?.length ?? 0) > 0
        ? BIAS_COLORS
        : null,
  };
}

/** Theme / server fill colors with the user's override of the fill on top. */
export function applyFill(theme: FillTheme, o: GeoItemStyle | undefined): FillTheme {
  return o ? { up: o.color ?? theme.up, down: o.colorBelow ?? theme.down } : theme;
}

/** Color of a divergence line of `kind`: the user's, else `fallback`. */
export function divColor(
  kind: DivergenceKind,
  fallback: string,
  g: GeometryStyle["divlines"],
): string {
  if (kind === "bull") return g?.bullColor ?? fallback;
  if (kind === "bear") return g?.bearColor ?? fallback;
  return fallback;
}

/** Color of a pattern mark of `bias`: the user's, else `fallback`. */
export function markColor(
  bias: "bullish" | "bearish" | "neutral",
  fallback: string,
  g: GeometryStyle["marks"],
): string {
  const own = bias === "bullish" ? g?.bullColor : bias === "bearish" ? g?.bearColor : g?.neutralColor;
  return own ?? fallback;
}

/** Visibility of a geometry item; unset means shown. */
export const shown = (o: { visible?: boolean } | undefined): boolean =>
  o?.visible !== false;

/** The fill override if it was saved for this fill (same wire color), else undefined. */
export function matchFill(
  o: GeoItemStyle | undefined,
  fill: { color?: string },
): GeoItemStyle | undefined {
  return o && (o.ref === undefined || o.ref === fill.color) ? o : undefined;
}

/**
 * The level override if it was saved for this level: same wire color and,
 * outside volume profiles (whose levels move with every payload), same `y`.
 */
export function matchLevel(
  o: GeoItemStyle | undefined,
  hline: { y: number; color?: string },
  profile: boolean,
): GeoItemStyle | undefined {
  if (!o || (o.ref !== undefined && o.ref !== hline.color)) return undefined;
  if (!profile && o.y !== undefined && Math.abs(o.y - hline.y) > 1e-9 * Math.max(1, Math.abs(hline.y))) {
    return undefined;
  }
  return o;
}

/** The indicator without the fills the user hid, for axis scaling. */
export function visibleFills(
  ind: SeriesIndicator,
  overrides: Record<string, GeoItemStyle> | undefined,
): SeriesIndicator {
  if (!overrides) return ind;
  const on = (i: number) => shown(matchFill(overrides[i], ind.fills?.[i] ?? {}));
  return {
    ...ind,
    fills: ind.fills?.filter((_, i) => on(i)),
    ...(ind.future ? { future: { ...ind.future, fills: ind.future.fills.filter((f) => on(f.index)) } } : {}),
  };
}

/**
 * The geometry overrides that still apply to the payload `info` was built
 * from: entries for fills / levels / groups that no longer exist or whose
 * guard differs are dropped.
 */
export function pruneGeometry(
  g: GeometryStyle,
  info: GeometryInfo,
): GeometryStyle {
  const keep = (
    m: Record<string, GeoItemStyle> | undefined,
    ok: (o: GeoItemStyle, i: number) => boolean,
  ) => {
    const out = Object.fromEntries(Object.entries(m ?? {}).filter(([k, o]) => ok(o, Number(k))));
    return Object.keys(out).length > 0 ? out : undefined;
  };
  const fills = keep(g.fills, (o, i) => !!info.fills[i] && !!matchFill(o, { color: info.fills[i]!.ref }));
  const hlines = keep(
    g.hlines,
    (o, i) => {
      const h = info.hlines[i];
      return !!h && !!matchLevel(o, { y: h.y, color: h.ref }, info.hbars !== null);
    },
  );
  return {
    ...(fills ? { fills } : {}),
    ...(hlines ? { hlines } : {}),
    ...(g.hbars && info.hbars ? { hbars: g.hbars } : {}),
    ...(g.divlines && info.divlines ? { divlines: g.divlines } : {}),
    ...(g.marks && info.marks ? { marks: g.marks } : {}),
  };
}
