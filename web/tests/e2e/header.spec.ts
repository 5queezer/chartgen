// Header/toolbar chrome: it never scrolls the page sideways, collapses into a
// "⋯" menu as the window narrows, keeps a fixed height, and shows a compact
// error badge (with the explanation over the chart) when a symbol fails.

import type { Page } from "@playwright/test";

import { test, expect, expectChartRendered } from "./fixtures/app";

const SIZES = [
  { width: 1280, height: 720 },
  { width: 1024, height: 700 },
  { width: 800, height: 600 },
] as const;

const symbolButton = (page: Page) =>
  page.getByRole("button", { name: /^Symbol:/ });
const headerHeight = (page: Page) =>
  page.locator("header").evaluate((el) => el.getBoundingClientRect().height);

/** Change the timeframe through the type-to-act popup (works at any width). */
async function typeTimeframe(page: Page, tf: string): Promise<void> {
  await page.locator(".chart-container").click({ position: { x: 300, y: 160 } });
  await page.keyboard.type(tf);
  await page.keyboard.press("Enter");
}

test.describe("header at narrow widths", () => {
  for (const size of SIZES) {
    test(`${size.width}x${size.height}: no page scroll, fixed height, one-line status`, async ({
      bootedApp: page,
    }) => {
      await page.setViewportSize(size);
      await typeTimeframe(page, "15");
      await expect(page.getByTestId("layout-dirty")).toBeVisible();

      // (The chart canvas follows the resize a frame later.)
      await expect
        .poll(() =>
          page.evaluate(
            () => document.documentElement.scrollWidth - window.innerWidth,
          ),
        )
        .toBeLessThanOrEqual(0);

      expect(await headerHeight(page)).toBe(52);

      // Status text and live badge stay on one line.
      for (const id of ["status-text", "live-status"]) {
        const el = page.getByTestId(id);
        if ((await el.count()) === 0 || !(await el.isVisible())) continue;
        const h = await el.evaluate((n) => n.getBoundingClientRect().height);
        expect(h, id).toBeLessThan(24);
      }

      // The unsaved dot is on screen, not clipped away.
      const dot = await page.getByTestId("layout-dirty").boundingBox();
      expect(dot).not.toBeNull();
      expect(dot!.width).toBeGreaterThan(0);
      expect(dot!.x + dot!.width).toBeLessThanOrEqual(size.width);
    });
  }

  test("below 1280 screenshot/share/settings live in the ⋯ menu", async ({
    bootedApp: page,
  }) => {
    await expect(page.getByRole("button", { name: "Take screenshot" })).toBeVisible();
    await expect(page.getByTestId("toolbar-more")).toHaveCount(0);

    await page.setViewportSize({ width: 1100, height: 720 });
    await expect(page.getByRole("button", { name: "Take screenshot" })).toHaveCount(0);
    await page.getByTestId("toolbar-more").click();
    for (const name of ["Take screenshot", "Copy link to chart", "Chart settings"]) {
      await expect(page.getByRole("menuitem", { name })).toBeVisible();
    }
    await page.getByRole("menuitem", { name: "Chart settings" }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
  });

  test("below 1024 the timeframe buttons collapse into the dropdown", async ({
    bootedApp: page,
  }) => {
    await page.setViewportSize({ width: 900, height: 700 });
    await expect(page.getByRole("button", { name: "5m", exact: true })).toHaveCount(0);
    await page.getByRole("button", { name: /^More timeframes/ }).click();
    await page.getByRole("menuitem", { name: "5m", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "More timeframes (current: 5m)" }),
    ).toBeVisible();
  });
});

