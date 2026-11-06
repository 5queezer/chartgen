// Canvas painters of the Volume Footprint and TPO chart types (ADR-0048), built like
// the series indicator of ADR-0030: klinecharts keeps its hidden price series, for
// its autoscale, crosshair and last price line, and one indicator on the candle pane
// paints the rest. A negative zLevel paints with `destination-over`, so in the two
// main painters what is on top is painted first.

import {
  registerIndicator,
  registerYAxis,
  type IndicatorDrawParams,
  type KLineData,
} from "klinecharts";

import { barRect, drawSpan, isUpBar } from "./chartTypes";
import { formatColor, parseColor } from "./color";
import {
  cellMode,
  firstAtOrAfter,
  footprintSummary,
  gradientIndex,
  sessionCvd,
  shortNumber,
  stackedEnd,
  TEXT_MIN_BAR,
  tpoCellWidth,
  tpoHue,
  tpoSummary,
  type CellMode,
} from "./orderFlow";
import type { FootprintSettings, TpoSettings } from "../stores/orderFlow";
import type { FootprintBar, TpoProfile } from "../types/responses";

export const FOOTPRINT_REG_NAME = "chartgen__footprint";
export const TPO_REG_NAME = "chartgen__tpo";
export const ORDER_FLOW_TABLE_REG_NAME = "chartgen__orderflow_table";
export const ORDER_FLOW_TABLE_PANE = "orderflow_table_pane";
export const ORDER_FLOW_TABLE_AXIS = "chartgen_orderflow_table_axis";
export const ORDER_FLOW_TABLE_HEIGHT = 170;

const LETTER_FONT = "Helvetica Neue, Arial, sans-serif";
const POC_COLOR = "#ff9800";
const LINE_GRAY = "rgba(120, 123, 134, 0.9)";

interface Colors {
  up: string;
  down: string;
  text: string;
  muted: string;
  surface: string;
}

export interface FootprintExtend extends Colors {
  kind: "footprint";
  bars: ReadonlyMap<number, FootprintBar>;
  settings: FootprintSettings;
  rowSize: number;
  decimals: number;
  intraday: boolean;
  /** Told which way the bars on screen are drawn, for the status. */
  report: (mode: CellMode) => void;
}

export interface TpoExtend extends Colors {
  kind: "tpo";
  profiles: ReadonlyMap<number, TpoProfile>;
  settings: TpoSettings;
  rowSize: number;
  decimals: number;
}

const withAlpha = (color: string, a: number) => {
  const { hex, alpha } = parseColor(color);
  return formatColor(hex, alpha * a);
};

function drawCandle(
  ctx: CanvasRenderingContext2D,
  bars: readonly KLineData[],
  i: number,
  x: number,
  width: number,
  y: (v: number) => number,
  c: Colors,
): void {
  const b = bars[i]!;
  ctx.fillStyle = isUpBar(bars, i) ? c.up : c.down;
  const body = barRect(x, y(b.open), y(b.close), width);
  ctx.fillRect(body.x, body.y, body.width, body.height);
  const wick = barRect(x, y(b.high), y(b.low), 1);
  ctx.fillRect(wick.x, wick.y, wick.width, wick.height);
}

/** Text at `x` only if it fits in `room`: a number cut off reads as another number. */
function fitText(
  ctx: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  room: number,
): void {
  if (ctx.measureText(text).width <= room) ctx.fillText(text, x, y);
}

