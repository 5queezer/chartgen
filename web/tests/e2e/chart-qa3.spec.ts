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

test.describe("loading the active layout", () => {
  const layoutButton = (page: Page) => page.getByRole("button", { name: /^Layout:/ });
  const dirtyDot = (page: Page) => page.getByTestId("layout-dirty");
  const activeTimeframe = (page: Page) =>
    page.getByRole("group", { name: "Timeframe" }).locator("[aria-pressed=true]");
  const chip = (page: Page, name: string) =>
    page.getByRole("button", { name: `Remove ${name}`, exact: true });

  async function pickActive(page: Page): Promise<void> {
    await layoutButton(page).click();
    await page.getByRole("menuitem", { name: /^Load layout/ }).click();
    await page.getByRole("button", { name: /^Unnamed/ }).click();
  }

  test("discards unsaved changes after the prompt, keeps them on Cancel", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: "15m", exact: true }).click();
    await chip(page, "ema_stack").click();
    await page.getByRole("button", { name: "Compare symbol" }).waitFor();
    await addCompare(page, "ETHUSDT");
    await expect(dirtyDot(page)).toBeVisible();

    await pickActive(page);
    const prompt = page.getByRole("dialog");
    await expect(prompt).toContainText("Unsaved changes");
    await prompt.getByRole("button", { name: "Cancel" }).click();
    await expect(activeTimeframe(page)).toHaveText("15m");
    await expect(dirtyDot(page)).toBeVisible();

    await pickActive(page);
    await page.getByRole("dialog").getByRole("button", { name: "Don't save" }).click();
    await expect(activeTimeframe(page)).toHaveText("1h");
    await expect(chip(page, "ema_stack")).toBeVisible();
    await expect(compareLegend(page, "ETHUSDT")).toHaveCount(0);
    await expect(
      page.getByRole("group", { name: "Price scale" }).getByRole("button", { name: "%", exact: true }),
    ).toHaveAttribute("aria-pressed", "false");
    await expect(dirtyDot(page)).toHaveCount(0);
  });

  test("Save in the prompt keeps the changes as the saved state", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: "15m", exact: true }).click();
    await pickActive(page);
    await page.getByRole("dialog").getByRole("button", { name: "Save", exact: true }).click();
    await expect(activeTimeframe(page)).toHaveText("15m");
    await expect(dirtyDot(page)).toHaveCount(0);
  });

  test("with nothing changed it does not prompt or reload the chart", async ({
    bootedApp: page,
  }) => {
    let loads = 0;
    page.on("request", (req) => {
      if (req.url().endsWith("/mcp") && /"generate_chart"/.test(req.postData() ?? "")) {
        const bars = (req.postDataJSON() as { params?: { arguments?: { bars?: number } } }).params
          ?.arguments?.bars;
        if ((bars ?? 0) > 2) loads++;
      }
    });
    await pickActive(page);
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.waitForTimeout(500);
    expect(loads).toBe(0);
  });
});

