// Right-click menus across the app, with real right clicks: each surface opens
// its own menu, each action hits the thing that was clicked, the price under the
// pointer holds on every axis mode, and everywhere else the browser's menu stays.

import type { Page } from "@playwright/test";

import { test, expect, openSidebarTab } from "./fixtures/app";

test.use({
  viewport: { width: 1280 + 320, height: 720 },
  permissions: ["clipboard-read", "clipboard-write"],
});

const CANDLE = "candle_pane";
const menu = (page: Page, name: string | RegExp) => page.getByRole("menu", { name });
const chartMenu = (page: Page) => menu(page, "Chart actions");
const axisMenu = (page: Page) => menu(page, "Price scale actions");
const drawingMenu = (page: Page) => menu(page, "Drawing actions");
const anyItem = (page: Page, name: string | RegExp) =>
  page.getByRole("menu").locator('[role^="menuitem"]').filter({ hasText: name });
const clipboard = (page: Page) => page.evaluate(() => navigator.clipboard.readText());

type Xy = { x: number; y: number };

/** Viewport point `at` inside the chart container. */
const viewportPoint = async (page: Page, at: Xy): Promise<Xy> => {
  const box = await page.locator(".chart-container").boundingBox();
  if (!box) throw new Error("chart container not laid out");
  return { x: box.x + at.x, y: box.y + at.y };
};

async function rightClick(page: Page, at: Xy): Promise<void> {
  const p = await viewportPoint(page, at);
  await page.mouse.click(p.x, p.y, { button: "right" });
}

async function leftClick(page: Page, at: Xy): Promise<void> {
  const p = await viewportPoint(page, at);
  await page.mouse.move(p.x, p.y, { steps: 3 });
  await page.mouse.down();
  await page.mouse.up();
  // klinecharts treats two clicks in quick succession as a double click.
  await page.waitForTimeout(700);
}

interface ChartHook {
  getDataList(): { high: number; low: number; close: number }[];
  getVisibleRange(): { realFrom: number; realTo: number };
  getSize(pane?: string, position?: string): { width: number; height: number; top: number };
  getBarSpace(): number;
  setBarSpace(n: number): void;
  convertToPixel(p: object, f: object): { x?: number; y?: number };
  convertFromPixel(p: object[], f: object): { value?: number }[];
}
declare global {
  interface Window {
    __chartgenTest: { chart(): ChartHook };
    __chartgenDrawings(): { id: string; name: string; lock: boolean; visible: boolean; points: { value: number }[]; pixels: { y?: number }[] }[];
    __chartgenAlertLines(): { alertId: string; value?: number; y?: number }[];
    __nativeMenu?: boolean;
  }
}

/** A visible bar's high: where it is on screen, and what a pixel is worth there. */
const barHigh = (page: Page) =>
  page.evaluate((pane) => {
    const c = window.__chartgenTest.chart();
    const { realFrom, realTo } = c.getVisibleRange();
    const index = Math.floor((realFrom + realTo) / 2);
    const high = c.getDataList()[index]!.high;
    const p = c.convertToPixel({ dataIndex: index, value: high }, { paneId: pane, absolute: true });
    const [one, two] = c.convertFromPixel([{ y: p.y }, { y: p.y! + 1 }], { paneId: pane, absolute: true });
    return { high, x: p.x!, y: p.y!, perPixel: Math.abs(one!.value! - two!.value!) };
  }, CANDLE);

const paneSizes = (page: Page) =>
  page.evaluate((pane) => {
    const c = window.__chartgenTest.chart();
    return { main: c.getSize(pane, "main"), axis: c.getSize(pane, "yAxis") };
  }, CANDLE);

/** A point on the price axis at chart height `y`. */
const onPriceAxis = async (page: Page, y: number): Promise<Xy> => {
  const { main, axis } = await paneSizes(page);
  return { x: main.width + axis.width / 2, y };
};

/** A point on the time axis. */
const onTimeAxis = async (page: Page): Promise<Xy> => {
  const box = await page.locator(".chart-container").boundingBox();
  return { x: 300, y: box!.height - 8 };
};

const scaleButton = (page: Page, name: string) =>
  page.getByRole("group", { name: "Price scale" }).getByRole("button", { name, exact: true });

/** The chart height of a legend row's sub-pane, well inside it. */
const inSubPane = async (page: Page, key: string): Promise<Xy> => {
  const box = await page.getByTestId(`indicator-legend-${key}`).boundingBox();
  const chart = await page.locator(".chart-container").boundingBox();
  return { x: 500, y: box!.y - chart!.y + 40 };
};

const drawings = (page: Page) => page.evaluate(() => window.__chartgenDrawings());

async function drawTrendLine(page: Page, a: Xy, b: Xy): Promise<void> {
  await page.getByRole("button", { name: "Trend line" }).click();
  await leftClick(page, a);
  await leftClick(page, b);
}

const A = { x: 300, y: 120 };
const B = { x: 560, y: 220 };
const MID = { x: (A.x + B.x) / 2, y: (A.y + B.y) / 2 };

/** Whether the browser's menu would have opened for the next right click. */
const watchNativeMenu = (page: Page) =>
  page.evaluate(() => {
    window.addEventListener(
      "contextmenu",
      (e) => {
        // After every listener has had it.
        setTimeout(() => (window.__nativeMenu = !e.defaultPrevented));
      },
      true,
    );
  });
const nativeMenu = async (page: Page) => {
  await page.waitForTimeout(50);
  return page.evaluate(() => window.__nativeMenu);
};