function drawCells(
  ctx: CanvasRenderingContext2D,
  fp: FootprintBar,
  x: number,
  barPx: number,
  y: (v: number) => number,
  mode: CellMode,
  ext: FootprintExtend,
): void {
  const s = ext.settings;
  const row = ext.rowSize;
  const width = Math.max(4, barPx - 2);
  const left = Math.round(x - width / 2);
  const mid = left + width / 2;
  const text = mode === "cells_text";
  const buys = new Set(fp.buy_imbalances);
  const sells = new Set(fp.sell_imbalances);
  const inValueArea = (p: number) =>
    fp.val === null || fp.vah === null || (p >= fp.val && p < fp.vah);
  const mostSide = Math.max(...fp.rows.map(([, b, sell]) => Math.max(b, sell)), 1e-12);
  const mostTotal = Math.max(...fp.rows.map(([, b, sell]) => b + sell), 1e-12);
  const mostDelta = Math.max(...fp.rows.map(([, b, sell]) => Math.abs(b - sell)), 1e-12);

  const box = (p: number) => {
    const a = y(p);
    const b = y(p + row);
    const top = Math.round(Math.min(a, b));
    return { top, height: Math.max(1, Math.round(Math.max(a, b)) - top) };
  };

  // Top first: POC outline, value area edges, numbers, then the fills under them.
  if (s.showPoc && fp.poc !== null) {
    const { top, height } = box(fp.poc);
    ctx.strokeStyle = POC_COLOR;
    ctx.lineWidth = 1;
    ctx.strokeRect(left + 0.5, top + 0.5, width - 1, Math.max(0, height - 1));
  }
  if (s.showValueArea && fp.val !== null && fp.vah !== null) {
    ctx.fillStyle = LINE_GRAY;
    for (const edge of [fp.val, fp.vah]) ctx.fillRect(left, Math.round(y(edge)), width, 1);
  }

  const font = Math.min(10, barPx < 44 ? 9 : 10);
  ctx.font = `${font}px ${LETTER_FONT}`;
  ctx.textBaseline = "middle";
  for (const [p, b, sell] of fp.rows) {
    const { top, height } = box(p);
    const ladderShift = s.type === "ladder" ? Math.round(height / 2) : 0;
    const textY = top + height / 2;
    if (s.showImbalance) {
      ctx.fillStyle = ext.up;
      if (buys.has(p)) ctx.fillRect(left + width - 2, top - ladderShift, 2, height);
      ctx.fillStyle = ext.down;
      if (sells.has(p)) ctx.fillRect(left, top, 2, height);
    }
    if (text) {
      const dim = inValueArea(p) ? ext.text : ext.muted;
      ctx.fillStyle = dim;
      if (s.type === "delta" || s.type === "total") {
        ctx.textAlign = "center";
        fitText(ctx, shortNumber(s.type === "delta" ? b - sell : b + sell), mid, textY, width);
      } else {
        ctx.textAlign = "right";
        ctx.fillStyle = s.showImbalance && sells.has(p) ? ext.down : dim;
        fitText(ctx, shortNumber(sell), mid - 2, textY, width / 2 - 3);
        ctx.textAlign = "left";
        ctx.fillStyle = s.showImbalance && buys.has(p) ? ext.up : dim;
        fitText(ctx, shortNumber(b), mid + 2, textY - ladderShift, width / 2 - 3);
      }
    }
  }

  for (const [p, b, sell] of fp.rows) {
    const { top, height } = box(p);
    const ladderShift = s.type === "ladder" ? Math.round(height / 2) : 0;
    const dim = inValueArea(p) ? 1 : 0.55;
    const fill = (color: string, share: number, xAt: number, w: number, yAt = top) => {
      ctx.fillStyle = s.gradient
        ? withAlpha(s.gradientColors[gradientIndex(share)]!, dim)
        : withAlpha(color, (0.18 + 0.62 * share) * dim);
      ctx.fillRect(xAt, yAt, Math.max(1, w), height);
    };
    if (s.type === "delta" || s.type === "total") {
      const delta = s.type === "delta";
      const share = delta ? Math.abs(b - sell) / mostDelta : (b + sell) / mostTotal;
      const color = delta ? (b >= sell ? ext.up : ext.down) : ext.muted;
      const w = s.display === "profile" ? width * share : width;
      fill(color, share, left, w);
    } else {
      const half = width / 2 - 0.5;
      const sellShare = sell / mostSide;
      const buyShare = b / mostSide;
      const sellW = s.display === "profile" ? half * sellShare : half;
      const buyW = s.display === "profile" ? half * buyShare : half;
      fill(ext.down, sellShare, mid - 0.5 - sellW, sellW);
      fill(ext.up, buyShare, mid + 0.5, buyW, top - ladderShift);
    }
  }
}

/** Total, buy, sell and delta under a footprint. */
function drawInfoBox(
  ctx: CanvasRenderingContext2D,
  fp: FootprintBar,
  x: number,
  width: number,
  y: (v: number) => number,
  ext: FootprintExtend,
): void {
  const low = fp.rows[0];
  if (!low) return;
  const delta = fp.buy - fp.sell;
  const lines: [string, string][] = [
    [shortNumber(fp.buy + fp.sell), ext.text],
    [shortNumber(fp.buy), ext.up],
    [shortNumber(fp.sell), ext.down],
    [shortNumber(delta), delta >= 0 ? ext.up : ext.down],
  ];
  ctx.font = `9px ${LETTER_FONT}`;
  ctx.textAlign = "center";
  ctx.textBaseline = "top";
  const top = Math.max(y(low[0]), y(low[0] + ext.rowSize)) + 2;
  lines.forEach(([text, color], k) => {
    ctx.fillStyle = color;
    fitText(ctx, text, x, top + k * 10, width);
  });
}

