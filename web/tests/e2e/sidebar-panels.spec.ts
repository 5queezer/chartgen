// Sidebar Data Window and Object Tree tabs: crosshair values, indicator
// hide/show from the tree, drawing removal and selection, alerts listing.
// Set SIDEBAR_SHOTS_DIR to also write screenshots there.

import type { Page } from "@playwright/test";

import { test, expect, reloadApp, openSidebarTab } from "./fixtures/app";

const shot = async (page: Page, name: string) => {
  const dir = process.env.SIDEBAR_SHOTS_DIR;
  if (dir) await page.screenshot({ path: `${dir}/${name}.png` });
};

const host = (page: Page) => page.locator(".chart-container");
const tab = (page: Page, name: string) => page.getByRole("tab", { name });
const dw = (page: Page, id: string) => page.getByTestId(`dw-${id}`);

/** Move the mouse over the chart at a fraction of its width; real steps so klinecharts sees it. */
async function hover(page: Page, fx: number, fy = 0.4): Promise<void> {
  const box = await host(page).boundingBox();
  if (!box) throw new Error("chart container not laid out");
  await page.mouse.move(box.x + box.width * fx, box.y + box.height * fy, { steps: 5 });
}

const overlays = (page: Page) =>
  page.evaluate(() =>
    (
      window as unknown as { __chartgenDrawings: () => { visible: boolean }[] }
    ).__chartgenDrawings(),
  );

async function clickAt(page: Page, at: { x: number; y: number }): Promise<void> {
  const box = await host(page).boundingBox();
  if (!box) throw new Error("chart container not laid out");
  await page.mouse.move(box.x + at.x, box.y + at.y, { steps: 5 });
  await page.mouse.down();
  await page.mouse.up();
  // klinecharts treats two clicks in quick succession as a double click.
  await page.waitForTimeout(700);
}

async function drawTrendLine(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Trend line" }).click();
  await clickAt(page, { x: 300, y: 160 });
  await clickAt(page, { x: 560, y: 260 });
  await expect.poll(() => overlays(page)).toHaveLength(1);
}

