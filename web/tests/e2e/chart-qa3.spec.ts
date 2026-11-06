// QA pass 3: lock ratio with compare, reverting the active layout, the legend
// after a keyboard zoom, watchlist hover buttons, indicator settings title,
// the x-axis labels.

import type { Page } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

type Hook = {
  chart: () => {
    convertFromPixel: (
      p: { x?: number; y?: number }[],
      o: { paneId: string },
    ) => { value: number; dataIndex: number }[];
    getSize: (id: string, w: string) => { height: number; width: number; left: number } | null;
    getBarSpace: () => number;
    setBarSpace: (n: number) => void;
    getVisibleRange: () => { from: number; to: number };
    getDataList: () => { high: number; low: number; close: number; timestamp: number }[];
    getIndicatorByPaneId: (
      paneId: string,
      name: string,
    ) => { result: (Record<string, number | null> | undefined)[] } | null;
  } | null;
};

/** Every visible bar and compare value relative to the axis bounds, as a count outside. */
const offScreen = (page: Page) =>
  page.evaluate(() => {
    const c = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!;
    const h = c.getSize("candle_pane", "yAxis")!.height;
    const [a, b] = c.convertFromPixel([{ y: 0 }, { y: h }], { paneId: "candle_pane" });
    const lo = Math.min(a!.value, b!.value);
    const hi = Math.max(a!.value, b!.value);
    const { from, to } = c.getVisibleRange();
    const data = c.getDataList();
    const cmp = c.getIndicatorByPaneId("candle_pane", "chartgen__compare")?.result ?? [];
    let bars = 0;
    let lines = 0;
    let seen = 0;
    for (let i = from; i < to; i++) {
      const d = data[i]!;
      if (d.low < lo - 1e-9 || d.high > hi + 1e-9) bars++;
      for (const v of Object.values(cmp[i] ?? {})) {
        if (typeof v !== "number") continue;
        seen++;
        if (v < lo - 1e-9 || v > hi + 1e-9) lines++;
      }
    }
    return { bars, lines, seen };
  });

const setBarSpace = (page: Page, n: number) =>
  page.evaluate((v) => {
    (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!.setBarSpace(v);
  }, n);

async function lockViaMenu(page: Page): Promise<void> {
  const box = await page.locator(".chart-host").boundingBox();
  if (!box) throw new Error("chart host has no bounding box");
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2, { button: "right" });
  await page.getByRole("menuitemcheckbox", { name: /Lock price to bar ratio/ }).click();
  await expect(page.getByRole("menu")).toBeHidden();
}

async function addCompare(page: Page, ticker: string): Promise<void> {
  await page.getByRole("button", { name: "Compare symbol" }).click();
  await page.getByRole("dialog").getByRole("combobox").fill(ticker);
  await expect(page.getByRole("option").first()).toContainText(ticker);
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog")).toHaveCount(0);
}

const span = (page: Page) =>
  page.evaluate(() => {
    const c = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!;
    const h = c.getSize("candle_pane", "yAxis")!.height;
    const [a, b] = c.convertFromPixel([{ y: 0 }, { y: h }], { paneId: "candle_pane" });
    return Math.abs(a!.value - b!.value);
  });

const compareLegend = (page: Page, sym: string) => page.getByTestId(`compare-legend-${sym}`);

test.describe("lock price to bar ratio with compare", () => {
  test("adding a compare symbol keeps every series on screen, zoomed too", async ({
    bootedApp: page,
  }) => {
    await setBarSpace(page, 8);
    await lockViaMenu(page);
    await addCompare(page, "ETHUSDT");
    await expect(compareLegend(page, "ETHUSDT")).toBeVisible();
    await expect.poll(async () => (await offScreen(page)).seen).toBeGreaterThan(0);
    await expect.poll(() => offScreen(page)).toMatchObject({ bars: 0, lines: 0 });
    await setBarSpace(page, 24);
    await page.waitForTimeout(300);
    expect(await offScreen(page)).toMatchObject({ bars: 0, lines: 0 });

    // A second symbol, and the same after a reload with the list stored.
    await addCompare(page, "SOLUSDT");
    await expect(compareLegend(page, "SOLUSDT")).toBeVisible();
    await expect.poll(() => offScreen(page)).toMatchObject({ bars: 0, lines: 0 });
    await reloadApp(page);
    await expect.poll(async () => (await offScreen(page)).seen).toBeGreaterThan(0);
    await expect.poll(() => offScreen(page)).toMatchObject({ bars: 0, lines: 0 });
  });

  test("already on the percent axis: compare still drops a zoomed-in lock", async ({
    bootedApp: page,
  }) => {
    // The scale mode does not change here, so only the compare list can release the crop.
    await page
      .getByRole("group", { name: "Price scale" })
      .getByRole("button", { name: "%", exact: true })
      .click();
    await setBarSpace(page, 8);
    await lockViaMenu(page);
    await setBarSpace(page, 24);
    await expect.poll(async () => (await offScreen(page)).bars).toBeGreaterThan(0);
    await addCompare(page, "ETHUSDT");
    await expect.poll(async () => (await offScreen(page)).seen).toBeGreaterThan(0);
    await expect.poll(() => offScreen(page)).toMatchObject({ bars: 0, lines: 0 });
  });

  test("removing the last compare symbol resumes the lock from a fresh fit", async ({
    bootedApp: page,
  }) => {
    await setBarSpace(page, 8);
    await lockViaMenu(page);
    await expect.poll(() => span(page)).toBeGreaterThan(0);
    const before = await span(page);
    await addCompare(page, "ETHUSDT");
    await expect.poll(async () => (await offScreen(page)).seen).toBeGreaterThan(0);

    await compareLegend(page, "ETHUSDT").hover();
    await compareLegend(page, "ETHUSDT").getByRole("button", { name: "Remove ETHUSDT" }).click();
    await expect(compareLegend(page, "ETHUSDT")).toHaveCount(0);
    expect(
      await page.evaluate(() => localStorage.getItem("chartgen.layout.lockRatio")),
    ).toBe("true");
    await expect
      .poll(async () => Math.abs((await span(page)) / before - 1) < 0.05)
      .toBe(true);
    // The lock is back on: twice the bar width, half the price range.
    await setBarSpace(page, 16);
    await expect
      .poll(async () => Math.abs((await span(page)) / before - 0.5) < 0.05)
      .toBe(true);
  });
});
