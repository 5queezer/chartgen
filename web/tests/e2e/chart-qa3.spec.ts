// QA pass 3: lock ratio with compare, reverting the active layout, the legend
// after a keyboard zoom, watchlist hover buttons, indicator settings title,
// the x-axis labels.

import type { Page } from "@playwright/test";

import { test, expect, gotoApp, reloadApp } from "./fixtures/app";

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

/**
 * Axis span over the range of everything drawn on the price pane in view.
 * klinecharts' auto-fit with the default gap makes this 1.3 whatever the data
 * is; a locked range makes it something else.
 */
const fitRatio = (page: Page) =>
  page.evaluate(() => {
    const c = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()! as unknown as {
      convertFromPixel: (p: { y: number }[], o: { paneId: string }) => { value: number }[];
      getSize: (id: string, w: string) => { height: number };
      getVisibleRange: () => { from: number; to: number };
      getDataList: () => { high: number; low: number }[];
      getIndicatorByPaneId: (
        id: string,
      ) => Map<string, { figures: { key: string }[]; result: (Record<string, unknown> | undefined)[] }>;
    };
    const h = c.getSize("candle_pane", "yAxis").height;
    const [a, b] = c.convertFromPixel([{ y: 0 }, { y: h }], { paneId: "candle_pane" });
    const { from, to } = c.getVisibleRange();
    const values: number[] = [];
    for (let i = from; i < to; i++) {
      const d = c.getDataList()[i]!;
      values.push(d.low, d.high);
      c.getIndicatorByPaneId("candle_pane").forEach((ind) => {
        for (const f of ind.figures) {
          const v = ind.result[i]?.[f.key];
          if (typeof v === "number") values.push(v);
        }
      });
    }
    return Math.abs(a!.value - b!.value) / (Math.max(...values) - Math.min(...values));
  });