test.describe("sidebar panels", () => {
  test("Data Window follows the crosshair and shows the RSI value", async ({
    bootedApp: page,
  }) => {
    await openSidebarTab(page, "Data Window");
    await expect(dw(page, "symbol")).toHaveText("BTCUSDT");
    await expect(dw(page, "time")).not.toHaveText("—");
    const lastBar = await dw(page, "time").textContent();

    await hover(page, 0.6);
    await expect(dw(page, "time")).not.toHaveText(lastBar!);
    const first = {
      time: await dw(page, "time").textContent(),
      o: await dw(page, "o").textContent(),
      c: await dw(page, "c").textContent(),
    };
    for (const k of ["o", "h", "l", "c"]) {
      await expect(dw(page, k)).toHaveText(/^\d+\.\d+$/);
    }
    await expect(dw(page, "ind-rsi").getByText(/^\d+\.\d+$/)).toBeVisible();
    await shot(page, "data-window");

    await hover(page, 0.8);
    await expect(dw(page, "time")).not.toHaveText(first.time!);
    const second = {
      o: await dw(page, "o").textContent(),
      c: await dw(page, "c").textContent(),
    };
    expect([second.o, second.c]).not.toEqual([first.o, first.c]);

    // Off the chart it returns to the last bar.
    await page.mouse.move(5, 5);
    await expect(dw(page, "time")).toHaveText(lastBar!);
  });

  test("Object Tree: hiding an indicator hides its legend", async ({
    bootedApp: page,
  }) => {
    await openSidebarTab(page, "Object Tree");
    const row = page.getByTestId("tree-ind-rsi");
    await expect(row).toBeVisible();
    await expect(page.getByTestId("tree-main")).toContainText("BTCUSDT");
    const legend = page.locator(".pane-header", { hasText: "RSI" });
    await expect(legend).toHaveCount(1);
    await shot(page, "object-tree");

    await row.getByRole("button", { name: /^Hide / }).click();
    await expect(legend).toHaveClass(/pane-header-hidden/);
    await expect(row.getByRole("button", { name: /^Show / })).toBeVisible();
    // A hidden instance keeps its display name, as its legend row does.
    await expect(row).toContainText("RSI");
    await expect(row).not.toContainText("rsi");

    await row.getByRole("button", { name: /^Show / }).click();
    await expect(legend).not.toHaveClass(/pane-header-hidden/);

    await row.getByRole("button", { name: /^Settings for / }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await page.keyboard.press("Escape");

    await row.getByRole("button", { name: /^Remove / }).click();
    await expect(row).toHaveCount(0);
    await expect(legend).toHaveCount(0);
  });

  test("Object Tree: drawings are listed and selectable", async ({
    bootedApp: page,
  }) => {
    await page.setViewportSize({ width: 1280 + 320, height: 720 });
    await drawTrendLine(page);
    await openSidebarTab(page, "Object Tree");
    const row = page.locator("[data-testid^=tree-drawing-]");
    await expect(row).toHaveCount(1);
    await expect(row).toContainText(/^Trend line \d{4}-\d{2}-\d{2}/);
    await shot(page, "object-tree-drawing");

    // Nothing is selected yet, so Delete does nothing.
    await clickAt(page, { x: 900, y: 400 });
    await page.keyboard.press("Delete");
    await expect.poll(() => overlays(page)).toHaveLength(1);

    // A tree selection is ours, not klinecharts': Delete acts on it at once.
    await row.click();
    await page.keyboard.press("Delete");
    await expect.poll(() => overlays(page)).toHaveLength(0);
    await expect(row).toHaveCount(0);
  });

  test("selecting from the tree never alters the stored drawing", async ({
    bootedApp: page,
  }) => {
    await page.setViewportSize({ width: 1280 + 320, height: 720 });
    await drawTrendLine(page);
    await openSidebarTab(page, "Object Tree");
    const stored = () =>
      page.evaluate(() => localStorage.getItem("chartgen.drawings.BTCUSDT"));
    const before = await stored();
    await page.locator("[data-testid^=tree-drawing-]").click();
    // A press on the empty chart ends the selection; the highlight leaves no trace.
    await clickAt(page, { x: 900, y: 400 });
    await page.keyboard.press("Delete");
    await expect.poll(() => overlays(page)).toHaveLength(1);
    expect(await stored()).toBe(before);
  });

  test("selecting a drawing whose point is in the future scrolls to the latest bars", async ({
    bootedApp: page,
  }) => {
    await page.setViewportSize({ width: 1280 + 320, height: 720 });
    await page.getByRole("button", { name: "Trend line" }).click();
    await clickAt(page, { x: 1100, y: 200 });
    await clickAt(page, { x: 700, y: 260 });
    await expect.poll(() => overlays(page)).toHaveLength(1);
    await openSidebarTab(page, "Object Tree");

    const range = () =>
      page.evaluate(() => {
        const t = (
          window as unknown as {
            __chartgenTest: {
              chart: () => {
                getVisibleRange: () => { to: number };
                getDataList: () => unknown[];
              };
            };
          }
        ).__chartgenTest;
        return {
          to: t.chart().getVisibleRange().to,
          last: t.chart().getDataList().length - 1,
        };
      });
    // Drag the view back in time so the future point is off screen.
    const box = (await host(page).boundingBox())!;
    // On the candles: a press on a legend row of a lower pane would not pan.
    await page.mouse.move(box.x + 300, box.y + 300);
    await page.mouse.down();
    await page.mouse.move(box.x + 900, box.y + 300, { steps: 10 });
    await page.mouse.up();
    const away = await range();
    expect(away.to).toBeLessThan(away.last - 20);

    await page.locator("[data-testid^=tree-drawing-]").click();
    await expect
      .poll(async () => (await range()).to)
      .toBeGreaterThanOrEqual(away.last - 1);
  });

  test("Object Tree rows keep focus across drawing commits", async ({
    bootedApp: page,
  }) => {
    await page.setViewportSize({ width: 1280 + 320, height: 720 });
    await drawTrendLine(page);
    await openSidebarTab(page, "Object Tree");
    const row = page.locator("[data-testid^=tree-drawing-]");
    await row.getByRole("button", { name: /^Lock / }).click();
    await expect(row.getByRole("button", { name: /^Unlock / })).toBeFocused();
  });

  test("Object Tree: drawing hide, lock and remove", async ({ bootedApp: page }) => {
    await page.setViewportSize({ width: 1280 + 320, height: 720 });
    await drawTrendLine(page);
    await openSidebarTab(page, "Object Tree");
    const row = page.locator("[data-testid^=tree-drawing-]");
    await expect(row).toHaveCount(1);
    await row.getByRole("button", { name: /^Hide / }).click();
    await expect.poll(async () => (await overlays(page))[0]?.visible).toBe(false);
    await row.getByRole("button", { name: /^Show / }).click();
    await expect.poll(async () => (await overlays(page))[0]?.visible).toBe(true);
    await row.getByRole("button", { name: /^Lock / }).click();
    await expect(row.getByRole("button", { name: /^Unlock / })).toBeVisible();

    await row.getByRole("button", { name: /^Remove / }).click();
    await expect.poll(() => overlays(page)).toHaveLength(0);
    await expect(row).toHaveCount(0);
  });

  test("Object Tree lists the symbol's alerts", async ({ bootedApp: page }) => {
    await openSidebarTab(page, "Object Tree");
    await expect(page.locator("[data-testid^=tree-alert-]")).toHaveCount(0);

    await page.keyboard.press("Alt+KeyA");
    const dialog = page.getByRole("dialog");
    await dialog.getByLabel("Condition").selectOption("greater");
    await dialog.getByLabel("Price").fill("987654");
    await dialog.getByRole("button", { name: "Create" }).click();
    await expect(dialog).toHaveCount(0);

    const row = page.locator("[data-testid^=tree-alert-]");
    await expect(row).toHaveCount(1);
    await expect(row).toContainText("Greater than");
    await expect(row).toContainText("987654");

    await row.getByRole("button", { name: "Delete alert" }).click();
    await expect(row).toHaveCount(0);
  });

  test("the selected tab is remembered, and an old stored value still works", async ({
    bootedApp: page,
  }) => {
    await openSidebarTab(page, "Data Window");
    await reloadApp(page);
    await expect(tab(page, "Data Window")).toHaveAttribute("aria-selected", "true");
    await page.evaluate(() => localStorage.setItem("chartgen.sidebar.tab", "alerts"));
    await reloadApp(page);
    await expect(tab(page, /^Alerts/)).toHaveAttribute("aria-selected", "true");
  });
});
