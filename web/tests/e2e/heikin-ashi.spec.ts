// Heikin Ashi chart type. The transform runs server-side (ADR-0003): the
// client only sends `chart_style` with every generate_chart request and
// renders the returned bars. The suite's server serves deterministic
// sample bars; BTCUSDT is 24/7, so the tail is polled every 2 s.

import type { Page, Request } from "@playwright/test";

import { test, expect, expectChartRendered } from "./fixtures/app";

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

function recordCalls(page: Page): ChartArgs[] {
  const calls: ChartArgs[] = [];
  page.on("request", (req) => {
    const a = chartArgs(req);
    if (a) calls.push(a);
  });
  return calls;
}

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

const fullLoadWith = (calls: ChartArgs[], style: string) =>
  calls.some((c) => c.chart_style === style && c.bars !== 2);

test.describe("heikin ashi", () => {
  test("selecting it reloads with chart_style and changes the legend OHLC", async ({
    bootedApp: page,
  }) => {
    const candlesOpen = await legendOpen(page);
    const calls = recordCalls(page);

    await pickChartType(page, "Heikin Ashi");
    await expect(
      page.getByRole("button", { name: "Chart type: Heikin Ashi" }),
    ).toBeVisible();
    await expect.poll(() => fullLoadWith(calls, "heikin_ashi")).toBe(true);
    await expectChartRendered(page);
    await expect.poll(() => legendOpen(page)).not.toBe(candlesOpen);

    // Back to candles: the cached candle load is adopted (the query key
    // includes the style) and the next tail poll carries the default style.
    calls.length = 0;
    await pickChartType(page, "Candles");
    await expect.poll(() => legendOpen(page)).toBe(candlesOpen);
    await expect
      .poll(
        () => calls.some((c) => c.bars === 2 && c.chart_style === "candles"),
        {
          timeout: 15_000,
        },
      )
      .toBe(true);
  });

  test("the style persists across reload and live ticks keep carrying it", async ({
    bootedApp: page,
  }) => {
    await pickChartType(page, "Heikin Ashi");
    await expectChartRendered(page);

    const calls = recordCalls(page);
    await page.reload();
    await expectChartRendered(page);
    await expect(
      page.getByRole("button", { name: "Chart type: Heikin Ashi" }),
    ).toBeVisible();
    expect(calls.find((c) => c.bars !== 2)?.chart_style).toBe("heikin_ashi");

    // The forming bar keeps moving: tail polls carry the style and the
    // rendered close changes.
    const before = await lastClose(page);
    await expect
      .poll(() => calls.filter((c) => c.bars === 2).length, { timeout: 15_000 })
      .toBeGreaterThanOrEqual(2);
    expect(
      calls
        .filter((c) => c.bars === 2)
        .every((c) => c.chart_style === "heikin_ashi"),
    ).toBe(true);
    await expect
      .poll(() => lastClose(page), { timeout: 15_000 })
      .not.toBe(before);
  });
});