const expectAutoFit = (page: Page) =>
  expect
    .poll(async () => Math.abs((await fitRatio(page)) - 1.3) < 0.05, { timeout: 8_000 })
    .toBe(true);

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
    await expectAutoFit(page);
    await expect.poll(() => offScreen(page)).toMatchObject({ bars: 0, lines: 0 });
    await setBarSpace(page, 24);
    await expectAutoFit(page);
    await expect.poll(() => offScreen(page)).toMatchObject({ bars: 0, lines: 0 });

    // A second symbol, and the same after a reload with the list stored.
    await addCompare(page, "SOLUSDT");
    await expect(compareLegend(page, "SOLUSDT")).toBeVisible();
    await expectAutoFit(page);
    await expect.poll(() => offScreen(page)).toMatchObject({ bars: 0, lines: 0 });
    await reloadApp(page);
    await expect.poll(async () => (await offScreen(page)).seen).toBeGreaterThan(0);
    await expectAutoFit(page);
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
    await expect.poll(() => span(page)).toBeGreaterThan(0);
    const before = await span(page);
    // The lock holds: three times the bar width, a third of the price range.
    await setBarSpace(page, 24);
    await expect
      .poll(async () => Math.abs((await span(page)) / before - 1 / 3) < 0.03)
      .toBe(true);
    await addCompare(page, "ETHUSDT");
    await expect.poll(async () => (await offScreen(page)).seen).toBeGreaterThan(0);
    await expectAutoFit(page);
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

  test("Save with storage full keeps the chart as it is", async ({ bootedApp: page }) => {
    await page.getByRole("button", { name: "15m", exact: true }).click();
    await page.evaluate(() => {
      const real = Storage.prototype.setItem;
      Storage.prototype.setItem = function (key: string, value: string) {
        if (key === "chartgen.layouts") throw new DOMException("full", "QuotaExceededError");
        real.call(this, key, value);
      };
    });
    await pickActive(page);
    await page.getByRole("dialog").getByRole("button", { name: "Save", exact: true }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.waitForTimeout(300);
    await expect(activeTimeframe(page)).toHaveText("15m");
  });

  test("with nothing changed it writes nothing", async ({ bootedApp: page }) => {
    await page.evaluate(() => {
      const w = window as unknown as { layoutWrites: number };
      w.layoutWrites = 0;
      const real = Storage.prototype.setItem;
      Storage.prototype.setItem = function (key: string, value: string) {
        if (key === "chartgen.layouts") w.layoutWrites++;
        real.call(this, key, value);
      };
    });
    await pickActive(page);
    await expect(page.getByRole("dialog")).toHaveCount(0);
    expect(await page.evaluate(() => (window as unknown as { layoutWrites: number }).layoutWrites)).toBe(0);
  });

  /** Log scale, ETHUSDT added (percent axis, log to come back to), saved. */
  async function saveLogWithCompare(page: Page): Promise<void> {
    await page
      .getByRole("group", { name: "Price scale" })
      .getByRole("button", { name: "log", exact: true })
      .click();
    await addCompare(page, "ETHUSDT");
    await expect(compareLegend(page, "ETHUSDT")).toBeVisible();
    await page.keyboard.press("Control+s");
    await expect(dirtyDot(page)).toHaveCount(0);
  }

  async function removeCompareExpectingLog(page: Page): Promise<void> {
    await compareLegend(page, "ETHUSDT").hover();
    await compareLegend(page, "ETHUSDT").getByRole("button", { name: "Remove ETHUSDT" }).click();
    await expect(compareLegend(page, "ETHUSDT")).toHaveCount(0);
    await expect(
      page.getByRole("group", { name: "Price scale" }).getByRole("button", { name: "log", exact: true }),
    ).toHaveAttribute("aria-pressed", "true");
  }

  test("with nothing changed, the compare symbol still restores the log scale", async ({
    bootedApp: page,
  }) => {
    await saveLogWithCompare(page);
    await pickActive(page);
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await removeCompareExpectingLog(page);
  });

  test("Save in the prompt keeps the log scale to restore", async ({ bootedApp: page }) => {
    await saveLogWithCompare(page);
    await page.getByRole("button", { name: "15m", exact: true }).click();
    await pickActive(page);
    await page.getByRole("dialog").getByRole("button", { name: "Save", exact: true }).click();
    await expect(activeTimeframe(page)).toHaveText("15m");
    await removeCompareExpectingLog(page);
  });

  test("Don't save keeps the log scale to restore when the compare list is the same", async ({
    bootedApp: page,
  }) => {
    await saveLogWithCompare(page);
    await page.getByRole("button", { name: "15m", exact: true }).click();
    await pickActive(page);
    await page.getByRole("dialog").getByRole("button", { name: "Don't save" }).click();
    await expect(activeTimeframe(page)).toHaveText("1h");
    await removeCompareExpectingLog(page);
  });
});

test.describe("legend after a keyboard zoom", () => {
  /** Index and close of the bar at x (host pixels), or of the last bar. */
  const barAt = (page: Page, x: number | null) =>
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
  /** The legend reads the bar at x. Both sides are read on each try: a tick moves the last bar. */
  const expectLegendAt = (page: Page, x: number | null) =>
    expect
      .poll(async () => {
        const text = await page.getByTestId("legend-c").innerText();
        const { close } = await barAt(page, x);
        return Math.abs(Number(text.replaceAll(",", "")) - close) < 0.01;
      })
      .toBe(true);

  test("follows the bar under a resting crosshair", async ({ bootedApp: page }) => {
    const box = (await page.locator(".chart-host").boundingBox())!;
    const x = 200;
    await page.mouse.move(box.x + x, box.y + box.height / 3);
    const before = await barAt(page, x);
    await expectLegendAt(page, x);

    for (let n = 0; n < 4; n++) await page.keyboard.press("ArrowUp");
    expect((await barAt(page, x)).i).toBeGreaterThan(before.i);
    await expectLegendAt(page, x);
  });

  test("reads the last bar once the pointer is on the price axis", async ({
    bootedApp: page,
  }) => {
    const box = (await page.locator(".chart-host").boundingBox())!;
    await page.mouse.move(box.x + 200, box.y + box.height / 3);
    await expectLegendAt(page, 200);

    await page.mouse.move(box.x + box.width - 20, box.y + box.height / 3);
    await expectLegendAt(page, null);
    await page.keyboard.press("ArrowUp");
    await expectLegendAt(page, null);
  });

  test("reads the last bar with the pointer off the chart", async ({ bootedApp: page }) => {
    const box = (await page.locator(".chart-host").boundingBox())!;
    await page.mouse.move(box.x + 200, box.y + box.height / 3);
    await page.mouse.move(box.x + box.width / 2, 2);
    await page.keyboard.press("ArrowUp");
    await expectLegendAt(page, null);
  });
});

