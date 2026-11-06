import { describe, expect, it } from "vitest";

import { drawFootprint, drawTpo, type FootprintExtend, type TpoExtend } from "./orderFlowDraw";
import { defaultOrderFlow } from "../stores/orderFlow";
import type { FootprintBar, TpoProfile } from "../types/responses";

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
  color: string;
}

/** A context that records what is filled, with one pixel per unit of price. */
function recorder() {
  const rects: Rect[] = [];
  const ctx = {
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
    font: "",
    textAlign: "",
    textBaseline: "",
    setLineDash() {},
    beginPath() {},
    moveTo() {},
    lineTo() {},
    stroke() {},
    strokeRect() {},
    fillText() {},
    measureText: () => ({ width: 1 }),
    fillRect(x: number, y: number, w: number, h: number) {
      rects.push({ x, y, w, h, color: ctx.fillStyle });
    },
  };
  return { ctx, rects };
}

const bar = (i: number) => ({
  timestamp: i * 3_600_000,
  open: 100,
  high: 110,
  low: 90,
  close: 105,
  volume: 1,
});

const axes = {
  xAxis: { convertToPixel: (i: number) => i * 50 },
  yAxis: { convertToPixel: (v: number) => 1_000 - v },
};

const colors = { up: "#00ff00", down: "#ff0000", text: "#fff", muted: "#888", surface: "#000" };

function draw(
  painter: typeof drawFootprint,
  extendData: unknown,
  count: number,
  ctx: unknown,
): void {
  painter({
    ctx,
    kLineDataList: Array.from({ length: count }, (_, i) => bar(i)),
    indicator: { extendData },
    visibleRange: { from: 0, to: count, realFrom: 0, realTo: count },
    barSpace: { bar: 50, gapBar: 40, halfBar: 25, halfGapBar: 20 },
    bounding: { width: 500, height: 500, left: 0, right: 0, top: 0, bottom: 0 },
    ...axes,
  } as never);
}

const footprint = (t: number, row: number): FootprintBar => ({
  t,
  row,
  buy: 3,
  sell: 3,
  min_delta: 0,
  max_delta: 0,
  poc: null,
  vah: null,
  val: null,
  rows: [
    [100, 1, 1],
    [100 + row, 2, 2],
  ],
  buy_imbalances: [],
  sell_imbalances: [],
  stacked: [],
});

describe("a footprint", () => {
  it("draws each bar with the row size it was built with", () => {
    const { ctx, rects } = recorder();
    const ext: FootprintExtend = {
      ...colors,
      kind: "footprint",
      bars: new Map([
        [0, footprint(0, 5)],
        [3_600, footprint(3_600, 10)],
      ]),
      settings: { ...defaultOrderFlow().footprint, summary: "none", showPoc: false, showValueArea: false },
      rowSize: 5,
      decimals: 2,
      intraday: true,
      report: () => {},
    };
    draw(drawFootprint, ext, 2, ctx);
    const heights = (x: number) =>
      new Set(rects.filter((r) => Math.abs(r.x - x) < 25).map((r) => r.h));
    expect(heights(0)).toEqual(new Set([5]));
    expect(heights(50)).toEqual(new Set([10]));
  });
});

const profile = (first: number, end: number): TpoProfile => ({
  t: first - 1_800,
  end,
  row: 5,
  block_times: [first],
  open: 100,
  close: 100,
  high: 105,
  low: 100,
  mid: 102.5,
  poc: 100,
  vah: 105,
  val: 100,
  poor_high: false,
  poor_low: false,
  single_prints: [],
  ib_high: 105,
  ib_low: 100,
  total_blocks: 1,
  blocks_above_poc: 0,
  blocks_below_poc: 0,
  rotation_factor: 0,
  volume: 1,
  rows: [{ p: 100, letters: "A", blocks: [0], v: 1 }],
});

describe("a TPO", () => {
  const settings = {
    ...defaultOrderFlow().tpo,
    showIb: false,
    showOpenClose: false,
    showPoc: false,
    showValueArea: false,
    style: "blocks" as const,
  };
  const ext = (profiles: TpoProfile[]): TpoExtend => ({
    ...colors,
    kind: "tpo",
    profiles: new Map(profiles.map((p) => [p.t, p])),
    settings,
    decimals: 2,
  });
  /** Bars drawn as candles: a body, in a candle's colors, where the bar is. */
  const candlesAt = (rects: Rect[]) =>
    [
      ...new Set(
        rects.filter((r) => r.color === colors.up || r.color === colors.down).map((r) => Math.round((r.x + r.w / 2) / 50)),
      ),
    ].sort((a, b) => a - b);

  it("draws a candle for any bar no profile covers", () => {
    const { ctx, rects } = recorder();
    // Bars 4 and 5 have their profile; bars 0 to 3 are older than any.
    draw(drawTpo, ext([profile(4 * 3_600, 6 * 3_600)]), 6, ctx);
    expect(candlesAt(rects)).toEqual([0, 1, 2, 3]);
  });

  it("draws candles for all of them without a profile", () => {
    const { ctx, rects } = recorder();
    draw(drawTpo, ext([]), 3, ctx);
    expect(candlesAt(rects)).toEqual([0, 1, 2]);
  });

  it("starts a profile at its first block, not at its period's start", () => {
    const { ctx, rects } = recorder();
    // Its period started at bar 0, and its blocks only at bar 2.
    draw(drawTpo, ext([{ ...profile(2 * 3_600, 4 * 3_600), t: 0 }]), 5, ctx);
    const blocks = rects.filter((r) => r.color.startsWith("hsla"));
    expect(blocks.length).toBeGreaterThan(0);
    // The edge of bar 2, half a bar before its center.
    expect(Math.min(...blocks.map((r) => r.x))).toBe(2 * 50 - 25);
  });

  it("draws rows at the profile's own row size", () => {
    const { ctx, rects } = recorder();
    draw(drawTpo, ext([{ ...profile(0, 3_600), row: 20 }]), 1, ctx);
    const block = rects.find((r) => r.color.startsWith("hsla"))!;
    expect(block.h).toBe(19);
  });
});
