// Indicator settings "Style" tab (ADR-0017): per-plot color / visibility,
// applied client-side without refetching and persisted across reloads.

import type { Page } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

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
    // Full chart loads only: the live feed keeps polling the tail (and a
    // catch-up after a stall polls a longer one) whatever the style does.
    // A full load asks for INITIAL_BARS (500, src/lib/liveFeed.ts) with no
    // end_time.
    const chartRequests: string[] = [];
    page.on("request", (req) => {
      try {
        const b = req.postDataJSON() as {
          params?: {
            name?: string;
            arguments?: { bars?: number; end_time?: number };
          };
        } | null;
        const a = b?.params?.arguments;
        if (
          b?.params?.name === "generate_chart" &&
          a?.bars === 500 &&
          a.end_time === undefined
        ) {
          chartRequests.push(req.url());
        }
      } catch {
        // Not a JSON body: not an MCP call.
      }
    });

    const legendValue = rsiRow(page).locator(".pane-header-value");
    await expect(legendValue).toHaveCount(1);
    const before = await legendValue.evaluate((el) => getComputedStyle(el).color);

    let dialog = await openStyleTab(page);
    await dialog.getByRole("button", { name: "RSI color", exact: true }).click();
    await page.getByLabel("Color #f23645").click();
    await page.screenshot({ path: "/tmp/claude-1000/chartgen-shots/style/style-tab.png" });
    await dialog.getByRole("tab", { name: "Style" }).click();
    await dialog.getByLabel("RSI width").selectOption("3");
    await dialog.getByLabel("RSI line style").selectOption("dashed");
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect(dialog).toHaveCount(0);

    // The preset keeps the opacity of RSI's server color (0.8, ADR-0026).
    await expect(legendValue).toHaveCSS("color", "rgba(242, 54, 69, 0.8)");
    await page.screenshot({ path: "/tmp/claude-1000/chartgen-shots/style/applied.png" });
    expect(before).not.toBe("rgba(242, 54, 69, 0.8)");
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
          "l:RSI": {
            color: "rgba(242, 54, 69, 0.8)",
            width: 3,
            lineStyle: "dashed",
            visible: false,
          },
        },
      },
    });
    await reloadApp(page);
    await expect(rsiRow(page)).toBeVisible();
    await expect(legendValue).toHaveCount(0);

    // Defaults on the Style tab resets the overrides.
    dialog = await openStyleTab(page);
    await dialog.getByRole("button", { name: "Defaults" }).click();
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect(legendValue).toHaveCount(1);
    await expect(legendValue).toHaveCSS("color", before);
  });

  test("Cancel discards style edits", async ({ bootedApp: page }) => {
    const legendValue = rsiRow(page).locator(".pane-header-value");
    const before = await legendValue.evaluate((el) => getComputedStyle(el).color);

    const dialog = await openStyleTab(page);
    await dialog.getByRole("button", { name: "RSI color", exact: true }).click();
    await page.getByLabel("Color #f23645").click();
    await dialog.getByRole("tab", { name: "Style" }).click();
    await dialog.getByLabel("RSI width").selectOption("4");
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toHaveCount(0);

    await expect(legendValue).toHaveCSS("color", before);
    const stored = await page.evaluate(() =>
      localStorage.getItem("chartgen.indicators.active"),
    );
    expect(stored ?? "").not.toContain("style");

    const reopened = await openStyleTab(page);
    // RSI's server width (ADR-0026).
    await expect(reopened.getByLabel("RSI width")).toHaveValue("2");
  });

  test("reopening shows the saved width and line style", async ({
    bootedApp: page,
  }) => {
    let dialog = await openStyleTab(page);
    // Not 2: that is RSI's server width, so it would read back unsaved too.
    await dialog.getByLabel("RSI width").selectOption("3");
    await dialog.getByLabel("RSI line style").selectOption("dotted");
    await dialog.getByRole("button", { name: "OK" }).click();
    await expect(dialog).toHaveCount(0);

    dialog = await openStyleTab(page);
    await expect(dialog.getByLabel("RSI width")).toHaveValue("3");
    await expect(dialog.getByLabel("RSI line style")).toHaveValue("dotted");
    await expect(dialog.getByRole("button", { name: "Defaults" })).toBeEnabled();
  });
});
