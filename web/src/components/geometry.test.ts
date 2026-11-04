import { describe, expect, it } from "vitest";

import {
  DIVLINE_COLORS,
  divergenceKind,
  divlineSegments,
  fillColors,
  fillPolygons,
  fillTheme,
  hbarColors,
  hbarRects,
  hbarTheme,
  hbarTiers,
  hlineStrokes,
  payloadToDataIndex,
  toDataSpace,
  valueExtents,
  wireColor,
  type Mapper,
  type SeriesDivLine,
  type SeriesHBar,
  type SeriesIndicator,
} from "./geometry";

// Identity-ish mapping: 10px per bar, y flipped around 100 like a canvas.
const map: Mapper = { x: (i) => i * 10, y: (v) => 100 - v };
const all = { from: 0, to: 1000 };

describe("fillPolygons", () => {
  it("builds one closed polygon for a contiguous band", () => {
    const polys = fillPolygons([3, 4, 5], [1, 1, 1], all, map);
    expect(polys).toHaveLength(1);
    expect(polys[0]!.up).toBe(true);
    expect(polys[0]!.points).toEqual([
      { x: 0, y: 97 },
      { x: 10, y: 96 },
      { x: 20, y: 95 },
      { x: 20, y: 99 },
      { x: 10, y: 99 },
      { x: 0, y: 99 },
    ]);
  });

  it("splits at null gaps (pre-warmup and mid-series)", () => {
    const polys = fillPolygons(
      [null, 3, 3, null, 3, 3, 3],
      [null, 1, 1, 1, 1, null, 1],
      all,
      map,
    );
    // [1,2] is a run; 3 is null on y1; [4] alone; 5 null on y2; [6] alone.
    expect(polys).toHaveLength(1);
    expect(polys[0]!.points.map((p) => p.x)).toEqual([10, 20, 20, 10]);
  });

  it("returns nothing for all-null input", () => {
    expect(fillPolygons([null, null], [null, null], all, map)).toEqual([]);
  });

  it("returns nothing for a single finite point", () => {
    expect(fillPolygons([null, 2, null], [null, 1, null], all, map)).toEqual([]);
  });

  it("returns nothing for empty or mismatched-length arrays", () => {
    expect(fillPolygons([], [], all, map)).toEqual([]);
    expect(fillPolygons([1, 2, 3], [0], all, map)).toEqual([]);
  });

  it("splits at a crossing and interpolates the crossing point", () => {
    // d = y1 - y2: +2, -2 → crossing halfway between bar 0 and bar 1.
    const polys = fillPolygons([2, 0], [0, 2], all, map);
    expect(polys).toHaveLength(2);
    expect(polys[0]!.up).toBe(true);
    expect(polys[1]!.up).toBe(false);
    const cross = { x: 5, y: 99 };
    expect(polys[0]!.points).toEqual([
      { x: 0, y: 98 },
      cross,
      cross,
      { x: 0, y: 100 },
    ]);
    expect(polys[1]!.points).toEqual([
      cross,
      { x: 10, y: 100 },
      { x: 10, y: 98 },
      cross,
    ]);
  });

  it("does not split when the lines merely touch", () => {
    const polys = fillPolygons([2, 1, 2], [1, 1, 1], all, map);
    expect(polys).toHaveLength(1);
    expect(polys[0]!.up).toBe(true);
  });

  it("splits when the sign flips through a touching bar", () => {
    const polys = fillPolygons([2, 1, 0], [1, 1, 1], all, map);
    expect(polys.map((p) => p.up)).toEqual([true, false]);
  });

  it("takes the orientation of the first non-zero difference", () => {
    const polys = fillPolygons([1, 1, 0], [1, 1, 1], all, map);
    expect(polys).toHaveLength(1);
    expect(polys[0]!.up).toBe(false);
  });

  it("clips to the visible range plus one bar of overhang", () => {
    const y1 = Array.from({ length: 100 }, () => 2);
    const y2 = Array.from({ length: 100 }, () => 1);
    const polys = fillPolygons(y1, y2, { from: 40, to: 50 }, map);
    expect(polys).toHaveLength(1);
    const xs = polys[0]!.points.map((p) => p.x);
    expect(Math.min(...xs)).toBe(390);
    expect(Math.max(...xs)).toBe(500);
  });

  it("does not overhang past the data bounds", () => {
    const polys = fillPolygons([2, 2, 2], [1, 1, 1], { from: 0, to: 3 }, map);
    const xs = polys[0]!.points.map((p) => p.x);
    expect(Math.min(...xs)).toBe(0);
    expect(Math.max(...xs)).toBe(20);
  });

  it("returns nothing when the visible range is past the data", () => {
    expect(fillPolygons([2, 2], [1, 1], { from: 10, to: 20 }, map)).toEqual([]);
  });
});

