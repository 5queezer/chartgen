// Indicator settings / visibility / removal via the pane legend.

import type { Page } from "@playwright/test";

import { test, expect, gotoApp, reloadApp } from "./fixtures/app";

const rsiRow = (page: Page) =>
  page.locator(".pane-header", { hasText: "RSI" }).first();

/** Bars RSI draws a value on; null when the chart has no RSI at all. */
const rsiDrawn = (page: Page) =>
  page.evaluate(() => {
    const c = (
      window as unknown as {
        __chartgenTest: {
          chart: () => {
            getIndicatorByPaneId: () => Map<string, Map<string, { result: Record<string, unknown>[] }>>;
          };
        };
      }
    ).__chartgenTest.chart();
    for (const inds of c.getIndicatorByPaneId().values()) {
      const rsi = inds.get("chartgen_rsi");
      if (rsi) return rsi.result.filter((r) => Object.values(r).some((v) => typeof v === "number")).length;
    }
    return null;
  });

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

  test("the legend eye hides an indicator in place, and shows it again", async ({
    bootedApp: page,
  }) => {
    const row = page.getByTestId("indicator-legend-rsi");
    await expect(row.locator(".pane-header-value").first()).toBeVisible();
    await expect.poll(() => rsiDrawn(page)).toBeGreaterThan(0);

    await row.hover();
    await row.getByRole("button", { name: "Hide RSI" }).click();
    // The row stays, dimmed and without values, and its pane stays empty.
    const show = row.getByRole("button", { name: "Show RSI" });
    await expect(show).toHaveAttribute("aria-pressed", "true");
    await expect(row.locator(".pane-header-value")).toHaveCount(0);
    await expect(row.locator(".pane-header-label")).toHaveCSS("opacity", "0.5");
    await expect.poll(() => rsiDrawn(page)).toBe(0);
    // Still requested: the pane and the row need its payload.
    expect(
      await page.evaluate(() =>
        (
          window as unknown as { __chartgenTest: { panes: () => Array<{ indicators: unknown[] }> } }
        ).__chartgenTest.panes()[0]!.indicators,
      ),
    ).toContainEqual({ name: "rsi", length: 14 });
    // Without hover only the crossed-out eye shows.
    await page.mouse.move(0, 0);
    await expect(show).toBeVisible();
    await expect(row.getByRole("button", { name: "Remove RSI" })).toBeHidden();

    await reloadApp(page);
    await expect(show).toBeVisible();
    await expect(row.locator(".pane-header-value")).toHaveCount(0);

    await show.click();
    await expect(row.getByRole("button", { name: "Hide RSI" })).toHaveAttribute("aria-pressed", "false");
    await expect(row.locator(".pane-header-value").first()).toBeVisible();
    await expect.poll(() => rsiDrawn(page)).toBeGreaterThan(0);
  });

  test("remove via the legend drops the indicator and persists", async ({
    bootedApp: page,
  }) => {
    await rsiRow(page).hover();
    await rsiRow(page).getByRole("button", { name: "Remove RSI" }).click();
    await expect(page.locator(".pane-header", { hasText: "RSI" })).toHaveCount(0);

    await reloadApp(page);
    await expect(page.locator(".pane-header", { hasText: "RSI" })).toHaveCount(0);
    expect(await rsiDrawn(page)).toBeNull();
  });

  test("the legend eye keeps keyboard focus across toggles", async ({
    bootedApp: page,
  }) => {
    const eye = rsiRow(page).getByRole("button", { name: /^(Hide|Show) RSI$/ });
    await eye.focus();
    await page.keyboard.press("Enter");
    await expect(eye).toHaveAccessibleName("Show RSI");
    await expect(eye).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(eye).toHaveAccessibleName("Hide RSI");
    await expect(eye).toBeFocused();
  });

  test("an indicator without params says so, and keeps its Style tab", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: /^Settings for EMA/ }).first().click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toContainText("This indicator has no settings.");
    await expect(dialog).not.toContainText("Settings unavailable");
    await dialog.getByRole("tab", { name: "Style" }).click();
    await expect(dialog.getByText("EMA 200")).toBeVisible();
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect(dialog).toHaveCount(0);
  });

  test("a registry that cannot load still says the settings are unavailable", async ({ page }) => {
    await page.route("**/mcp", async (route) => {
      if (route.request().postData()?.includes('"list_indicators"')) await route.abort();
      else await route.fallback();
    });
    await gotoApp(page);
    await page.getByRole("button", { name: /^Settings for EMA/ }).first().click();
    await expect(page.getByRole("dialog")).toContainText("Settings unavailable");
  });
});
