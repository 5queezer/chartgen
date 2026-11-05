// Layout guards: a usable price pane with many sub-panes, and a legend that
// stays inside the chart pane at narrow widths.

import type { Page } from "@playwright/test";

import { test, expect, gotoApp } from "./fixtures/app";

type Hook = {
  chart: () => { getSize: (id: string) => { height: number } | null } | null;
};

const candleHeight = (page: Page) =>
  page.evaluate(
    () =>
      (window as unknown as { __chartgenTest: Hook }).__chartgenTest
        .chart()!
        .getSize("candle_pane")!.height,
  );

test.describe("layout", () => {
  test("five sub-panes keep the price pane at 200px+", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 720 });
    await gotoApp(
      page,
      "/?ind=rsi,rsi(length:7),rsi(length:21),rsi(length:30),rsi(length:5)",
    );
    await expect(page.locator(".pane-header")).toHaveCount(6);
    await expect.poll(() => candleHeight(page)).toBeGreaterThanOrEqual(200);
  });

  for (const width of [700, 1024]) {
    test(`legend stays inside the pane at ${width}px`, async ({ page }) => {
      // The boot waits for the status text, which is hidden when narrow.
      await gotoApp(page, "/");
      await page.setViewportSize({ width, height: 700 });
      const legend = page.getByTestId("price-legend");
      await expect(legend).toBeVisible();
      // The axis badge re-measures on its one-second tick after a resize.
      await expect
        .poll(() => page.evaluate(() => document.documentElement.scrollWidth))
        .toBeLessThanOrEqual(width);
      const m = await page.evaluate(() => {
        const host = document.querySelector(".chart-host")!.getBoundingClientRect();
        const rights = [...document.querySelectorAll(".pane-header")].map(
          (e) => e.getBoundingClientRect().right,
        );
        return { hostRight: host.right, maxRight: Math.max(...rights) };
      });
      expect(m.maxRight).toBeLessThanOrEqual(m.hostRight);
    });
  }
});