async function expectOnScreen(page: Page, name: string | RegExp): Promise<void> {
  const box = (await menu(page, name).boundingBox())!;
  const { width, height } = page.viewportSize()!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(width);
  expect(box.y + box.height).toBeLessThanOrEqual(height);
}

test.describe("chart canvas", () => {
  test("the menu opens every time, and its plain actions work", async ({ bootedApp: page }) => {
    for (let i = 0; i < 5; i++) {
      await rightClick(page, { x: 300 + i * 40, y: 160 });
      await expect(chartMenu(page)).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(chartMenu(page)).toHaveCount(0);
    }

    await page.evaluate(() => window.__chartgenTest.chart().setBarSpace(20));
    await rightClick(page, A);
    await anyItem(page, "Reset chart view").click();
    await expect.poll(() => page.evaluate(() => window.__chartgenTest.chart().getBarSpace())).toBe(8);

    await rightClick(page, A);
    await anyItem(page, "Invert scale").click();
    await expect(scaleButton(page, "inv")).toHaveAttribute("aria-pressed", "true");

    await rightClick(page, A);
    await anyItem(page, "Lock price to bar ratio").click();
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "false");

    await rightClick(page, A);
    await anyItem(page, "Settings…").click();
    await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
  });

  for (const mode of [
    { name: "regular", button: null },
    { name: "log", button: "log" },
    { name: "percent", button: "%" },
    { name: "inverted", button: "inv" },
  ]) {
    test(`the price under the pointer is right on a ${mode.name} scale`, async ({ bootedApp: page }) => {
      if (mode.button) {
        await scaleButton(page, mode.button).click();
        await expect(scaleButton(page, mode.button)).toHaveAttribute("aria-pressed", "true");
        await page.waitForTimeout(300);
      }
      const bar = await barHigh(page);
      await rightClick(page, bar);
      await expect(chartMenu(page)).toBeVisible();
      const label = await anyItem(page, /^Copy price/).textContent();
      const shown = Number(label!.replace("Copy price ", ""));
      // Live ticks move the scale a little: the price within a couple of pixels.
      expect(Math.abs(shown - bar.high)).toBeLessThan(bar.perPixel * 3 + 0.01);

      await anyItem(page, /^Copy price/).click();
      await expect.poll(() => clipboard(page)).toBe(String(label!.replace("Copy price ", "")));

      // The whole way: the line it adds is drawn where the click was.
      await rightClick(page, bar);
      await anyItem(page, /^Add horizontal line at/).click();
      await expect.poll(async () => (await drawings(page)).length).toBe(1);
      const [line] = await drawings(page);
      expect(line!.name).toBe("horizontalStraightLine");
      expect(line!.points[0]!.value).toBeCloseTo(shown, 6);
      expect(Math.abs(line!.pixels[0]!.y! - bar.y)).toBeLessThan(4);
    });
  }

  test("Add alert opens the dialog at the price under the pointer", async ({ bootedApp: page }) => {
    const bar = await barHigh(page);
    await rightClick(page, bar);
    const label = await anyItem(page, /^Add alert at/).textContent();
    const shown = Number(label!.replace("Add alert at ", "").replace("…", ""));
    await anyItem(page, /^Add alert at/).click();
    const value = Number(await page.getByRole("dialog").getByLabel("Price").inputValue());
    expect(Math.abs(value - shown)).toBeLessThan(0.01);
  });

  test("a sub-pane offers its indicator and no prices", async ({ bootedApp: page }) => {
    const at = await inSubPane(page, "rsi");
    await rightClick(page, at);
    const pane = menu(page, "Pane actions");
    await expect(pane).toBeVisible();
    await expect(anyItem(page, /^Copy price/)).toHaveCount(0);
    await expect(anyItem(page, /^Add horizontal line/)).toHaveCount(0);
    await expect(anyItem(page, "Invert scale")).toHaveCount(0);
    await expect(anyItem(page, /^Copy value/)).toBeVisible();

    await page.keyboard.press("Escape");

    // The value is the one at that height on this pane's own scale, where the chart draws 50.
    const fifty = await page.evaluate(() => {
      const c = window.__chartgenTest.chart();
      const paneId = document.querySelector<HTMLElement>('[data-indicator="rsi"]')!.dataset.paneId!;
      const y = c.convertToPixel({ value: 50 }, { paneId, absolute: true }).y!;
      const [one, two] = c.convertFromPixel([{ y }, { y: y + 1 }], { paneId, absolute: true });
      return { y, perPixel: Math.abs(one!.value! - two!.value!) };
    });
    await rightClick(page, { x: at.x, y: fifty.y });
    const shown = Number((await anyItem(page, /^Copy value/).textContent())!.replace("Copy value ", ""));
    expect(Math.abs(shown - 50)).toBeLessThan(fifty.perPixel * 2 + 0.01);

    await anyItem(page, /^Hide RSI/).click();
    await expect(page.getByTestId("indicator-legend-rsi")).toHaveCount(0);
    await expect(page.getByTestId("indicator-legend-volume")).toBeVisible();
  });

  test("a right press does not pan or scale", async ({ bootedApp: page }) => {
    const range = () => page.evaluate(() => JSON.stringify(window.__chartgenTest.chart().getVisibleRange()));
    const before = await range();
    const p = await viewportPoint(page, A);
    await page.mouse.move(p.x, p.y);
    await page.mouse.down({ button: "right" });
    await page.mouse.move(p.x + 120, p.y, { steps: 5 });
    await page.mouse.up({ button: "right" });
    expect(await range()).toBe(before);
    await page.keyboard.press("Escape");

    const axis = await viewportPoint(page, await onPriceAxis(page, 150));
    await page.mouse.move(axis.x, axis.y);
    await page.mouse.down({ button: "right" });
    await page.mouse.move(axis.x, axis.y + 60, { steps: 5 });
    await page.mouse.up({ button: "right" });
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "true");
  });
});

