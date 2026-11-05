// Bottom bar: date-range buttons, time zone selector and Go to date. The
// chart is observed through the DEV-only `window.__chartgenTest` hook
// (App.tsx). The server runs on deterministic sample data, so history
// pages exist for any range.

import type { Page, Request } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

interface Hook {
  chart: () => {
    getDataList: () => Array<{ timestamp: number }>;
    getVisibleRange: () => { from: number; to: number };
    getTimezone: () => string;
  } | null;
  series: () => { interval: string | null; bars: Array<{ t: number }> } | null;
}

const hook = (page: Page) =>
  page.evaluate(() => {
    const h = (window as unknown as { __chartgenTest: Hook }).__chartgenTest;
    const c = h.chart();
    const list = c?.getDataList() ?? [];
    const range = c?.getVisibleRange() ?? { from: 0, to: 0 };
    return {
      interval: h.series()?.interval ?? null,
      timezone: c?.getTimezone() ?? null,
      count: list.length,
      range,
      timestamps: list.map((d) => d.timestamp),
    };
  });

/** Time span (ms) between the first and last visible bar. */
async function visibleSpan(page: Page): Promise<number> {
  const h = await hook(page);
  const first = h.timestamps[Math.max(0, h.range.from)]!;
  const last = h.timestamps[Math.min(h.count, h.range.to) - 1]!;
  return last - first;
}

const DAY = 86_400_000;

const utcLabel = (ms: number) => new Date(ms).toISOString().slice(0, 16).replace("T", " ");

interface ChartArgs {
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

/** The server's answer to the next generate_chart call matching `match`. */
const chartResponse = (
  page: Page,
  match: (a: ChartArgs) => boolean = () => true,
) =>
  page.waitForResponse((res) => {
    const a = chartArgs(res.request());
    return a !== null && match(a);
  });

/**
 * Retry `check` after each generate_chart answer until it passes: loads,
 * history pages and the 2 s tail polls of BTCUSDT drive the chart, so the
 * wait follows the server instead of a fixed window.
 */
const afterChartResponses = (page: Page, check: () => Promise<void>) =>
  expect(async () => {
    await chartResponse(page);
    await check();
  }).toPass();

test.describe("bottom bar", () => {
  // `afterChartResponses` retries on generate_chart answers, which tail
  // polls keep coming; with server push (ADR-0021) there are none after
  // the last history page, so these specs run on the polling path.
  test.use({ viewport: { width: 2400, height: 900 }, push: false });

  test("5D switches to 5m and fits about five days", async ({ bootedApp: page }) => {
    const load = chartResponse(
      page,
      (a) => a.timeframe === "5m" && a.bars !== 2 && a.end_time === undefined,
    );
    await page.getByTestId("range-5D").click();
    await load;
    await expect.poll(async () => (await hook(page)).interval).toBe("5m");
    // Loading ends when the range is paged in and fitted.
    await afterChartResponses(page, async () => {
      await expect(page.getByTestId("bottom-notice")).not.toHaveText("Loading…", {
        timeout: 1_000,
      });
      const span = await visibleSpan(page);
      expect(span).toBeGreaterThan(4.5 * DAY);
      expect(span).toBeLessThan(5.5 * DAY);
    });
  });

  test("the time zone moves the legend time and the axis", async ({ bootedApp: page }) => {
    const lastBar = async () => (await hook(page)).timestamps.at(-1)!;
    await page.getByTestId("tz-select").selectOption("utc");
    await expect(page.getByTestId("legend-time")).toHaveText(utcLabel(await lastBar()));
    expect((await hook(page)).timezone).toBe("UTC");

    await page.getByTestId("tz-select").selectOption("Asia/Tokyo");
    await expect(page.getByTestId("legend-time")).toHaveText(
      utcLabel((await lastBar()) + 9 * 3600_000),
    );
    expect((await hook(page)).timezone).toBe("Asia/Tokyo");
    await expect(page.getByTestId("clock")).toContainText("UTC+9");
  });

  test("Go to date 30 days back pages in history and shows the bar", async ({
    bootedApp: page,
  }) => {
    await page.getByTestId("tz-select").selectOption("utc");
    const before = await hook(page);
    expect(before.interval).toBe("1h");
    const target = before.timestamps.at(-1)! - 30 * DAY;
    expect(target).toBeLessThan(before.timestamps[0]!);

    await page.keyboard.press("Alt+g");
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    const day = new Date(target).toISOString().slice(0, 10);
    await dialog.getByLabel("Date").fill(day);
    await dialog.getByLabel("Time (optional)").fill("12:00");
    const paged = chartResponse(page, (a) => a.end_time !== undefined);
    await dialog.getByRole("button", { name: "OK" }).click();
    await paged;

    const noon = Date.parse(`${day}T12:00:00Z`);
    await afterChartResponses(page, async () => {
      const h = await hook(page);
      const idx = h.timestamps.findIndex((t) => t >= noon);
      expect(idx).not.toBe(-1);
      expect(idx).toBeGreaterThanOrEqual(h.range.from);
      expect(idx).toBeLessThan(h.range.to);
    });
  });

  test("the time zone persists across reload", async ({ bootedApp: page }) => {
    await page.getByTestId("tz-select").selectOption("Asia/Tokyo");
    await reloadApp(page);
    await expect(page.getByTestId("tz-select")).toHaveValue("Asia/Tokyo");
    await expect.poll(async () => (await hook(page)).timezone).toBe("Asia/Tokyo");
  });
});
