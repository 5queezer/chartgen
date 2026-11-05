// Light/dark theme toggle: DOM and canvas both repaint, and the choice
// persists across reload (ADR-0016).

import type { Page } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

const bodyBackground = (page: Page) =>
  page.evaluate(() => getComputedStyle(document.body).backgroundColor);

/** Mean luminance (0-255) of the chart's wide canvases (candle + indicator
 *  panes, excluding the price axis), composited over the body background and
 *  averaged down to one pixel. Mostly empty area, so it tracks the theme. */
const chartLuminance = (page: Page) =>
  page.evaluate(() => {
    const host = document.querySelector(".chart-host")!;
    const out = document.createElement("canvas");
    out.width = out.height = 1;
    const ctx = out.getContext("2d")!;
    ctx.fillStyle = getComputedStyle(document.body).backgroundColor;
    ctx.fillRect(0, 0, 1, 1);
    for (const c of host.querySelectorAll("canvas")) {
      if (c.width > 500) ctx.drawImage(c, 0, 0, 1, 1);
    }
    const [r, g, b] = ctx.getImageData(0, 0, 1, 1).data;
    return 0.299 * r! + 0.587 * g! + 0.114 * b!;
  });

test.describe("theme", () => {
  test("toggle switches to light and persists across reload", async ({
    bootedApp: page,
  }) => {
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
