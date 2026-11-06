// Custom intervals (ADR-0038): typed in the popup, served by the server's
// grammar, and carried by everything keyed by an interval. The sample source
// serves any interval on the same grid as the live resampling.

import type { Page } from "@playwright/test";

import { test, expect, gotoApp, reloadApp, expectChartRendered } from "./fixtures/app";

interface Hook {
  chart: () => { getDataList: () => Array<{ timestamp: number; close: number }> } | null;
  series: () => { interval: string | null } | null;
}

const trigger = (page: Page, label: string) =>
  page.getByRole("button", { name: `More timeframes (current: ${label})` });

/** The chart's interval and its bars' spacing, as the chart holds them. */
const bars = (page: Page) =>
  page.evaluate(() => {
    const h = (window as unknown as { __chartgenTest: Hook }).__chartgenTest;
    const ts = (h.chart()?.getDataList() ?? []).map((d) => d.timestamp);
    return {
      interval: h.series()?.interval ?? null,
      count: ts.length,
      last: ts.at(-1) ?? 0,
      lastClose: h.chart()?.getDataList().at(-1)?.close ?? 0,
      gaps: [...new Set(ts.slice(1).map((t, i) => t - ts[i]!))],
    };
  });

async function typeInterval(page: Page, text: string): Promise<void> {
  await page.getByRole("button", { name: /^More timeframes/ }).click();
  await page.getByRole("menuitem", { name: /Custom interval/ }).click();
  await page.getByRole("dialog").getByRole("textbox").fill(text);
}

test.describe("custom intervals", () => {
  for (const [typed, canonical, label, ms] of [
    ["7", "7m", "7m", 420_000],
    ["45m", "45m", "45m", 2_700_000],
    ["90m", "90m", "90m", 5_400_000],
    ["8h", "8h", "8h", 28_800_000],
    ["2D", "2d", "2D", 172_800_000],
    ["2W", "2wk", "2W", 1_209_600_000],
  ] as const) {
    test(`${typed} loads ${canonical} bars`, async ({ bootedApp: page }) => {
      await typeInterval(page, typed);
      await expect(page.getByRole("dialog")).toContainText(`switch to ${label}`);
      await page.keyboard.press("Enter");
      await expect(trigger(page, label)).toBeVisible();
      await expectChartRendered(page);
      await expect.poll(async () => (await bars(page)).interval).toBe(canonical);
      const b = await bars(page);
      expect(b.count).toBeGreaterThan(10);
      expect(b.gaps).toEqual([ms]);
    });
  }

  test("nonsense stays in the popup with a reason", async ({ bootedApp: page }) => {
    const before = await bars(page);
    for (const bad of ["61m", "53W", "0", "7x"]) {
      await typeInterval(page, bad);
      await expect(page.getByRole("dialog")).toContainText("At most 365D");
      await page.keyboard.press("Enter");
      await expect(page.getByRole("dialog")).toBeVisible();
      await page.keyboard.press("Escape");
    }
    expect((await bars(page)).interval).toBe(before.interval);
  });

  test("a custom interval survives a reload and joins the recent list", async ({
    bootedApp: page,
  }) => {
    await typeInterval(page, "7m");
    await page.keyboard.press("Enter");
    await expect(trigger(page, "7m")).toBeVisible();
    await page.getByRole("button", { name: "15m", exact: true }).click();
    await expect(page.getByRole("button", { name: "More timeframes", exact: true })).toBeVisible();

    await reloadApp(page);
    await page.getByRole("button", { name: "More timeframes", exact: true }).click();
    await expect(page.getByRole("group", { name: "Recent" })).toBeVisible();
    await page.getByRole("menuitem", { name: "7m", exact: true }).click();
    await expect(trigger(page, "7m")).toBeVisible();
    await expect.poll(async () => (await bars(page)).interval).toBe("7m");

    await reloadApp(page);
    await expect(trigger(page, "7m")).toBeVisible();
    await expect.poll(async () => (await bars(page)).interval).toBe("7m");
  });

  test("the live feed keeps 7m bars on their grid", async ({ bootedApp: page }) => {
    await typeInterval(page, "7m");
    await page.keyboard.press("Enter");
    await expect.poll(async () => (await bars(page)).interval).toBe("7m");
    const first = await bars(page);
    await expect
      .poll(async () => (await bars(page)).lastClose, { timeout: 15_000 })
      .not.toBe(first.lastClose);
    const later = await bars(page);
    expect(later.gaps).toEqual([420_000]);
    expect(later.last % 420_000).toBe(0);
  });

  test("a share link carries one, and a bad one is ignored", async ({ page }) => {
    await gotoApp(page, "/?symbol=ETHUSDT&interval=2W");
    await expect(trigger(page, "2W")).toBeVisible();
    await expect.poll(async () => (await bars(page)).interval).toBe("2wk");
    const monday = (await bars(page)).last;
    expect(new Date(monday).getUTCDay()).toBe(1);

    await gotoApp(page, "/?symbol=ETHUSDT&interval=61m");
    await expect(trigger(page, "2W")).toBeVisible();
  });
});
