// Heikin Ashi chart type. The transform runs server-side (ADR-0003): the
// client only sends `chart_style` with every generate_chart request and
// renders the returned bars. The suite's server serves deterministic
// sample bars; BTCUSDT is 24/7, so the tail is polled every 2 s. Server
// push (ADR-0021) is off so tail polls can be inspected; realtime-push
// covers chart_style on subscribe_chart.

import type { Page, Request } from "@playwright/test";

import {
  test,
  expect,
  expectChartRendered,
  reloadApp,
} from "./fixtures/app";

test.use({ push: false });

interface ChartArgs {
  bars?: number;
  end_time?: number;
  chart_style?: string;
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

/** The next generate_chart request whose arguments match `match`. */
const chartRequest = (page: Page, match: (a: ChartArgs) => boolean) =>
  page.waitForRequest((req) => {
    const a = chartArgs(req);
    return a !== null && match(a);
  });

const isTail = (a: ChartArgs) => a.bars === 2 && a.end_time === undefined;
const isFullLoad = (a: ChartArgs) => !isTail(a) && a.end_time === undefined;

const legendOpen = (page: Page) =>
  page.getByTestId("legend-o").first().innerText();

const lastClose = (page: Page) =>
  page.evaluate(
    () =>
      (
        window as unknown as {
          __chartgenTest: {
            chart: () => { getDataList: () => Array<{ close: number }> } | null;
          };
        }
      ).__chartgenTest
        .chart()
        ?.getDataList()
        .at(-1)?.close ?? null,
  );

async function pickChartType(page: Page, name: string): Promise<void> {
  await page.getByRole("button", { name: /^Chart type/ }).click();
  await page.getByRole("menuitemradio", { name, exact: true }).click();
}

test.describe("heikin ashi", () => {
  test("selecting it reloads with chart_style and changes the legend OHLC", async ({
    bootedApp: page,
  }) => {
    const candlesOpen = await legendOpen(page);

    const haLoad = chartRequest(
      page,
      (a) => isFullLoad(a) && a.chart_style === "heikin_ashi",
    );
    await pickChartType(page, "Heikin Ashi");
    await expect(
      page.getByRole("button", { name: "Chart type: Heikin Ashi" }),
    ).toBeVisible();
    await haLoad;
    await expectChartRendered(page);
    await expect.poll(() => legendOpen(page)).not.toBe(candlesOpen);

    // Back to candles: the cached candle load is adopted (the query key
    // includes the style) and the next tail poll carries the default style.
    const candleTail = chartRequest(
      page,
      (a) => isTail(a) && a.chart_style === "candles",
    );
    await pickChartType(page, "Candles");
    await expect.poll(() => legendOpen(page)).toBe(candlesOpen);
    await candleTail;
  });

  test("the style persists across reload and live ticks keep carrying it", async ({
    bootedApp: page,
  }) => {
    await pickChartType(page, "Heikin Ashi");
    await expectChartRendered(page);

    const load = chartRequest(page, isFullLoad);
    await reloadApp(page);
    await expect(
      page.getByRole("button", { name: "Chart type: Heikin Ashi" }),
    ).toBeVisible();
    expect(chartArgs(await load)?.chart_style).toBe("heikin_ashi");

    // The forming bar keeps moving: tail polls carry the style and the
    // rendered close changes. Each check waits for the next tail poll, not
    // for a fixed window.
    const before = await lastClose(page);
    for (let i = 0; i < 2; i++) {
      const tail = await chartRequest(page, isTail);
      expect(chartArgs(tail)?.chart_style).toBe("heikin_ashi");
    }
    await expect(async () => {
      await page.waitForResponse((res) => {
        const a = chartArgs(res.request());
        return a !== null && isTail(a);
      });
      expect(await lastClose(page)).not.toBe(before);
    }).toPass();
  });
});