test.describe("failed symbol load", () => {
  test("compact badge, centered message, Back restores the previous symbol", async ({
    bootedApp: page,
  }) => {
    const before = await symbolButton(page).innerText();
    const height = await headerHeight(page);
    await page.route("**/mcp", async (route) => {
      const body = route.request().postData() ?? "";
      if (body.includes('"generate_chart"') && body.includes('"ZZZZQQQ"')) {
        await route.fulfill({
          contentType: "application/json",
          body: JSON.stringify({
            jsonrpc: "2.0",
            id: JSON.parse(body).id,
            result: {
              isError: true,
              content: [
                {
                  type: "text",
                  text: "Data fetch failed: Yahoo API returned status 404 Not Found",
                },
              ],
            },
          }),
        });
        return;
      }
      await route.continue();
    });

    await symbolButton(page).click();
    await page.getByRole("combobox").fill("zzzzqqq");
    await page.keyboard.press("Enter");

    const badge = page.getByTestId("status-text");
    await expect(badge).toHaveText("error", { timeout: 15_000 });
    await expect(badge).toHaveAttribute("title", /404 Not Found/);
    expect(await headerHeight(page)).toBe(height);

    const panel = page.getByTestId("chart-load-error");
    await expect(panel).toContainText("Couldn't load ZZZZQQQ — symbol not found");
    await expect(panel.getByRole("button", { name: "Retry" })).toBeVisible();

    await panel.getByRole("button", { name: /^Back to / }).click();
    await expect(symbolButton(page)).toHaveText(before);
    await expect(panel).toHaveCount(0);
    await expectChartRendered(page);
    expect(await headerHeight(page)).toBe(height);
  });
});

test.describe("chrome fixes", () => {
  test("a modal backdrop covers the sidebar", async ({ bootedApp: page }) => {
    await page.getByRole("button", { name: /Indicators/ }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
    const hit = await page.evaluate(() => {
      const r = document.querySelector("aside")!.getBoundingClientRect();
      const el = document.elementFromPoint(r.x + r.width / 2, r.y + 40);
      return {
        inSidebar: el?.closest("aside") != null,
        bg: el ? getComputedStyle(el).backgroundColor : "",
      };
    });
    expect(hit.inSidebar).toBe(false);
    expect(hit.bg).not.toBe("rgba(0, 0, 0, 0)");
  });

  test("hide-all drawings toggles its label and pressed state", async ({
    bootedApp: page,
  }) => {
    // The button only has an effect once a drawing exists.
    await page.getByRole("button", { name: "Trend line" }).click();
    const box = (await page.locator(".chart-container").boundingBox())!;
    for (const [x, y] of [
      [300, 200],
      [500, 250],
    ] as const) {
      await page.mouse.move(box.x + x, box.y + y, { steps: 5 });
      await page.mouse.down();
      await page.mouse.up();
      await page.waitForTimeout(700);
    }
    await page.getByRole("button", { name: "Hide all drawings" }).click();
    const show = page.getByRole("button", { name: "Show all drawings" });
    await expect(show).toHaveAttribute("aria-pressed", "true");
    await expect(show).toHaveAttribute("title", "Show all drawings");
    await show.click();
    await expect(
      page.getByRole("button", { name: "Hide all drawings" }),
    ).toHaveAttribute("aria-pressed", "false");
  });

  test("indicator chips and picker show display names, not ids", async ({
    bootedApp: page,
  }) => {
    const chips = page.locator("header + div");
    await expect(chips).toContainText("EMA");
    await expect(chips).not.toContainText("ema_stack");

    await page.getByRole("button", { name: /Indicators/ }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByPlaceholder("Search indicators...").fill("bbands");
    const row = dialog.locator("li button").first();
    await expect(row).toContainText("Bollinger");
    await expect(row.locator("[title=bbands]")).toBeVisible();
  });

  test("toasts sit above the bottom bar and are capped at three", async ({
    bootedApp: page,
  }) => {
    await page.evaluate(async () => {
      // Link copied toasts: click the copy-link button several times.
      const btn = document.querySelector<HTMLElement>(
        '[data-testid="toolbar-copy-link"]',
      )!;
      for (let i = 0; i < 5; i++) btn.click();
    });
    const toasts = page.locator("[data-toast]");
    await expect(toasts.first()).toBeVisible();
    expect(await toasts.count()).toBeLessThanOrEqual(3);
    const toastBottom = await toasts
      .last()
      .evaluate((el) => el.getBoundingClientRect().bottom);
    const barTop = await page
      .getByTestId("clock")
      .evaluate((el) => el.parentElement!.getBoundingClientRect().top);
    expect(toastBottom).toBeLessThanOrEqual(barTop);
  });
});