const hbar = (o: Partial<SeriesHBar>): SeriesHBar => ({
  y: 50,
  height: 10,
  width: 0.25,
  offset: 0,
  left: false,
  ...o,
});

describe("hbarRects", () => {
  it("anchors right-side bars to the right edge, width ∝ fraction", () => {
    const [r] = hbarRects([hbar({ width: 0.25 })], 400, 100, map.y);
    expect(r).toEqual({ x: 300, y: 45, width: 100, height: 10, index: 0 });
  });

  it("anchors left-side bars to the left edge", () => {
    const [r] = hbarRects([hbar({ left: true, width: 0.1 })], 400, 100, map.y);
    expect(r!.x).toBe(0);
    expect(r!.width).toBe(40);
  });

  it("stacks offset bars away from the anchored edge (up/down split)", () => {
    const rects = hbarRects(
      [hbar({ width: 0.1 }), hbar({ width: 0.05, offset: 0.1 })],
      400,
      100,
      map.y,
    );
    expect(rects.map((r) => [r.x, r.width])).toEqual([
      [360, 40],
      [340, 20],
    ]);
    const left = hbarRects(
      [hbar({ left: true, width: 0.05, offset: 0.1 })],
      400,
      100,
      map.y,
    );
    expect(left[0]!.x).toBe(40);
  });

  it("scales with pane size and price mapping", () => {
    const zoomed = (v: number) => 200 - 2 * v;
    const [r] = hbarRects([hbar({ width: 0.5 })], 800, 200, zoomed);
    expect(r).toEqual({ x: 400, y: 90, width: 400, height: 20, index: 0 });
  });

  it("drops bars entirely outside the pane but keeps partial ones", () => {
    const rects = hbarRects(
      [hbar({ y: 500 }), hbar({ y: -500 }), hbar({ y: 98 }), hbar({ y: 50 })],
      400,
      100,
      map.y,
    );
    expect(rects.map((r) => r.index)).toEqual([2, 3]);
  });
});

describe("hbarTiers", () => {
  it("marks the widest bar as POC and bars within VAL..VAH as value area", () => {
    const bars = [
      hbar({ y: 10, width: 0.05 }),
      hbar({ y: 20, width: 0.2 }),
      hbar({ y: 30, width: 0.25 }),
      hbar({ y: 40, width: 0.1 }),
      hbar({ y: 50, width: 0.02 }),
    ];
    // POC, VAH, VAL order as emitted by volume_profile / tpo.
    const tiers = hbarTiers(bars, [{ y: 30 }, { y: 45 }, { y: 15 }]);
    expect(tiers).toEqual(["out", "va", "poc", "va", "out"]);
  });

  it("falls back to POC-only without value-area hlines", () => {
    expect(hbarTiers([hbar({ width: 0.1 }), hbar({ width: 0.2 })], [])).toEqual([
      "out",
      "poc",
    ]);
  });

  it("handles no bars", () => {
    expect(hbarTiers([], [])).toEqual([]);
  });
});

const div = (o: Partial<SeriesDivLine>): SeriesDivLine => ({
  x1: 10,
  t1: null,
  y1: 50,
  x2: 20,
  t2: null,
  y2: 40,
  dashed: true,
  ...o,
});

