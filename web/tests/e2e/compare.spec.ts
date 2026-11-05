// Compare symbols (ADR-0022): add one from the toolbar, the price axis
// switches to percent and comes back when the last one is removed, the
// legend row shows value and change, the list survives a reload and a
// timeframe switch refetches it. Sample data source, so BTCUSDT and
// ETHUSDT both have bars offline.

import type { Page, Request } from "@playwright/test";

import { test, expect } from "./fixtures/app";

const scaleButton = (page: Page, name: string) =>
  page
    .getByRole("group", { name: "Price scale" })
    .getByRole("button", { name, exact: true });
const legend = (page: Page, sym: string) =>
  page.getByTestId(`compare-legend-${sym}`);
const dialog = (page: Page) => page.getByRole("dialog");

async function addCompare(page: Page, query: string): Promise<void> {
  await page.getByRole("button", { name: "Compare symbol" }).click();
  await dialog(page).getByRole("combobox").fill(query);
  await expect(page.getByRole("option").first()).toContainText(query);
  await page.keyboard.press("Enter");
  await expect(dialog(page)).toHaveCount(0);
}

/** generate_chart calls for `ticker` from now on (resolves with `timeframe`). */
function chartCalls(page: Page, ticker: string) {
  const calls: Array<{ timeframe?: string; bars?: number; end_time?: number }> =
    [];
  page.on("request", (req: Request) => {
    if (req.method() !== "POST" || !req.url().endsWith("/mcp")) return;
    try {
      const b = req.postDataJSON() as {
        method?: string;
        params?: {
          name?: string;
          arguments?: {
            ticker?: string;
            timeframe?: string;
            bars?: number;
            end_time?: number;
          };
        };
      };
      const a = b.params?.arguments;
      if (
        b.method === "tools/call" &&
        b.params?.name === "generate_chart" &&
        a?.ticker === ticker
      ) {
        calls.push(a);
      }
    } catch {
      // Not JSON: not ours.
    }
  });
  return calls;
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
    await expect(legend(page, "ETHUSDT")).toHaveText(VALUE_ROW, {
      timeout: 15_000,
    });

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
    await expect(legend(page, "ETHUSDT")).toHaveText(VALUE_ROW, {
      timeout: 15_000,
    });
    await expect(legend(page, "SOLUSDT")).toHaveText(VALUE_ROW, {
      timeout: 15_000,
    });

    await page.reload();
    await expect(legend(page, "ETHUSDT")).toHaveText(VALUE_ROW, {
      timeout: 30_000,
    });
    await expect(legend(page, "SOLUSDT")).toHaveText(VALUE_ROW);
    await expect(scaleButton(page, "%")).toHaveAttribute("aria-pressed", "true");
  });

  test("a timeframe switch refetches the compare symbol", async ({
    bootedApp: page,
  }) => {
    const calls = chartCalls(page, "ETHUSDT");
    await addCompare(page, "ETHUSDT");
    await expect(legend(page, "ETHUSDT")).toHaveText(VALUE_ROW, {
      timeout: 15_000,
    });
    await expect.poll(() => calls.some((c) => c.timeframe === "1h" && c.bars! > 2)).toBe(true);

    await page.getByRole("button", { name: "15m", exact: true }).click();
    await expect
      .poll(() => calls.some((c) => c.timeframe === "15m" && c.bars! > 2), {
        timeout: 15_000,
      })
      .toBe(true);
    await expect(legend(page, "ETHUSDT")).toHaveText(VALUE_ROW, {
      timeout: 15_000,
    });
  });

  test("the compare symbol follows live ticks", async ({ bootedApp: page }) => {
    const calls = chartCalls(page, "ETHUSDT");
    await addCompare(page, "ETHUSDT");
    await expect(legend(page, "ETHUSDT")).toHaveText(VALUE_ROW, {
      timeout: 15_000,
    });
    // 24/7 symbol: tail polls of 2 bars every 2 s.
    await expect
      .poll(() => calls.filter((c) => c.bars === 2).length, { timeout: 15_000 })
      .toBeGreaterThanOrEqual(2);
  });

  test("making a compare symbol the main symbol removes it from the list", async ({
    bootedApp: page,
  }) => {
    await addCompare(page, "ETHUSDT");
    await expect(legend(page, "ETHUSDT")).toHaveText(VALUE_ROW, {
      timeout: 15_000,
    });
    await page.getByRole("button", { name: /^Symbol:/ }).click();
    await dialog(page).getByRole("combobox").fill("ETHUSDT");
    await expect(page.getByRole("option").first()).toContainText("ETHUSDT");
    await page.keyboard.press("Enter");
    await expect(legend(page, "ETHUSDT")).toHaveCount(0);
    await expect(scaleButton(page, "%")).toHaveAttribute("aria-pressed", "false");
  });
});
