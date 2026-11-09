// Chart types drawn by the series indicator (ADR-0030): Line with markers,
// Step line, Baseline, Columns and High-low. Each one is picked from the
// menu, must paint its own pixels on the price pane (counted by color, so a
// live tick cannot pass for it), keep the OHLC legend, survive a reload and
// give the candles back when Candles is picked again.

import type { Page } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

const SHOTS = "/tmp/claude-1000/chartgen-shots/chart-types";

interface Probe {
  r: number;
  g: number;
  b: number;
  /** Accepted alpha range, 0-255. */
  a: [number, number];
}

// Dark theme defaults: palette bull/bear and the line color.
const rgb = (hex: number) => ({ r: hex >> 16, g: (hex >> 8) & 255, b: hex & 255 });
const LINE: Probe = { ...rgb(0x2196f3), a: [250, 255] };
// The whole 2px line, its anti-aliased edges too: the opaque core alone shrinks
// as the line gets steeper, which moves with the sample data's clock.
const LINE_EDGES: Probe = { ...rgb(0x2196f3), a: [100, 255] };
const BULL: Probe = { ...rgb(0x26a69a), a: [250, 255] };
const COLUMN_UP: Probe = { ...rgb(0x26a69a), a: [120, 135] };
const COLUMN_DOWN: Probe = { ...rgb(0xef5350), a: [120, 135] };
// Baseline fills: a 0.28 -> 0.05 gradient towards the base level (alpha 13-71).
// The probes take the whole gradient: a narrower band sees only the part far
// from the base, whose size moves with the sample data's clock.
const FILL_UP: Probe = { ...rgb(0x26a69a), a: [5, 80] };
const FILL_DOWN: Probe = { ...rgb(0xef5350), a: [5, 80] };

interface Case {
  label: string;
  /** Pixel colors only this type paints; each must gain `min` pixels. */
  probes: Probe[];
  min: number;
}

const CASES: Case[] = [
  { label: "Line with markers", probes: [LINE], min: 1500 },
  { label: "Step line", probes: [LINE], min: 1500 },
  // Over 168 sample clocks a drawn side gains at least ~8500 px; a side whose
  // fill is missing gains at most ~600 (the edges of its line).
  { label: "Baseline", probes: [FILL_UP, FILL_DOWN], min: 3000 },
  { label: "Columns", probes: [COLUMN_UP, COLUMN_DOWN], min: 3000 },
  { label: "High-low", probes: [LINE], min: 3000 },
  // Over 168 sample clocks the close line gains at least ~1800 px; without it, ~20.
  { label: "HLC area", probes: [LINE_EDGES], min: 1000 },
];

/** Pixels per probe on the price pane's canvases (below the first legend row). */
const priceCounts = (page: Page, probes: Probe[]) =>
  page.evaluate((probes) => {
    const label = document.querySelector(".pane-header-label");
    const out = probes.map(() => 0);
    if (!label) return out;
    const y = label.getBoundingClientRect().bottom + 20;
    for (const canvas of document.querySelectorAll<HTMLCanvasElement>(
      ".chart-container canvas",
    )) {
      const rect = canvas.getBoundingClientRect();
      if (rect.width < 300 || y < rect.top || y >= rect.bottom) continue;
      const ctx = canvas.getContext("2d");
      if (!ctx || canvas.width === 0 || canvas.height === 0) continue;
      const d = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
      probes.forEach((p, k) => {
        const tol = p.a[0] < 100 ? 24 : 8;
        for (let i = 0; i < d.length; i += 4) {
          const a = d[i + 3]!;
          if (
            a >= p.a[0] &&
            a <= p.a[1] &&
            Math.abs(d[i]! - p.r) <= tol &&
            Math.abs(d[i + 1]! - p.g) <= tol &&
            Math.abs(d[i + 2]! - p.b) <= tol
          )
            out[k]!++;
        }
      });
    }
    return out;
  }, probes);

async function pickChartType(page: Page, name: string): Promise<void> {
  await page.getByRole("button", { name: /^Chart type/ }).click();
  await page.getByRole("menuitemradio", { name, exact: true }).click();
  await expect(page.getByRole("button", { name: `Chart type: ${name}` })).toBeVisible();
}

/** Least gain over `base` across the probes. */
const minGain = async (page: Page, probes: Probe[], base: number[]) =>
  Math.min(...(await priceCounts(page, probes)).map((n, k) => n - base[k]!));