test.describe("legend after a keyboard zoom", () => {
  /** Close of the bar at x (host pixels) or of the last bar, as the legend prints it. */
  const closeAt = (page: Page, x: number | null) =>
    page.evaluate((px) => {
      const hook = (window as unknown as { __chartgenTest: Hook }).__chartgenTest;
      const c = hook.chart()!;
      const data = c.getDataList();
      const i =
        px === null
          ? data.length - 1
          : Math.min(data.length - 1, c.convertFromPixel([{ x: px }], { paneId: "candle_pane" })[0]!.dataIndex);
      return { i, close: data[i]!.close };
    }, x);
  const legendClose = async (page: Page) =>
    Number((await page.getByTestId("legend-c").innerText()).replaceAll(",", ""));

  test("follows the bar under a resting crosshair", async ({ bootedApp: page }) => {
    const box = (await page.locator(".chart-host").boundingBox())!;
    const x = 200;
    await page.mouse.move(box.x + x, box.y + box.height / 3);
    const before = await closeAt(page, x);
    await expect.poll(() => legendClose(page)).toBeCloseTo(before.close, 2);

    for (let n = 0; n < 4; n++) await page.keyboard.press("ArrowUp");
    const after = await closeAt(page, x);
    expect(after.i).toBeGreaterThan(before.i);
    await expect.poll(() => legendClose(page)).toBeCloseTo(after.close, 2);
  });

  test("reads the last bar once the pointer is on the price axis", async ({
    bootedApp: page,
  }) => {
    const box = (await page.locator(".chart-host").boundingBox())!;
    await page.mouse.move(box.x + 200, box.y + box.height / 3);
    const hovered = await closeAt(page, 200);
    await expect.poll(() => legendClose(page)).toBeCloseTo(hovered.close, 2);

    await page.mouse.move(box.x + box.width - 20, box.y + box.height / 3);
    const last = await closeAt(page, null);
    expect(last.i).not.toBe(hovered.i);
    await expect.poll(() => legendClose(page)).toBeCloseTo(last.close, 2);
    await page.keyboard.press("ArrowUp");
    await expect.poll(() => legendClose(page)).toBeCloseTo(last.close, 2);
  });

  test("reads the last bar with the pointer off the chart", async ({ bootedApp: page }) => {
    const box = (await page.locator(".chart-host").boundingBox())!;
    await page.mouse.move(box.x + 200, box.y + box.height / 3);
    await page.mouse.move(box.x + box.width / 2, 2);
    await page.keyboard.press("ArrowUp");
    const last = await closeAt(page, null);
    await expect.poll(() => legendClose(page)).toBeCloseTo(last.close, 2);
  });
});

test.describe("watchlist hover buttons", () => {
  /** Hover the row and measure its buttons against the text of the numeric cells. */
  const overlaps = async (page: Page, symbol: string) => {
    const row = page.locator(`.wl-row[data-symbol="${symbol}"]`);
    await page.mouse.move(0, 0);
    const lefts = () =>
      row.locator('[role="cell"]').evaluateAll((cells) =>
        cells.map((c) => c.getBoundingClientRect().left),
      );
    const before = await lefts();
    await row.hover();
    // Showing the buttons does not move the columns.
    expect(await lefts()).toEqual(before);
    await expect(row.getByRole("button", { name: `Remove ${symbol} from watchlist` })).toHaveCSS(
      "opacity",
      "1",
    );
    return row.evaluate((el) => {
      const rect = (e: Element) => e.getBoundingClientRect();
      const text = (e: Element) => {
        const r = document.createRange();
        r.selectNodeContents(e);
        return r.getBoundingClientRect();
      };
      const cells = [...el.querySelectorAll('[role="cell"]')].slice(1);
      const buttons = [...el.querySelectorAll(".wl-flag, .wl-remove")].map(rect);
      const hits = cells.filter((c) => {
        const t = text(c);
        return buttons.some((b) => b.left < t.right && b.right > t.left);
      });
      return { numbers: hits.map((c) => c.textContent), buttons: buttons.length };
    });
  };

  test("cover none of the numbers", async ({ bootedApp: page }) => {
    const result = await overlaps(page, "BTCUSDT");
    expect(result.buttons).toBe(2);
    expect(result.numbers).toEqual([]);
    await page.locator('.wl-row[data-symbol="BTCUSDT"]').screenshot({
      path: test.info().outputPath("watchlist-row-hover.png"),
    });
  });

  test("cover none of the numbers at the narrowest column width", async ({ bootedApp: page }) => {
    // Large quotes squeeze the symbol column to its minimum.
    await page.locator(".wl-table").evaluate((el) => {
      (el as HTMLElement).style.width = "19rem";
    });
    await page.locator('.wl-row[data-symbol="BTCUSDT"] [role="cell"]').nth(1).evaluate((el) => {
      el.textContent = "1,234,567.89";
    });
    const result = await overlaps(page, "BTCUSDT");
    expect(result.numbers).toEqual([]);
  });
});
