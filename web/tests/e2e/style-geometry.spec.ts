// Style tab: geometry colors (fills, levels, histogram palettes) are
// editable and can be hidden, like TradingView's Style tab. Overrides apply
// on the client without a refetch and persist across reloads.

import type { Page } from "@playwright/test";

import { test, expect, gotoApp, reloadApp } from "./fixtures/app";

type Rgb = [number, number, number];

/** Pixels on the chart canvases whose color is `rgb` (+-`tol`) at any alpha in `alpha`. */
function countPixels(page: Page, rgb: Rgb, alpha: [number, number], tol = 24) {
  return page.evaluate(
    ({ rgb, alpha, tol }) => {
      let n = 0;
      for (const c of document.querySelectorAll<HTMLCanvasElement>(
        ".chart-container canvas",
      )) {
        const ctx = c.getContext("2d");
        if (!ctx || c.width === 0 || c.height === 0) continue;
        const d = ctx.getImageData(0, 0, c.width, c.height).data;
        for (let i = 0; i < d.length; i += 4) {
          if (
            d[i + 3]! >= alpha[0] &&
            d[i + 3]! <= alpha[1] &&
            Math.abs(d[i]! - rgb[0]) <= tol &&
            Math.abs(d[i + 1]! - rgb[1]) <= tol &&
            Math.abs(d[i + 2]! - rgb[2]) <= tol
          ) {
            n++;
          }
        }
      }
      return n;
    },
    { rgb, alpha, tol },
  );
}

async function useIndicators(page: Page, set: unknown[]) {
  await page.addInitScript((s) => {
    if (!localStorage.getItem("chartgen.indicators.active")) {
      localStorage.setItem("chartgen.indicators.active", JSON.stringify(s));
    }
  }, set);
  await gotoApp(page);
}