export function drawFootprint({
  ctx,
  kLineDataList: bars,
  indicator,
  visibleRange,
  barSpace,
  bounding,
  xAxis,
  yAxis,
}: IndicatorDrawParams): boolean {
  const ext = indicator.extendData as FootprintExtend | undefined;
  const [start, end] = drawSpan(visibleRange, bars.length);
  if (!ext || end <= start) return true;
  const y = (v: number) => yAxis.convertToPixel(v);
  const px = (i: number) => xAxis.convertToPixel(i);
  ctx.setLineDash([]);
  const last = bars[end - 1]!;
  const rowPx = Math.abs(y(last.close + ext.rowSize) - y(last.close));
  const mode = cellMode(barSpace.bar, rowPx);
  ext.report(ext.bars.size > 0 ? mode : "candles");

  for (let i = start; i < end; i++) {
    const b = bars[i]!;
    const fp = ext.bars.get(Math.floor(b.timestamp / 1000));
    if (mode === "candles" || !fp || fp.rows.length === 0) {
      drawCandle(ctx, bars, i, px(i), barSpace.gapBar, y, ext);
      continue;
    }
    if (ext.settings.summary === "box" && barSpace.bar >= TEXT_MIN_BAR) {
      drawInfoBox(ctx, fp, px(i), barSpace.bar - 2, y, ext);
    }
    drawCells(ctx, fp, px(i), barSpace.bar, y, mode, ext);
  }

  // Stacked zones lie under everything, from any bar on the chart, projected until
  // price comes back to them.
  if (ext.settings.showStacked && mode !== "candles") {
    ctx.font = `9px ${LETTER_FONT}`;
    ctx.textBaseline = "middle";
    ctx.textAlign = "left";
    for (const fp of ext.bars.values()) {
      if (fp.stacked.length === 0) continue;
      const i = firstAtOrAfter(bars, fp.t * 1000);
      if (i >= bars.length || bars[i]!.timestamp !== fp.t * 1000 || i >= end) continue;
      for (const zone of fp.stacked) {
        const stop = stackedEnd(zone, i, bars);
        if (stop !== null && stop < start) continue;
        const x0 = px(i);
        const x1 = stop === null ? bounding.width : px(stop);
        const top = Math.min(y(zone.low), y(zone.high));
        const height = Math.max(1, Math.abs(y(zone.low) - y(zone.high)));
        const color = zone.side === "buy" ? ext.up : ext.down;
        ctx.fillStyle = color;
        ctx.fillText(String(zone.count), x1 + 2, top + height / 2);
        ctx.fillRect(x0, Math.round(top), x1 - x0, 1);
        ctx.fillStyle = withAlpha(color, 0.14);
        ctx.fillRect(x0, top, x1 - x0, height);
      }
    }
  }
  return true;
}

// ---- TPO ----