describe("divergenceKind", () => {
  it("oscillator pane: falling = bearish (lower highs), rising = bullish", () => {
    expect(divergenceKind(div({ y1: 50, y2: 40 }), false)).toBe("bear");
    expect(divergenceKind(div({ y1: -50, y2: -40 }), false)).toBe("bull");
  });

  it("price pane: rising = bearish (higher highs), falling = bullish", () => {
    expect(divergenceKind(div({ y1: 100, y2: 110 }), true)).toBe("bear");
    expect(divergenceKind(div({ y1: 100, y2: 90 }), true)).toBe("bull");
  });

  it("normalizes endpoint order and reports flat lines", () => {
    expect(divergenceKind(div({ x1: 20, y1: 40, x2: 10, y2: 50 }), false)).toBe(
      "bear",
    );
    expect(divergenceKind(div({ y1: 5, y2: 5 }), false)).toBe("flat");
  });
});

describe("divlineSegments", () => {
  it("maps endpoints to pixels and keeps the dashed flag", () => {
    const [s] = divlineSegments([div({})], all, map, false);
    expect(s).toEqual({
      from: { x: 100, y: 50 },
      to: { x: 200, y: 60 },
      dashed: true,
      kind: "bear",
      color: DIVLINE_COLORS.bear,
    });
  });

  it("keeps segments overlapping the visible range, drops others", () => {
    const lines = [
      div({ x1: 0, x2: 5 }), // fully left
      div({ x1: 5, x2: 15 }), // straddles from
      div({ x1: 12, x2: 18 }), // inside
      div({ x1: 18, x2: 30 }), // straddles to
      div({ x1: 40, x2: 50 }), // fully right
    ];
    const segs = divlineSegments(lines, { from: 10, to: 20 }, map, false);
    expect(segs.map((s) => s.from.x)).toEqual([50, 120, 180]);
  });

  it("returns nothing for no divlines", () => {
    expect(divlineSegments([], all, map, false)).toEqual([]);
  });
});

describe("fillTheme", () => {
  it("gives the ichimoku cloud distinct up/down colors", () => {
    const t = fillTheme("ichimoku", 0);
    expect(t.up).not.toBe(t.down);
  });

  it("uses per-index colors for cipher_b and falls back past the table", () => {
    expect(fillTheme("cipher_b", 0)).not.toEqual(fillTheme("cipher_b", 1));
    expect(fillTheme("cipher_b", 9)).toEqual(fillTheme("bbands", 0));
  });

  it("colors the Kalman bands and zones cyan/red in fill order", () => {
    const k = [0, 1, 2, 3].map((i) => fillTheme("kalman", i).up);
    expect(k).toEqual([
      "rgba(0, 188, 212, 0.15)",
      "rgba(255, 23, 68, 0.15)",
      "rgba(255, 23, 68, 0.10)",
      "rgba(0, 188, 212, 0.10)",
    ]);
    expect(fillTheme("kvf", 0)).toEqual(fillTheme("kalman_volume", 0));
  });

  it("uses a single band color for generic channels", () => {
    const t = fillTheme("bbands", 0);
    expect(t.up).toBe(t.down);
  });
});

describe("hbarTheme", () => {
  it("colors TPO apart from the volume profile but keeps the gold POC", () => {
    expect(hbarTheme("tpo").va).not.toBe(hbarTheme("volume_profile").va);
    expect(hbarTheme("tpo").poc).toBe(hbarTheme("vpvr").poc);
  });
});

describe("payloadToDataIndex", () => {
  it("is the identity when every bar has a timestamp", () => {
    expect(payloadToDataIndex([{ t: 1 }, { t: 2 }, { t: 3 }])).toEqual([
      0, 1, 2,
    ]);
  });

  it("maps dropped (t === null) bars to -1 and shifts later bars left", () => {
    expect(
      payloadToDataIndex([{ t: null }, { t: 1 }, { t: null }, { t: 2 }]),
    ).toEqual([-1, 0, -1, 1]);
  });

  it("handles empty and all-null input", () => {
    expect(payloadToDataIndex([])).toEqual([]);
    expect(payloadToDataIndex([{ t: null }])).toEqual([-1]);
  });
});

const indicator = (o: Partial<SeriesIndicator>): SeriesIndicator => ({
  label: "X",
  is_overlay: false,
  ...o,
});

