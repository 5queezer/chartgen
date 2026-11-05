// Chart settings dialog and chart context menu (ADR-0019): live preview,
// OK / Cancel, persistence, and the right-click actions.

import type { Page } from "@playwright/test";

import { test, expect, expectChartRendered } from "./fixtures/app";

const DRAWINGS_KEY = "chartgen.drawings.BTCUSDT";
const SETTINGS_KEY = "chartgen.chartSettings";
const UP = { r: 0x26, g: 0xa6, b: 0x9a };
const YELLOW = { r: 0xff, g: 0xeb, b: 0x3b };
const GRID = { r: 0x2a, g: 0x2e, b: 0x39 };
const A = { x: 300, y: 160 };
const B = { x: 560, y: 260 };
const MID = { x: (A.x + B.x) / 2, y: (A.y + B.y) / 2 };

// Same viewport as drawing.spec.ts: the coordinates above sit inside the plot.
test.use({
  viewport: { width: 1280 + 288, height: 720 },
  permissions: ["clipboard-read", "clipboard-write"],
});

type Rgb = { r: number; g: number; b: number };

/** Opaque canvas pixels of the chart that match `c` (+-6 per channel). */
const countPixels = (page: Page, c: Rgb) =>
  page.evaluate((c) => {
    let n = 0;
    for (const canvas of document.querySelectorAll<HTMLCanvasElement>(
      ".chart-container canvas",
    )) {
      const ctx = canvas.getContext("2d");
      if (!ctx || canvas.width === 0 || canvas.height === 0) continue;
      const d = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
      for (let i = 0; i < d.length; i += 4) {
        if (
          d[i + 3]! === 255 &&
          Math.abs(d[i]! - c.r) <= 6 &&
          Math.abs(d[i + 1]! - c.g) <= 6 &&
          Math.abs(d[i + 2]! - c.b) <= 6
        )
          n++;
      }
    }
    return n;
  }, c);

/** The candles and the grid are drawn: the pixel probes below mean something. */
async function chartPainted(page: Page, candles: Rgb = UP): Promise<void> {
  await expect.poll(() => countPixels(page, candles)).toBeGreaterThan(200);
  await expect.poll(() => countPixels(page, GRID)).toBeGreaterThan(500);
}

const dialog = (page: Page) => page.getByRole("dialog", { name: "Settings" });

async function openSettings(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Chart settings" }).click();
  await expect(dialog(page)).toBeVisible();
}

async function pickUpBodyYellow(page: Page): Promise<void> {
  await dialog(page).getByRole("button", { name: "Body up color" }).click();
  await page.getByLabel("Color #ffeb3b").click();
  // Close the color popover; the preview is already live.
  await page.keyboard.press("Escape");
}

const container = (page: Page) => page.locator(".chart-container");

/** Move with intermediate steps, then press: klinecharts needs mouse-move first. */
async function pressAt(
  page: Page,
  at: { x: number; y: number },
  button: "left" | "right",
): Promise<void> {
  const box = await container(page).boundingBox();
  if (!box) throw new Error("chart container not laid out");
  await page.mouse.move(box.x + at.x, box.y + at.y, { steps: 5 });
  await page.mouse.down({ button });
  await page.mouse.up({ button });
  // klinecharts reads two quick left clicks as a double click.
  if (button === "left") await page.waitForTimeout(700);
}

const overlays = (page: Page) =>
  page.evaluate(() =>
    (
      window as unknown as { __chartgenDrawings: () => unknown[] }
    ).__chartgenDrawings(),
  );

const chartMenu = (page: Page) =>
  page.getByRole("menu", { name: "Chart actions" });

test.describe("chart settings dialog", () => {
  test("OK applies the up-candle color and it survives a reload", async ({
    bootedApp: page,
  }) => {
    await chartPainted(page);
    const before = await countPixels(page, YELLOW);
    const tealBefore = await countPixels(page, UP);

    await openSettings(page);
    await pickUpBodyYellow(page);
    await expect
      .poll(() => countPixels(page, YELLOW))
      .toBeGreaterThan(before + 200);
    await dialog(page).getByRole("button", { name: "OK" }).click();
    await expect(dialog(page)).toHaveCount(0);
    expect(await countPixels(page, UP)).toBeLessThan(tealBefore / 2);

    await page.reload();
    await expectChartRendered(page);
    await chartPainted(page, YELLOW);
    const saved = await page.evaluate(
      (k) => JSON.parse(localStorage.getItem(k)!),
      SETTINGS_KEY,
    );
    expect(saved.themes.dark.bodyUp).toBe("#ffeb3b");
  });

  test("Cancel reverts the preview", async ({ bootedApp: page }) => {
    await chartPainted(page);
    const before = await countPixels(page, YELLOW);
    await openSettings(page);
    await pickUpBodyYellow(page);
    await expect
      .poll(() => countPixels(page, YELLOW))
      .toBeGreaterThan(before + 200);
    await dialog(page).getByRole("button", { name: "Cancel" }).click();
    await expect(dialog(page)).toHaveCount(0);
    await expect
      .poll(() => countPixels(page, YELLOW))
      .toBeLessThan(before + 20);
    expect(
      await page.evaluate((k) => localStorage.getItem(k), SETTINGS_KEY),
    ).toBeNull();
  });

  test("grid lines turn off, and Reset settings brings them back", async ({
    bootedApp: page,
  }) => {
    await chartPainted(page);
    const grid = await countPixels(page, GRID);

    await openSettings(page);
    await dialog(page).getByRole("tab", { name: "Appearance" }).click();
    await dialog(page).getByText("Vertical grid lines").click();
    await dialog(page).getByText("Horizontal grid lines").click();
    await expect.poll(() => countPixels(page, GRID)).toBeLessThan(grid / 4);
    await dialog(page).getByRole("button", { name: "OK" }).click();

    await page.reload();
    await expectChartRendered(page);
    await expect.poll(() => countPixels(page, UP)).toBeGreaterThan(200);
    await expect.poll(() => countPixels(page, GRID)).toBeLessThan(grid / 4);

    await openSettings(page);
    await dialog(page).getByRole("button", { name: "Reset settings" }).click();
    await expect.poll(() => countPixels(page, GRID)).toBeGreaterThan(grid / 2);
    await dialog(page).getByRole("button", { name: "OK" }).click();
    await expect.poll(() => countPixels(page, GRID)).toBeGreaterThan(grid / 2);
  });

  test("left scale placement moves the legends clear of the axis", async ({
    bootedApp: page,
  }) => {
    const header = page.locator(".pane-header").first();
    const left = () =>
      header.evaluate((el) => el.getBoundingClientRect().left) as Promise<number>;
    const host = await page.locator(".chart-host").boundingBox();
    await openSettings(page);
    await dialog(page).getByRole("tab", { name: "Scales" }).click();
    await dialog(page).getByLabel("Scale placement").selectOption("left");
    await expect.poll(left).toBeGreaterThan(host!.x + 40);
    await dialog(page).getByRole("button", { name: "Cancel" }).click();
    await expect.poll(left).toBeLessThan(host!.x + 20);
  });

  test("status line toggles hide legend parts", async ({ bootedApp: page }) => {
    await expect(page.getByTestId("legend-o")).toBeVisible();
    await expect(page.getByTestId("legend-v")).toBeVisible();
    await openSettings(page);
    await dialog(page).getByRole("tab", { name: "Status line" }).click();
    await dialog(page).getByText("OHLC values").click();
    await dialog(page).getByText("Volume").click();
    await expect(page.getByTestId("legend-o")).toHaveCount(0);
    await expect(page.getByTestId("legend-v")).toHaveCount(0);
    await expect(page.getByTestId("legend-change")).toBeVisible();
    await dialog(page).getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByTestId("legend-o")).toBeVisible();
  });
});

