// Chart-level conveniences: price legend, bar-close countdown, scale
// controls, screenshot download, fullscreen button and chart keys.

import type { Page } from "@playwright/test";

import { test, expect, expectChartRendered } from "./fixtures/app";

const PRICE = /^\d+\.\d+$/;

const countdown = (page: Page) => page.getByTestId("bar-countdown");
const scaleButton = (page: Page, name: string) =>
  page
    .getByRole("group", { name: "Price scale" })
    .getByRole("button", { name, exact: true });

/** Move the mouse over the plot at a fraction of the chart width. */
async function hoverChart(page: Page, xFraction: number): Promise<void> {
  const box = await page.locator(".chart-host").boundingBox();
  if (!box) throw new Error("chart host has no bounding box");
  await page.mouse.move(box.x + box.width * xFraction, box.y + box.height / 3);
}

test.describe("chart ux", () => {
  test("legend shows O/H/L/C, change and volume at rest and on hover", async ({
    bootedApp: page,
  }) => {
    const legend = page.getByTestId("price-legend");
    await expect(legend).toBeVisible();
    for (const id of ["o", "h", "l", "c"]) {
      await expect(page.getByTestId(`legend-${id}`)).toHaveText(PRICE);
    }
    await expect(page.getByTestId("legend-change")).toHaveText(
      /^[+-]?\d+\.\d+ \([+-]?\d+\.\d{2}%\)$/,
    );
    await expect(page.getByTestId("legend-v")).toHaveText(/^\d/);

    const restClose = await page.getByTestId("legend-c").textContent();
    await hoverChart(page, 0.3);
    await expect
      .poll(() => page.getByTestId("legend-c").textContent())
      .not.toBe(restClose);
    for (const id of ["o", "h", "l", "c"]) {
      await expect(page.getByTestId(`legend-${id}`)).toHaveText(PRICE);
    }
  });

  test("countdown badge sits on the price axis and ticks", async ({
    bootedApp: page,
  }) => {
    const badge = countdown(page);
    await expect(badge).toBeVisible();
    await expect(badge).toHaveText(/^(\d+d )?\d{2}(:\d{2}){1,2}$/);
    const before = await badge.textContent();
    await page.waitForTimeout(2100);
    expect(await badge.textContent()).not.toBe(before);
  });

  test("log scale persists across reload", async ({ bootedApp: page }) => {
    const log = scaleButton(page, "log");
    await expect(log).toHaveAttribute("aria-pressed", "false");
    await log.click();
    await expect(log).toHaveAttribute("aria-pressed", "true");
    expect(
      await page.evaluate(() => localStorage.getItem("chartgen.layout.scaleMode")),
    ).toBe('"log"');

    await page.reload();
    await expectChartRendered(page);
    await expect(scaleButton(page, "log")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(countdown(page)).toBeVisible();
  });

  test("percent and log are mutually exclusive; auto resets by double-click", async ({
    bootedApp: page,
  }) => {
    await scaleButton(page, "log").click();
    await scaleButton(page, "%").click();
    await expect(scaleButton(page, "%")).toHaveAttribute("aria-pressed", "true");
    await expect(scaleButton(page, "log")).toHaveAttribute(
      "aria-pressed",
      "false",
    );

    const auto = scaleButton(page, "auto");
    await expect(auto).toHaveAttribute("aria-pressed", "true");
    await auto.click();
    await expect(auto).toHaveAttribute("aria-pressed", "false");
    const box = await page.locator(".chart-host").boundingBox();
    if (!box) throw new Error("chart host has no bounding box");
    await page.mouse.dblclick(box.x + box.width - 20, box.y + box.height / 3);
    await expect(auto).toHaveAttribute("aria-pressed", "true");
  });

  test("screenshot button downloads a named PNG", async ({
    bootedApp: page,
  }) => {
    const download = page.waitForEvent("download");
    await page.getByRole("button", { name: "Take screenshot" }).click();
    const dl = await download;
    expect(dl.suggestedFilename()).toMatch(
      /^BTCUSDT_1h_\d{4}-\d{2}-\d{2}_\d{4}\.png$/,
    );
    const path = await dl.path();
    const { size } = await import("node:fs").then((fs) => fs.statSync(path));
    expect(size).toBeGreaterThan(1000);
  });

  test("fullscreen button toggles the pressed state", async ({
    bootedApp: page,
  }) => {
    const button = page.getByRole("button", { name: "Fullscreen" });
    await expect(button).toHaveAttribute("aria-pressed", "false");
    await button.click();
    await expect(button).toHaveAttribute("aria-pressed", "true");
    await button.click();
    await expect(button).toHaveAttribute("aria-pressed", "false");
  });

  test("chart keys work without errors", async ({ bootedApp: page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));

    await page.locator(".chart-host").click({ position: { x: 200, y: 200 } });
    for (const key of [
      "Home",
      "ArrowRight",
      "ArrowLeft",
      "ArrowUp",
      "ArrowDown",
      "+",
      "-",
      "End",
      "Alt+r",
    ]) {
      await page.keyboard.press(key);
    }
    await page.getByRole("button", { name: "Reset chart" }).click();

    expect(errors).toEqual([]);
    await expect(countdown(page)).toBeVisible();
    // Keys must not have opened the symbol search or timeframe popups.
    await expect(page.getByRole("dialog")).toHaveCount(0);
  });
});