describe("toDataSpace", () => {
  const remap = [-1, 0, -1, 1, 2];

  it("compacts fill arrays to the surviving bars", () => {
    const out = toDataSpace(
      indicator({ fills: [{ y1: [9, 1, 9, 2, 3], y2: [9, 0, 9, null, 0] }] }),
      remap,
    );
    expect(out.fills).toEqual([{ y1: [1, 2, 3], y2: [0, null, 0] }]);
  });

  it("remaps divline endpoints, dropping ones on dropped or missing bars", () => {
    const out = toDataSpace(
      indicator({
        divlines: [
          div({ x1: 1, x2: 4 }),
          div({ x1: 0, x2: 3 }), // starts on a dropped bar
          div({ x1: 3, x2: 99 }), // past the payload
        ],
      }),
      remap,
    );
    expect(out.divlines!.map((d) => [d.x1, d.x2])).toEqual([[0, 2]]);
  });

  it("passes hbars and metadata through, keeps absent channels absent", () => {
    const hbars = [hbar({})];
    const out = toDataSpace(indicator({ hbars, y_range: [0, 1] }), remap);
    expect(out.hbars).toEqual([expect.objectContaining(hbars[0]!)]);
    expect(out.y_range).toEqual([0, 1]);
    expect(out.fills).toBeUndefined();
    expect(out.divlines).toBeUndefined();
  });

  it("shares fills and divlines when the payload is already in data space", () => {
    const ind = indicator({
      fills: [{ y1: [1, 2], y2: [0, 0] }],
      divlines: [div({ x1: 0, x2: 1 })],
      hbars: [hbar({})],
    });
    const out = toDataSpace(ind, null, "vpvr");
    expect(out.fills).toBe(ind.fills);
    expect(out.divlines).toBe(ind.divlines);
    expect(out.hbars![0]!.color).toBe(hbarTheme("vpvr").poc);
  });
});

describe("valueExtents", () => {
  it("spans every fill edge per bar, skipping nulls", () => {
    const ext = valueExtents(
      indicator({
        fills: [
          { y1: [5, null, 2], y2: [1, null, null] },
          { y1: [7, 3, null], y2: [0, null, null] },
        ],
      }),
      3,
    );
    expect(ext).toEqual([
      { lo: 0, hi: 7 },
      { lo: 3, hi: 3 },
      { lo: 2, hi: 2 },
    ]);
  });

  it("includes y_range on every bar for oscillator panes", () => {
    const ext = valueExtents(
      indicator({ y_range: [-2.5, 2.5], fills: [{ y1: [3, 0], y2: [1, 0] }] }),
      2,
    );
    expect(ext).toEqual([
      { lo: -2.5, hi: 3 },
      { lo: -2.5, hi: 2.5 },
    ]);
  });

  it("gives a geometry-only pane an axis from y_range alone", () => {
    expect(valueExtents(indicator({ y_range: [0, 100] }), 2)).toEqual([
      { lo: 0, hi: 100 },
      { lo: 0, hi: 100 },
    ]);
  });

  it("ignores y_range on overlays and null y_range bounds", () => {
    expect(
      valueExtents(indicator({ is_overlay: true, y_range: [0, 1] }), 1),
    ).toEqual([{ lo: null, hi: null }]);
    expect(valueExtents(indicator({ y_range: [null, 5] }), 1)).toEqual([
      { lo: null, hi: 5 },
    ]);
  });
});

describe("wireColor", () => {
  it("accepts the server's rgba/rgb strings and rejects anything else", () => {
    expect(wireColor("rgba(10,20,30,0.25)")).toBe("rgba(10,20,30,0.25)");
    expect(wireColor("rgba(255,0,0,1)")).toBe("rgba(255,0,0,1)");
    expect(wireColor("rgb(1, 2, 3)")).toBe("rgb(1, 2, 3)");
    expect(wireColor("red")).toBeUndefined();
    expect(wireColor("rgba(1,2)")).toBeUndefined();
    expect(wireColor("")).toBeUndefined();
    expect(wireColor(undefined)).toBeUndefined();
  });
});

