// Funding, Open Interest and Long/Short panels (src/indicators/external.rs).
// The server runs on the sample source, which answers these series without
// Binance: BTCUSDT has them, BTCFDUSD (spot only) has no rows and XERRUSDT
// always fails. Each case checks what the legend says, and that a tail poll
// never turns the panel blank or into a reload loop.

import type { Page } from "@playwright/test";

import { test, expect, gotoApp } from "./fixtures/app";

const SHOTS = "/tmp/claude-1000/chartgen-shots/futures-panels";

async function openWith(page: Page, symbol: string, timeframe: string) {
  await page.addInitScript(
    ([sym, tf]) => {
      localStorage.setItem(
        "chartgen.indicators.active",
        JSON.stringify(["funding", "oi", "long_short"]),
      );
      localStorage.setItem("chartgen.layout.symbol", JSON.stringify(sym));
      localStorage.setItem("chartgen.layout.timeframe", JSON.stringify(tf));
    },
    [symbol, timeframe] as const,
  );
  // Full loads (more bars than a tail poll's 2), counted from the first.
  const loads: number[] = [];
  page.on("request", (req) => {
    if (req.method() !== "POST" || !req.url().endsWith("/mcp")) return;
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
        (a?.bars ?? 0) > 5 &&
        a?.end_time === undefined
      )
        loads.push(Date.now());
    } catch {
      /* not JSON */
    }
  });
  await gotoApp(page);
  return loads;
}

const header = (page: Page, label: string) =>
  page
    .locator(".pane-header")
    .filter({ has: page.locator(".pane-header-label", { hasText: label }) })
    .first();

/** The legend text of each of the three panels, by pane. */
const legends = (page: Page) =>
  page.locator(".pane-header-label").allTextContents();

test.describe("futures panels", () => {
  for (const tf of ["1h", "1d"]) {
    test(`a crypto symbol shows values on ${tf}, and a tail poll keeps them`, async ({
      page,
    }) => {
      const loads = await openWith(page, "BTCUSDT", tf);
      for (const name of ["Funding", "OI", "L/S"]) {
        await expect(header(page, name)).toBeVisible({ timeout: 20_000 });
      }
      // No reason in the label: there is data.
      const labels = await legends(page);
      expect(
        labels.filter(
          (l) =>
            /\(/.test(l) &&
            /Binance|futures market|unavailable|30 days/.test(l),
        ),
      ).toEqual([]);
      // A value is read for each: a number in the legend, where an empty panel shows the
      // label and a dash (`OI —`).
      for (const name of ["Funding", "OI", "L/S"]) {
        await expect
          .poll(async () =>
            (await header(page, name).locator(".pane-header-value").allTextContents()).join(" "),
          )
          .toMatch(/\d/);
      }
      const before = loads.length;
      await page.waitForTimeout(6_000); // three live tail polls
      expect(loads.length).toBe(before);
      for (const name of ["Funding", "OI", "L/S"]) {
        await expect(header(page, name)).toBeVisible();
      }
      await page.screenshot({ path: `${SHOTS}/btcusdt-${tf}.png` });
    });
  }

  for (const [symbol, timeframe, note] of [
    ["AAPL", "1d", "Binance futures only"],
    ["BTCFDUSD", "1h", "no futures market"],
    ["XERRUSDT", "1h", "unavailable"],
  ] as const) {
    test(`${symbol}: the legend says "${note}" and nothing reloads`, async ({
      page,
    }) => {
      const loads = await openWith(page, symbol, timeframe);
      for (const name of ["Funding", "OI", "L/S"]) {
        await expect(header(page, `${name} (${note})`)).toBeVisible({
          timeout: 20_000,
        });
      }
      const before = loads.length;
      await page.waitForTimeout(6_000);
      expect(loads.length).toBe(before);
      await expect(header(page, `Funding (${note})`)).toBeVisible();
      await page.screenshot({
        path: `${SHOTS}/${symbol.toLowerCase()}-${timeframe}.png`,
      });
    });
  }
});
