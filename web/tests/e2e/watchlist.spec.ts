// Right-side watchlist — default rows with live quotes (sample source),
// add via the symbol search dialog, row click switches the chart, hover
// remove, and persistence of the list and the collapsed state.

import type { Page } from "@playwright/test";

import { test, expect, expectChartRendered } from "./fixtures/app";

const table = (page: Page) => page.getByRole("table", { name: "Watchlist" });
const row = (page: Page, sym: string) =>
  table(page).locator(`[role=row][data-symbol="${sym}"]`);
const rows = (page: Page) => table(page).locator("[role=row][data-symbol]");
const lastCell = (page: Page, sym: string) =>
  row(page, sym).getByRole("cell").nth(1);
const symbolButton = (page: Page) =>
  page.getByRole("button", { name: /^Symbol:/ });

const NUMBER = /^[+−]?\d+\.\d+%?$/;

test.describe("watchlist", () => {
  test("shows the default rows with numbers that tick over time", async ({
    bootedApp: page,
  }) => {
    await expect(rows(page)).toHaveCount(7);
    await expect(rows(page).first()).toHaveAttribute("data-symbol", "BTCUSDT");
    await expect(lastCell(page, "BTCUSDT")).toHaveText(NUMBER, {
      timeout: 15_000,
    });
    for (const sym of ["ETHUSDT", "AAPL", "SPY"]) {
      const cells = row(page, sym).getByRole("cell");
      for (const i of [1, 2, 3]) await expect(cells.nth(i)).toHaveText(NUMBER);
    }
    // Change cells are colored by direction.
    await expect(row(page, "AAPL").getByRole("cell").nth(2)).toHaveClass(
      /color-(bull|bear)/,
    );

    // The sample source moves every second; the 5 s poll picks it up and
    // flashes the Last cell.
    const first = await lastCell(page, "BTCUSDT").textContent();
    await expect(lastCell(page, "BTCUSDT")).not.toHaveText(first ?? "", {
      timeout: 15_000,
    });
    await expect(table(page).locator("[data-flash]").first()).toBeAttached({
      timeout: 15_000,
    });
  });

  test("add via search, click switches chart, remove, reload persists", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: "Add symbol" }).click();
    const input = page.getByRole("dialog").getByRole("combobox");
    await expect(input).toBeFocused();
    await input.fill("doge");
    await page.keyboard.press("Enter");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(rows(page)).toHaveCount(8);
    await expect(rows(page).last()).toHaveAttribute("data-symbol", "DOGEUSDT");
    await expect(lastCell(page, "DOGEUSDT")).toHaveText(NUMBER, {
      timeout: 15_000,
    });
    // The chart symbol is unchanged by adding.
    await expect(symbolButton(page)).toContainText("BTCUSDT");
    await expect(row(page, "BTCUSDT")).toHaveAttribute("aria-current", "true");

    await row(page, "ETHUSDT").click();
    await expect(symbolButton(page)).toContainText("ETHUSDT");
    await expect(row(page, "ETHUSDT")).toHaveAttribute("aria-current", "true");
    await expect(row(page, "BTCUSDT")).not.toHaveAttribute("aria-current");
    await expectChartRendered(page);

    await row(page, "AAPL").hover();
    await page
      .getByRole("button", { name: "Remove AAPL from watchlist" })
      .click();
    await expect(row(page, "AAPL")).toHaveCount(0);
    // Removing never switches the chart.
    await expect(symbolButton(page)).toContainText("ETHUSDT");

    await page.reload();
    await expectChartRendered(page);
    await expect(rows(page)).toHaveCount(7);
    await expect(row(page, "DOGEUSDT")).toHaveCount(1);
    await expect(row(page, "AAPL")).toHaveCount(0);
    await expect(row(page, "ETHUSDT")).toHaveAttribute("aria-current", "true");
  });

  test("collapse persists across reload", async ({ bootedApp: page }) => {
    await page.getByRole("button", { name: "Collapse watchlist" }).click();
    await expect(table(page)).toHaveCount(0);
    const expand = page.getByRole("button", { name: "Expand watchlist" });
    await expect(expand).toBeVisible();

    await page.reload();
    await expectChartRendered(page);
    await expect(expand).toBeVisible();
    await expect(table(page)).toHaveCount(0);

    await expand.click();
    await expect(rows(page)).toHaveCount(7);
    await expect(lastCell(page, "BTCUSDT")).toHaveText(NUMBER, {
      timeout: 15_000,
    });
  });
});
