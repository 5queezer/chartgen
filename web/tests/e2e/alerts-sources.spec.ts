// Drawing and indicator alerts (ADR-0034): an alert on a horizontal line
// and on a trend line (from its right-click menu), an indicator alert from
// a legend row, "close crossing EMA 200", and a deleted drawing leaving its
// alert inactive.
//
// The sample source's live price moves every second, so firing is made
// deterministic the way alerts.spec.ts does it: prices are injected with
// the DEV hook window.__chartgenTest.feedPrice, in one synchronous step,
// far enough below and above the line that the live ticks cannot matter
// (a "once" alert fires either way). Set ALERT_SHOTS_DIR to also write
// screenshots there.

import type { Page } from "@playwright/test";

import { test, expect } from "./fixtures/app";

const shot = async (page: Page, name: string) => {
  const dir = process.env.ALERT_SHOTS_DIR;
  if (dir) await page.screenshot({ path: `${dir}/${name}.png` });
};

const A = { x: 300, y: 160 };
const B = { x: 560, y: 260 };
const MID = { x: (A.x + B.x) / 2, y: (A.y + B.y) / 2 };
const DRAWINGS_KEY = "chartgen.drawings.BTCUSDT";

// As drawing.spec.ts: the points assume a chart as wide as 1280 px.
test.use({ viewport: { width: 1280 + 320, height: 720 } });

const dialog = (page: Page) => page.getByRole("dialog");
const alertsTab = (page: Page) => page.getByRole("tab", { name: /^Alerts/ });
const rows = (page: Page) => page.locator("[data-alert-id]");
const toasts = (page: Page) => page.locator("[data-toast]");
const drawingMenu = (page: Page) => page.getByRole("menu", { name: "Drawing actions" });

async function clickAt(
  page: Page,
  at: { x: number; y: number },
  button: "left" | "right" = "left",
): Promise<void> {
  const box = await page.locator(".chart-container").boundingBox();
  if (!box) throw new Error("chart container not laid out");
  await page.mouse.move(box.x + at.x, box.y + at.y, { steps: 5 });
  await page.mouse.down({ button });
  await page.mouse.up({ button });
  // klinecharts treats two clicks in quick succession as a double click.
  await page.waitForTimeout(700);
}

const storedDrawings = (page: Page) =>
  page.evaluate(
    (k) => JSON.parse(localStorage.getItem(k) ?? "[]") as {
      id?: string;
      name: string;
      points: { value: number }[];
    }[],
    DRAWINGS_KEY,
  );

/** Inject prices, in order, in one synchronous step (no live tick in between). */
const feed = (page: Page, prices: number[]) =>
  page.evaluate((ps) => {
    const w = window as unknown as {
      __chartgenTest: { feedPrice: (s: string, p: number) => void };
    };
    for (const p of ps) w.__chartgenTest.feedPrice("BTCUSDT", p);
  }, prices);

async function addHorizontalLine(page: Page): Promise<number> {
  await clickAt(page, A, "right");
  const menu = page.getByRole("menu", { name: "Chart actions" });
  await menu.getByRole("menuitem", { name: /^Add horizontal line at/ }).click();
  // Alt+A only reaches the page once the menu is gone.
  await expect(menu).toHaveCount(0);
  await expect.poll(async () => (await storedDrawings(page)).length).toBe(1);
  const [rec] = await storedDrawings(page);
  expect(rec?.id).toBeTruthy();
  return rec!.points[0]!.value;
}

