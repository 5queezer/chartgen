// Drawing modifier keys, as in TradingView: Ctrl/Cmd held inverts the
// magnet while placing or dragging a point.

import type { Page } from "@playwright/test";

import { test, expect, pickTool } from "./fixtures/app";

test.use({ viewport: { width: 1280 + 320, height: 720 } });

const host = (page: Page) => page.locator(".chart-container");

type Point = { timestamp?: number; value?: number };

const overlays = (page: Page) =>
  page.evaluate(() =>
    (
      window as unknown as {
        __chartgenDrawings: () => {
          points: Point[];
          pixels: { x?: number; y?: number }[];
        }[];
      }
    ).__chartgenDrawings(),
  );

/** True when the point's value is the open, high, low or close of its bar. */
const onOhlc = (page: Page, p: Point) =>
  page.evaluate((pt) => {
    const chart = (
      window as unknown as {
        __chartgenTest: {
          chart: () => {
            getDataList: () => {
              timestamp: number;
              open: number;
              high: number;
              low: number;
              close: number;
            }[];
          };
        };
      }
    ).__chartgenTest.chart();
    const bar = chart.getDataList().find((d) => d.timestamp === pt.timestamp);
    if (!bar) throw new Error(`no bar at ${pt.timestamp}`);
    return [bar.open, bar.high, bar.low, bar.close].includes(pt.value!);
  }, p);

async function clickAt(page: Page, at: { x: number; y: number }): Promise<void> {
  const box = (await host(page).boundingBox())!;
  await page.mouse.move(box.x + at.x, box.y + at.y, { steps: 5 });
  await page.mouse.down();
  await page.mouse.up();
  // klinecharts treats two clicks in quick succession as a double click.
  await page.waitForTimeout(700);
}

const magnetButton = (page: Page, state: "off" | "strong") =>
  page.getByRole("button", { name: `Magnet: ${state}` });

test.describe("drawing modifiers", () => {
  test("Ctrl inverts the magnet for placing and dragging a point", async ({
    bootedApp: page,
  }) => {
    await expect(magnetButton(page, "off")).toBeVisible();

    // Placing with Ctrl held snaps to the bar's OHLC.
    await pickTool(page, "Horizontal ray");
    await page.keyboard.down("Control");
    await expect(magnetButton(page, "strong")).toHaveAttribute("aria-pressed", "true");
    await clickAt(page, { x: 300, y: 165 });
    await page.keyboard.up("Control");
    await expect(magnetButton(page, "off")).toHaveAttribute("aria-pressed", "false");
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    expect(await onOhlc(page, (await overlays(page))[0]!.points[0]!)).toBe(true);

    // Released: the next one lands where it is clicked.
    await pickTool(page, "Horizontal ray");
    await clickAt(page, { x: 420, y: 175 });
    await expect.poll(async () => (await overlays(page)).length).toBe(2);
    expect(await onOhlc(page, (await overlays(page))[1]!.points[0]!)).toBe(false);

    // Dragging that point with Ctrl held snaps it.
    const from = (await overlays(page))[1]!.pixels[0]!;
    const box = (await host(page).boundingBox())!;
    await page.mouse.move(box.x + from.x!, box.y + from.y!, { steps: 5 });
    await page.keyboard.down("Control");
    await page.mouse.down();
    await page.mouse.move(box.x + from.x! + 40, box.y + from.y! + 7, { steps: 8 });
    await page.mouse.up();
    await page.keyboard.up("Control");
    await page.waitForTimeout(300);
    expect(await onOhlc(page, (await overlays(page))[1]!.points[0]!)).toBe(true);

    // Ctrl+Z leaves the magnet as chosen.
    await page.keyboard.press("Control+z");
    await expect(magnetButton(page, "off")).toBeVisible();
  });

  test("Ctrl pressed mid-drawing applies to the moving point", async ({ bootedApp: page }) => {
    await page.getByRole("button", { name: "Trend line" }).click();
    await clickAt(page, { x: 300, y: 165 });
    await page.keyboard.down("Control");
    await clickAt(page, { x: 560, y: 260 });
    await page.keyboard.up("Control");
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    const [first, second] = (await overlays(page))[0]!.points;
    expect(await onOhlc(page, first!)).toBe(false);
    expect(await onOhlc(page, second!)).toBe(true);
  });

  test("Ctrl turns a chosen magnet off and leaves the choice alone", async ({
    bootedApp: page,
  }) => {
    await magnetButton(page, "off").click();
    await expect(page.getByRole("button", { name: "Magnet: weak" })).toBeVisible();
    await page.keyboard.down("Control");
    await expect(magnetButton(page, "off")).toHaveAttribute("aria-pressed", "false");
    await page.keyboard.up("Control");
    await expect(page.getByRole("button", { name: "Magnet: weak" })).toBeVisible();
  });

  test("blur while Ctrl is held restores the magnet", async ({ bootedApp: page }) => {
    await page.keyboard.down("Control");
    await expect(magnetButton(page, "strong")).toBeVisible();
    await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await expect(magnetButton(page, "off")).toBeVisible();
    await page.keyboard.up("Control");
    await expect(magnetButton(page, "off")).toBeVisible();
  });
});
