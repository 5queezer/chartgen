// Clicking the selected sidebar tab collapses it to a rail of the four tab
// icons; a rail icon opens that tab. Set SIDEBAR_SHOTS_DIR to write screenshots.

import type { Page } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

const NAMES = ["Watchlist", "Alerts", "Data Window", "Object Tree"];
const chartWidth = (page: Page) =>
  page.locator(".chart-host").evaluate((host) => {
    const rects = Array.from(host.querySelectorAll("canvas"))
      .map((c) => c.getBoundingClientRect())
      .filter((r) => r.width > 0);
    return Math.max(...rects.map((r) => r.right)) - Math.min(...rects.map((r) => r.left));
  });
const rail = (page: Page) => page.getByRole("navigation", { name: "Sidebar (collapsed)" });

for (const name of NAMES) {
  test(`${name} tab: clicking it again collapses to the rail, an icon reopens it`, async ({
    bootedApp: page,
  }) => {
    const tab = page.getByRole("tab", { name });
    // The default tab is already selected, and clicking it would collapse.
    if ((await tab.getAttribute("aria-selected")) !== "true") await tab.click();
    await expect(tab).toHaveAttribute("aria-selected", "true");
    const before = await chartWidth(page);
    const shots = process.env.SIDEBAR_SHOTS_DIR;
    if (shots) await page.screenshot({ path: `${shots}/open-${name.replace(" ", "-")}.png` });

    await tab.click();
    await expect(rail(page)).toBeVisible();
    await expect(page.getByRole("tablist", { name: "Sidebar panels" })).toHaveCount(0);
    await expect.poll(() => chartWidth(page)).toBeGreaterThan(before + 200);
    if (shots) await page.screenshot({ path: `${shots}/rail-${name.replace(" ", "-")}.png` });

    await page.getByRole("button", { name }).click();
    await expect(page.getByRole("tab", { name })).toHaveAttribute("aria-selected", "true");
    await expect(rail(page)).toHaveCount(0);
    await expect.poll(() => chartWidth(page)).toBeCloseTo(before, 0);
  });
}

test("the rail shows all four icons, opens each tab, survives a reload", async ({
  bootedApp: page,
}) => {
  await page.getByRole("tab", { name: "Watchlist" }).click();
  for (const name of NAMES) await expect(rail(page).getByRole("button", { name })).toBeVisible();
  await reloadApp(page);
  await expect(rail(page)).toBeVisible();
  for (const name of NAMES) {
    await rail(page).getByRole("button", { name }).focus();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("tab", { name })).toHaveAttribute("aria-selected", "true");
    await page.getByRole("tab", { name }).click();
    await expect(rail(page)).toBeVisible();
  }
  await rail(page).getByRole("button", { name: "Alerts" }).click();
  await expect(page.getByRole("button", { name: /Collapse|Expand|»/ })).toHaveCount(0);
});

test("the tab row still fits at the narrowest desktop width (768 px)", async ({
  bootedApp: page,
}) => {
  await page.setViewportSize({ width: 768, height: 800 });
  const row = page.getByRole("tablist", { name: "Sidebar panels" });
  await expect(row).toBeVisible();
  expect(await row.evaluate((e) => e.scrollWidth <= e.clientWidth)).toBe(true);
});
