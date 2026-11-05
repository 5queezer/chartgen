// Right-side watchlist — default rows with live quotes (sample source),
// add via the symbol search dialog, row click switches the chart, hover
// remove, and persistence of the list and the collapsed state.

import type { Page, Request } from "@playwright/test";

import { test, expect, expectChartRendered, reloadApp } from "./fixtures/app";

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

    // The sample source moves every second, so each 5 s poll brings a new
    // Last and flashes the cell. Wait on the polls themselves, not on a
    // fixed window: a starved browser or server can delay a poll well past
    // its period. The flash lasts 700 ms, so it is watched in the page on
    // every frame from before the tick.
    const flashed = page.waitForFunction(
      () =>
        document.querySelector(
          '[role="table"][aria-label="Watchlist"] [data-flash]',
        ) !== null,
    );
    const first = (await lastCell(page, "BTCUSDT").textContent()) ?? "";
    await expect(async () => {
      await page.waitForResponse((res) => isQuotesRequest(res.request()));
      await expect(lastCell(page, "BTCUSDT")).not.toHaveText(first, {
        timeout: 1_000,
      });
    }).toPass();
    await flashed;
  });

  test("columns stay separate and aligned with large values", async ({
    bootedApp: page,
  }) => {
    const LONG = "VERYLONGSYMBOLNAME1234567890USDT";
    const syms = ["BTCUSDT", LONG, "AAPL"];
    await page.route("**/mcp", async (route) => {
      const req = route.request();
      if (!isQuotesRequest(req)) return route.continue();
      const body = req.postDataJSON() as { id: number };
      const quotes = syms.map((symbol) => ({
        symbol,
        last: 80424.01,
        change: 2062.0,
        change_pct: 2.63,
        prev_close: 78362.01,
        high: 81000,
        low: 78000,
        volume: 1,
        source: "sample",
      }));
      await route.fulfill({
        contentType: "application/json",
        json: {
          jsonrpc: "2.0",
          id: body.id,
          result: {
            content: [{ type: "text", text: JSON.stringify({ quotes }) }],
          },
        },
      });
    });
    await page.evaluate(
      (v) => localStorage.setItem("chartgen.watchlist.symbols", v),
      JSON.stringify(syms),
    );
    await reloadApp(page);
    await expect(lastCell(page, LONG)).toHaveText("80424.01", {
      timeout: 15_000,
    });
    await expect(row(page, "BTCUSDT").getByRole("cell").nth(2)).toHaveText(
      /2062\.00/,
    );

    type Box = { l: number; r: number; fits: boolean };
    const measureAll = (els: Element[]): Box[] =>
      els.map((el) => {
        const b = el.getBoundingClientRect();
        return {
          l: b.left,
          r: b.right,
          fits: el.scrollWidth <= el.clientWidth,
        };
      });
    const header = await table(page)
      .getByRole("columnheader")
      .evaluateAll(measureAll);
    expect(header).toHaveLength(4);

    for (const sym of syms) {
      const r = row(page, sym);
      const [rowBox] = await r.evaluateAll(measureAll);
      const cells = await r.getByRole("cell").evaluateAll(measureAll);
      expect(cells).toHaveLength(4);
      for (const c of cells) {
        expect(c.fits, `${sym}: cell content fits`).toBe(true);
        expect(c.l).toBeGreaterThanOrEqual(rowBox.l - 1);
        expect(c.r).toBeLessThanOrEqual(rowBox.r + 1);
      }
      if (sym !== LONG) {
        // The ellipsis is on the symbol button inside the cell.
        const [label] = await r
          .getByRole("button", { name: /^Show / })
          .evaluateAll(measureAll);
        expect(label.fits, `${sym}: symbol not truncated`).toBe(true);
      }
      for (let i = 1; i < 4; i++) {
        expect(
          cells[i - 1].r,
          `${sym}: column ${i} clear of ${i - 1}`,
        ).toBeLessThanOrEqual(cells[i].l);
        expect(
          Math.abs(cells[i].r - header[i].r),
          `${sym}: header ${i} aligned`,
        ).toBeLessThanOrEqual(1);
      }
    }
  });

  test("the settings icon differs from the theme icon", async ({
    bootedApp: page,
  }) => {
    const svg = (name: RegExp | string) =>
      page.getByRole("button", { name }).locator("svg").innerHTML();
    expect(await svg("Chart settings")).not.toBe(
      await svg(/^Switch to (light|dark) theme$/),
    );
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

    await reloadApp(page);
    await expect(rows(page)).toHaveCount(7);
    await expect(row(page, "DOGEUSDT")).toHaveCount(1);
    await expect(row(page, "AAPL")).toHaveCount(0);
    await expect(row(page, "ETHUSDT")).toHaveAttribute("aria-current", "true");
  });

  test("collapse resizes the chart and persists across reload", async ({
    bootedApp: page,
  }) => {
    // Span of all chart canvases. Not one pane canvas: the split between
    // pane and price axis follows the axis label width, which moves with
    // the ticking price.
    const chartWidth = () =>
      page.locator(".chart-host").evaluate((host) => {
        const rects = Array.from(host.querySelectorAll("canvas"))
          .map((c) => c.getBoundingClientRect())
          .filter((r) => r.width > 0);
        return (
          Math.max(...rects.map((r) => r.right)) -
          Math.min(...rects.map((r) => r.left))
        );
      });
    const expandedWidth = await chartWidth();

    await page.getByRole("button", { name: "Collapse watchlist" }).click();
    await expect(table(page)).toHaveCount(0);
    const expand = page.getByRole("button", { name: "Expand watchlist" });
    await expect(expand).toBeVisible();
    // The chart takes over the panel's width.
    await expect.poll(chartWidth).toBeGreaterThan(expandedWidth + 200);

    await reloadApp(page);
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