function drawProfile(
  ctx: CanvasRenderingContext2D,
  p: TpoProfile,
  x0: number,
  x1: number,
  xOfBlock: (k: number) => number,
  y: (v: number) => number,
  ext: TpoExtend,
): void {
  const s = ext.settings;
  const row = ext.rowSize;
  const total = x1 - x0;
  const profileW = s.showProfile ? Math.round(total * 0.25) : 0;
  const blocksFrom = s.showProfile && s.profileSide === "left" ? x0 + profileW : x0;
  const blocksTo = s.showProfile && s.profileSide === "right" ? x1 - profileW : x1;
  const room = Math.max(1, blocksTo - blocksFrom);
  const longest = Math.max(...p.rows.map((r) => r.blocks.length), 1);
  const cell = tpoCellWidth(room, longest);
  const box = (price: number) => {
    const a = y(price);
    const b = y(price + row);
    const top = Math.round(Math.min(a, b));
    return { top, height: Math.max(1, Math.round(Math.max(a, b)) - top) };
  };
  const letters = s.style === "letters";
  const inValueArea = (price: number) => price >= p.val && price < p.vah;

  // Top first: lines and labels, then the blocks, then the volume profile.
  const line = (price: number, color: string, dash: number[] = []) => {
    ctx.strokeStyle = color;
    ctx.lineWidth = 1;
    ctx.setLineDash(dash);
    ctx.beginPath();
    const at = Math.round(y(price)) + 0.5;
    ctx.moveTo(x0, at);
    ctx.lineTo(x1, at);
    ctx.stroke();
    ctx.setLineDash([]);
  };
  if (s.showPoc) line(p.poc + row / 2, POC_COLOR);
  if (s.showValueArea) {
    line(p.vah, LINE_GRAY, [3, 3]);
    line(p.val, LINE_GRAY, [3, 3]);
  }
  if (s.showMidpoint) line(p.mid, ext.muted, [1, 3]);
  if (s.showIb) {
    const a = y(p.ib_low);
    const b = y(p.ib_high);
    ctx.fillStyle = "#2196f3";
    ctx.fillRect(x0, Math.min(a, b), 2, Math.max(1, Math.abs(a - b)));
  }
  if (s.showOpenClose) {
    ctx.fillStyle = ext.text;
    ctx.fillRect(x0 - 3, Math.round(y(p.open)) - 1, 4, 2);
    ctx.fillRect(x1 - 1, Math.round(y(p.close)) - 1, 4, 2);
  }
  if (s.showPoor) {
    ctx.fillStyle = ext.down;
    if (p.poor_high) ctx.fillRect(x0, Math.round(y(p.high)), Math.min(room, 24), 2);
    if (p.poor_low) ctx.fillRect(x0, Math.round(y(p.low)), Math.min(room, 24), 2);
  }
  if (s.showSingles) {
    ctx.fillStyle = withAlpha(POC_COLOR, 0.8);
    for (const [low, high] of p.single_prints) {
      const a = y(low);
      const b = y(high);
      ctx.fillRect(blocksFrom + 3, Math.min(a, b), 3, Math.max(1, Math.abs(a - b)));
    }
  }

  // One way for the whole profile: rows round to different heights, and letters on
  // some of them and blocks on others would read as a difference in the market.
  const rowPx = Math.abs(y(p.poc + row) - y(p.poc));
  const useLetters = letters && rowPx >= 9 && (s.splitByBlocks || cell >= 7);
  ctx.font = `${Math.min(10, Math.floor(rowPx) - 1)}px ${LETTER_FONT}`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  for (const r of p.rows) {
    const { top, height } = box(r.p);
    const alpha = inValueArea(r.p) ? 1 : s.outsideOpacity;
    r.blocks.forEach((block, k) => {
      const at = s.splitByBlocks ? xOfBlock(block) : blocksFrom + k * cell;
      if (at < x0 - 1 || at > x1) return;
      const color = `hsla(${tpoHue(block)}, 75%, 62%, ${alpha})`;
      const w = s.splitByBlocks ? Math.max(1, xOfBlock(block + 1) - at - 1) : Math.max(1, cell - 1);
      ctx.fillStyle = color;
      if (useLetters) {
        ctx.fillText(r.letters[k]!, at + w / 2, top + height / 2);
      } else {
        ctx.fillRect(at, top, w, Math.max(1, height - 1));
      }
    });
  }

  if (s.showProfile && profileW > 0) {
    const most = Math.max(...p.rows.map((r) => r.v), 1e-12);
    for (const r of p.rows) {
      const { top, height } = box(r.p);
      const w = Math.max(1, (profileW - 2) * (r.v / most));
      ctx.fillStyle = r.p === p.poc
        ? POC_COLOR
        : withAlpha(ext.muted, inValueArea(r.p) ? 0.7 : 0.35);
      const at = s.profileSide === "right" ? x1 - w : x0;
      ctx.fillRect(at, top, w, Math.max(1, height - 1));
    }
  }
}

export function drawTpo({
  ctx,
  kLineDataList: bars,
  indicator,
  visibleRange,
  barSpace,
  xAxis,
  yAxis,
}: IndicatorDrawParams): boolean {
  const ext = indicator.extendData as TpoExtend | undefined;
  const [start, end] = drawSpan(visibleRange, bars.length);
  if (!ext || end <= start) return true;
  const y = (v: number) => yAxis.convertToPixel(v);
  ctx.setLineDash([]);
  // Nothing to show yet, or not in replay: the candles, rather than an empty chart.
  if (ext.profiles.size === 0) {
    for (let i = start; i < end; i++) {
      drawCandle(ctx, bars, i, xAxis.convertToPixel(i), barSpace.gapBar, y, ext);
    }
    return true;
  }
  const edge = (t: number) => xAxis.convertToPixel(firstAtOrAfter(bars, t * 1000)) - barSpace.bar / 2;
  const first = bars[start]!.timestamp / 1000;
  const lastT = bars[end - 1]!.timestamp / 1000;
  for (const p of ext.profiles.values()) {
    if (p.end <= first || p.t > lastT) continue;
    const x0 = edge(p.t);
    const x1 = Math.max(x0 + barSpace.bar, edge(p.end) - 2);
    const xOfBlock = (k: number) => {
      const t = p.block_times[k] ?? p.end;
      return edge(t);
    };
    drawProfile(ctx, p, x0, x1, xOfBlock, y, ext);
  }
  return true;
}

