// Drawing tools — draw, persist, restore, select/delete, context menu,
// undo and the toolbar's armed/lock/hide states.

import type { Page } from "@playwright/test";

import { test, expect, expectChartRendered } from "./fixtures/app";

const KEY = "chartgen.drawings.BTCUSDT";
const A = { x: 300, y: 160 };
const B = { x: 560, y: 260 };
const MID = { x: (A.x + B.x) / 2, y: (A.y + B.y) / 2 };

const host = (page: Page) => page.locator(".chart-container");

/**
 * Click with real mouse movement first: klinecharts only advances a
 * drawing on mouse-move + click, not on a teleporting click.
 */
async function clickAt(
  page: Page,
  at: { x: number; y: number },
  button: "left" | "right" = "left",
): Promise<void> {
  const box = await host(page).boundingBox();
  if (!box) throw new Error("chart container not laid out");
  await page.mouse.move(box.x + at.x, box.y + at.y, { steps: 5 });
  await page.mouse.down({ button });
  await page.mouse.up({ button });
  // klinecharts treats two clicks in quick succession as a double click.
  await page.waitForTimeout(700);
}

/** Overlays alive on the chart, via the DEV-only hook in DrawingToolbar. */
const overlays = (page: Page) =>
  page.evaluate(
    () =>
      (
        window as unknown as {
          __chartgenDrawings: () => {
            name: string;
            lock: boolean;
            visible: boolean;
          }[];
        }
      ).__chartgenDrawings(),
  );

const stored = (page: Page) =>
  page.evaluate((k) => {
    const raw = localStorage.getItem(k);
    return raw === null ? null : (JSON.parse(raw) as Record<string, unknown>[]);
  }, KEY);

async function drawTrendLine(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Trend line" }).click();
  await clickAt(page, A);
  await clickAt(page, B);
  await expect.poll(() => overlays(page)).toHaveLength(1);
}

test.describe("drawing tools", () => {
  test("trend line: draw, persist, restore, survive timeframe change", async ({
    bootedApp: page,
  }) => {
    await drawTrendLine(page);

    const saved = await stored(page);
    expect(saved).toHaveLength(1);
    expect(saved?.[0]).toMatchObject({ name: "segment", lock: false });
    const points = (saved?.[0] as { points: Record<string, number>[] }).points;
    expect(points).toHaveLength(2);
    for (const p of points) {
      expect(Object.keys(p).sort()).toEqual(["timestamp", "value"]);
    }

    // One drawing per arming: the tool is released after drawing.
    await expect(page.getByRole("button", { name: "Trend line" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );

    await page.reload();
    await expectChartRendered(page);
    await expect.poll(() => overlays(page)).toHaveLength(1);

    await page.getByRole("button", { name: "15m", exact: true }).click();
    await expect(
      page.getByRole("group", { name: "Timeframe" }).locator("[aria-pressed=true]"),
    ).toHaveText("15m");
    await expect.poll(() => overlays(page)).toHaveLength(1);
    expect(await stored(page)).toEqual(saved);
  });

  test("select a drawing and press Delete", async ({ bootedApp: page }) => {
    await drawTrendLine(page);
    await clickAt(page, MID);
    await page.keyboard.press("Delete");
    await expect.poll(() => overlays(page)).toHaveLength(0);
    expect(await stored(page)).toBeNull();
  });

  test("restored drawing can be selected and deleted after reload", async ({
    bootedApp: page,
  }) => {
    await drawTrendLine(page);
    await page.reload();
    await expectChartRendered(page);
    await expect.poll(() => overlays(page)).toHaveLength(1);
    await clickAt(page, MID);
    await page.keyboard.press("Backspace");
    await expect.poll(() => overlays(page)).toHaveLength(0);
    expect(await stored(page)).toBeNull();
  });

  test("right-click menu removes a drawing", async ({ bootedApp: page }) => {
    await drawTrendLine(page);
    await clickAt(page, MID, "right");
    const menu = page.getByRole("menu", { name: "Drawing actions" });
    await expect(menu).toBeVisible();
    await menu.getByRole("menuitem", { name: "Remove" }).click();
    await expect.poll(() => overlays(page)).toHaveLength(0);
    await expect(menu).toHaveCount(0);
  });

  test("right-click menu locks a drawing", async ({ bootedApp: page }) => {
    await drawTrendLine(page);
    await clickAt(page, MID, "right");
    await page.getByRole("menuitem", { name: "Lock" }).click();
    expect((await stored(page))?.[0]).toMatchObject({ lock: true });
  });

  test("Esc cancels an armed tool and keyboard shortcuts arm tools", async ({
    bootedApp: page,
  }) => {
    const hline = page.getByRole("button", { name: "Horizontal line" });
    await hline.click();
    await expect(hline).toHaveAttribute("aria-pressed", "true");
    await page.keyboard.press("Escape");
    await expect(hline).toHaveAttribute("aria-pressed", "false");
    await expect.poll(() => overlays(page)).toHaveLength(0);

    await page.keyboard.press("Alt+h");
    await expect(hline).toHaveAttribute("aria-pressed", "true");
    await clickAt(page, A);
    await expect.poll(() => overlays(page)).toHaveLength(1);
    expect((await stored(page))?.[0]).toMatchObject({
      name: "horizontalStraightLine",
    });
  });

  test("Ctrl+Z undoes the last drawing", async ({ bootedApp: page }) => {
    await drawTrendLine(page);
    await page.keyboard.press("Control+z");
    await expect.poll(() => overlays(page)).toHaveLength(0);
    expect(await stored(page)).toBeNull();
  });

  test("hide, lock and remove all", async ({ bootedApp: page }) => {
    await drawTrendLine(page);

    await page.getByRole("button", { name: "Hide all drawings" }).click();
    expect(await overlays(page)).toMatchObject([{ visible: false }]);
    await page.getByRole("button", { name: "Hide all drawings" }).click();
    expect(await overlays(page)).toMatchObject([{ visible: true }]);

    await page.getByRole("button", { name: "Lock all drawings" }).click();
    expect(await overlays(page)).toMatchObject([{ lock: true }]);

    page.once("dialog", (d) => void d.accept());
    await page.getByRole("button", { name: "Remove all drawings" }).click();
    await expect.poll(() => overlays(page)).toHaveLength(0);
    expect(await stored(page)).toBeNull();
  });

  test("drawings are stored per symbol", async ({ bootedApp: page }) => {
    await drawTrendLine(page);
    await page.getByRole("button", { name: /^Symbol:/ }).click();
    await page.getByRole("combobox").fill("ETHUSDT");
    await page.keyboard.press("Enter");
    await expect.poll(() => overlays(page)).toHaveLength(0);
    expect(await stored(page)).toHaveLength(1);
  });
});