test.describe("axes", () => {
  test("price axis: its menu, and each scale action", async ({ bootedApp: page }) => {
    const at = await onPriceAxis(page, 150);
    await rightClick(page, at);
    await expect(axisMenu(page)).toBeVisible();
    await expect(chartMenu(page)).toHaveCount(0);

    await anyItem(page, "Logarithmic").click();
    await expect(scaleButton(page, "log")).toHaveAttribute("aria-pressed", "true");
    await rightClick(page, at);
    await anyItem(page, "Percent").click();
    await expect(scaleButton(page, "%")).toHaveAttribute("aria-pressed", "true");
    await expect(scaleButton(page, "log")).toHaveAttribute("aria-pressed", "false");
    await rightClick(page, at);
    await anyItem(page, "Regular").click();
    await expect(scaleButton(page, "%")).toHaveAttribute("aria-pressed", "false");

    await rightClick(page, at);
    await anyItem(page, "Invert scale").click();
    await expect(scaleButton(page, "inv")).toHaveAttribute("aria-pressed", "true");
    await rightClick(page, at);
    await anyItem(page, "Invert scale").click();
    await expect(scaleButton(page, "inv")).toHaveAttribute("aria-pressed", "false");

    await rightClick(page, at);
    await anyItem(page, "Auto (fit data to screen)").click();
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "false");
    await rightClick(page, at);
    await anyItem(page, "Reset price scale").click();
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "true");

    await rightClick(page, at);
    await anyItem(page, "Lock price to bar ratio").click();
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "false");
    await rightClick(page, at);
    await expect(anyItem(page, "Lock price to bar ratio")).toHaveAttribute("aria-checked", "true");
    await page.keyboard.press("Escape");
  });

  test("price axis: copy price reads the height of the click", async ({ bootedApp: page }) => {
    const bar = await barHigh(page);
    await rightClick(page, await onPriceAxis(page, bar.y));
    const text = (await anyItem(page, /^Copy price/).textContent())!.replace("Copy price ", "");
    expect(Math.abs(Number(text) - bar.high)).toBeLessThan(bar.perPixel * 3 + 0.01);
    await anyItem(page, /^Copy price/).click();
    await expect.poll(() => clipboard(page)).toBe(text);
  });

  test("a scale button opens the price axis menu", async ({ bootedApp: page }) => {
    await scaleButton(page, "log").click({ button: "right" });
    await expect(axisMenu(page)).toBeVisible();
  });

  test("time axis: reset and go to date", async ({ bootedApp: page }) => {
    const at = await onTimeAxis(page);
    await page.evaluate(() => window.__chartgenTest.chart().setBarSpace(20));
    await rightClick(page, at);
    await expect(menu(page, "Time scale actions")).toBeVisible();
    await expect(anyItem(page, /^Copy price/)).toHaveCount(0);
    await anyItem(page, "Reset time scale").click();
    await expect.poll(() => page.evaluate(() => window.__chartgenTest.chart().getBarSpace())).toBe(8);

    await rightClick(page, at);
    await anyItem(page, "Go to date…").click();
    await expect(page.getByRole("dialog").getByLabel("Date")).toBeVisible();
  });
});

test.describe("drawings", () => {
  test("a right click picks the drawing under the pointer, not the chart", async ({ bootedApp: page }) => {
    await drawTrendLine(page, A, B);
    const other = { x: 800, y: 200 };
    await drawTrendLine(page, { x: 700, y: 150 }, { x: 900, y: 250 });
    await expect.poll(async () => (await drawings(page)).length).toBe(2);
    const [first, second] = await drawings(page);

    await rightClick(page, other);
    await expect(drawingMenu(page)).toBeVisible();
    await expect(chartMenu(page)).toHaveCount(0);
    await anyItem(page, "Remove").click();
    await expect.poll(async () => (await drawings(page)).map((d) => d.id)).toEqual([first!.id]);
    expect(second!.id).not.toBe(first!.id);

    // Beside the line it is the chart again.
    await rightClick(page, { x: 450, y: 320 });
    await expect(chartMenu(page)).toBeVisible();
    await expect(drawingMenu(page)).toHaveCount(0);
  });

  test("style, lock, hide, clone and add alert", async ({ bootedApp: page }) => {
    await drawTrendLine(page, A, B);
    await expect.poll(async () => (await drawings(page)).length).toBe(1);
    await page.keyboard.press("Escape");

    await rightClick(page, MID);
    await anyItem(page, "Style…").click();
    await expect(page.getByRole("toolbar", { name: /style/i })).toBeVisible();
    await page.keyboard.press("Escape");

    await rightClick(page, MID);
    await anyItem(page, "Lock").click();
    await expect.poll(async () => (await drawings(page))[0]!.lock).toBe(true);
    await rightClick(page, MID);
    await anyItem(page, "Unlock").click();
    await expect.poll(async () => (await drawings(page))[0]!.lock).toBe(false);

    await rightClick(page, MID);
    await anyItem(page, "Clone").click();
    await expect.poll(async () => (await drawings(page)).length).toBe(2);
    const [orig, copy] = await drawings(page);
    expect(copy!.id).not.toBe(orig!.id);
    expect(copy!.name).toBe(orig!.name);
    // Shown beside the original, not on top of it.
    expect(copy!.pixels[0]!.y).toBeGreaterThan(orig!.pixels[0]!.y!);

    await rightClick(page, MID);
    await anyItem(page, "Hide").click();
    await expect.poll(async () => (await drawings(page)).find((d) => d.id === orig!.id)!.visible).toBe(false);
    expect((await drawings(page)).find((d) => d.id === copy!.id)!.visible).toBe(true);

    // The alert is on the drawing that was clicked: the clone is still showing.
    const y = copy!.pixels;
    const midY = (y[0]!.y! + y[1]!.y!) / 2;
    await rightClick(page, { x: MID.x, y: midY });
    await anyItem(page, /^Add alert on Trend line/).click();
    await expect(page.getByRole("dialog").getByLabel("Source")).toBeVisible();
  });

  test("an armed tool: a right click cancels it, with no menu of any kind", async ({ bootedApp: page }) => {
    await watchNativeMenu(page);
    const tool = page.getByRole("button", { name: "Trend line" });
    for (const at of [A, await onPriceAxis(page, 150)]) {
      await tool.click();
      await expect(tool).toHaveAttribute("aria-pressed", "true");
      await rightClick(page, at);
      await expect(tool).toHaveAttribute("aria-pressed", "false");
      await expect(page.getByRole("menu")).toHaveCount(0);
      expect(await nativeMenu(page)).toBe(false);
    }

    // Placed one point, same.
    await tool.click();
    await leftClick(page, A);
    await rightClick(page, B);
    await expect(tool).toHaveAttribute("aria-pressed", "false");
    await expect(page.getByRole("menu")).toHaveCount(0);
    expect(await drawings(page)).toHaveLength(0);

    // The tool is gone, and the next right click is the chart's.
    await rightClick(page, B);
    await expect(chartMenu(page)).toBeVisible();
  });
});

