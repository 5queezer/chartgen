// The wordmark: its "gen" follows the theme through a token, it has one accessible
// name, and neither the theme nor the web font moves the header.

import type { Page } from "@playwright/test";

import { dark, light } from "../../src/lib/theme";
import { test, expect, gotoApp, reloadApp } from "./fixtures/app";

const rgb = (hex: string) => {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return `rgb(${r}, ${g}, ${b})`;
};

const fills = (page: Page) =>
  page.evaluate(() =>
    [...document.querySelectorAll("header h1 path")].map((p) => getComputedStyle(p).fill),
  );

const boxes = (page: Page) =>
  page.evaluate(() =>
    ["header", "header h1", 'header [role="toolbar"]'].map((selector) => {
      const b = document.querySelector(selector)!.getBoundingClientRect();
      return [b.x, b.y, b.width, b.height];
    }),
  );

test.describe("wordmark", () => {
  test("is one word to assistive technology", async ({ bootedApp: page }) => {
    const heading = page.getByRole("heading", { name: "chartgen", exact: true });
    await expect(heading).toBeVisible();
    await expect(heading.getByRole("img", { name: "chartgen", exact: true })).toBeVisible();
    await expect(page.locator("header h1")).toHaveText("");
  });

  test("gen follows the theme in both directions, chart keeps the text colour", async ({
    bootedApp: page,
  }) => {
    expect(await fills(page)).toEqual([rgb(dark.fg), rgb(dark.brand)]);
    const before = await boxes(page);

    await page.getByRole("button", { name: "Switch to light theme" }).click();
    await expect.poll(() => fills(page)).toEqual([rgb(light.fg), rgb(light.brand)]);
    expect(await boxes(page)).toEqual(before);

    await page.getByRole("button", { name: "Switch to dark theme" }).click();
    await expect.poll(() => fills(page)).toEqual([rgb(dark.fg), rgb(dark.brand)]);
    expect(await boxes(page)).toEqual(before);
  });

  test("the first colour a stored light theme paints is already the light one", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: "Switch to light theme" }).click();
    await expect.poll(() => fills(page)).toEqual([rgb(light.fg), rgb(light.brand)]);
    // Read when the wordmark enters the DOM, in the same task, before any paint.
    await page.addInitScript(() => {
      const seen: string[] = [];
      (window as unknown as { __wordmarkFills: string[] }).__wordmarkFills = seen;
      new MutationObserver((_, observer) => {
        // "gen" by position among the paths: the load sweep's overlay follows it.
        const gen = document.querySelector("header h1 path:nth-of-type(2)");
        if (!gen) return;
        seen.push(getComputedStyle(gen).fill);
        observer.disconnect();
      }).observe(document, { childList: true, subtree: true });
    });
    await reloadApp(page);
    const seen = await page.evaluate(
      () => (window as unknown as { __wordmarkFills: string[] }).__wordmarkFills,
    );
    expect(seen).toEqual([rgb(light.brand)]);
  });

  test("the first colour with no stored theme follows the OS, light included", async ({ page }) => {
    await page.emulateMedia({ colorScheme: "light" });
    await gotoApp(page);
    expect(await fills(page)).toEqual([rgb(light.fg), rgb(light.brand)]);
  });
});
