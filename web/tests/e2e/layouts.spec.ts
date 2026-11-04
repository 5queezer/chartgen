// Named layouts (ADR-0015) — create, switch, persist, rename, delete,
// the unsaved-changes dot and Ctrl+S.

import type { Page } from "@playwright/test";

import { test, expect } from "./fixtures/app";

const A = { x: 300, y: 160 };
const B = { x: 560, y: 260 };

/**
 * Click with real mouse movement first: klinecharts only advances a
 * drawing on mouse-move + click, not on a teleporting click.
 */
async function clickAt(page: Page, at: { x: number; y: number }) {
  const box = await page.locator(".chart-container").boundingBox();
  if (!box) throw new Error("chart container not laid out");
  await page.mouse.move(box.x + at.x, box.y + at.y, { steps: 5 });
  await page.mouse.down();
  await page.mouse.up();
  await page.waitForTimeout(700);
}

const overlays = (page: Page) =>
  page.evaluate(() =>
    (
      window as unknown as { __chartgenDrawings: () => unknown[] }
    ).__chartgenDrawings(),
  );

const layoutButton = (page: Page) =>
  page.getByRole("button", { name: /^Layout:/ });
const layoutName = (page: Page) => page.getByTestId("layout-name");
const dirtyDot = (page: Page) => page.getByTestId("layout-dirty");
const activeTimeframe = (page: Page) =>
  page.getByRole("group", { name: "Timeframe" }).locator("[aria-pressed=true]");
const chip = (page: Page, name: string) =>
  page.getByRole("button", { name: `Remove ${name}` });

async function menu(page: Page, item: string | RegExp) {
  await layoutButton(page).click();
  await page.getByRole("menuitem", { name: item }).click();
}

async function nameDialog(page: Page, name: string) {
  const input = page.getByRole("textbox", { name: "Layout name" });
  await input.fill(name);
  await input.press("Enter");
  await expect(page.getByRole("dialog")).toHaveCount(0);
}

async function load(page: Page, name: string) {
  await menu(page, /^Load layout/);
  await page.getByRole("button", { name: new RegExp(`^${name}`) }).click();
}

async function drawTrendLine(page: Page) {
  await page.getByRole("button", { name: "Trend line" }).click();
  await clickAt(page, A);
  await clickAt(page, B);
  await expect.poll(() => overlays(page)).toHaveLength(1);
}

test.describe("layouts", () => {
  test("fresh install starts with a clean Unnamed layout", async ({
    bootedApp: page,
  }) => {
    await expect(layoutName(page)).toHaveText("Unnamed");
    await expect(dirtyDot(page)).toHaveCount(0);
  });

  test("dirty dot appears on change and Ctrl+S clears it with focus on the chart", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: "15m", exact: true }).click();
    await expect(dirtyDot(page)).toBeVisible();

    await page.locator(".chart-container").click({ position: A });
    let downloads = 0;
    page.on("download", () => downloads++);
    await page.keyboard.press("Control+s");
    await expect(dirtyDot(page)).toHaveCount(0);
    expect(downloads).toBe(0);

    await page.reload();
    await expect(layoutName(page)).toHaveText("Unnamed");
    await expect(activeTimeframe(page)).toHaveText("15m");
    await expect(dirtyDot(page)).toHaveCount(0);
  });

  test("create, switch, unsaved prompt, persist, rename, delete", async ({
    bootedApp: page,
  }) => {
    // Layout A: default indicators, a drawing, 1h.
    await drawTrendLine(page);
    await expect(dirtyDot(page)).toBeVisible();
    await page.keyboard.press("Control+s");
    await expect(dirtyDot(page)).toHaveCount(0);
    await menu(page, /^Rename/);
    await nameDialog(page, "Alpha");
    await expect(layoutName(page)).toHaveText("Alpha");

    // Layout B: no indicators, no drawings, 15m.
    await menu(page, /^Create new layout/);
    await nameDialog(page, "Beta");
    await expect(layoutName(page)).toHaveText("Beta");
    await expect(chip(page, "ema_stack")).toHaveCount(0);
    await expect.poll(() => overlays(page)).toHaveLength(0);
    await page.getByRole("button", { name: "15m", exact: true }).click();
    await expect(dirtyDot(page)).toBeVisible();
    await page.keyboard.press("Control+s");
    await expect(dirtyDot(page)).toHaveCount(0);

    // Back to A: everything restored, drawing included.
    await load(page, "Alpha");
    await expect(layoutName(page)).toHaveText("Alpha");
    await expect(activeTimeframe(page)).toHaveText("1h");
    await expect(chip(page, "ema_stack")).toBeVisible();
    await expect.poll(() => overlays(page)).toHaveLength(1);
    await expect(dirtyDot(page)).toHaveCount(0);

    // Leaving a dirty layout asks first; Cancel stays put.
    await page.getByRole("button", { name: "4h", exact: true }).click();
    await expect(dirtyDot(page)).toBeVisible();
    await load(page, "Beta");
    const prompt = page.getByRole("dialog");
    await expect(prompt).toContainText("Unsaved changes");
    await prompt.getByRole("button", { name: "Cancel" }).click();
    await expect(layoutName(page)).toHaveText("Alpha");
    await expect(activeTimeframe(page)).toHaveText("4h");

    // Don't save: the 4h change is dropped when coming back.
    await load(page, "Beta");
    await page.getByRole("dialog").getByRole("button", { name: "Don't save" }).click();
    await expect(layoutName(page)).toHaveText("Beta");
    await expect(activeTimeframe(page)).toHaveText("15m");
    await expect(chip(page, "ema_stack")).toHaveCount(0);
    await expect.poll(() => overlays(page)).toHaveLength(0);
    await load(page, "Alpha");
    await expect(activeTimeframe(page)).toHaveText("1h");

    // Save in the prompt keeps the change.
    await page.getByRole("button", { name: "4h", exact: true }).click();
    await load(page, "Beta");
    await page.getByRole("dialog").getByRole("button", { name: "Save", exact: true }).click();
    await expect(layoutName(page)).toHaveText("Beta");
    await load(page, "Alpha");
    await expect(activeTimeframe(page)).toHaveText("4h");
    await expect(dirtyDot(page)).toHaveCount(0);

    // The active layout survives a reload, clean.
    await page.reload();
    await expect(layoutName(page)).toHaveText("Alpha");
    await expect(activeTimeframe(page)).toHaveText("4h");
    await expect(dirtyDot(page)).toHaveCount(0);

    // Rename: a clash gets a suffix.
    await menu(page, /^Rename/);
    await nameDialog(page, "beta");
    await expect(layoutName(page)).toHaveText("beta (2)");
    await menu(page, /^Rename/);
    await nameDialog(page, "Gamma");
    await expect(layoutName(page)).toHaveText("Gamma");

    // Delete the active layout: falls back to the other one.
    await menu(page, /^Delete layout/);
    await page.getByRole("dialog").getByRole("button", { name: "Delete" }).click();
    await expect(layoutName(page)).toHaveText("Beta");
    await expect(activeTimeframe(page)).toHaveText("15m");
    await menu(page, /^Load layout/);
    const list = page.getByRole("dialog");
    await expect(list.getByRole("button", { name: /^Gamma/ })).toHaveCount(0);
    await list.getByRole("textbox", { name: "Search layouts" }).fill("zzz");
    await expect(list).toContainText("No layouts match");
  });
});