test.describe("chart context menu", () => {
  test("Copy price writes a number to the clipboard", async ({
    bootedApp: page,
  }) => {
    await pressAt(page, A, "right");
    await expect(chartMenu(page)).toBeVisible();
    await chartMenu(page).getByRole("menuitem", { name: /^Copy price/ }).click();
    await expect
      .poll(() => page.evaluate(() => navigator.clipboard.readText()))
      .toMatch(/^\d+(\.\d+)?$/);
  });

  test("Add horizontal line creates a persisted drawing", async ({
    bootedApp: page,
  }) => {
    await pressAt(page, A, "right");
    const item = chartMenu(page).getByRole("menuitem", {
      name: /^Add horizontal line at/,
    });
    const price = Number((await item.innerText()).replace(/^\D+/, ""));
    expect(Number.isFinite(price)).toBe(true);
    await item.click();

    await expect.poll(() => overlays(page)).toHaveLength(1);
    const saved = await page.evaluate(
      (k) => JSON.parse(localStorage.getItem(k)!),
      DRAWINGS_KEY,
    );
    expect(saved).toHaveLength(1);
    expect(saved[0].name).toBe("horizontalStraightLine");
    expect(saved[0].points[0].value).toBeCloseTo(price, 1);

    await page.reload();
    await expectChartRendered(page);
    await expect.poll(() => overlays(page)).toHaveLength(1);
  });

  test("Remove all drawings asks first", async ({ bootedApp: page }) => {
    await pressAt(page, A, "right");
    await chartMenu(page)
      .getByRole("menuitem", { name: /^Add horizontal line at/ })
      .click();
    await expect.poll(() => overlays(page)).toHaveLength(1);

    page.once("dialog", (d) => void d.dismiss());
    await pressAt(page, B, "right");
    await chartMenu(page)
      .getByRole("menuitem", { name: "Remove all drawings" })
      .click();
    await expect.poll(() => overlays(page)).toHaveLength(1);

    page.once("dialog", (d) => void d.accept());
    await pressAt(page, B, "right");
    await chartMenu(page)
      .getByRole("menuitem", { name: "Remove all drawings" })
      .click();
    await expect.poll(() => overlays(page)).toHaveLength(0);
  });

  test("Remove all indicators asks first", async ({ bootedApp: page }) => {
    const none = /^Indicators\s*0$/;
    const button = page.getByRole("button", { name: /^Indicators/ });
    await expect(button).not.toHaveText(none);

    page.once("dialog", (d) => void d.dismiss());
    await pressAt(page, A, "right");
    await chartMenu(page)
      .getByRole("menuitem", { name: "Remove all indicators" })
      .click();
    await expect(button).not.toHaveText(none);

    page.once("dialog", (d) => void d.accept());
    await pressAt(page, A, "right");
    await chartMenu(page)
      .getByRole("menuitem", { name: "Remove all indicators" })
      .click();
    await expect(button).toHaveText(none);
  });

  test("Settings opens the dialog", async ({ bootedApp: page }) => {
    await pressAt(page, A, "right");
    await chartMenu(page).getByRole("menuitem", { name: "Settings…" }).click();
    await expect(dialog(page)).toBeVisible();
  });

  test("right-click on a drawing still shows the drawing menu", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: "Trend line" }).click();
    await pressAt(page, A, "left");
    await pressAt(page, B, "left");
    await expect.poll(() => overlays(page)).toHaveLength(1);

    await pressAt(page, MID, "right");
    await expect(
      page.getByRole("menu", { name: "Drawing actions" }),
    ).toBeVisible();
    await expect(chartMenu(page)).toHaveCount(0);
  });
});
