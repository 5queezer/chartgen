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
  labelBoxes,
  markAt,
  patternDrawings,
  patternText,
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

  it("themes a repeated instance like its indicator (ADR-0028)", () => {
    const bar = (y: number, width: number) => ({ y, height: 1, width, offset: 0, left: true });
    const bars = [bar(1, 1), bar(2, 0.2)];
    expect(fillColors(fill, "ichimoku#2", 0)).toEqual(fillTheme("ichimoku", 0));
    expect(hbarColors(bars, [], "tpo#2")).toEqual(hbarColors(bars, [], "tpo"));
    expect(hbarColors(bars, [], "tpo#2")).not.toEqual(hbarColors(bars, [], "vpvr"));
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
      { y: 70, color: "rgba(1,2,3,0.3)", index: 0 },
      { y: 30, color: "rgba(120, 123, 134, 0.80)", index: 1 },
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

describe("pattern marks (ADR-0046)", () => {
  const map = { x: (i: number) => i * 10, y: (v: number) => 1000 - v };
  const range = { from: 0, to: 50 };
  const measure = (t: string) => t.length * 6;
  const label = (x: number, above: boolean, text = "H") => ({
    x,
    t: x,
    y: 100,
    above,
    text,
    key: "k",
    name: "N",
    bias: above ? ("bearish" as const) : ("bullish" as const),
    tooltip: `tip ${text}`,
  });

  it("a label sits past the bar's high or low, centered on the bar", () => {
    const [up, down] = labelBoxes([label(5, true), label(5, false)], range, map, measure);
    expect(up!.y + up!.height).toBeLessThan(map.y(100));
    expect(down!.y).toBeGreaterThan(map.y(100));
    expect(up!.x + up!.width / 2).toBe(50);
    expect(up!.color).toBe("rgba(242,54,69,1)");
    expect(down!.color).toBe("rgba(41,98,255,1)");
  });

  it("labels of one bar and side stack instead of covering each other", () => {
    const [a, b, c] = labelBoxes(
      [label(5, true, "A"), label(5, true, "B"), label(6, true, "C")],
      range,
      map,
      measure,
    );
    expect(b!.y + b!.height).toBeLessThanOrEqual(a!.y);
    expect(c!.y).toBe(a!.y);
  });

  it("only labels on visible bars are boxed", () => {
    const got = labelBoxes([label(200, true)], range, map, measure);
    expect(got).toEqual([]);
  });

  it("the forming bar's label is hollow and faint, a closed one is not", () => {
    const [closed, forming] = labelBoxes(
      [label(5, true), { ...label(6, true), tentative: true }],
      range,
      map,
      measure,
    );
    expect(closed!.hollow).toBe(false);
    expect(forming!.hollow).toBe(true);
    expect(forming!.faint).toBe(true);
  });

  const pattern = (status: "forming" | "confirmed" | "failed", a = 2, b = 8) => ({
    key: "double_top",
    name: "Double Top",
    bias: "bearish" as const,
    status,
    strokes: [{ x1: a, t1: a, y1: 100, x2: b, t2: b, y2: 110, dashed: true }],
    label: { x: 5, t: 5, y: 110, above: true },
    tooltip: "tip",
  });

  it("a pattern's label names its status", () => {
    expect(patternText(pattern("forming"))).toBe("Double Top · forming");
    expect(patternText(pattern("failed"))).toBe("Double Top · failed");
    const [d] = patternDrawings([pattern("forming")], range, map, measure);
    expect(d!.box.text).toBe("Double Top · forming");
    expect(d!.box.faint).toBe(true);
    expect(d!.strokes[0]).toEqual({
      from: { x: 20, y: 900 },
      to: { x: 80, y: 890 },
      dashed: true,
    });
    const [ok] = patternDrawings([pattern("confirmed")], range, map, measure);
    expect(ok!.box.faint).toBe(false);
  });

  it("a pattern off screen is skipped, one reaching into view is not", () => {
    expect(patternDrawings([pattern("forming", -90, -80)], range, map, measure)).toEqual([]);
    expect(patternDrawings([pattern("forming", -90, 3)], range, map, measure)).toHaveLength(1);
  });

  it("the topmost box under a point wins, and nothing outside any", () => {
    const boxes = labelBoxes([label(5, true, "A"), label(5, false, "B")], range, map, measure);
    const b = boxes[1]!;
    expect(markAt(boxes, b.x + 1, b.y + 1)?.text).toBe("B");
    expect(markAt(boxes, b.x - 50, b.y)).toBeNull();
    const twin = [{ ...b, text: "under" }, { ...b, text: "over" }];
    expect(markAt(twin, b.x + 1, b.y + 1)?.text).toBe("over");
  });

  it("untimed bars remap labels and strokes, and drop what lost its bar", () => {
    const remap = [0, -1, 1, 2];
    const out = toDataSpace(
      {
        label: "P",
        is_overlay: true,
        labels: [label(2, true), label(1, true)],
        patterns: [
          {
            ...pattern("forming"),
            strokes: [
              { x1: 0, t1: 0, y1: 1, x2: 3, t2: 3, y2: 2, dashed: false },
              { x1: 1, t1: 1, y1: 1, x2: 3, t2: 3, y2: 2, dashed: false },
            ],
          },
        ],
      },
      remap,
    );
    expect(out.labels!.map((l) => l.x)).toEqual([1]);
    expect(out.patterns![0]!.strokes).toHaveLength(1);
    expect(out.patterns![0]!.strokes[0]).toMatchObject({ x1: 0, x2: 2 });
  });
});
