// Geometry colors of an indicator (fills, hlines, volume-profile bars,
// divergence lines): what the Style tab lists and how the user's overrides
// (GeometryStyle) apply over the server / theme colors at render time.

import {
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
  fills: Array<{ label: string; up: string; down: string; twoTone: boolean }>;
  hlines: Array<{ label: string; color: string }>;
  hbars: { color: string; pocColor: string } | null;
  divlines: { bull: string; bear: string } | null;
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
    const other = tiers.findIndex((t) => t !== "poc");
    hbarInfo = {
      color: colors[other >= 0 ? other : 0]!,
      pocColor: colors[poc >= 0 ? poc : 0]!,
    };
  }
  return {
    fills: (ind.fills ?? []).map((f, i) => {
      const t = fillColors(f, name, i);
      return { label: `Fill ${i + 1}`, up: t.up, down: t.down, twoTone: t.up !== t.down };
    }),
    hlines: (ind.hlines ?? []).map((h) => ({
      label: fmtLevel(h.y),
      color: wireColor(h.color) ?? HLINE_FALLBACK,
    })),
    hbars: hbarInfo,
    divlines:
      (ind.divlines ?? []).length > 0
        ? { bull: DIVLINE_COLORS.bull, bear: DIVLINE_COLORS.bear }
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

/** Visibility of a geometry item; unset means shown. */
export const shown = (o: { visible?: boolean } | undefined): boolean =>
  o?.visible !== false;