test.describe("alert lines", () => {
  test("a line's menu edits, pauses and deletes that alert", async ({ bootedApp: page }) => {
    const bar = await barHigh(page);
    await rightClick(page, { x: 300, y: 150 });
    await anyItem(page, /^Add alert at/).click();
    await page.getByRole("dialog").getByRole("button", { name: "Create" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => window.__chartgenAlertLines().length)).toBe(1);
    const [line] = await page.evaluate(() => window.__chartgenAlertLines());
    const at = { x: bar.x, y: line!.y! };

    await rightClick(page, at);
    await expect(menu(page, "Alert actions")).toBeVisible();
    await expect(chartMenu(page)).toHaveCount(0);
    await anyItem(page, "Edit…").click();
    await expect(page.getByRole("dialog").getByRole("button", { name: "Save" })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);

    await rightClick(page, at);
    await anyItem(page, "Delete").click();
    await expect.poll(() => page.evaluate(() => window.__chartgenAlertLines().length)).toBe(0);
  });
});

test.describe("indicators", () => {
  test("a legend row: settings, hide, add alert and remove reach that indicator", async ({ bootedApp: page }) => {
    const row = page.getByTestId("indicator-legend-rsi");
    await row.click({ button: "right", position: { x: 10, y: 8 } });
    await expect(menu(page, /^RSI.* actions$/)).toBeVisible();
    await expect(chartMenu(page)).toHaveCount(0);
    await anyItem(page, /^Settings for RSI/).click();
    await expect(page.getByRole("dialog")).toContainText("RSI");
    await page.keyboard.press("Escape");

    await row.click({ button: "right", position: { x: 10, y: 8 } });
    await anyItem(page, /^Add alert on RSI/).click();
    await expect(page.getByRole("dialog").getByLabel("Source")).toBeVisible();
    await page.keyboard.press("Escape");

    await row.click({ button: "right", position: { x: 10, y: 8 } });
    await anyItem(page, /^Remove RSI/).click();
    await expect(page.getByTestId("indicator-legend-rsi")).toHaveCount(0);
    await expect(page.getByTestId("indicator-legend-volume")).toBeVisible();
  });

  test("the price legend is not an indicator's: it opens the chart menu", async ({ bootedApp: page }) => {
    await page.getByTestId("price-legend").click({ button: "right", force: true });
    await expect(chartMenu(page)).toBeVisible();
  });

  test("a compare symbol: its legend row and tree row can only be removed", async ({ bootedApp: page }) => {
    const add = async (ticker: string) => {
      await page.getByRole("button", { name: "Compare symbol" }).click();
      await page.getByRole("dialog").getByRole("combobox").fill(ticker);
      await expect(page.getByRole("option").first()).toContainText(ticker);
      await page.keyboard.press("Enter");
    };
    await add("ETHUSDT");
    const legend = page.getByTestId("compare-legend-ETHUSDT");
    await expect(legend).toBeVisible();
    await legend.click({ button: "right", position: { x: 10, y: 8 } });
    await expect(page.getByRole("menu").locator('[role^="menuitem"]')).toHaveCount(1);
    await anyItem(page, /^Remove ETHUSDT/).click();
    await expect(legend).toHaveCount(0);

    await add("ETHUSDT");
    await openSidebarTab(page, "Object Tree");
    await page.getByTestId("tree-compare-ETHUSDT").click({ button: "right" });
    await anyItem(page, /^Remove ETHUSDT/).click();
    await expect(page.getByTestId("tree-compare-ETHUSDT")).toHaveCount(0);
  });

  test("several instances: each menu reaches its own", async ({ bootedApp: page }) => {
    await page.getByRole("button", { name: /Indicators/ }).click();
    const picker = page.getByRole("dialog");
    await picker.getByPlaceholder("Search indicators...").fill("rsi");
    await picker
      .locator("li button[type='button']")
      .filter({ has: page.locator('[title="rsi"]') })
      .filter({ hasText: /added/ })
      .click();
    await picker.getByRole("button", { name: "Close" }).click();
    const first = page.getByTestId("indicator-legend-rsi");
    const second = page.getByTestId("indicator-legend-rsi#2");
    await expect(second).toBeVisible();

    // Its sub-pane, then its legend row.
    const box = (await second.boundingBox())!;
    await page.mouse.click(box.x + 400, box.y + 30, { button: "right" });
    await expect(menu(page, "Pane actions")).toBeVisible();
    await anyItem(page, /^Hide /).click();
    await expect(second).toHaveCount(0);
    await expect(first).toBeVisible();

    // Hidden, it has no legend: its tree row brings it back, by key.
    await openSidebarTab(page, "Object Tree");
    await page.getByTestId("tree-ind-rsi#2").click({ button: "right" });
    await anyItem(page, /^Show /).click();
    await expect(second).toBeVisible();

    await page.getByTestId("tree-ind-rsi").click({ button: "right" });
    await anyItem(page, /^Remove /).click();
    await expect(first).toHaveCount(0);
    await expect(second).toBeVisible();

    // A legend row builds its menu from its own callbacks.
    await second.click({ button: "right", position: { x: 10, y: 8 } });
    await anyItem(page, /^Remove /).click();
    await expect(second).toHaveCount(0);
    await expect(page.getByTestId("indicator-legend-volume")).toBeVisible();
  });

  test("Hide finds its instance when chosen, whatever moved since the menu opened", async ({ bootedApp: page }) => {
    await openSidebarTab(page, "Object Tree");
    await page.getByTestId("tree-ind-rsi").click({ button: "right" });
    await expect(menu(page, /^RSI.* actions$/)).toBeVisible();
    // No user path moves the list under an open menu: the store is moved by hand.
    await page.evaluate(async () => {
      const path = "/src/stores/indicators.ts";
      const { activeIndicators, setActiveIndicators } = (await import(/* @vite-ignore */ path)) as {
        activeIndicators: () => unknown[];
        setActiveIndicators: (next: unknown[]) => void;
      };
      setActiveIndicators([...activeIndicators()].reverse());
    });
    await anyItem(page, /^Hide RSI/).click();
    await expect(page.getByTestId("indicator-legend-rsi")).toHaveCount(0);
    await expect(page.getByTestId("indicator-legend-volume")).toBeVisible();
  });

  test("a chip's menu acts on that chip", async ({ bootedApp: page }) => {
    // Not the legend row, which has a Remove button of the same name.
    const chip = page.locator("span.rounded-full", { has: page.getByRole("button", { name: /^Remove volume/i }) });
    const openChip = () => chip.getByText(/^volume/i).click({ button: "right" });
    await openChip();
    await expect(menu(page, /^volume.* actions$/i)).toBeVisible();
    await anyItem(page, /^Hide volume/i).click();
    await expect(chip.getByRole("button", { name: /^Show volume/i })).toBeVisible();
    await expect(page.getByTestId("indicator-legend-rsi")).toBeVisible();

    await openChip();
    await anyItem(page, /^Show volume/i).click();
    await expect(chip.getByRole("button", { name: /^Hide volume/i })).toBeVisible();
    await openChip();
    await anyItem(page, /^Remove volume/i).click();
    await expect(chip).toHaveCount(0);
    await expect(page.getByTestId("indicator-legend-rsi")).toBeVisible();
  });
});

