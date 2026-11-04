// Indicator settings / visibility / removal via the pane legend and chips.

import { test, expect, expectChartRendered } from "./fixtures/app";

const rsiRow = (page: import("@playwright/test").Page) =>
  page.locator(".pane-header", { hasText: "RSI" }).first();

test.describe("indicator settings", () => {
  test("gear dialog changes RSI length and the legend follows", async ({
    bootedApp: page,
  }) => {
    await expect(rsiRow(page)).toContainText("(14)");

    await rsiRow(page).hover();
    await rsiRow(page).getByRole("button", { name: "Settings for RSI" }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();

    // Invalid input is rejected inline, Defaults restores the schema default.
    await dialog.getByLabel("length").fill("-3");
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect(dialog.getByRole("alert")).toBeVisible();
    await dialog.getByRole("button", { name: "Defaults" }).click();
    await expect(dialog.getByLabel("length")).toHaveValue("14");

    await dialog.getByLabel("length").fill("21");
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect(dialog).toHaveCount(0);

    await expect(rsiRow(page)).toContainText("(21)");
    const stored = await page.evaluate(() =>
      localStorage.getItem("chartgen.indicators.active"),
    );
    expect(JSON.parse(stored!)).toContainEqual({ name: "rsi", length: 21 });
  });

  test("hide and unhide persist across reload", async ({ bootedApp: page }) => {
    await rsiRow(page).hover();
    await rsiRow(page).getByRole("button", { name: "Hide RSI" }).click();
    await expect(page.locator(".pane-header", { hasText: "RSI" })).toHaveCount(0);

    const show = page.getByRole("button", { name: "Show rsi", exact: true });
    await expect(show).toBeVisible();

    await page.reload();
    await expect(show).toBeVisible();
    await expectChartRendered(page);
    await expect(page.locator(".pane-header", { hasText: "RSI" })).toHaveCount(0);

    await show.click();
    await expect(rsiRow(page)).toBeVisible();
  });

  test("remove via the legend drops the indicator and persists", async ({
    bootedApp: page,
  }) => {
    await rsiRow(page).hover();
    await rsiRow(page).getByRole("button", { name: "Remove RSI" }).click();
    await expect(page.locator(".pane-header", { hasText: "RSI" })).toHaveCount(0);

    const chipRemove = page.getByRole("button", { name: "Remove rsi", exact: true });
    await expect(chipRemove).toHaveCount(0);
    await page.reload();
    await expectChartRendered(page);
    await expect(chipRemove).toHaveCount(0);
    await expect(page.locator(".pane-header", { hasText: "RSI" })).toHaveCount(0);
  });

  test("chip eye button keeps keyboard focus across toggles", async ({
    bootedApp: page,
  }) => {
    const eye = page.getByRole("button", { name: /^(Hide|Show) rsi$/ });
    await eye.focus();
    await page.keyboard.press("Enter");
    await expect(page.locator(".pane-header", { hasText: "RSI" })).toHaveCount(0);
    await expect(eye).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(rsiRow(page)).toBeVisible();
    await expect(eye).toBeFocused();
  });
});
