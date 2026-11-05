// Compare symbols (ADR-0022): add one from the toolbar, the price axis
// switches to percent and comes back when the last one is removed, the
// legend row shows value and change, the list survives a reload and a
// timeframe switch refetches it. Sample data source, so BTCUSDT and
// ETHUSDT both have bars offline.

import type { Page, Request } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

const scaleButton = (page: Page, name: string) =>
  page
    .getByRole("group", { name: "Price scale" })
    .getByRole("button", { name, exact: true });
const legend = (page: Page, sym: string) =>
  page.getByTestId(`compare-legend-${sym}`);
const dialog = (page: Page) => page.getByRole("dialog");


interface ChartArgs {
  ticker?: string;
  timeframe?: string;
  bars?: number;
  end_time?: number;
}

/** generate_chart arguments of an MCP request, or null for anything else. */
function chartArgs(req: Request): ChartArgs | null {
  if (req.method() !== "POST" || !req.url().endsWith("/mcp")) return null;
  try {
    const b = req.postDataJSON() as {
      method?: string;
      params?: { name?: string; arguments?: ChartArgs };
    };
    return b.method === "tools/call" && b.params?.name === "generate_chart"
      ? (b.params.arguments ?? {})
      : null;
  } catch {
    return null;
  }
}

/**
 * The server's answer to the next full load of `ticker` (not a tail poll
 * or history page), optionally for one timeframe. Waiting on it instead of
 * a fixed window keeps the specs independent of how slow the machine is.
 */
const loaded = (page: Page, ticker: string, timeframe?: string) =>
  page.waitForResponse((res) => {
    const a = chartArgs(res.request());
    return (
      a?.ticker === ticker &&
      a.end_time === undefined &&
      (a.bars ?? 0) > 2 &&
      (timeframe === undefined || a.timeframe === timeframe)
    );
  });

/** The next live tail poll (2 bars) of `ticker`. */
const tailPolled = (page: Page, ticker: string) =>
  page.waitForRequest((req) => {
    const a = chartArgs(req);
    return a?.ticker === ticker && a.bars === 2 && a.end_time === undefined;
  });

/** Add `ticker` as a compare symbol and wait for the server to answer its load. */
async function addCompare(page: Page, ticker: string): Promise<void> {
  await page.getByRole("button", { name: "Compare symbol" }).click();
  await dialog(page).getByRole("combobox").fill(ticker);
  await expect(page.getByRole("option").first()).toContainText(ticker);
  const load = loaded(page, ticker);
  await page.keyboard.press("Enter");
  await expect(dialog(page)).toHaveCount(0);
  await load;
}

const VALUE_ROW = /\d+\.\d+\s+[+-]\d+\.\d\d%/;

test.describe("compare symbols", () => {
  test("adding switches to percent, removing restores the scale", async ({
    bootedApp: page,
  }) => {
    await expect(scaleButton(page, "%")).toHaveAttribute("aria-pressed", "false");
    await addCompare(page, "ETHUSDT");

    await expect(scaleButton(page, "%")).toHaveAttribute("aria-pressed", "true");
    await expect(legend(page, "ETHUSDT")).toContainText("ETHUSDT");
    await expect(legend(page, "ETHUSDT")).toHaveText(VALUE_ROW);

    await legend(page, "ETHUSDT").hover();
    await legend(page, "ETHUSDT")
      .getByRole("button", { name: "Remove ETHUSDT" })
      .click();
    await expect(legend(page, "ETHUSDT")).toHaveCount(0);
    await expect(scaleButton(page, "%")).toHaveAttribute("aria-pressed", "false");
    expect(
      await page.evaluate(() => localStorage.getItem("chartgen.layout.scaleMode")),
    ).toBe('"normal"');
  });

  test("restores the scale mode that was active before", async ({
    bootedApp: page,
  }) => {
    await scaleButton(page, "log").click();
    await addCompare(page, "ETHUSDT");
    await expect(scaleButton(page, "%")).toHaveAttribute("aria-pressed", "true");
    await legend(page, "ETHUSDT").hover();
    await legend(page, "ETHUSDT")
      .getByRole("button", { name: "Remove ETHUSDT" })
      .click();
    await expect(scaleButton(page, "log")).toHaveAttribute("aria-pressed", "true");
  });

  test("two compare symbols get their own rows; the list persists across reload", async ({
    bootedApp: page,
  }) => {
    await addCompare(page, "ETHUSDT");
    await addCompare(page, "SOLUSDT");
    await expect(legend(page, "ETHUSDT")).toHaveText(VALUE_ROW);
    await expect(legend(page, "SOLUSDT")).toHaveText(VALUE_ROW);

    const reloaded = Promise.all([
      loaded(page, "ETHUSDT"),
      loaded(page, "SOLUSDT"),
    ]);
    await reloadApp(page);
    await reloaded;
    await expect(legend(page, "ETHUSDT")).toHaveText(VALUE_ROW);
    await expect(legend(page, "SOLUSDT")).toHaveText(VALUE_ROW);
    await expect(scaleButton(page, "%")).toHaveAttribute("aria-pressed", "true");
  });

  test("a timeframe switch refetches the compare symbol", async ({
    bootedApp: page,
  }) => {
    const first = loaded(page, "ETHUSDT", "1h");
    await addCompare(page, "ETHUSDT");
    await first;
    await expect(legend(page, "ETHUSDT")).toHaveText(VALUE_ROW);

    const refetched = loaded(page, "ETHUSDT", "15m");
    await page.getByRole("button", { name: "15m", exact: true }).click();
    await refetched;
    await expect(legend(page, "ETHUSDT")).toHaveText(VALUE_ROW);
  });

  test("the compare symbol follows live ticks", async ({ bootedApp: page }) => {
    await addCompare(page, "ETHUSDT");
    await expect(legend(page, "ETHUSDT")).toHaveText(VALUE_ROW);
    // 24/7 symbol: tail polls of 2 bars every 2 s; wait for two of them.
    await tailPolled(page, "ETHUSDT");
    await tailPolled(page, "ETHUSDT");
  });

  test("making a compare symbol the main symbol removes it from the list", async ({
    bootedApp: page,
  }) => {
    await addCompare(page, "ETHUSDT");
    await expect(legend(page, "ETHUSDT")).toHaveText(VALUE_ROW);
    await page.getByRole("button", { name: /^Symbol:/ }).click();
    await dialog(page).getByRole("combobox").fill("ETHUSDT");
    await expect(page.getByRole("option").first()).toContainText("ETHUSDT");
    await page.keyboard.press("Enter");
    await expect(legend(page, "ETHUSDT")).toHaveCount(0);
    await expect(scaleButton(page, "%")).toHaveAttribute("aria-pressed", "false");
  });
});