test.describe("sidebar lists", () => {
  const row = (page: Page, sym: string) => page.locator(`.wl-row[data-symbol="${sym}"]`);

  test("a watchlist row: open, alert, flag, copy and remove", async ({ bootedApp: page }) => {
    const target = row(page, "AAPL");
    await target.click({ button: "right" });
    await expect(menu(page, "AAPL actions")).toBeVisible();

    await anyItem(page, "Copy symbol").click();
    await expect.poll(() => clipboard(page)).toBe("AAPL");

    await target.click({ button: "right" });
    await anyItem(page, /^Add alert on AAPL/).click();
    await expect(page.getByRole("dialog")).toContainText("AAPL");
    await page.keyboard.press("Escape");

    await target.click({ button: "right" });
    await anyItem(page, "Flag…").click();
    await page.getByRole("menuitem", { name: "Flag red" }).click();
    await expect(target).toHaveAttribute("data-flag", "red");

    await row(page, "ETHUSDT").click({ button: "right" });
    await anyItem(page, /^Open ETHUSDT in chart/).click();
    await expect(row(page, "ETHUSDT")).toHaveAttribute("aria-current", "true");

    await target.click({ button: "right" });
    await anyItem(page, "Remove from watchlist").click();
    await expect(target).toHaveCount(0);
    await expect(row(page, "ETHUSDT")).toBeVisible();
  });

  test("a section header: collapse, rename and remove", async ({ bootedApp: page }) => {
    await page.getByRole("button", { name: "Add section" }).click();
    const input = page.getByRole("textbox", { name: "Section name" });
    await input.fill("Tech");
    await input.press("Enter");
    const section = page.locator(".wl-section");
    await section.click({ button: "right", position: { x: 120, y: 8 } });
    await expect(menu(page, "Section Tech actions")).toBeVisible();
    await anyItem(page, "Collapse").click();
    await expect(page.getByRole("button", { name: "Section Tech", exact: true })).toHaveAttribute("aria-expanded", "false");

    await section.click({ button: "right", position: { x: 120, y: 8 } });
    await anyItem(page, "Rename…").click();
    const rename = page.getByRole("textbox", { name: "Section name" });
    await rename.fill("Big tech");
    await rename.press("Enter");
    await expect(page.getByRole("button", { name: "Section Big tech", exact: true })).toBeVisible();

    await section.click({ button: "right", position: { x: 120, y: 8 } });
    await anyItem(page, "Remove section").click();
    await expect(page.locator(".wl-section")).toHaveCount(0);
  });

  test("an alerts panel row: edit, pause and delete that alert", async ({ bootedApp: page }) => {
    for (const y of [150, 190]) {
      await rightClick(page, { x: 300, y });
      await anyItem(page, /^Add alert at/).click();
      await page.getByRole("dialog").getByRole("button", { name: "Create" }).click();
      await expect(page.getByRole("dialog")).toHaveCount(0);
    }
    await openSidebarTab(page, /^Alerts/);
    const rows = page.locator("[data-alert-id]");
    await expect(rows).toHaveCount(2);
    const first = rows.first();
    const second = rows.last();

    await first.click({ button: "right" });
    await expect(menu(page, "Alert actions")).toBeVisible();
    await anyItem(page, "Pause").click();
    await expect(first).toHaveAttribute("data-status", "paused");
    await expect(second).toHaveAttribute("data-status", "active");

    await first.click({ button: "right" });
    await anyItem(page, "Resume").click();
    await expect(first).toHaveAttribute("data-status", "active");

    await second.click({ button: "right" });
    await anyItem(page, "Edit…").click();
    await expect(page.getByRole("dialog").getByRole("button", { name: "Save" })).toBeVisible();
    await page.keyboard.press("Escape");

    const keep = await first.getAttribute("data-alert-id");
    await second.click({ button: "right" });
    await anyItem(page, "Delete").click();
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toHaveAttribute("data-alert-id", keep!);
  });

  test("object tree rows: an indicator, a drawing and an alert", async ({ bootedApp: page }) => {
    await drawTrendLine(page, A, B);
    await expect.poll(async () => (await drawings(page)).length).toBe(1);
    await rightClick(page, { x: 300, y: 300 });
    await anyItem(page, /^Add alert at/).click();
    await page.getByRole("dialog").getByRole("button", { name: "Create" }).click();
    await openSidebarTab(page, "Object Tree");

    await page.getByTestId("tree-ind-rsi").click({ button: "right" });
    await anyItem(page, /^Hide RSI/).click();
    await expect(page.getByTestId("indicator-legend-rsi")).toHaveCount(0);

    const drawingRow = page.locator("[data-testid^=tree-drawing-]");
    await drawingRow.click({ button: "right" });
    await expect(drawingMenu(page)).toBeVisible();
    await anyItem(page, "Hide").click();
    await expect.poll(async () => (await drawings(page))[0]!.visible).toBe(false);
    // Hidden, the same menu brings it back: nothing else on screen can.
    await drawingRow.click({ button: "right" });
    await expect(anyItem(page, "Style…")).toHaveAttribute("aria-disabled", "true");
    await anyItem(page, "Show").click();
    await expect.poll(async () => (await drawings(page))[0]!.visible).toBe(true);
    await drawingRow.click({ button: "right" });
    await anyItem(page, "Remove").click();
    await expect(drawingRow).toHaveCount(0);

    const alertRow = page.locator("[data-testid^=tree-alert-]");
    await alertRow.click({ button: "right" });
    await anyItem(page, "Delete").click();
    await expect(alertRow).toHaveCount(0);

    // The main series has no actions: the browser's menu stays.
    await watchNativeMenu(page);
    await page.getByTestId("tree-main").click({ button: "right" });
    await expect(page.getByRole("menu")).toHaveCount(0);
    expect(await nativeMenu(page)).toBe(true);
  });
});

