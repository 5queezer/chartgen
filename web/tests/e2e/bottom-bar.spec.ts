// Bottom bar: date-range buttons, time zone selector and Go to date. The
// chart is observed through the DEV-only `window.__chartgenTest` hook
// (App.tsx). The server runs on deterministic sample data, so history
// pages exist for any range.

import type { Page, Request } from "@playwright/test";

import { test, expect } from "./fixtures/app";

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

function pageCalls(page: Page): Array<{ end_time?: number }> {
  const calls: Array<{ end_time?: number }> = [];
  page.on("request", (req: Request) => {
    if (req.method() !== "POST" || !req.url().endsWith("/mcp")) return;
    try {
      const b = req.postDataJSON() as {
        method?: string;
        params?: { name?: string; arguments?: { end_time?: number } };
      };
      if (b.method === "tools/call" && b.params?.name === "generate_chart") {
        calls.push(b.params.arguments ?? {});
      }
    } catch {
      // Not a JSON-RPC call.
    }
  });
  return calls;
}

test.describe("bottom bar", () => {
  test.use({ viewport: { width: 2400, height: 900 } });

  test("5D switches to 5m and fits about five days", async ({ bootedApp: page }) => {
    await page.getByTestId("range-5D").click();
    await expect.poll(async () => (await hook(page)).interval, { timeout: 20_000 }).toBe("5m");
    // Loading ends when the range is paged in and fitted.
    await expect(page.getByTestId("bottom-notice")).not.toHaveText("Loading…", {
      timeout: 30_000,
    });
    const span = await visibleSpan(page);
    expect(span).toBeGreaterThan(4.5 * DAY);
    expect(span).toBeLessThan(5.5 * DAY);
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
    const calls = pageCalls(page);
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
    await dialog.getByRole("button", { name: "OK" }).click();

    await expect.poll(() => calls.some((c) => c.end_time !== undefined)).toBe(true);
    const noon = Date.parse(`${day}T12:00:00Z`);
    await expect
      .poll(async () => {
        const h = await hook(page);
        const idx = h.timestamps.findIndex((t) => t >= noon);
        return idx !== -1 && idx >= h.range.from && idx < h.range.to;
      }, { timeout: 30_000 })
      .toBe(true);
  });

  test("the time zone persists across reload", async ({ bootedApp: page }) => {
    await page.getByTestId("tz-select").selectOption("Asia/Tokyo");
    await page.reload();
    await expect(page.getByTestId("tz-select")).toHaveValue("Asia/Tokyo");
    await expect.poll(async () => (await hook(page)).timezone).toBe("Asia/Tokyo");
  });
});