async function openStyleTab(page: Page, chip: string) {
  await page.getByRole("button", { name: `Settings for ${chip}`, exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByRole("tab", { name: "Style" }).click();
  return dialog;
}

async function pick(page: Page, dialog: ReturnType<Page["getByRole"]>, label: string, color: string) {
  await dialog.getByRole("button", { name: label, exact: true }).click();
  await page.getByLabel(`Color ${color}`).click();
  // Escape closes only the popover; the dialog stays open.
  await page.keyboard.press("Escape");
  await expect(page.getByLabel(`Color ${color}`)).toHaveCount(0);
}

const stored = (page: Page) =>
  page.evaluate(() => JSON.parse(localStorage.getItem("chartgen.indicators.active")!));

const SHOTS = "/tmp/claude-1000/chartgen-shots/style-geometry";

test.describe("geometry colors in the Style tab", () => {
  test("Bollinger fill: recolor, hide, persist, Defaults", async ({ page }) => {
    await useIndicators(page, ["bbands"]);
    const BLUE: Rgb = [33, 150, 243];
    const RED: Rgb = [242, 54, 69];
    const blue = () => countPixels(page, BLUE, [18, 34]);
    const red = () => countPixels(page, RED, [18, 34]);
    await expect.poll(blue, { timeout: 15_000 }).toBeGreaterThan(500);
    expect(await red()).toBeLessThan(50);

    let dialog = await openStyleTab(page, "bbands");
    await expect(dialog.getByLabel("Show Fill 1")).toBeChecked();
    await page.screenshot({ path: `${SHOTS}/style-tab.png` });
    await pick(page, dialog, "Fill 1 color", "#f23645");
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect(dialog).toHaveCount(0);
    await expect.poll(red).toBeGreaterThan(500);
    expect(await blue()).toBeLessThan(150);
    await page.screenshot({ path: `${SHOTS}/fill-recolored.png` });

    // Hide it: neither color remains. Survives a reload.
    dialog = await openStyleTab(page, "bbands");
    await dialog.getByLabel("Show Fill 1").uncheck();
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect.poll(red).toBeLessThan(50);
    expect(await blue()).toBeLessThan(150);
    expect(await stored(page)).toContainEqual({
      name: "bbands",
      style: {
        plots: {},
        geometry: {
          fills: {
            "0": { color: "rgba(242, 54, 69, 0.1)", visible: false, ref: "rgba(33,150,243,0.1)" },
          },
        },
      },
    });
    await reloadApp(page);
    await page.waitForTimeout(1000);
    expect(await red()).toBeLessThan(50);
    expect(await blue()).toBeLessThan(150);

    // Still hidden after live ticks, and the dialog shows the saved state.
    dialog = await openStyleTab(page, "bbands");
    await expect(dialog.getByLabel("Show Fill 1")).not.toBeChecked();
    await dialog.getByRole("button", { name: "Defaults" }).click();
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect.poll(blue).toBeGreaterThan(500);
    expect(await stored(page)).toContainEqual("bbands");
  });

  test("RSI levels: recolor 70, hide 30", async ({ page }) => {
    await useIndicators(page, [{ name: "rsi", length: 14 }]);
    const ORANGE: Rgb = [255, 152, 0];
    const orange = () => countPixels(page, ORANGE, [60, 255], 12);
    await page.waitForTimeout(1500);
    expect(await orange()).toBeLessThan(30);

    let dialog = await openStyleTab(page, "rsi");
    await expect(dialog.getByLabel("Show Level 70")).toBeChecked();
    await expect(dialog.getByLabel("Show Level 30")).toBeChecked();
    await pick(page, dialog, "Level 70 color", "#ff9800");
    await pick(page, dialog, "Level 30 color", "#ff9800");
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect.poll(orange).toBeGreaterThan(300);
    const both = await orange();

    dialog = await openStyleTab(page, "rsi");
    await dialog.getByLabel("Show Level 30").uncheck();
    await dialog.getByLabel("Level 70 line style").selectOption("dashed");
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect.poll(orange).toBeLessThan(both * 0.7);
    expect(await orange()).toBeGreaterThan(100);
    await page.screenshot({ path: `${SHOTS}/rsi-levels.png` });

    await reloadApp(page);
    await expect.poll(orange).toBeLessThan(both * 0.7);
    const entry = (await stored(page)).find((e: { name?: string }) => e.name === "rsi");
    expect(entry.style.geometry.hlines["1"]).toMatchObject({ visible: false });
    expect(entry.style.geometry.hlines["0"]).toMatchObject({ lineStyle: "dashed" });
  });

  test("the color popover survives edits; Escape closes only the popover", async ({ page }) => {
    await useIndicators(page, ["bbands"]);
    const dialog = await openStyleTab(page, "bbands");
    await dialog.getByRole("button", { name: "Fill 1 color", exact: true }).click();
    const slider = page.getByLabel("Opacity");
    await expect(slider).toBeVisible();
    await slider.fill("40");
    await expect(slider.locator("xpath=..").getByText("40%", { exact: true })).toBeVisible();
    await slider.fill("60");
    await expect(slider.locator("xpath=..").getByText("60%", { exact: true })).toBeVisible();
    await page.getByLabel("Color #f23645").click();
    await expect(slider).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(slider).toHaveCount(0);
    await expect(dialog).toBeVisible();
  });

  test("a histogram color follows the server color, not the palette position", async ({
    page,
  }) => {
    await useIndicators(page, ["volume"]);
    const GREEN: Rgb = [38, 166, 154];
    const PINK: Rgb = [233, 30, 99];
    const green = () => countPixels(page, GREEN, [60, 255], 12);
    const pink = () => countPixels(page, PINK, [60, 255], 12);
    await expect.poll(green, { timeout: 15_000 }).toBeGreaterThan(200);

    // Recolor only the second palette color (the one the swatch labels 2).
    const dialog = await openStyleTab(page, "volume");
    await pick(page, dialog, "Volume color 2", "#e91e63");
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect.poll(pink).toBeGreaterThan(100);
    const entry = (await stored(page)).find((e: { name?: string }) => e.name === "volume");
    const palette = entry.style.plots.hist0.palette as Record<string, string>;
    expect(Object.keys(palette)).toHaveLength(1);
    expect(Object.keys(palette)[0]).toMatch(/^rgba\(/);
    expect(entry.style.plots.hist0.color).toBeUndefined();

    // Another window (the first bar, so the server's palette order, differs):
    // the same server color is still pink, the other one stays green.
    for (const tf of ["4h", "15m"]) {
      await page.getByRole("button", { name: tf, exact: true }).click();
      await page.waitForTimeout(1500);
      await expect.poll(pink).toBeGreaterThan(50);
      expect(await green()).toBeGreaterThan(50);
    }
  });

  test("MACD histogram palette and Volume bars recolor",async ({ page }) => {
    await useIndicators(page, ["macd", "volume"]);
    const ORANGE: Rgb = [255, 152, 0];
    const PINK: Rgb = [233, 30, 99];
    const orange = () => countPixels(page, ORANGE, [60, 255], 12);
    const pink = () => countPixels(page, PINK, [60, 255], 12);
    await page.waitForTimeout(1500);

    // MACD: the histogram has a multi-color palette: one swatch per color.
    let dialog = await openStyleTab(page, "macd");
    const swatches = dialog.getByRole("button", { name: /^Histogram color \d+$/ });
    expect(await swatches.count()).toBeGreaterThanOrEqual(2);
    await pick(page, dialog, "Histogram color 1", "#ff9800");
    await pick(page, dialog, "Histogram color 2", "#ff9800");
    await page.screenshot({ path: `${SHOTS}/macd-palette.png` });
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect.poll(orange).toBeGreaterThan(200);

    // Volume: up / down bars.
    dialog = await openStyleTab(page, "volume");
    const vol = dialog.getByRole("button", { name: /^Volume color \d+$/ });
    expect(await vol.count()).toBe(2);
    await pick(page, dialog, "Volume color 1", "#e91e63");
    await pick(page, dialog, "Volume color 2", "#e91e63");
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect.poll(pink).toBeGreaterThan(200);

    await reloadApp(page);
    await expect.poll(pink).toBeGreaterThan(200);
    await expect.poll(orange).toBeGreaterThan(200);
    // Live ticks keep the colors.
    await page.waitForTimeout(3000);
    expect(await pink()).toBeGreaterThan(200);

    dialog = await openStyleTab(page, "volume");
    await dialog.getByRole("button", { name: "Defaults" }).click();
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect.poll(pink).toBeLessThan(30);
  });
});