test.describe("watchlist hover buttons", () => {
  /** Hover the row and measure its buttons against the text of the numeric cells. */
  const overlaps = async (page: Page, symbol: string) => {
    const row = page.locator(`.wl-row[data-symbol="${symbol}"]`);
    const lefts = () =>
      row.locator('[role="cell"]').evaluateAll((cells) =>
        cells.map((c) => c.getBoundingClientRect().left),
      );
    // Showing the buttons does not move the columns. Retried: a live quote can
    // change a column's width between the two reads.
    await expect(async () => {
      await page.mouse.move(0, 0);
      const before = await lefts();
      await row.hover();
      expect(await lefts()).toEqual(before);
    }).toPass({ timeout: 10_000 });
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

test.describe("watchlist row buttons without hover", () => {
  const row = (page: Page, symbol: string) => page.locator(`.wl-row[data-symbol="${symbol}"]`);
  const button = (page: Page, name: string) => page.getByRole("button", { name, exact: true });

  test("are hidden until hover with a mouse", async ({ bootedApp: page }) => {
    await page.mouse.move(0, 0);
    await expect(button(page, "Remove BTCUSDT from watchlist")).toHaveCSS("opacity", "0");
    await row(page, "BTCUSDT").hover();
    await expect(button(page, "Remove BTCUSDT from watchlist")).toHaveCSS("opacity", "1");
  });

  test("keyboard focus reserves their room, at the narrowest column too", async ({
    bootedApp: page,
  }) => {
    await page.mouse.move(0, 0);
    // Large quotes squeeze the symbol column to its minimum.
    await page.locator(".wl-table").evaluate((el) => {
      (el as HTMLElement).style.width = "19rem";
    });
    await row(page, "BTCUSDT").locator('[role="cell"]').nth(1).evaluate((el) => {
      el.textContent = "1,234,567.89";
    });
    await row(page, "BTCUSDT").focus();
    // Past the symbol button and the grip.
    await page.keyboard.press("Tab");
    await page.keyboard.press("Tab");
    await page.keyboard.press("Tab");
    const flag = button(page, "Flag BTCUSDT");
    await expect(flag).toBeFocused();
    await expect(flag).toHaveCSS("opacity", "1");
    const gap = await row(page, "BTCUSDT").evaluate((el) => {
      const symbol = el.querySelector('button[aria-label^="Show "]')!.getBoundingClientRect();
      const flagBox = el.querySelector(".wl-flag")!.getBoundingClientRect();
      return flagBox.left - symbol.right;
    });
    expect(gap).toBeGreaterThanOrEqual(0);
  });

  test.describe("on a touch screen", () => {
    test.use({ hasTouch: true, isMobile: true });

    test("show always, so a tap never lands on a hidden button", async ({ bootedApp: page }) => {
      expect(await page.evaluate(() => matchMedia("(hover: none)").matches)).toBe(true);
      const remove = button(page, "Remove BTCUSDT from watchlist");
      await expect(remove).toHaveCSS("opacity", "1");
      await expect(button(page, "Flag BTCUSDT")).toHaveCSS("opacity", "1");
      await expect(button(page, "Drag BTCUSDT to reorder")).toHaveCSS("opacity", "1");

      // The symbol does not run under them, and a tap on it selects the row.
      const gap = await row(page, "ETHUSDT").evaluate((el) => {
        const symbol = el.querySelector('button[aria-label^="Show "]')!.getBoundingClientRect();
        const flagBox = el.querySelector(".wl-flag")!.getBoundingClientRect();
        return flagBox.left - symbol.right;
      });
      expect(gap).toBeGreaterThanOrEqual(0);
      await page.getByRole("button", { name: "Show ETHUSDT on the chart" }).tap();
      await expect(row(page, "ETHUSDT")).toHaveAttribute("aria-current", "true");

      await remove.tap();
      await expect(row(page, "BTCUSDT")).toHaveCount(0);
    });
  });
});

test.describe("indicator settings title", () => {
  test("shows the chip's name, not the raw id", async ({ page }) => {
    await gotoApp(page, "/?ind=bbands,ema_stack");
    for (const id of ["bbands", "ema_stack"]) {
      const shown = await page.locator(`[title="${id}"]`).first().innerText();
      await page.getByRole("button", { name: `Settings for ${id}`, exact: true }).click();
      const heading = page.getByRole("dialog").getByRole("heading");
      await expect(heading).toHaveText(shown);
      await expect(heading).not.toHaveText(id);
      await page.getByRole("dialog").getByRole("button", { name: "Cancel" }).click();
    }
  });
});

test.describe("x-axis labels", () => {
  interface Tick {
    text: string;
    coord: number;
    /** Label width, measured in the axis font. */
    width: number;
  }
  /**
   * The ticks klinecharts draws and the width of the axis. They have no public
   * getter; geometry holds at any time of day, unlike which pixels a label
   * happens to cover.
   */
  const axis = (page: Page) =>
    page.evaluate(() => {
      const c = (window as unknown as { __chartgenTest: { chart: () => unknown } }).__chartgenTest.chart() as {
        _xAxisPane: {
          getAxisComponent: () => {
            getTicks: () => { text: string; coord: number }[];
            getSelfBounding: () => { width: number };
          };
        };
        getStyles: () => { xAxis: { tickText: { size: number; weight: string; family: string } } };
      };
      const a = c._xAxisPane.getAxisComponent();
      const { size, weight, family } = c.getStyles().xAxis.tickText;
      const ctx = document.createElement("canvas").getContext("2d")!;
      ctx.font = `${weight} ${size}px ${family}`;
      return {
        width: a.getSelfBounding().width,
        ticks: a.getTicks().map((t) => ({
          text: t.text,
          coord: t.coord,
          width: ctx.measureText(t.text).width,
        })),
      };
    });

  const scrollBy = (page: Page, distance: number) =>
    page.evaluate((d) => {
      const c = (window as unknown as { __chartgenTest: { chart: () => unknown } }).__chartgenTest.chart() as {
        scrollByDistance: (d: number) => void;
      };
      c.scrollByDistance(d);
      return new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    }, distance);

  test("labels stay whole at both edges, and their grid lines stay", async ({
    bootedApp: page,
  }) => {
    const cut: Tick[] = [];
    let blankLeft = 0;
    let blankRight = 0;
    // Small steps, so several ticks land inside each edge's 2 px margin.
    for (let i = 0; i < 120; i++) {
      await scrollBy(page, 4);
      const { width, ticks } = await axis(page);
      expect(ticks.length).toBeGreaterThan(1);
      for (const t of ticks) {
        if (t.text === "") {
          if (t.coord < width / 2) blankLeft++;
          else blankRight++;
        } else if (t.coord - t.width / 2 < 1.5 || t.coord + t.width / 2 > width - 1.5) {
          cut.push(t);
        }
      }
    }
    expect(cut).toEqual([]);
    // The ticks near the edges were kept with no text: GridView draws from them.
    expect(blankLeft).toBeGreaterThan(0);
    expect(blankRight).toBeGreaterThan(0);
    await page.locator(".chart-host").screenshot({
      path: test.info().outputPath("x-axis-left-edge.png"),
    });
  });

  test("a tick alone on the axis keeps its label", async ({ bootedApp: page }) => {
    // Narrow and zoomed in: one tick, with the long date label that cannot fit.
    await page.locator(".chart-host").evaluate((el) => {
      (el as HTMLElement).style.width = "130px";
      (el as HTMLElement).style.flex = "none";
    });
    await setBarSpace(page, 50);
    let alone = 0;
    for (let i = 0; i < 30; i++) {
      await scrollBy(page, 17);
      const { ticks } = await axis(page);
      if (ticks.length !== 1) continue;
      alone++;
      expect(ticks[0]!.text).not.toBe("");
    }
    expect(alone).toBeGreaterThan(0);
  });
});
