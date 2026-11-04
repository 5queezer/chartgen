// Indicator settings "Style" tab (ADR-0017): per-plot color / visibility,
// applied client-side without refetching and persisted across reloads.

import type { Page } from "@playwright/test";

import { test, expect, expectChartRendered } from "./fixtures/app";

const rsiRow = (page: Page) =>
  page.locator(".pane-header", { hasText: "RSI" }).first();

async function openStyleTab(page: Page) {
  await rsiRow(page).hover();
  await rsiRow(page).getByRole("button", { name: "Settings for RSI" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByRole("tab", { name: "Style" }).click();
  return dialog;
}

test.describe("indicator style tab", () => {
  test("color and visibility apply without a refetch and persist", async ({
    bootedApp: page,
  }) => {
    const chartRequests: string[] = [];
    page.on("request", (req) => {
      if ((req.postData() ?? "").includes('"generate_chart"')) {
        chartRequests.push(req.url());
      }
    });

    const legendValue = rsiRow(page).locator(".pane-header-value");
    await expect(legendValue).toHaveCount(1);
    const before = await legendValue.evaluate((el) => getComputedStyle(el).color);

    let dialog = await openStyleTab(page);
    await dialog.getByRole("button", { name: /color$/ }).click();
    await page.getByLabel("Color #f23645").click();
    await page.screenshot({ path: "/tmp/claude-1000/chartgen-shots/style/style-tab.png" });
    await page.keyboard.press("Escape");
    await dialog.getByLabel("RSI width").selectOption("3");
    await dialog.getByLabel("RSI line style").selectOption("dashed");
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect(dialog).toHaveCount(0);

    await expect(legendValue).toHaveCSS("color", "rgb(242, 54, 69)");
    await page.screenshot({ path: "/tmp/claude-1000/chartgen-shots/style/applied.png" });
    expect(before).not.toBe("rgb(242, 54, 69)");
    expect(chartRequests).toEqual([]);

    // Hide the plot: its legend value disappears, the legend row stays.
    dialog = await openStyleTab(page);
    await dialog.getByLabel(/^Show /).first().uncheck();
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect(dialog).toHaveCount(0);
    await expect(legendValue).toHaveCount(0);
    await expect(rsiRow(page)).toBeVisible();
    expect(chartRequests).toEqual([]);

    // Persisted shape and reload.
    const stored = await page.evaluate(() =>
      localStorage.getItem("chartgen.indicators.active"),
    );
    expect(JSON.parse(stored!)).toContainEqual({
      name: "rsi",
      length: 14,
      style: {
        plots: {
          line0: { color: "#f23645", width: 3, lineStyle: "dashed", visible: false },
        },
      },
    });
    await page.reload();
    await expectChartRendered(page);
    await expect(rsiRow(page)).toBeVisible();
    await expect(legendValue).toHaveCount(0);

    // Defaults on the Style tab resets the overrides.
    dialog = await openStyleTab(page);
    await dialog.getByRole("button", { name: "Defaults" }).click();
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect(legendValue).toHaveCount(1);
    await expect(legendValue).toHaveCSS("color", before);
  });
});