async function expectLegend(page: Page): Promise<void> {
  for (const k of ["o", "h", "l", "c"]) {
    await expect(page.getByTestId(`legend-${k}`).first()).toHaveText(/^\d[\d,.]*$/);
  }
}

interface Bar {
  timestamp: number;
  open: number;
  high: number;
  low: number;
  close: number;
}

interface TestWindow {
  __chartgenTest: {
    chart: () => {
      getDataList: () => Bar[];
      getVisibleRange: () => { realFrom: number; realTo: number };
      convertToPixel: (
        points: Array<{ dataIndex: number }>,
        finder: { paneId: string },
      ) => Array<{ x?: number }>;
      getDom: () => HTMLElement | null;
      scrollToDataIndex: (index: number) => void;
    };
  };
}

/**
 * Columns of the other color than the last price line that close well above
 * it, in a window of bars that ends with the line's bar and holds its close
 * well inside the window's price range (so the line is not pinned to an edge).
 * `pick`: of the bars loaded, the one to end the chart at (a replay start)
 * with the most such columns; scroll it into view and return its x in the
 * page. Otherwise: how many such columns the bars on screen hold.
 */
const columnsAboveLine = (page: Page, pick: boolean) =>
  page.evaluate((pick) => {
    const chart = (window as unknown as TestWindow).__chartgenTest.chart();
    const bars = chart.getDataList();
    const { realFrom, realTo } = chart.getVisibleRange();
    const from = Math.max(0, realFrom);
    const to = Math.min(bars.length, realTo);
    // klinecharts colors the line by its bar's close against its open; a
    // column takes the up color when its close is at or above the previous one.
    const up = (i: number) => bars[i]!.close >= (i > 0 ? bars[i - 1]!.close : bars[i]!.open);
    const count = (start: number, last: number) => {
      const line = bars[last]!;
      if (line.close === line.open) return 0;
      let lo = Infinity;
      let hi = -Infinity;
      for (let i = start; i <= last; i++) {
        lo = Math.min(lo, bars[i]!.low);
        hi = Math.max(hi, bars[i]!.high);
      }
      const margin = (hi - lo) * 0.1;
      if (line.close < lo + margin || line.close > hi - margin) return 0;
      let n = 0;
      for (let i = start; i < last; i++) {
        if (up(i) !== line.close > line.open && bars[i]!.close > line.close + margin / 2) n++;
      }
      return n;
    };
    if (!pick) return { n: to > from ? count(from, to - 1) : 0, index: to - 1, x: 0 };
    const width = to - from;
    let best = { n: 0, index: bars.length - 1 };
    for (let last = width - 1; last < bars.length - 1; last++) {
      const n = count(last - width + 1, last);
      if (n > best.n) best = { n, index: last };
    }
    if (best.n === 0) throw new Error("no replay start has other-colored columns above its last price line");
    // Into the right half of the view, clear of the edges.
    chart.scrollToDataIndex(Math.min(bars.length - 1, best.index + Math.floor(width / 4)));
    const [p] = chart.convertToPixel([{ dataIndex: best.index }], { paneId: "candle_pane" });
    return { ...best, x: chart.getDom()!.getBoundingClientRect().left + (p?.x ?? 0) };
  }, pick);

test.use({ viewport: { width: 1440, height: 900 } });

