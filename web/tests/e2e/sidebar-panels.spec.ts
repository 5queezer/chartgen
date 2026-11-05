// Sidebar Data Window and Object Tree tabs: crosshair values, indicator
// hide/show from the tree, drawing removal and selection, alerts listing.
// Set SIDEBAR_SHOTS_DIR to also write screenshots there.

import type { Page } from "@playwright/test";

import { test, expect } from "./fixtures/app";

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
    await tab(page, "Data Window").click();
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
    await tab(page, "Object Tree").click();
    const row = page.getByTestId("tree-ind-rsi");
    await expect(row).toBeVisible();
    await expect(page.getByTestId("tree-main")).toContainText("BTCUSDT");
    const legend = page.locator(".pane-header", { hasText: "RSI" });
    await expect(legend).toHaveCount(1);
    await shot(page, "object-tree");

    await row.getByRole("button", { name: /^Hide / }).click();
    await expect(legend).toHaveCount(0);
    await expect(row.getByRole("button", { name: /^Show / })).toBeVisible();

    await row.getByRole("button", { name: /^Show / }).click();
    await expect(legend).toHaveCount(1);

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
    await page.setViewportSize({ width: 1280 + 288, height: 720 });
    await drawTrendLine(page);
    await tab(page, "Object Tree").click();
    const row = page.locator("[data-testid^=tree-drawing-]");
    await expect(row).toHaveCount(1);
    await expect(row).toContainText(/^Trend line \d{4}-\d{2}-\d{2}/);
    await shot(page, "object-tree-drawing");

    // Nothing is selected yet, so Delete does nothing.
    await clickAt(page, { x: 900, y: 400 });
    await page.keyboard.press("Delete");
    await expect.poll(() => overlays(page)).toHaveLength(1);

    // Selecting from the tree is what a click on the line does: Delete then removes it.
    await row.click();
    await page.waitForTimeout(300);
    await page.keyboard.press("Delete");
    await expect.poll(() => overlays(page)).toHaveLength(0);
    await expect(row).toHaveCount(0);
  });

  test("Object Tree: drawing hide, lock and remove", async ({ bootedApp: page }) => {
    await page.setViewportSize({ width: 1280 + 288, height: 720 });
    await drawTrendLine(page);
    await tab(page, "Object Tree").click();
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
    await tab(page, "Object Tree").click();
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
    await tab(page, "Data Window").click();
    await page.reload();
    await expect(tab(page, "Data Window")).toHaveAttribute("aria-selected", "true");
    await page.evaluate(() => localStorage.setItem("chartgen.sidebar.tab", "alerts"));
    await page.reload();
    await expect(tab(page, /^Alerts/)).toHaveAttribute("aria-selected", "true");
  });
});
