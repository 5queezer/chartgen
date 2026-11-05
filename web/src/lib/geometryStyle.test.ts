import { describe, expect, it } from "vitest";

import { sanitizeStyle } from "../stores/indicators";
import type { SeriesIndicator } from "../components/geometry";
import {
  applyFill,
  divColor,
  geometryOf,
  matchFill,
  matchLevel,
  pruneGeometry,
  shown,
  visibleFills,
} from "./geometryStyle";
import { figureOverrides, paletteBarColor, plotColorAt, plotsOf } from "./plotStyle";

const colorAt = (n: number) => `#00000${n}`;

describe("geometry style sanitizing", () => {
  it("keeps valid overrides and drops invalid ones", () => {
    const s = sanitizeStyle({
      plots: {},
      geometry: {
        fills: { "0": { color: "#ff0000", visible: false }, x: { color: "#fff" }, "1": { color: "nope" } },
        hlines: { "1": { color: "rgba(1, 2, 3, 0.5)", lineStyle: "dashed", visible: true, bogus: 1 } },
        hbars: { color: "#00ff00", pocColor: 5 },
        divlines: { bullColor: "#111111", visible: false },
        extra: {},
      },
    });
    expect(s).toEqual({
      plots: {},
      geometry: {
        fills: { "0": { color: "#ff0000", visible: false } },
        hlines: { "1": { color: "rgba(1, 2, 3, 0.5)", lineStyle: "dashed", visible: true } },
        hbars: { color: "#00ff00" },
        divlines: { bullColor: "#111111", visible: false },
      },
    });
  });

  it("loads a style stored before geometry existed unchanged", () => {
    const old = { plots: { "l:RSI": { color: "#ff0000", width: 2 } } };
    expect(sanitizeStyle(old)).toEqual(old);
  });

  it("keeps a geometry-only style and drops an empty one", () => {
    expect(sanitizeStyle({ geometry: { fills: { "0": { visible: false } } } })).toEqual({
      plots: {},
      geometry: { fills: { "0": { visible: false } } },
    });
    expect(sanitizeStyle({ plots: {}, geometry: { fills: {}, hbars: {} } })).toBeUndefined();
  });

  it("validates histogram palettes", () => {
    const pal = { "rgba(38, 166, 154, 0.5)": "#ff0000" };
    expect(sanitizeStyle({ plots: { hist0: { palette: pal } } })?.plots.hist0?.palette).toEqual(pal);
    expect(sanitizeStyle({ plots: { hist0: { palette: { x: "#ff0000" } } } })).toBeUndefined();
    expect(sanitizeStyle({ plots: { hist0: { palette: ["#ff0000"] } } })).toBeUndefined();
    expect(sanitizeStyle({ plots: { hist0: { palette: { "#00ff00": "bad" } } } })).toBeUndefined();
  });
});

describe("override precedence", () => {
  it("a fill override beats the server / theme color, per side", () => {
    const theme = { up: "rgba(1,1,1,0.2)", down: "rgba(2,2,2,0.2)" };
    expect(applyFill(theme, undefined)).toBe(theme);
    expect(applyFill(theme, { color: "#ff0000" })).toEqual({ up: "#ff0000", down: theme.down });
    expect(applyFill(theme, { colorBelow: "#00ff00" })).toEqual({ up: theme.up, down: "#00ff00" });
  });

  it("divergence colors override per kind, flat keeps the server color", () => {
    const g = { bullColor: "#111111", bearColor: "#222222" };
    expect(divColor("bull", "srv", g)).toBe("#111111");
    expect(divColor("bear", "srv", g)).toBe("#222222");
    expect(divColor("flat", "srv", g)).toBe("srv");
    expect(divColor("bull", "srv", undefined)).toBe("srv");
  });

  it("visibility defaults to shown", () => {
    expect(shown(undefined)).toBe(true);
    expect(shown({})).toBe(true);
    expect(shown({ visible: false })).toBe(false);
  });

  it("a histogram palette recolors bars by their server color; a plot color beats it", () => {
    // Keyed by server color: an order swap of the server palette changes nothing.
    const server = ["#26a69a", "#ef5350"];
    const user = { "#26a69a": "#111111" };
    expect(paletteBarColor("#26a69a", user)).toBe("#111111");
    expect(paletteBarColor("#ef5350", user)).toBe("#ef5350");
    expect(paletteBarColor("#ef5350", undefined)).toBe("#ef5350");

    const ind = { histogram: [{ palette: server, color_idx: [0, 1] }] };
    const [plot] = plotsOf(ind, colorAt);
    const style = { plots: { hist0: { palette: { "#ef5350": "#222222" } } } };
    expect(plotColorAt(plot!, ind, style, 1)).toBe("#222222");
    expect(plotColorAt(plot!, ind, style, 0)).toBe("#26a69a");
    expect(plotColorAt(plot!, ind, undefined, 1)).toBe("#ef5350");
    expect(plotColorAt(plot!, ind, { plots: { hist0: { ...style.plots.hist0, color: "#333333" } } }, 1)).toBe(
      "#333333",
    );
  });

  it("figureOverrides carries the geometry, so live ticks and theme switches keep it", () => {
    const style = { plots: {}, geometry: { fills: { "0": { color: "#ff0000" } } } };
    expect(figureOverrides([], style).geometry).toEqual(style.geometry);
    expect(figureOverrides([], undefined).geometry).toBeUndefined();
  });
});