test.describe("chart types", () => {
  test("candles reference shot", async ({ bootedApp: page }) => {
    await expect
      .poll(async () => (await priceCounts(page, [BULL]))[0], { timeout: 15_000 })
      .toBeGreaterThan(1000);
    await page.screenshot({ path: `${SHOTS}/candles.png` });
  });

  test("the series sits beneath the last price line", async ({ bootedApp: page }) => {
    await pickChartType(page, "Columns");
    // Whether any column reaches the line depends on where the data puts the
    // last close, and the sample data ends at the wall clock. A replay ends the
    // chart, and the line, at a bar of our choosing: start one whose close has
    // columns of the other color closing above it.
    const start = await columnsAboveLine(page, true);
    await page.getByTestId("toolbar-replay").click();
    await expect(page.getByTestId("replay-bar")).toHaveAttribute("data-mode", "select");
    const box = (await page.locator(".chart-container").boundingBox())!;
    await page.mouse.move(start.x, box.y + box.height * 0.3);
    await expect(page.getByTestId("replay-cutline").locator("div").first()).toBeVisible();
    await page.mouse.click(start.x, box.y + box.height * 0.3);
    await expect(page.getByTestId("replay-bar")).toHaveAttribute("data-mode", "active");
    await expect
      .poll(async () => (await columnsAboveLine(page, false)).n, {
        message: "columns of the other color above the last price line",
      })
      .toBeGreaterThanOrEqual(10);
    // In the price line's row (the row with the most opaque up/down pixels:
    // columns are half transparent), count line pixels over a column of the
    // other color. Painted beneath the line, the column leaves them opaque
    // and unblended; painted over it, they would be a mix of both colors.
    const crossings = () =>
      page.evaluate(
        ({ up, down }) => {
          const label = document.querySelector(".pane-header-label");
          if (!label) return 0;
          const y = label.getBoundingClientRect().bottom + 20;
          const near = (d: Uint8ClampedArray, i: number, c: number[], tol: number) =>
            Math.abs(d[i]! - c[0]!) <= tol &&
            Math.abs(d[i + 1]! - c[1]!) <= tol &&
            Math.abs(d[i + 2]! - c[2]!) <= tol;
          let best = 0;
          for (const canvas of document.querySelectorAll<HTMLCanvasElement>(
            ".chart-container canvas",
          )) {
            const rect = canvas.getBoundingClientRect();
            if (rect.width < 300 || y < rect.top || y >= rect.bottom) continue;
            const ctx = canvas.getContext("2d");
            if (!ctx || canvas.width === 0 || canvas.height === 0) continue;
            const w = canvas.width;
            const d = ctx.getImageData(0, 0, w, canvas.height).data;
            const opaque = (i: number, c: number[]) => d[i + 3]! >= 250 && near(d, i, c, 8);
            let lineRow = -1;
            let lineColor = up;
            let most = 0;
            for (let row = 0; row < canvas.height; row++) {
              for (const c of [up, down]) {
                let n = 0;
                for (let x = 0; x < w; x++) if (opaque((row * w + x) * 4, c)) n++;
                if (n > most) [most, lineRow, lineColor] = [n, row, c];
              }
            }
            if (lineRow < 0 || lineRow + 4 >= canvas.height) continue;
            const other = lineColor === up ? down : up;
            let n = 0;
            for (let x = 0; x < w; x++) {
              const below = ((lineRow + 4) * w + x) * 4;
              const columnBelow = Math.abs(d[below + 3]! - 128) <= 8 && near(d, below, other, 8);
              if (columnBelow && opaque((lineRow * w + x) * 4, lineColor)) n++;
            }
            best = Math.max(best, n);
          }
          return best;
        },
        { up: [0x26, 0xa6, 0x9a], down: [0xef, 0x53, 0x50] },
      );
    await expect.poll(crossings, { timeout: 15_000 }).toBeGreaterThan(20);
  });

  for (const c of CASES) {
    test(`${c.label}: draws, keeps the legend, persists, and gives candles back`, async ({
      bootedApp: page,
    }) => {
      // Sample the baseline only once the candles are painted.
      await expect
        .poll(async () => (await priceCounts(page, [BULL]))[0], { timeout: 15_000 })
        .toBeGreaterThan(1000);
      const base = await priceCounts(page, c.probes);
      const [candles] = await priceCounts(page, [BULL]);
      await pickChartType(page, c.label);
      await expect
        .poll(() => minGain(page, c.probes, base), { timeout: 15_000 })
        .toBeGreaterThan(c.min);
      // klinecharts' own candles are hidden: what is left in the up color is
      // the last price line. Baseline draws its upper line in that color.
      if (c.label !== "Baseline") {
        expect((await priceCounts(page, [BULL]))[0]).toBeLessThan(candles! / 4);
      }
      await expectLegend(page);
      // The crosshair legend still reads the hovered bar's OHLC.
      const box = (await page.locator(".chart-host").boundingBox())!;
      await page.mouse.move(box.x + box.width * 0.5, box.y + box.height * 0.3);
      await expectLegend(page);
      await page.mouse.move(box.x + box.width + 50, box.y);
      await page.screenshot({
        path: `${SHOTS}/${c.label.toLowerCase().replace(/[^a-z]+/g, "-")}.png`,
      });

      await reloadApp(page);
      await expect(page.getByRole("button", { name: `Chart type: ${c.label}` })).toBeVisible();
      await expect
        .poll(() => minGain(page, c.probes, base), { timeout: 15_000 })
        .toBeGreaterThan(c.min);

      await pickChartType(page, "Candles");
      await expect
        .poll(async () => (await priceCounts(page, [BULL]))[0], { timeout: 15_000 })
        .toBeGreaterThan(1000);
      await expect.poll(() => minGain(page, c.probes, base)).toBeLessThan(c.min / 2);
    });
  }
});