// ---- Summary tables, in a pane of their own ----

const TABLE_LABEL_WIDTH = 58;

function drawTable(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  columns: { x: number; width: number; rows: [string, string][] }[],
  c: Colors,
): void {
  const names = columns[0]?.rows.map((r) => r[0]) ?? [];
  if (names.length === 0) {
    ctx.fillStyle = c.muted;
    ctx.font = `11px ${LETTER_FONT}`;
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    ctx.fillText("Zoom in to see the table", 8, height / 2);
    return;
  }
  const rowH = Math.max(9, Math.min(13, height / names.length));
  ctx.font = `9px ${LETTER_FONT}`;
  ctx.textBaseline = "middle";
  ctx.textAlign = "center";
  ctx.fillStyle = c.text;
  for (const col of columns) {
    col.rows.forEach(([, value], k) => {
      // Half hidden by the labels, a number reads as another number.
      const half = ctx.measureText(value).width / 2;
      if (col.x - half >= TABLE_LABEL_WIDTH) fitText(ctx, value, col.x, rowH * (k + 0.5), col.width);
    });
  }
  ctx.fillStyle = c.surface;
  ctx.fillRect(0, 0, Math.min(TABLE_LABEL_WIDTH, width), height);
  ctx.fillStyle = c.muted;
  ctx.textAlign = "left";
  names.forEach((name, k) => ctx.fillText(name, 4, rowH * (k + 0.5)));
}

function drawOrderFlowTable({
  ctx,
  kLineDataList: bars,
  indicator,
  visibleRange,
  barSpace,
  bounding,
  xAxis,
}: IndicatorDrawParams): boolean {
  const ext = indicator.extendData as FootprintExtend | TpoExtend | undefined;
  const [start, end] = drawSpan(visibleRange, bars.length);
  if (!ext || end <= start) return true;
  const fmt = (n: number | null | undefined) => (n == null ? "-" : n.toFixed(ext.decimals));
  const columns: { x: number; width: number; rows: [string, string][] }[] = [];
  if (ext.kind === "footprint") {
    if (barSpace.bar >= TEXT_MIN_BAR) {
      const sorted = [...ext.bars.values()].sort((a, b) => a.t - b.t);
      const cvd = sessionCvd(sorted, ext.intraday);
      for (let i = start; i < end; i++) {
        const t = Math.floor(bars[i]!.timestamp / 1000);
        const fp = ext.bars.get(t);
        if (!fp) continue;
        const prev = i > 0 ? ext.bars.get(Math.floor(bars[i - 1]!.timestamp / 1000)) : undefined;
        columns.push({
          x: xAxis.convertToPixel(i),
          width: barSpace.bar - 2,
          rows: footprintSummary(fp, prev, cvd.get(t), ext.rowSize, fmt),
        });
      }
    }
  } else {
    const first = bars[start]!.timestamp / 1000;
    const last = bars[end - 1]!.timestamp / 1000;
    for (const p of ext.profiles.values()) {
      if (p.end <= first || p.t > last) continue;
      const a = xAxis.convertToPixel(firstAtOrAfter(bars, p.t * 1000));
      const b = xAxis.convertToPixel(firstAtOrAfter(bars, p.end * 1000));
      if (b - a < 50) continue;
      columns.push({ x: (a + b) / 2, width: b - a - 4, rows: tpoSummary(p, fmt) });
    }
  }
  drawTable(ctx, bounding.width, bounding.height, columns, ext);
  return true;
}

let registered = false;

/** Register (once) the order-flow painters. No figures: nothing adds to the autoscale. */
export function ensureOrderFlowRegistered(): void {
  if (registered) return;
  registered = true;
  for (const [name, draw] of [
    [FOOTPRINT_REG_NAME, drawFootprint],
    [TPO_REG_NAME, drawTpo],
  ] as const) {
    registerIndicator({
      name,
      shortName: "ORDERFLOW",
      precision: 10,
      zLevel: -1,
      figures: [],
      calc: () => [],
      draw,
    });
  }
  // The table's pane has no scale to show.
  registerYAxis({ name: ORDER_FLOW_TABLE_AXIS, createTicks: () => [] });
  registerIndicator({
    name: ORDER_FLOW_TABLE_REG_NAME,
    shortName: "",
    precision: 2,
    figures: [],
    calc: () => [],
    draw: drawOrderFlowTable,
  });
}