describe("geometryOf", () => {
  const ind = {
    label: "RSI",
    is_overlay: false,
    fills: [{ y1: [1], y2: [2], color: "rgba(1, 2, 3, 0.1)" }],
    hlines: [{ y: 70, color: "rgba(4, 5, 6, 0.8)" }, { y: 30 }],
    divlines: [{ x1: 0, t1: null, y1: 1, x2: 1, t2: null, y2: 2, dashed: false }],
  } as unknown as SeriesIndicator;

  it("lists groups with their default colors and labels", () => {
    const g = geometryOf("rsi", ind);
    expect(g.fills).toEqual([
      { label: "Fill 1", up: "rgba(1, 2, 3, 0.1)", down: "rgba(1, 2, 3, 0.1)", twoTone: false, ref: "rgba(1, 2, 3, 0.1)" },
    ]);
    expect(g.hlines.map((h) => [h.label, h.color])).toEqual([
      ["70", "rgba(4, 5, 6, 0.8)"],
      ["30", "rgba(120, 123, 134, 0.80)"],
    ]);
    expect(g.hbars).toBeNull();
    expect(g.divlines).not.toBeNull();
  });
});

describe("override guards", () => {
  it("keeps a fill override only for the fill it was saved for", () => {
    const o = { color: "#ff0000", ref: "rgba(1, 2, 3, 0.1)" };
    expect(matchFill(o, { color: "rgba(1, 2, 3, 0.1)" })).toBe(o);
    expect(matchFill(o, { color: "rgba(9, 9, 9, 0.1)" })).toBeUndefined();
    expect(matchFill({ color: "#ff0000" }, { color: "anything" })).toBeDefined();
  });

  it("keeps a level override only for the same color and y, except in profiles", () => {
    const o = { color: "#ff0000", ref: "rgba(1, 2, 3, 0.1)", y: 70 };
    const h = { y: 70, color: "rgba(1, 2, 3, 0.1)" };
    expect(matchLevel(o, h, false)).toBe(o);
    expect(matchLevel(o, { ...h, y: 30 }, false)).toBeUndefined();
    expect(matchLevel(o, { ...h, y: 30 }, true)).toBe(o);
    expect(matchLevel(o, { ...h, color: "other" }, false)).toBeUndefined();
  });

  it("hidden fills do not scale the axis", () => {
    const ind = {
      is_overlay: true,
      fills: [
        { y1: [1], y2: [2], color: "a" },
        { y1: [50], y2: [60], color: "b" },
      ],
    } as unknown as SeriesIndicator;
    const out = visibleFills(ind, { "1": { visible: false, ref: "b" } });
    expect(out.fills).toHaveLength(1);
    // The override was saved for another fill: it is ignored, the fill counts.
    expect(visibleFills(ind, { "1": { visible: false, ref: "zzz" } }).fills).toHaveLength(2);
  });

  it("prunes overrides for missing or changed entries", () => {
    const info = geometryOf("rsi", {
      is_overlay: false,
      fills: [{ y1: [1], y2: [2], color: "rgba(1, 2, 3, 0.1)" }],
      hlines: [{ y: 70, color: "rgba(4, 5, 6, 0.8)" }],
    } as unknown as SeriesIndicator);
    const pruned = pruneGeometry(
      {
        fills: { "0": { color: "#f00", ref: "rgba(1, 2, 3, 0.1)" }, "3": { color: "#f00" } },
        hlines: { "0": { color: "#f00", y: 30 } },
        hbars: { color: "#f00" },
        divlines: { visible: false },
      },
      info,
    );
    expect(pruned).toEqual({ fills: { "0": { color: "#f00", ref: "rgba(1, 2, 3, 0.1)" } } });
  });
});
