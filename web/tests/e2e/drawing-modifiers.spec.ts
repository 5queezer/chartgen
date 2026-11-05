// Drawing modifier keys, as in TradingView: Ctrl/Cmd held inverts the magnet and
// Shift held snaps a line to horizontal, vertical or 45 degrees, while placing or
// dragging a point.

import type { Page } from "@playwright/test";

import { test, expect, pickTool, reloadApp } from "./fixtures/app";

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

const storedRecords = (page: Page) =>
  page.evaluate(() => {
    const raw = localStorage.getItem("chartgen.drawings.BTCUSDT");
    return raw === null ? null : (JSON.parse(raw) as { points: Point[] }[]);
  });

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

/** Draw a two-point line, Shift held for the second point. */
async function shiftDraw(
  page: Page,
  tool: string,
  from: { x: number; y: number },
  to: { x: number; y: number },
): Promise<void> {
  await pickTool(page, tool);
  await clickAt(page, from);
  await page.keyboard.down("Shift");
  await clickAt(page, to);
  await page.keyboard.up("Shift");
}

/** |dx| = |dy| on screen, within the rounding of a value through the axis. */
function expectDiagonal(pixels: { x?: number; y?: number }[]): void {
  const dx = Math.abs(pixels[1]!.x! - pixels[0]!.x!);
  const dy = Math.abs(pixels[1]!.y! - pixels[0]!.y!);
  expect(dx).toBeGreaterThan(0);
  expect(Math.abs(dx - dy)).toBeLessThan(1);
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

  for (const tool of ["Trend line", "Ray", "Extended line"]) {
    test(`Shift snaps a ${tool} to horizontal, vertical or 45 degrees`, async ({
      bootedApp: page,
    }) => {
      const errors: Error[] = [];
      page.on("pageerror", (e) => errors.push(e));

      await shiftDraw(page, tool, { x: 300, y: 130 }, { x: 560, y: 150 });
      await shiftDraw(page, tool, { x: 640, y: 130 }, { x: 648, y: 300 });
      await shiftDraw(page, tool, { x: 900, y: 130 }, { x: 1020, y: 245 });
      // Mouse back on the anchor: zero length, no NaN.
      await shiftDraw(page, tool, { x: 1100, y: 130 }, { x: 1100, y: 130 });
      await expect.poll(async () => (await overlays(page)).length).toBe(4);

      const [level, upright, diagonal, zero] = await overlays(page);
      expect(level!.points[1]!.value).toBe(level!.points[0]!.value);
      expect(level!.points[1]!.timestamp).toBeGreaterThan(level!.points[0]!.timestamp!);
      expect(upright!.points[1]!.timestamp).toBe(upright!.points[0]!.timestamp);
      expect(upright!.points[1]!.value).not.toBe(upright!.points[0]!.value);
      expectDiagonal(diagonal!.pixels);
      expect(zero!.points[1]).toEqual(zero!.points[0]);

      // All stored, the vertical one still vertical after a restore.
      await expect.poll(async () => (await storedRecords(page))?.length).toBe(4);
      await reloadApp(page);
      await expect.poll(async () => (await overlays(page)).length).toBe(4);
      const restored = (await overlays(page))[1]!.points;
      expect(restored[1]!.timestamp).toBe(restored[0]!.timestamp);
      expect(errors).toEqual([]);
    });
  }

  test("Ctrl+Shift: the snap wins over the magnet on the axis it fixes", async ({
    bootedApp: page,
  }) => {
    await page.keyboard.down("Control");
    await shiftDraw(page, "Trend line", { x: 300, y: 130 }, { x: 560, y: 150 });
    await shiftDraw(page, "Trend line", { x: 900, y: 130 }, { x: 1020, y: 245 });
    await page.keyboard.up("Control");
    await expect.poll(async () => (await overlays(page)).length).toBe(2);
    const [level, diagonal] = await overlays(page);
    expect(level!.points[1]!.value).toBe(level!.points[0]!.value);
    expectDiagonal(diagonal!.pixels);
  });

  for (const scale of ["inv", "log", "%"]) {
    test(`Shift gives 45 degrees on screen on the ${scale} axis`, async ({ bootedApp: page }) => {
      await page
        .getByRole("group", { name: "Price scale" })
        .getByRole("button", { name: scale, exact: true })
        .click();
      await page.waitForTimeout(300);
      await shiftDraw(page, "Trend line", { x: 300, y: 130 }, { x: 420, y: 245 });
      await expect.poll(async () => (await overlays(page)).length).toBe(1);
      const { pixels } = (await overlays(page))[0]!;
      expectDiagonal(pixels);
      expect(pixels[1]!.y!).toBeGreaterThan(pixels[0]!.y!);
    });
  }

  test("Shift snaps a dragged point against the other one", async ({ bootedApp: page }) => {
    await page.getByRole("button", { name: "Trend line" }).click();
    await clickAt(page, { x: 300, y: 130 });
    await clickAt(page, { x: 560, y: 260 });
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    const box = (await host(page).boundingBox())!;

    /**
     * Drag point `index` so that the mouse lands at `dx`, `dy` from the other point.
     * Stay inside the price pane: klinecharts ignores a pressed move over another pane.
     */
    const drag = async (index: number, dx: number, dy: number, shift = true) => {
      const { pixels } = (await overlays(page))[0]!;
      const [from, other] = [pixels[index]!, pixels[1 - index]!];
      // klinecharts drops moves within 20 ms of the last one, without a trailing
      // call, and a point can only be grabbed once a move has landed on the line.
      const moveSettled = async (x: number, y: number) => {
        await page.mouse.move(box.x + x - 1, box.y + y, { steps: 5 });
        await page.waitForTimeout(40);
        await page.mouse.move(box.x + x, box.y + y);
      };
      await moveSettled(from.x!, from.y!);
      if (shift) await page.keyboard.down("Shift");
      await page.mouse.down();
      await moveSettled(other.x! + dx, other.y! + dy);
      await page.mouse.up();
      if (shift) await page.keyboard.up("Shift");
      await page.waitForTimeout(300);
      await page.mouse.move(box.x + 40, box.y + 40);
      return (await overlays(page))[0]!;
    };

    const anchor = (await overlays(page))[0]!.points[0];

    let line = await drag(1, 260, 25);
    expect(line.points[0]).toEqual(anchor);
    expect(line.points[1]!.value).toBe(anchor!.value);
    expect(line.points[1]!.timestamp).toBeGreaterThan(anchor!.timestamp!);

    line = await drag(1, 8, 170);
    expect(line.points[0]).toEqual(anchor);
    expect(line.points[1]!.timestamp).toBe(anchor!.timestamp);
    expect(line.points[1]!.value).not.toBe(anchor!.value);

    line = await drag(1, 150, 140);
    expect(line.points[0]).toEqual(anchor);
    expectDiagonal(line.pixels);

    // Dragging the first point snaps against the second.
    const second = line.points[1];
    line = await drag(0, -200, 15);
    expect(line.points[1]!.timestamp).toBe(second!.timestamp);
    expect(line.points[1]!.value).toBe(second!.value);
    expect(line.points[0]!.value).toBe(second!.value);

    line = await drag(0, 6, -120);
    expect(line.points[1]!.value).toBe(second!.value);
    expect(line.points[0]!.timestamp).toBe(second!.timestamp);
    expect(line.pixels[0]!.x).toBe(line.pixels[1]!.x);

    // Released: the point follows the mouse again, unsnapped.
    line = await drag(0, -200, -60, false);
    expect(line.points[1]!.value).toBe(second!.value);
    expect(Math.abs(line.pixels[0]!.y! - line.pixels[1]!.y! + 60)).toBeLessThan(2);
    expect(line.pixels[1]!.x! - line.pixels[0]!.x!).toBeGreaterThan(150);
  });

  test("Shift leaves other tools alone", async ({ bootedApp: page }) => {
    await page.getByRole("button", { name: "Rectangle" }).click();
    await clickAt(page, { x: 300, y: 165 });
    await page.keyboard.down("Shift");
    await clickAt(page, { x: 560, y: 260 });
    await page.keyboard.up("Shift");
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    const [first, second] = (await overlays(page))[0]!.points;
    expect(second!.value).not.toBe(first!.value);
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
