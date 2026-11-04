// Symbol search against the sample data source (offline): results come from
// the server's `search_symbols` tool, category tabs filter by type, and the
// watchlist "+" dialog uses the same search.

import type { Page, Response } from "@playwright/test";

import { test, expect, expectChartRendered } from "./fixtures/app";

const symbolButton = (page: Page) =>
  page.getByRole("button", { name: /^Symbol:/ });
const dialog = (page: Page) => page.getByRole("dialog");
const options = (page: Page) => dialog(page).getByRole("option");

function toolCall(page: Page, tool: string, ticker?: string) {
  return page.waitForResponse(
    (res: Response) => {
      if (res.request().method() !== "POST" || !res.url().endsWith("/mcp")) {
        return false;
      }
      try {
        const b = res.request().postDataJSON() as {
          params?: { name?: string; arguments?: { ticker?: string } };
        };
        return (
          b.params?.name === tool &&
          (ticker === undefined || b.params?.arguments?.ticker === ticker)
        );
      } catch {
        return false;
      }
    },
    { timeout: 30_000 },
  );
}

test.describe("symbol search", () => {
  test("shows server results and selects one with the keyboard", async ({
    bootedApp: page,
  }) => {
    await symbolButton(page).click();
    const input = dialog(page).getByRole("combobox");
    await input.fill("eth");

    // ETH / BTC exists only in the server's catalogue, not the curated list.
    const pair = options(page).filter({ hasText: "ETH / BTC" });
    await expect(pair).toBeVisible();
    await expect(pair).toContainText("ETHBTC");
    await expect(pair).toContainText("Binance");
    await expect(pair).toContainText("crypto");
    await expect(options(page).first()).toContainText("ETHUSDT");

    // Typing stays possible while results show.
    await expect(input).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(input).toHaveAttribute("aria-activedescendant", "symbol-row-1");
    await expect(pair).toHaveAttribute("aria-selected", "true");

    const call = toolCall(page, "generate_chart", "ETHBTC");
    await page.keyboard.press("Enter");
    await call;
    await expect(symbolButton(page)).toContainText("ETHBTC");
    await expect(dialog(page)).toHaveCount(0);
    await expectChartRendered(page);
  });

  test("category tabs filter results by type", async ({ bootedApp: page }) => {
    await symbolButton(page).click();
    await dialog(page).getByRole("combobox").fill("eur");

    const crypto = options(page).filter({ hasText: "BTCEUR" });
    const forex = options(page).filter({ hasText: "EURUSD=X" });
    await expect(crypto).toBeVisible();
    await expect(forex).toBeVisible();

    const search = toolCall(page, "search_symbols");
    await dialog(page).getByRole("tab", { name: "Forex" }).click();
    await search;
    await expect(dialog(page).getByRole("tab", { name: "Forex" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(forex).toBeVisible();
    await expect(crypto).toHaveCount(0);
    await expect(dialog(page).getByRole("combobox")).toBeFocused();

    await dialog(page).getByRole("tab", { name: "Funds" }).click();
    await expect(forex).toHaveCount(0);

    await dialog(page).getByRole("tab", { name: "All" }).click();
    await expect(crypto).toBeVisible();
  });

  test("empty query keeps the curated list; unknown text stays pickable", async ({
    bootedApp: page,
  }) => {
    await symbolButton(page).click();
    await expect(dialog(page).getByText("Popular")).toBeVisible();
    await expect(options(page).first()).toBeVisible();

    await dialog(page).getByRole("combobox").fill("qqzz");
    await expect(options(page)).toHaveCount(1);
    await expect(options(page).first()).toContainText('Use "QQZZ" as typed');
  });

  test("watchlist + adds a server-found symbol", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: "Add symbol" }).click();
    const input = dialog(page).getByRole("combobox");
    await input.fill("vti");
    // VTI is not in the curated list: this row comes from the server.
    const vti = options(page).filter({ hasText: "Vanguard Total Stock Market" });
    await expect(vti).toBeVisible();
    await page.keyboard.press("Enter");

    await expect(dialog(page)).toHaveCount(0);
    const table = page.getByRole("table", { name: "Watchlist" });
    await expect(
      table.locator("[role=row][data-symbol]").last(),
    ).toHaveAttribute("data-symbol", "VTI");
    // The chart symbol is unchanged by adding.
    await expect(symbolButton(page)).toContainText("BTCUSDT");
  });
});