test.describe("the menu itself", () => {
  test("elsewhere the browser's menu stays, and text fields keep theirs", async ({ bootedApp: page }) => {
    await watchNativeMenu(page);
    await page.locator("header h1").click({ button: "right" });
    expect(await nativeMenu(page)).toBe(true);
    await expect(page.getByRole("menu")).toHaveCount(0);

    await page.getByRole("button", { name: "Add section" }).click();
    const input = page.getByRole("textbox", { name: "Section name" });
    await page.evaluate(() => (window.__nativeMenu = undefined));
    await input.click({ button: "right" });
    expect(await nativeMenu(page)).toBe(true);
    await expect(page.getByRole("menu")).toHaveCount(0);
    await page.keyboard.press("Escape");

    // A field inside the chart area: the replay bar's date.
    await page.getByRole("button", { name: /replay/i }).first().click();
    const date = page.getByTestId("replay-select-date");
    // Its buttons are in the chart area too, and are not the chart.
    await page.evaluate(() => (window.__nativeMenu = undefined));
    await date.click({ button: "right" });
    expect(await nativeMenu(page)).toBe(true);
    await expect(chartMenu(page)).toHaveCount(0);
    await date.click();
    const field = page.locator('input[type="date"]').first();
    await page.evaluate(() => (window.__nativeMenu = undefined));
    await field.click({ button: "right" });
    expect(await nativeMenu(page)).toBe(true);
    await expect(chartMenu(page)).toHaveCount(0);
  });

  test("picking a replay start: a right click picks no bar and opens the chart menu", async ({ bootedApp: page }) => {
    await page.getByRole("button", { name: /replay/i }).first().click();
    await page.getByTestId("replay-select-bar").click();
    await expect(page.getByTestId("replay-cutline")).toBeVisible();
    await rightClick(page, A);
    await expect(chartMenu(page)).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("replay-cutline")).toBeVisible();
    const mode = () =>
      page.evaluate(() => (window as unknown as { __chartgenTest: { replay(): { mode: string } } }).__chartgenTest.replay().mode);
    expect(await mode()).toBe("select");
  });

  test("the keyboard's menu key opens the chart menu on the focused chart", async ({ bootedApp: page }) => {
    await page.locator(".chart-host").focus();
    await page.keyboard.press("Shift+F10");
    await expect(page.getByRole("menu").first()).toBeVisible();
  });

  test("Escape and an outside click close it; Escape leaves the selection and an armed tool alone", async ({
    bootedApp: page,
  }) => {
    await rightClick(page, A);
    await expect(chartMenu(page)).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(chartMenu(page)).toHaveCount(0);

    await rightClick(page, A);
    const p = await viewportPoint(page, { x: 900, y: 300 });
    await page.mouse.click(p.x, p.y);
    await expect(chartMenu(page)).toHaveCount(0);

    await drawTrendLine(page, A, B);
    await expect.poll(async () => (await drawings(page)).length).toBe(1);
    await leftClick(page, MID);
    const styleBar = page.getByRole("toolbar", { name: /style/i });
    await expect(styleBar).toBeVisible();
    await rightClick(page, { x: 900, y: 300 });
    await expect(chartMenu(page)).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(chartMenu(page)).toHaveCount(0);
    await expect(styleBar).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(styleBar).toHaveCount(0);

    // A legend row's menu leaves a tool armed; closing it must too.
    const tool = page.getByRole("button", { name: "Trend line" });
    await tool.click();
    await page.getByTestId("indicator-legend-rsi").click({ button: "right", position: { x: 10, y: 8 } });
    await expect(menu(page, /^RSI.* actions$/)).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu")).toHaveCount(0);
    await expect(tool).toHaveAttribute("aria-pressed", "true");
  });

  test.describe("a click that closes it does nothing else", () => {
    const current = (page: Page) => page.locator('.wl-row[aria-current="true"]');
    const row = (page: Page, sym: string) => page.locator(`.wl-row[data-symbol="${sym}"]`);
    const replayMode = (page: Page) =>
      page.evaluate(() => (window as unknown as { __chartgenTest: { replay(): { mode: string } } }).__chartgenTest.replay().mode);
    const openChartMenu = async (page: Page) => {
      await rightClick(page, A);
      await expect(chartMenu(page)).toBeVisible();
    };

    test("no replay start", async ({ bootedApp: page }) => {
      await page.getByRole("button", { name: /replay/i }).first().click();
      await page.getByTestId("replay-select-bar").click();
      await expect(page.getByTestId("replay-cutline")).toBeVisible();
      await openChartMenu(page);
      await leftClick(page, { x: 900, y: 200 });
      await expect(chartMenu(page)).toHaveCount(0);
      expect(await replayMode(page)).toBe("select");
      // Only that press: the next one is the pick.
      await leftClick(page, { x: 900, y: 200 });
      await expect.poll(() => replayMode(page)).toBe("active");
    });

    test("no drawing point", async ({ bootedApp: page }) => {
      const tool = page.getByRole("button", { name: "Trend line" });
      await tool.click();
      await page.getByTestId("indicator-legend-rsi").click({ button: "right", position: { x: 10, y: 8 } });
      await expect(menu(page, /^RSI.* actions$/)).toBeVisible();
      await leftClick(page, A);
      await expect(page.getByRole("menu")).toHaveCount(0);
      await expect(tool).toHaveAttribute("aria-pressed", "true");
      // Two points are still to come: had the closing click counted, this one would finish it.
      await leftClick(page, B);
      expect(await drawings(page)).toHaveLength(0);
      await expect(tool).toHaveAttribute("aria-pressed", "true");
      await leftClick(page, { x: 700, y: 180 });
      await expect.poll(async () => (await drawings(page)).length).toBe(1);
    });

    test("no symbol change from a watchlist row", async ({ bootedApp: page }) => {
      await expect(current(page)).toHaveAttribute("data-symbol", "BTCUSDT");
      await openChartMenu(page);
      await row(page, "ETHUSDT").click();
      await expect(chartMenu(page)).toHaveCount(0);
      await page.waitForTimeout(200);
      await expect(current(page)).toHaveAttribute("data-symbol", "BTCUSDT");
      await row(page, "ETHUSDT").click();
      await expect(current(page)).toHaveAttribute("data-symbol", "ETHUSDT");
    });

    test("no row removed by its ×", async ({ bootedApp: page }) => {
      await openChartMenu(page);
      await row(page, "AAPL").hover();
      const remove = page.getByRole("button", { name: "Remove AAPL from watchlist" });
      await remove.click();
      await expect(chartMenu(page)).toHaveCount(0);
      await page.waitForTimeout(200);
      await expect(row(page, "AAPL")).toBeVisible();
      await remove.click();
      await expect(row(page, "AAPL")).toHaveCount(0);
    });

    test("no stuck state when the press does not end in a click on it", async ({ bootedApp: page }) => {
      const eth = (await row(page, "ETHUSDT").boundingBox())!;
      const onEth = { x: eth.x + 40, y: eth.y + eth.height / 2 };

      // Released somewhere else, and the next click is a click.
      await openChartMenu(page);
      await page.mouse.move(onEth.x, onEth.y);
      await page.mouse.down();
      await page.mouse.move(600, 20, { steps: 3 });
      await page.mouse.up();
      await expect(chartMenu(page)).toHaveCount(0);
      await expect(current(page)).toHaveAttribute("data-symbol", "BTCUSDT");
      await page.mouse.click(onEth.x, onEth.y);
      await expect(current(page)).toHaveAttribute("data-symbol", "ETHUSDT");

      // Never released here: a key ends it, and what follows is no longer stopped.
      const sol = (await row(page, "SOLUSDT").boundingBox())!;
      await openChartMenu(page);
      await page.mouse.move(sol.x + 40, sol.y + sol.height / 2);
      await page.mouse.down();
      await page.keyboard.press("Shift");
      await page.mouse.up();
      await expect(current(page)).toHaveAttribute("data-symbol", "SOLUSDT");

      // Nor does a new press find anything left: as if the release was lost outside the window.
      await openChartMenu(page);
      const aapl = (await row(page, "AAPL").boundingBox())!;
      await page.mouse.move(aapl.x + 40, aapl.y + aapl.height / 2);
      await page.mouse.down();
      await page.evaluate(() =>
        document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "mouse", button: 0 })),
      );
      await page.mouse.up();
      await expect(current(page)).toHaveAttribute("data-symbol", "AAPL");
    });
  });

  test("the keyboard drives it, and focus goes back to the row", async ({ bootedApp: page }) => {
    const row = page.locator('.wl-row[data-symbol="AAPL"]');
    await row.click({ button: "right" });
    await expect(menu(page, "AAPL actions")).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(anyItem(page, /^Open AAPL/)).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(anyItem(page, /^Add alert on AAPL/)).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu")).toHaveCount(0);
    await expect(row).toBeFocused();

    await row.click({ button: "right" });
    await expect(menu(page, "AAPL actions")).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    await expect(page.locator('.wl-row[data-symbol="AAPL"]')).toHaveAttribute("aria-current", "true");
  });

  test("a right click elsewhere moves it; on the menu itself no browser menu opens", async ({ bootedApp: page }) => {
    await watchNativeMenu(page);
    await rightClick(page, A);
    await expect(chartMenu(page)).toBeVisible();
    const first = (await chartMenu(page).boundingBox())!;

    await page.mouse.click(first.x + 20, first.y + 20, { button: "right" });
    expect(await nativeMenu(page)).toBe(false);
    await expect(chartMenu(page)).toBeVisible();

    await rightClick(page, { x: 1000, y: 200 });
    await expect(chartMenu(page)).toBeVisible();
    expect((await chartMenu(page).boundingBox())!.x).not.toBe(first.x);

    await rightClick(page, await onPriceAxis(page, 150));
    await expect(axisMenu(page)).toBeVisible();
    await expect(chartMenu(page)).toHaveCount(0);

    // Clear of the open menu, or the press lands on that.
    await page.locator('.wl-row[data-symbol="AAPL"]').click({ button: "right", position: { x: 300, y: 8 } });
    await expect(menu(page, "AAPL actions")).toBeVisible();
    await expect(axisMenu(page)).toHaveCount(0);
    expect(await nativeMenu(page)).toBe(false);
  });

  test("scrolling closes it: what it was opened on moves away", async ({ bootedApp: page }) => {
    await rightClick(page, A);
    await expect(chartMenu(page)).toBeVisible();
    const p = await viewportPoint(page, { x: 1000, y: 200 });
    await page.mouse.move(p.x, p.y);
    await page.mouse.wheel(0, 120);
    await expect(chartMenu(page)).toHaveCount(0);
  });

  test("it stays on screen near every edge", async ({ bootedApp: page }) => {
    const box = (await page.locator(".chart-container").boundingBox())!;
    const { main } = await paneSizes(page);
    // The chart menu at the pane's bottom corners, the axis menu at the right edge.
    for (const at of [
      { x: 4, y: main.height - 4 },
      { x: main.width - 4, y: main.height - 4 },
    ]) {
      await rightClick(page, at);
      await expect(chartMenu(page)).toBeVisible();
      await expectOnScreen(page, "Chart actions");
      await page.keyboard.press("Escape");
    }
    await page.mouse.click(box.x + box.width - 4, box.y + box.height - 40, { button: "right" });
    await expect(page.getByRole("menu")).toBeVisible();
    await expectOnScreen(page, /actions$/);
    await page.keyboard.press("Escape");

    // A row at the bottom right of the window.
    const last = page.locator(".wl-row[data-symbol]").last();
    await last.click({ button: "right", position: { x: 300, y: 8 } });
    await expect(page.getByRole("menu")).toBeVisible();
    await expectOnScreen(page, /actions$/);
    await page.keyboard.press("Escape");

    // A window too short for the whole menu: it scrolls instead of leaving the screen.
    await page.setViewportSize({ width: 1280, height: 260 });
    await page.waitForTimeout(300);
    await page.mouse.click(600, 200, { button: "right" });
    await expect(page.getByRole("menu")).toBeVisible();
    await expectOnScreen(page, /actions$/);
  });

  test("a drawing's menu near the bottom stays on screen", async ({ bootedApp: page }) => {
    const { main } = await paneSizes(page);
    const low = { x: 400, y: main.height - 6 };
    await page.getByRole("button", { name: "Trend line" }).click();
    await leftClick(page, { x: 200, y: main.height - 6 });
    await leftClick(page, { x: 600, y: main.height - 6 });
    await expect.poll(async () => (await drawings(page)).length).toBe(1);
    await rightClick(page, low);
    await expect(drawingMenu(page)).toBeVisible();
    await expectOnScreen(page, "Drawing actions");
  });
});
