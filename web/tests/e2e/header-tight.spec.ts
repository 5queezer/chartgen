// The mouse header: short, with one control size, and highlights that sit inside the
// bar. Touch and the compact shell keep their larger targets (ADR-0039).

import type { Locator, Page } from "@playwright/test";

import { test, expect } from "./fixtures/app";

const BAR = 38;
const CONTROL = 27;
const MARGIN = 5;

const box = async (locator: Locator) => (await locator.boundingBox())!;
const background = (locator: Locator) =>
  locator.evaluate((el) => getComputedStyle(el).backgroundColor);
const TRANSPARENT = "rgba(0, 0, 0, 0)";

/** Every control showing in the bar, with its name, box and radius. */
const controls = (page: Page) =>
  page.locator("header button").evaluateAll((all) =>
    all
      .filter((b) => (b as HTMLElement).offsetParent !== null)
      .map((b) => {
        const r = b.getBoundingClientRect();
        return {
          name: b.getAttribute("aria-label") ?? b.textContent ?? "",
          top: r.top,
          bottom: r.bottom,
          radius: getComputedStyle(b).borderTopLeftRadius,
        };
      }),
  );

for (const width of [1440, 800]) {
  test.describe(`at ${width} px with a mouse`, () => {
    // After boot: the fixture waits on status text that a narrow header hides.
    test.beforeEach(async ({ bootedApp: page }) => {
      await page.setViewportSize({ width, height: 900 });
    });

    test("the bar is short and every control is one size with even margins", async ({
      bootedApp: page,
    }) => {
      expect((await box(page.locator("header"))).height).toBe(BAR);
      const all = await controls(page);
      expect(all.length).toBeGreaterThan(4);
      for (const c of all) {
        expect(c.bottom - c.top, c.name).toBe(CONTROL);
        expect(c.top, c.name).toBe(MARGIN);
        // Above the 1 px border, the same margin as on top.
        expect(BAR - 1 - c.bottom, c.name).toBe(MARGIN);
        expect(c.radius, c.name).toBe(all[0]!.radius);
      }
    });

    test("the chart starts under the bar, a hidden indicator too", async ({ bootedApp: page }) => {
      expect((await box(page.locator(".chart-host"))).y).toBe(BAR);

      // A hidden indicator keeps its legend row, which is how it comes back.
      const legend = page.getByTestId("indicator-legend-rsi");
      await legend.hover();
      await legend.getByRole("button", { name: "Hide RSI" }).click();
      await expect(legend.getByRole("button", { name: "Show RSI" })).toBeVisible();
      expect((await box(page.locator(".chart-host"))).y).toBe(BAR);

      await legend.getByRole("button", { name: "Show RSI" }).click();
      await expect(legend.getByRole("button", { name: "Hide RSI" })).toBeVisible();
      expect((await box(page.locator(".chart-host"))).y).toBe(BAR);
    });

    test("an open menu hangs below the bar", async ({ bootedApp: page }) => {
      const trigger = page.getByRole("button", { name: /^(More timeframes|Chart type)/ }).first();
      // Before opening: an open menu hides the rest of the page from role queries.
      const anchor = await box(trigger);
      await trigger.click();
      const menu = await box(page.getByRole("menu"));
      expect(menu.y).toBeGreaterThanOrEqual(anchor.y + CONTROL);
      await page.keyboard.press("Escape");
    });
  });
}

test.describe("highlights", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("a hovered button's background stays inside the bar, clear of its neighbour", async ({
    bootedApp: page,
  }) => {
    const group = page.getByRole("group", { name: "Timeframe" });
    const active = group.locator('[aria-pressed="true"]');
    await expect(active).toHaveCount(1);
    // The neighbour to hover: the button just before the active one.
    const index = await group
      .getByRole("button")
      .evaluateAll((all) => all.findIndex((b) => b.getAttribute("aria-pressed") === "true"));
    expect(index).toBeGreaterThan(0);
    const hovered = group.getByRole("button").nth(index - 1);
    await hovered.hover();
    await expect.poll(() => background(hovered)).not.toBe(TRANSPARENT);
    expect(await background(active)).not.toBe(TRANSPARENT);

    const bar = await box(page.locator("header"));
    const h = await box(hovered);
    const a = await box(active);
    expect(h.y - bar.y).toBe(MARGIN);
    expect(bar.y + bar.height - 1 - (h.y + h.height)).toBe(MARGIN);
    // The 1 px hairline between timeframe buttons.
    expect(a.x - (h.x + h.width)).toBe(1);
  });

  test("a focus ring fits in the margin and is not clipped by the bar", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: /^Symbol:/ }).focus();
    await page.keyboard.press("Tab");
    const ring = await page.evaluate(() => {
      const el = document.activeElement as HTMLElement;
      const s = getComputedStyle(el);
      const r = el.getBoundingClientRect();
      const reach = parseFloat(s.outlineWidth) + parseFloat(s.outlineOffset);
      return { style: s.outlineStyle, top: r.top - reach, bottom: r.bottom + reach, tag: el.tagName };
    });
    expect(ring.tag).toBe("BUTTON");
    expect(ring.style).not.toBe("none");
    expect(ring.top).toBeGreaterThanOrEqual(0);
    expect(ring.bottom).toBeLessThanOrEqual(BAR - 1);
  });

  test("the wordmark keeps its baseline level with the toolbar text", async ({
    bootedApp: page,
  }) => {
    const mark = await box(page.locator("header h1 svg"));
    // 21 px on whole pixels, centred in the 37 px inside the border.
    expect(mark.y).toBe(8);
    expect(mark.height).toBe(21);
  });
});

test("a long legend row in a narrow pane keeps its controls in reach", async ({ bootedApp: page }) => {
  await page.setViewportSize({ width: 1000, height: 800 });
  await page.getByTestId("grid-menu").click();
  await page.getByTestId("grid-option-4").click();
  const legend = page.locator(".chart-pane[data-active]").getByTestId("indicator-legend-ema_stack");
  await legend.hover();
  const row = await box(legend);
  const remove = await box(legend.getByRole("button", { name: /^Remove / }));
  expect(remove.x + remove.width).toBeLessThanOrEqual(row.x + row.width);
  // Nothing clipped may show past them, in the padding at the row's end.
  await expect(legend.locator(".pane-header-controls")).toHaveCSS("box-shadow", /4px/);
  await legend.getByRole("button", { name: /^Remove / }).click();
  await expect(legend).toHaveCount(0);
});

test.describe("touch keeps its targets", () => {
  test.use({ viewport: { width: 1180, height: 820 }, hasTouch: true, isMobile: true });

  test("a touch tablet in landscape keeps the taller bar and 40 px targets", async ({
    bootedApp: page,
  }) => {
    expect((await box(page.locator("header"))).height).toBe(52);
    const symbol = await box(page.getByRole("button", { name: /^Symbol:/ }));
    expect(symbol.height).toBeGreaterThanOrEqual(40);
    // No hover: a tap on a legend row brings its controls, at a size a finger hits.
    const legend = page.getByTestId("indicator-legend-rsi");
    const remove = legend.getByRole("button", { name: "Remove RSI" });
    await expect(remove).toBeHidden();
    await legend.tap();
    await expect(remove).toBeVisible();
    expect((await box(remove)).height).toBe(32);
  });
});
