// Ichimoku cloud past the last bar (ADR-0033). The series payload carries
// the leading spans' `future` block; the chart widens its right margin and
// paints the projected cloud there. The suite's sample source ticks the
// forming candle every ~500 ms over push, so the projection is re-sent
// with every tick. Pixels are read off the candle pane canvases; the chart
// is observed through the DEV-only `window.__chartgenTest` hook (App.tsx).

import type { Page } from "@playwright/test";

import { formatValue } from "../../src/lib/format";
import { test, expect, gotoApp, expectChartRendered } from "./fixtures/app";

const SHOTS = "/tmp/claude-1000/chartgen-shots/ichimoku";

interface Future {
  bars: number;
  lines: Array<{ index: number; values: Array<number | null> }>;
}

interface Hook {
  chart: () => {
    getDataList: () => Array<{ timestamp: number; close: number }>;
    convertToPixel: (
      point: { timestamp: number },
      finder: { paneId: string },
    ) => { x?: number };
    getOffsetRightDistance: () => number;
    getBarSpace: () => number;
  } | null;
  series: () => {
    symbol: string | null;
    indicators: Record<string, { future?: Future }>;
  } | null;
}

const hook = (page: Page) =>
  page.evaluate(() => {
    const h = (window as unknown as { __chartgenTest: Hook }).__chartgenTest;
    const c = h.chart()!;
    const list = c.getDataList();
    const last = list.at(-1)!;
    return {
      symbol: h.series()?.symbol ?? null,
      lastClose: last.close,
      lastX: c.convertToPixel({ timestamp: last.timestamp }, { paneId: "candle_pane" }).x!,
      offset: c.getOffsetRightDistance(),
      barSpace: c.getBarSpace(),
      future: h.series()?.indicators.ichimoku?.future ?? null,
    };
  });

/**
 * Cloud pixels right of x = `fromX` (CSS px from the chart's left edge):
 * the server's cloud colors (rgba(76,175,80,0.2) / rgba(239,83,80,0.2))
 * at their 20% alpha. Opaque marks (candles, labels) never match.
 */
const cloudPixelsRightOf = (page: Page, fromX: number) =>
  page.evaluate((from) => {
    const host = document.querySelector<HTMLElement>(".chart-container")!;
    const left = host.getBoundingClientRect().left;
    const colors = [
      [76, 175, 80],
      [239, 83, 80],
    ];
    let count = 0;
    for (const canvas of host.querySelectorAll<HTMLCanvasElement>("canvas")) {
      const ctx = canvas.getContext("2d");
      if (!ctx || canvas.width === 0 || canvas.height === 0) continue;
      const rect = canvas.getBoundingClientRect();
      const scale = canvas.width / rect.width;
      const x0 = Math.max(0, Math.ceil((left + from - rect.left) * scale));
      if (x0 >= canvas.width) continue;
      const d = ctx.getImageData(x0, 0, canvas.width - x0, canvas.height).data;
      for (let i = 0; i < d.length; i += 4) {
        if (Math.abs(d[i + 3]! - 51) > 8) continue;
        if (colors.some((c) => c.every((v, k) => Math.abs(d[i + k]! - v) <= 25))) count++;
      }
    }
    return count;
  }, fromX);

/** Cloud pixels more than one bar right of the last candle. */
async function projected(page: Page): Promise<number> {
  const h = await hook(page);
  return cloudPixelsRightOf(page, h.lastX + h.barSpace);
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("chartgen.indicators.active", JSON.stringify(["ichimoku"]));
  });
});

test("the cloud is drawn past the last bar and follows live ticks", async ({ page }) => {
  await gotoApp(page);
  await expect.poll(async () => (await hook(page)).future?.bars ?? 0).toBe(25);
  const h = await hook(page);
  // TradingView widens the right margin to hold the projection.
  expect(h.offset).toBeGreaterThanOrEqual(25 * h.barSpace);
  await expect.poll(() => projected(page), { timeout: 15_000 }).toBeGreaterThan(500);
  await page.screenshot({ path: `${SHOTS}/projection.png` });

  // Pushed ticks re-send the projection; it stays drawn.
  const badge = page.getByTestId("live-status");
  await expect(badge).toHaveAttribute("data-state", "push");
  const before = h.lastClose;
  await expect.poll(async () => (await hook(page)).lastClose, { timeout: 15_000 }).not.toBe(before);
  for (let i = 0; i < 5; i++) {
    await page.waitForTimeout(400);
    expect(await projected(page)).toBeGreaterThan(500);
    expect((await hook(page)).future?.bars).toBe(25);
  }
});

test("the legend reads the projected spans over the future bars", async ({ page }) => {
  await gotoApp(page);
  await expect.poll(() => projected(page), { timeout: 15_000 }).toBeGreaterThan(500);
  const h = await hook(page);
  const box = (await page.locator(".chart-container").boundingBox())!;
  // Ten bars past the last one.
  await page.mouse.move(box.x + h.lastX + 10 * h.barSpace, box.y + box.height * 0.4);
  const legend = page.getByTestId("price-legend").first();
  await expect(page.getByTestId("legend-c").first()).toHaveText("—");
  await expect(legend).toBeVisible();
  const f = (await hook(page)).future!;
  const spanA = f.lines.find((l) => l.index === 2)!.values[9]!;
  await expect(page.locator(".pane-header").filter({ hasText: "Ichimoku" })).toContainText(
    `Senkou A ${formatValue(spanA)}`,
  );
  await page.screenshot({ path: `${SHOTS}/crosshair.png` });
});

test("switching symbol replaces the projection; removing Ichimoku clears it", async ({
  page,
}) => {
  await gotoApp(page);
  await expect.poll(() => projected(page), { timeout: 15_000 }).toBeGreaterThan(500);
  const old = (await hook(page)).future!;

  await page.getByRole("button", { name: /^Symbol:/ }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("combobox").fill("ETHUSDT");
  await expect(dialog.getByRole("option").first()).toContainText("ETHUSDT");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("button", { name: /^Symbol:/ })).toContainText("ETHUSDT");
  await expectChartRendered(page);
  await expect.poll(async () => (await hook(page)).symbol).toBe("ETHUSDT");
  const fresh = (await hook(page)).future!;
  expect(fresh.bars).toBe(25);
  expect(fresh.lines[0]!.values).not.toEqual(old.lines[0]!.values);
  await expect.poll(() => projected(page), { timeout: 15_000 }).toBeGreaterThan(500);

  await page
    .getByTestId("indicator-legend-ichimoku")
    .getByRole("button", { name: "Remove Ichimoku" })
    .click();
  await expect.poll(() => cloudPixelsRightOf(page, 0), { timeout: 10_000 }).toBe(0);
  // Without a projection the right margin is the default again.
  await expect.poll(async () => (await hook(page)).offset).toBeLessThan(25 * (await hook(page)).barSpace);
});
