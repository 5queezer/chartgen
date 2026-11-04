// Right-side watchlist — default rows with live quotes (sample source),
// add via the symbol search dialog, row click switches the chart, hover
// remove, and persistence of the list and the collapsed state.

import type { Page, Request } from "@playwright/test";

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

function isQuotesRequest(req: Request): boolean {
  if (req.method() !== "POST" || !req.url().endsWith("/mcp")) return false;
  try {
    const b = req.postDataJSON() as { params?: { name?: string } };
    return b.params?.name === "get_quotes";
  } catch {
    return false;
  }
}

/** Fake the page visibility the way a tab switch does. */
async function setVisibility(page: Page, state: "visible" | "hidden") {
  await page.evaluate((s) => {
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => s,
    });
    Object.defineProperty(document, "hidden", {
      configurable: true,
      get: () => s === "hidden",
    });
    document.dispatchEvent(new Event("visibilitychange", { bubbles: true }));
  }, state);
}

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

  test("polling pauses while the tab is hidden and resumes when visible", async ({
    bootedApp: page,
  }) => {
    // Hold one quotes response so it completes while the tab is hidden:
    // the case that used to stop polling for good.
    let release!: () => void;
    const held = new Promise<void>((r) => (release = r));
    let holding = true;
    const heldRequest = page.waitForRequest(isQuotesRequest, {
      timeout: 15_000,
    });
    await page.route("**/mcp", async (route) => {
      if (holding && isQuotesRequest(route.request())) {
        holding = false;
        await held;
      }
      await route.continue();
    });
    await heldRequest;
    // The request seen may have been sent before the route existed; wait
    // until one is actually held.
    await expect.poll(() => holding).toBe(false);
    await setVisibility(page, "hidden");
    const heldResponse = page.waitForResponse((r) =>
      isQuotesRequest(r.request()),
    );
    release();
    await heldResponse;

    // Hidden: more than one poll period passes without a request.
    let requests = 0;
    page.on("request", (req) => {
      if (isQuotesRequest(req)) requests++;
    });
    await page.waitForTimeout(6_500);
    expect(requests).toBe(0);

    // Visible again: the next tick polls.
    const resumed = page.waitForRequest(isQuotesRequest, { timeout: 8_000 });
    await setVisibility(page, "visible");
    await resumed;
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

  test("collapse resizes the chart and persists across reload", async ({
    bootedApp: page,
  }) => {
    const chartWidth = () =>
      page
        .locator(".chart-host canvas")
        .first()
        .evaluate((c) => c.getBoundingClientRect().width);
    const expandedWidth = await chartWidth();

    await page.getByRole("button", { name: "Collapse watchlist" }).click();
    await expect(table(page)).toHaveCount(0);
    const expand = page.getByRole("button", { name: "Expand watchlist" });
    await expect(expand).toBeVisible();
    // The chart takes over the panel's width.
    await expect.poll(chartWidth).toBeGreaterThan(expandedWidth + 200);

    await page.reload();
    await expectChartRendered(page);
    await expect(expand).toBeVisible();
    await expect(table(page)).toHaveCount(0);

    await expand.click();
    await expect(rows(page)).toHaveCount(7);
    await expect.poll(chartWidth).toBeCloseTo(expandedWidth, 0);
    await expect(lastCell(page, "BTCUSDT")).toHaveText(NUMBER, {
      timeout: 15_000,
    });
  });
});
