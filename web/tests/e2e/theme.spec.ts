// Light/dark theme toggle: DOM and canvas both repaint, and the choice
// persists across reload (ADR-0016).

import type { Page } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

const bodyBackground = (page: Page) =>
  page.evaluate(() => getComputedStyle(document.body).backgroundColor);

/** Mean luminance (0-255) of the chart's wide canvases (candle + indicator
 *  panes, excluding the price axis), each averaged over all its pixels and
 *  composited over the body background. Mostly empty area, so it tracks the
 *  theme. (A `drawImage` down to one pixel samples only the canvas center,
 *  so a plot line crossing it would swing the result.) */
const chartLuminance = (page: Page) =>
  page.evaluate(() => {
    const host = document.querySelector(".chart-host")!;
    const bg = getComputedStyle(document.body).backgroundColor.match(/\d+/g)!.map(Number);
    let [r, g, b] = bg as [number, number, number];
    for (const c of host.querySelectorAll("canvas")) {
      if (c.width <= 500) continue;
      const d = c.getContext("2d")!.getImageData(0, 0, c.width, c.height).data;
      let sa = 0, sr = 0, sg = 0, sb = 0;
      for (let i = 0; i < d.length; i += 4) {
        const a = d[i + 3]! / 255;
        sa += a; sr += d[i]! * a; sg += d[i + 1]! * a; sb += d[i + 2]! * a;
      }
      // Source-over of the canvas' mean (premultiplied) color.
      const a = sa / (d.length / 4);
      if (sa === 0) continue;
      r = (sr / sa) * a + r * (1 - a);
      g = (sg / sa) * a + g * (1 - a);
      b = (sb / sa) * a + b * (1 - a);
    }
    return 0.299 * r + 0.587 * g + 0.114 * b;
  });

test.describe("theme", () => {
  test("toggle switches to light and persists across reload", async ({
    bootedApp: page,
  }) => {
    // The Volume pane's bars are bright in both themes and would swamp the
    // luminance probe, which is about the theme's empty chart area.
    await page.getByTestId("indicator-legend-volume").getByRole("button", { name: /^Remove / }).click();
    await expect(
      page.locator(".pane-header").filter({ hasText: "Volume" }),
    ).toHaveCount(0);
    expect(await bodyBackground(page)).toBe("rgb(19, 23, 34)");
    expect(await chartLuminance(page)).toBeLessThan(80);

    await page.getByRole("button", { name: "Switch to light theme" }).click();
    await expect.poll(() => bodyBackground(page)).toBe("rgb(255, 255, 255)");
    await expect
      .poll(() => chartLuminance(page))
      .toBeGreaterThan(200);
    await expect(
      page.getByRole("button", { name: "Switch to dark theme" }),
    ).toBeVisible();

    await reloadApp(page);
    expect(await bodyBackground(page)).toBe("rgb(255, 255, 255)");
    await expect
      .poll(() => chartLuminance(page))
      .toBeGreaterThan(200);
  });
});