test.describe("drawing alerts", () => {
  test("an alert on a horizontal line fires when the price crosses it", async ({
    bootedApp: page,
  }) => {
    const level = await addHorizontalLine(page);
    await page.keyboard.press("Alt+KeyA");
    await expect(dialog(page)).toBeVisible();
    await dialog(page).getByLabel("Source").selectOption({ label: "Horizontal line" });
    // A drawing is its own level: no value to type.
    await expect(dialog(page).getByLabel("Price")).toHaveCount(0);
    await dialog(page).getByRole("button", { name: "Create" }).click();
    await expect(dialog(page)).toHaveCount(0);

    await alertsTab(page).click();
    await expect(rows(page)).toHaveCount(1);
    await expect(rows(page)).toContainText("Close crossing Horizontal line");
    await expect(rows(page)).toHaveAttribute("data-status", "active");

    await feed(page, [level * 0.9, level * 1.1]);
    await expect(toasts(page).filter({ hasText: "Close crossing Horizontal line" })).toBeVisible();
    await expect(rows(page)).toHaveAttribute("data-status", "triggered");
  });

  test("a trend line alert from its right-click menu", async ({ bootedApp: page }) => {
    await page.getByRole("button", { name: "Trend line" }).click();
    await clickAt(page, A);
    await clickAt(page, B);
    await expect.poll(async () => (await storedDrawings(page)).length).toBe(1);

    await clickAt(page, MID, "right");
    await drawingMenu(page).getByRole("menuitem", { name: "Add alert on Trend line…" }).click();
    await expect(dialog(page)).toBeVisible();
    await expect(dialog(page).getByLabel("Source")).toHaveValue(/^drawing:/);
    await dialog(page).getByLabel("Condition").selectOption("crossing_up");
    await shot(page, "dialog-trend-line");
    await dialog(page).getByRole("button", { name: "Create" }).click();
    await alertsTab(page).click();
    await expect(rows(page)).toContainText("Close crossing up Trend line");

    // Far below, then far above any value the line has at the last bar.
    await feed(page, [1, 1e9]);
    await expect(rows(page)).toHaveAttribute("data-status", "triggered");
    await expect(toasts(page).filter({ hasText: /^Alert: BTCUSDT Close crossing up Trend line/ }))
      .toBeVisible();
  });

  test("deleting the drawing leaves its alert inactive", async ({ bootedApp: page }) => {
    await addHorizontalLine(page);
    await page.keyboard.press("Alt+KeyA");
    await expect(dialog(page)).toBeVisible();
    await dialog(page).getByLabel("Source").selectOption({ label: "Horizontal line" });
    await dialog(page).getByLabel("Condition").selectOption("greater");
    await dialog(page).getByRole("button", { name: "Create" }).click();
    await alertsTab(page).click();
    await expect(rows(page)).toHaveCount(1);

    await clickAt(page, A, "right");
    await drawingMenu(page).getByRole("menuitem", { name: "Remove" }).click();
    await expect.poll(async () => (await storedDrawings(page)).length).toBe(0);
    await expect(rows(page)).toHaveCount(1);
    await expect(rows(page)).toHaveAttribute("data-status", "inactive");
    await expect(rows(page).locator("[data-role=status]")).toHaveText(
      "inactive: drawing deleted",
    );
    await shot(page, "alerts-panel-drawing-deleted");
  });
});

test.describe("indicator alerts", () => {
  test("RSI greater than 0 fires on the next live update", async ({ bootedApp: page }) => {
    await page
      .getByTestId("indicator-legend-rsi")
      .getByRole("button", { name: "Add alert on RSI" })
      .click();
    await expect(dialog(page)).toBeVisible();
    await expect(dialog(page).getByLabel("Source")).toHaveValue(/^plot:/);
    await dialog(page).getByLabel("Condition").selectOption("greater");
    await dialog(page).getByLabel("Value", { exact: true }).fill("0");
    await shot(page, "dialog-indicator");
    await dialog(page).getByRole("button", { name: "Create" }).click();
    await alertsTab(page).click();
    await expect(rows(page)).toContainText("RSI (14) greater than 0");
    await expect(rows(page)).toHaveAttribute("data-status", "triggered", { timeout: 15_000 });
    await expect(toasts(page).filter({ hasText: "RSI (14) greater than 0" })).toBeVisible();
  });

  test("close crossing EMA 200 from the legend, inactive on another interval", async ({
    bootedApp: page,
  }) => {
    await page
      .getByTestId("indicator-legend-ema_stack")
      .getByRole("button", { name: "Add alert on EMA" })
      .click();
    await expect(dialog(page)).toBeVisible();
    // An overlay is the level the price crosses.
    await expect(dialog(page).getByLabel("Source")).toHaveValue("price");
    const level = dialog(page).getByLabel("Level");
    await expect(level.locator("option:checked")).toHaveText("EMA: EMA 8");
    await level.selectOption({ label: "EMA: EMA 200" });
    await dialog(page).getByLabel("Message").fill("trend change");
    // Live ticks meanwhile must not reset the selection.
    await page.waitForTimeout(1500);
    await expect(level.locator("option:checked")).toHaveText("EMA: EMA 200");
    await shot(page, "dialog-ema");
    await dialog(page).getByRole("button", { name: "Create" }).click();

    await alertsTab(page).click();
    await expect(rows(page)).toHaveCount(1);
    await expect(rows(page).locator("[data-role=description]")).toHaveText(
      "Close crossing EMA 200",
    );
    await expect(rows(page)).toHaveAttribute("data-status", "active");
    await shot(page, "alerts-panel");

    await page
      .getByRole("group", { name: "Timeframe" })
      .getByRole("button", { name: "4h", exact: true })
      .click();
    await expect(rows(page)).toHaveAttribute("data-status", "inactive");
    await expect(rows(page).locator("[data-role=status]")).toHaveText(
      "inactive: open BTCUSDT 1h",
    );
    await shot(page, "alerts-panel-inactive");
  });
});
