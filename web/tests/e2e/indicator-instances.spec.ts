// Several instances of one indicator (ADR-0028): RSI 14 and RSI 21 side by
// side, each with its own params, legend, visibility and removal.

import type { Page } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

const legend = (page: Page, key: string) => page.getByTestId(`indicator-legend-${key}`);
const value = (page: Page, key: string) =>
  legend(page, key).locator(".pane-header-value").first().innerText();

async function addSecondRsi(page: Page): Promise<void> {
  await page.getByRole("button", { name: /Indicators/ }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByPlaceholder("Search indicators...").fill("rsi");
  // The default set already has RSI: its row says "added" but still adds.
  const rsi = dialog
    .locator("li button[type='button']")
    .filter({ has: page.locator(".font-mono", { hasText: /^rsi\s*added$/ }) });
  await expect(rsi).toBeEnabled();
  await rsi.click();
  await dialog.getByRole("button", { name: "Close" }).click();
  await expect(dialog).toBeHidden();
}

async function setLength(page: Page, key: string, length: string): Promise<void> {
  await legend(page, key).hover();
  await legend(page, key).getByRole("button", { name: "Settings for RSI" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("length").fill(length);
  await dialog.getByRole("button", { name: "OK" }).click();
  await expect(dialog).toHaveCount(0);
}

test.describe("indicator instances", () => {
  test("two RSIs with their own length, kept across reload", async ({ bootedApp: page }) => {
    await addSecondRsi(page);
    await expect(legend(page, "rsi")).toBeVisible();
    await expect(legend(page, "rsi#2")).toBeVisible();

    await setLength(page, "rsi#2", "21");
    await expect(legend(page, "rsi")).toContainText("(14)");
    await expect(legend(page, "rsi#2")).toContainText("(21)");
    // Each pane reads its own series.
    await expect.poll(async () => (await value(page, "rsi")) !== (await value(page, "rsi#2"))).toBe(true);

    const stored = await page.evaluate(() => localStorage.getItem("chartgen.indicators.active"));
    expect(JSON.parse(stored!)).toEqual([
      "ema_stack",
      "volume",
      { name: "rsi", length: 14 },
      { name: "rsi", id: "rsi#2", length: 21 },
    ]);

    await reloadApp(page);
    await expect(legend(page, "rsi")).toContainText("(14)");
    await expect(legend(page, "rsi#2")).toContainText("(21)");
    // The chips tell the two apart.
    await expect(page.getByRole("button", { name: "Remove rsi (14)", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Remove rsi (21)", exact: true })).toBeVisible();
  });

  test("hiding or removing one instance leaves the other", async ({ bootedApp: page }) => {
    await addSecondRsi(page);
    await setLength(page, "rsi#2", "21");

    await legend(page, "rsi#2").hover();
    await legend(page, "rsi#2").getByRole("button", { name: "Hide RSI" }).click();
    await expect(legend(page, "rsi#2")).toHaveCount(0);
    await expect(legend(page, "rsi")).toContainText("(14)");

    const show = page.getByRole("button", { name: "Show rsi (21)", exact: true });
    await expect(show).toBeVisible();
    await show.click();
    await expect(legend(page, "rsi#2")).toContainText("(21)");

    await legend(page, "rsi").hover();
    await legend(page, "rsi").getByRole("button", { name: "Remove RSI" }).click();
    await expect(legend(page, "rsi")).toHaveCount(0);
    await expect(legend(page, "rsi#2")).toContainText("(21)");

    await reloadApp(page);
    await expect(legend(page, "rsi")).toHaveCount(0);
    await expect(legend(page, "rsi#2")).toContainText("(21)");
  });
});
