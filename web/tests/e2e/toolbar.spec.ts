// Top toolbar — auto-load, timeframe buttons, symbol search keyboard
// flow, type-to-act shortcuts, and layout persistence across reload.

import type { Page, Response } from "@playwright/test";

import { test, expect, expectChartRendered } from "./fixtures/app";

/** Resolve on the next generate_chart tools/call whose args match. */
function chartCall(page: Page, match: { ticker?: string; timeframe?: string }) {
  return page.waitForResponse(
    (res: Response) => {
      if (res.request().method() !== "POST" || !res.url().endsWith("/mcp")) {
        return false;
      }
      try {
        const b = res.request().postDataJSON() as {
          method?: string;
          params?: {
            name?: string;
            arguments?: { ticker?: string; timeframe?: string };
          };
        };
        const a = b.params?.arguments;
        return (
          b.method === "tools/call" &&
          b.params?.name === "generate_chart" &&
          (match.ticker === undefined || a?.ticker === match.ticker) &&
          (match.timeframe === undefined || a?.timeframe === match.timeframe)
        );
      } catch {
        return false;
      }
    },
    { timeout: 30_000 },
  );
}

const symbolButton = (page: Page) =>
  page.getByRole("button", { name: /^Symbol:/ });
const activeTimeframe = (page: Page) =>
  page.getByRole("group", { name: "Timeframe" }).locator("[aria-pressed=true]");

test.describe("toolbar", () => {
  test("timeframe button reloads the chart without a Load click", async ({
    bootedApp: page,
  }) => {
    await expect(activeTimeframe(page)).toHaveText("1h");
    const call = chartCall(page, { timeframe: "15m" });
    await page.getByRole("button", { name: "15m", exact: true }).click();
    await call;
    await expect(activeTimeframe(page)).toHaveText("15m");
    await expectChartRendered(page);
  });

  test("symbol search: type-to-open, filter, arrow keys, Enter, Esc", async ({
    bootedApp: page,
  }) => {
    await page.locator("main").click({ position: { x: 200, y: 200 } });
    await page.keyboard.press("e");
    const input = page.getByRole("dialog").getByRole("combobox");
    await expect(input).toBeFocused();
    await expect(input).toHaveValue("E");

    await page.keyboard.type("th");
    const options = page.getByRole("option");
    await expect(options.first()).toContainText("ETHUSDT");
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowUp");

    const call = chartCall(page, { ticker: "ETHUSDT" });
    await page.keyboard.press("Enter");
    await call;
    await expect(symbolButton(page)).toContainText("ETHUSDT");

    // Esc closes without changing the symbol; free text is uppercased.
    await page.keyboard.press("x");
    await page.keyboard.type("rpusdt");
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(symbolButton(page)).toContainText("ETHUSDT");

    await symbolButton(page).click();
    await page.getByRole("combobox").fill("zzzz");
    const free = chartCall(page, { ticker: "ZZZZ" });
    await page.keyboard.press("Enter");
    await free;
    await expect(symbolButton(page)).toContainText("ZZZZ");
  });

  test("typing digits opens the timeframe popup", async ({
    bootedApp: page,
  }) => {
    await page.locator("main").click({ position: { x: 200, y: 200 } });
    await page.keyboard.press("4");
    await page.keyboard.type("h");
    const call = chartCall(page, { timeframe: "4h" });
    await page.keyboard.press("Enter");
    await call;
    await expect(activeTimeframe(page)).toHaveText("4h");
  });

  test("symbol, timeframe and chart type persist across reload", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: "5m", exact: true }).click();
    await symbolButton(page).click();
    await page.getByRole("combobox").fill("AAPL");
    await page.keyboard.press("Enter");

    await page.getByRole("button", { name: /^Chart type/ }).click();
    await page.getByRole("menuitemradio", { name: "Area" }).click();
    await expect(
      page.getByRole("button", { name: "Chart type: Area" }),
    ).toBeVisible();

    await page.reload();
    await expectChartRendered(page);
    await expect(symbolButton(page)).toContainText("AAPL");
    await expect(activeTimeframe(page)).toHaveText("5m");
    await expect(
      page.getByRole("button", { name: "Chart type: Area" }),
    ).toBeVisible();
  });

  test("Retry refetches after a failed load and the stale chart is cleared", async ({
    bootedApp: page,
  }) => {
    let failNext = true;
    await page.route("**/mcp", async (route) => {
      const body = route.request().postData() ?? "";
      if (failNext && body.includes('"generate_chart"')) {
        failNext = false;
        await route.fulfill({ status: 500, body: "boom" });
        return;
      }
      await route.continue();
    });

    await page.getByRole("button", { name: "5m", exact: true }).click();
    await expect(page.getByText(/^error:/)).toBeVisible({ timeout: 15_000 });
    // The failed key has no data, so the old candles must not linger.
    await expect(page.locator(".pane-header")).toHaveCount(0);

    const call = chartCall(page, { timeframe: "5m" });
    await page.getByRole("button", { name: "Retry" }).click();
    await call;
    await expectChartRendered(page);
    await expect(page.getByText(/^error:/)).toHaveCount(0);
  });

  test("re-picking the active timeframe refetches", async ({
    bootedApp: page,
  }) => {
    const call = chartCall(page, { timeframe: "1h" });
    await page.getByRole("button", { name: "1h", exact: true }).click();
    await call;
  });

  test("focus returns to the symbol button when the dialog closes", async ({
    bootedApp: page,
  }) => {
    await symbolButton(page).click();
    await expect(page.getByRole("combobox")).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(symbolButton(page)).toBeFocused();
  });

  test("failed symbols are not saved as recents", async ({
    bootedApp: page,
  }) => {
    await page.route("**/mcp", async (route) => {
      const body = route.request().postData() ?? "";
      if (body.includes('"generate_chart"') && body.includes('"TYPOUSDT"')) {
        await route.fulfill({ status: 500, body: "boom" });
        return;
      }
      await route.continue();
    });
    await symbolButton(page).click();
    await page.getByRole("combobox").fill("typousdt");
    await page.keyboard.press("Enter");
    await expect(page.getByText(/^error:/)).toBeVisible({ timeout: 15_000 });
    await symbolButton(page).click();
    await expect(page.getByRole("option", { name: /TYPOUSDT/ })).toHaveCount(0);
  });
});