describe("wire colors vs theme fallback", () => {
  const fill = { y1: [1], y2: [0] };

  it("fillColors prefers the wire color, else the per-index theme", () => {
    expect(
      fillColors({ ...fill, color: "rgba(1,2,3,0.5)" }, "cipher_b", 1),
    ).toEqual({ up: "rgba(1,2,3,0.5)", down: "rgba(1,2,3,0.5)" });
    expect(fillColors(fill, "cipher_b", 1)).toEqual(fillTheme("cipher_b", 1));
    expect(fillColors({ ...fill, color: "bogus" }, "cipher_b", 1)).toEqual(
      fillTheme("cipher_b", 1),
    );
  });

  it("fillColors uses color_below for the y1 < y2 orientation", () => {
    const f = { ...fill, color: "rgba(1,2,3,0.2)", color_below: "rgba(4,5,6,0.2)" };
    expect(fillColors(f, "ichimoku", 0)).toEqual({
      up: "rgba(1,2,3,0.2)",
      down: "rgba(4,5,6,0.2)",
    });
    expect(fillColors({ ...f, color_below: "nope" }, "ichimoku", 0).down).toBe(
      "rgba(1,2,3,0.2)",
    );
  });

  it("toDataSpace keeps color_below", () => {
    const kept = toDataSpace(
      indicator({
        fills: [{ ...fill, color: "rgba(1,2,3,0.5)", color_below: "rgba(4,5,6,0.5)" }],
      }),
      [0],
    );
    expect(kept.fills![0]!.color_below).toBe("rgba(4,5,6,0.5)");
  });

  it("toDataSpace keeps the fill color", () => {
    const out = toDataSpace(
      indicator({ fills: [{ ...fill, color: "rgba(1,2,3,0.5)" }] }),
      [0],
    );
    expect(out.fills![0]!.color).toBe("rgba(1,2,3,0.5)");
  });

  it("hbarColors uses wire colors, else POC/value-area tiers", () => {
    const bars = [
      { y: 10, height: 1, width: 0.5, offset: 0, left: true, color: "rgba(9,9,9,1)" },
      { y: 20, height: 1, width: 0.2, offset: 0, left: true },
    ];
    const out = hbarColors(bars, [], "vpvr");
    expect(out[0]).toBe("rgba(9,9,9,1)");
    expect(out[1]).toBe(hbarTheme("vpvr").out);
  });

  it("divlineSegments colors by wire, else by slope", () => {
    const d = { x1: 0, t1: null, y1: 1, x2: 5, t2: null, y2: 9, dashed: true };
    const wired = divlineSegments([{ ...d, color: "rgba(1,2,3,1)" }], all, map, false);
    expect(wired[0]!.color).toBe("rgba(1,2,3,1)");
    expect(divlineSegments([d], all, map, false)[0]!.color).toBe(
      DIVLINE_COLORS.bull,
    );
  });

  it("hlineStrokes maps levels, clips to the pane and falls back to gray", () => {
    const out = hlineStrokes(
      [{ y: 30, color: "rgba(1,2,3,0.3)" }, { y: 70 }, { y: 500 }],
      100,
      (v) => 100 - v,
    );
    expect(out).toEqual([
      { y: 70, color: "rgba(1,2,3,0.3)" },
      { y: 30, color: "rgba(120, 123, 134, 0.80)" },
    ]);
  });

  it("valueExtents ignores hline levels, as the PNG auto-range does", () => {
    const hlines = [{ y: 0 }, { y: 70 }];
    expect(valueExtents(indicator({ hlines }), 1)).toEqual([{ lo: null, hi: null }]);
    expect(valueExtents(indicator({ hlines, y_range: [0, 100] }), 1)).toEqual([
      { lo: 0, hi: 100 },
    ]);
  });

  it("toDataSpace resolves hbar colors once (wire color, else tier)", () => {
    const hb = { y: 1, height: 1, width: 0.5, offset: 0, left: true };
    const out = toDataSpace(
      indicator({ hbars: [{ ...hb, color: "rgba(9,9,9,1)" }, { ...hb, width: 0.1 }] }),
      [],
      "vpvr",
    );
    expect(out.hbars![0]!.color).toBe("rgba(9,9,9,1)");
    expect(out.hbars![1]!.color).toBe(hbarTheme("vpvr").out);
  });
});
