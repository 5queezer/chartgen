// Drawing modifier keys, as in TradingView: Ctrl/Cmd held inverts the magnet and
// Shift held snaps a line to horizontal, vertical or 45 degrees, while placing or
// dragging a point.

import type { Page } from "@playwright/test";

import { test, expect, pickTool, reloadApp } from "./fixtures/app";

test.use({ viewport: { width: 1280 + 320, height: 720 } });

const host = (page: Page) => page.locator(".chart-container");
const dialog = (page: Page) => page.getByRole("dialog");
const alertRows = (page: Page) => page.locator("[data-alert-id]");
const drawingMenu = (page: Page) => page.getByRole("menu", { name: "Drawing actions" });

type At = { x: number; y: number };
type Point = { timestamp?: number; dataIndex?: number; value?: number };
interface TestChart {
  getDataList: () => { timestamp: number; open: number; high: number; low: number; close: number }[];
  convertFromPixel: (c: { x?: number; y?: number }[], f: { paneId: string }) => Point[];
  convertToPixel: (p: Point, f: { paneId: string }) => { y: number };
  subscribeAction: (type: string, fn: () => void) => void;
  unsubscribeAction: (type: string, fn: () => void) => void;
}
type TestWindow = {
  __chartgenTest: { chart: () => TestChart };
  __chartgenShiftHeld: () => boolean;
};

const overlays = (page: Page) =>
  page.evaluate(() =>
    (
      window as unknown as {
        __chartgenDrawings: () => {
          mode: string;
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
    const chart = (window as unknown as TestWindow).__chartgenTest.chart();
    const bar = chart.getDataList().find((d) => d.timestamp === pt.timestamp);
    if (!bar) throw new Error(`no bar at ${pt.timestamp}`);
    return [bar.open, bar.high, bar.low, bar.close].includes(pt.value!);
  }, p);

/** The bar under an x pixel of the price pane, and the y pixels of its range. */
const barAt = (page: Page, x: number) =>
  page.evaluate((px) => {
    const chart = (window as unknown as TestWindow).__chartgenTest.chart();
    const paneId = "candle_pane";
    const d = chart.getDataList()[chart.convertFromPixel([{ x: px }], { paneId })[0]!.dataIndex!]!;
    return {
      timestamp: d.timestamp,
      ohlc: [d.open, d.high, d.low, d.close],
      highY: chart.convertToPixel({ value: d.high }, { paneId }).y,
      lowY: chart.convertToPixel({ value: d.low }, { paneId }).y,
    };
  }, x);

/** The price at a y pixel of the price pane. */
const valueAtY = (page: Page, y: number) =>
  page.evaluate(
    (py) =>
      (window as unknown as TestWindow).__chartgenTest
        .chart()
        .convertFromPixel([{ y: py }], { paneId: "candle_pane" })[0]!.value!,
    y,
  );

const nearest = (values: number[], to: number) =>
  values.reduce((best, v) => (Math.abs(v - to) < Math.abs(best - to) ? v : best));

const shiftHeld = (page: Page) =>
  page.evaluate(() => (window as unknown as TestWindow).__chartgenShiftHeld());

async function clickAt(
  page: Page,
  at: At,
  button: "left" | "right" = "left",
): Promise<void> {
  const box = (await host(page).boundingBox())!;
  await page.mouse.move(box.x + at.x, box.y + at.y, { steps: 5 });
  await page.mouse.down({ button });
  await page.mouse.up({ button });
  // klinecharts treats two clicks in quick succession as a double click.
  await page.waitForTimeout(700);
}

/** Draw a two-point line, Shift held for the second point. */
async function shiftDraw(page: Page, tool: string, from: At, to: At): Promise<void> {
  await pickTool(page, tool);
  await clickAt(page, from);
  await page.keyboard.down("Shift");
  await clickAt(page, to);
  await page.keyboard.up("Shift");
}

/** Resolves on the next live tick, or after a second if none comes. */
const nextDataTick = (page: Page) =>
  page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        const chart = (window as unknown as TestWindow).__chartgenTest.chart();
        const done = () => {
          chart.unsubscribeAction("onDataReady", done);
          resolve();
        };
        chart.subscribeAction("onDataReady", done);
        setTimeout(done, 1000);
      }),
  );

/**
 * Press at `from` and drag to `to`, holding `keys`, then move away. Stay inside the
 * pane: klinecharts ignores a pressed move over another pane. The wait is for the
 * redraw after hovering: a point's figure only exists to be hit once the hover has
 * drawn it. A press then starts moving after 5 px (`_mouseMoveWithDownHandler`).
 *
 * The drag starts right after a live tick, because a tick between the last move and
 * the release puts the point back: DrawingToolbar's `refresh` restores every drawing
 * from its last committed state, also the one being dragged, with or without Shift.
 */
async function dragTo(page: Page, from: At, to: At, keys: string[] = []): Promise<void> {
  const box = (await host(page).boundingBox())!;
  await page.mouse.move(box.x + from.x, box.y + from.y, { steps: 5 });
  await page.waitForTimeout(60);
  await nextDataTick(page);
  for (const k of keys) await page.keyboard.down(k);
  await page.mouse.down();
  await page.mouse.move(box.x + to.x, box.y + to.y, { steps: 8 });
  await page.mouse.up();
  for (const k of keys) await page.keyboard.up(k);
  await page.waitForTimeout(300);
  await page.mouse.move(box.x + 40, box.y + 40);
}

/** Drag point `index` of drawing `n` so that the mouse lands at `dx`, `dy` from its other point. */
async function dragPoint(
  page: Page,
  n: number,
  index: number,
  dx: number,
  dy: number,
  keys: string[] = ["Shift"],
) {
  const { pixels } = (await overlays(page))[n]!;
  const [from, other] = [pixels[index]!, pixels[1 - index]!];
  await dragTo(
    page,
    { x: from.x!, y: from.y! },
    { x: other.x! + dx, y: other.y! + dy },
    keys,
  );
  return (await overlays(page))[n]!;
}

/** A free trend line starting far enough from the range of the bar at `x` for a vertical to it. */
async function freeLineFrom(page: Page, x: number) {
  const bar = await barAt(page, x);
  const y = bar.lowY > 200 ? bar.lowY - 150 : bar.lowY + 150;
  await page.getByRole("button", { name: "Trend line" }).click();
  await clickAt(page, { x, y });
  await clickAt(page, { x: x + 260, y: y + 20 });
  await expect.poll(async () => (await overlays(page)).length).toBe(1);
  return bar;
}

/** |dx| = |dy| on screen, within the rounding of a value through the axis. */
function expectDiagonal(pixels: { x?: number; y?: number }[]): void {
  const dx = Math.abs(pixels[1]!.x! - pixels[0]!.x!);
  const dy = Math.abs(pixels[1]!.y! - pixels[0]!.y!);
  expect(dx).toBeGreaterThan(0);
  expect(Math.abs(dx - dy)).toBeLessThan(1);
}

const magnetButton = (page: Page, state: "off" | "weak" | "strong") =>
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
    const free = (await overlays(page))[1]!.points[0]!;
    expect(await onOhlc(page, free)).toBe(false);

    // Dragging that point with Ctrl held snaps it.
    const from = (await overlays(page))[1]!.pixels[0]!;
    await dragTo(
      page,
      { x: from.x!, y: from.y! },
      { x: from.x! + 40, y: from.y! + 7 },
      ["Control"],
    );
    const snapped = (await overlays(page))[1]!.points[0]!;
    expect(await onOhlc(page, snapped)).toBe(true);

    // Ctrl+Z really undoes, and does not leave the magnet stuck.
    await page.keyboard.press("Control+z");
    await expect(magnetButton(page, "off")).toBeVisible();
    await expect.poll(async () => (await overlays(page))[1]!.points[0]!.value).toBe(free.value);
    expect((await overlays(page))[1]!.points[0]!.timestamp).toBe(free.timestamp);
    expect((await overlays(page))[1]!.mode).toBe("normal");
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
    await expect(magnetButton(page, "weak")).toBeVisible();
    await page.keyboard.down("Control");
    await expect(magnetButton(page, "off")).toHaveAttribute("aria-pressed", "false");
    await page.keyboard.up("Control");
    await expect(magnetButton(page, "weak")).toBeVisible();
  });

  test("the Meta key inverts the magnet too", async ({ bootedApp: page }) => {
    await page.keyboard.down("Meta");
    await expect(magnetButton(page, "strong")).toHaveAttribute("aria-pressed", "true");
    await page.keyboard.up("Meta");
    await expect(magnetButton(page, "off")).toHaveAttribute("aria-pressed", "false");
  });

  test("drawings rebuilt while Ctrl is held get the held magnet, corrected on release", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: "Trend line" }).click();
    await clickAt(page, { x: 300, y: 130 });
    await clickAt(page, { x: 560, y: 260 });
    await page.getByRole("button", { name: "Trend line" }).click();
    await clickAt(page, { x: 700, y: 130 });
    await clickAt(page, { x: 900, y: 260 });
    await expect.poll(async () => (await overlays(page)).length).toBe(2);
    expect((await overlays(page))[0]!.mode).toBe("normal");

    await page.keyboard.down("Control");
    await page.keyboard.press("z");
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    expect((await overlays(page))[0]!.mode).toBe("strong_magnet");
    await page.keyboard.up("Control");
    await expect.poll(async () => (await overlays(page))[0]!.mode).toBe("normal");
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
      // A vertical's value follows the mouse.
      expect(Math.abs(upright!.pixels[1]!.y! - 300)).toBeLessThan(1);
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

  test("Ctrl+Shift: the snap wins on the axis it fixes, the magnet acts on the other", async ({
    bootedApp: page,
  }) => {
    const ctrlShiftDraw = async (from: At, to: At) => {
      await page.getByRole("button", { name: "Trend line" }).click();
      await clickAt(page, from);
      await page.keyboard.down("Control");
      await page.keyboard.down("Shift");
      await clickAt(page, to);
      await page.keyboard.up("Shift");
      await page.keyboard.up("Control");
    };

    // Horizontal and diagonal: the value is the anchor's or derived, not the magnet's.
    await ctrlShiftDraw({ x: 300, y: 130 }, { x: 560, y: 150 });
    await ctrlShiftDraw({ x: 900, y: 130 }, { x: 1020, y: 245 });
    await expect.poll(async () => (await overlays(page)).length).toBe(2);
    const [level, diagonal] = await overlays(page);
    expect(level!.points[1]!.value).toBe(level!.points[0]!.value);
    expectDiagonal(diagonal!.pixels);

    // Vertical: the bar is the anchor's, and the magnet reads that bar, not the
    // one under the mouse.
    const x = 600;
    const bar = await barAt(page, x);
    expect((await barAt(page, x + 30)).timestamp).not.toBe(bar.timestamp);
    const y = bar.lowY > 200 ? bar.lowY - 150 : bar.lowY + 150;
    const toY = bar.lowY + 10;
    await ctrlShiftDraw({ x, y }, { x: x + 30, y: toY });
    await expect.poll(async () => (await overlays(page)).length).toBe(3);
    const [anchor, end] = (await overlays(page))[2]!.points;
    expect(anchor!.timestamp).toBe(bar.timestamp);
    expect(end!.timestamp).toBe(bar.timestamp);
    expect(end!.value).toBe(nearest(bar.ohlc, await valueAtY(page, toY)));
  });

  test("Shift with the toolbar magnet: a vertical snaps to the anchor's bar, from the mouse", async ({
    bootedApp: page,
  }) => {
    const bar = await freeLineFrom(page, 600);
    const anchorY = (await overlays(page))[0]!.pixels[0]!.y!;
    const dy = (y: number) => y - anchorY;

    // Strong, by dragging, Shift alone: the nearest of OHLC, whichever bar is under the mouse.
    await magnetButton(page, "off").click();
    await magnetButton(page, "weak").click();
    await expect(magnetButton(page, "strong")).toBeVisible();
    const toHigh = bar.highY - 4;
    const toLow = bar.lowY + 4;
    let line = await dragPoint(page, 0, 1, 30, dy(toLow));
    expect(line.points[1]!.timestamp).toBe(bar.timestamp);
    expect(line.points[1]!.value).toBe(nearest(bar.ohlc, await valueAtY(page, toLow)));
    expect(line.points[1]!.value).toBe(Math.min(...bar.ohlc));
    line = await dragPoint(page, 0, 1, 30, dy(toHigh));
    expect(line.points[1]!.value).toBe(Math.max(...bar.ohlc));

    // Weak: within klinecharts' 8 px only; further away it follows the mouse.
    await magnetButton(page, "strong").click();
    await magnetButton(page, "off").click();
    await expect(magnetButton(page, "weak")).toBeVisible();
    line = await dragPoint(page, 0, 1, 30, dy(toLow));
    expect(line.points[1]!.value).toBe(Math.min(...bar.ohlc));
    const far = bar.lowY + 30;
    line = await dragPoint(page, 0, 1, 30, dy(far));
    expect(line.points[1]!.timestamp).toBe(bar.timestamp);
    expect(Math.abs(line.pixels[1]!.y! - far)).toBeLessThan(1);
  });

  test("Ctrl+Shift in an indicator pane: no magnet, a vertical follows the mouse", async ({
    bootedApp: page,
  }) => {
    const hdr = await page.locator(".pane-header", { hasText: "RSI" }).first().boundingBox();
    const box = (await host(page).boundingBox())!;
    const y = hdr!.y - box.y + 30;
    await page.getByRole("button", { name: "Trend line" }).click();
    await clickAt(page, { x: 600, y });
    await page.keyboard.down("Control");
    await page.keyboard.down("Shift");
    await clickAt(page, { x: 606, y: y + 40 });
    await page.keyboard.up("Shift");
    await page.keyboard.up("Control");
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    const { points, pixels } = (await overlays(page))[0]!;
    expect(points[1]!.timestamp).toBe(points[0]!.timestamp);
    expect(Math.abs(pixels[1]!.y! - pixels[0]!.y! - 40)).toBeLessThan(1);
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

    const anchor = (await overlays(page))[0]!.points[0];

    let line = await dragPoint(page, 0, 1, 260, 25);
    expect(line.points[0]).toEqual(anchor);
    expect(line.points[1]!.value).toBe(anchor!.value);
    expect(line.points[1]!.timestamp).toBeGreaterThan(anchor!.timestamp!);

    line = await dragPoint(page, 0, 1, 8, 170);
    expect(line.points[0]).toEqual(anchor);
    expect(line.points[1]!.timestamp).toBe(anchor!.timestamp);
    expect(line.points[1]!.value).not.toBe(anchor!.value);

    line = await dragPoint(page, 0, 1, 150, 140);
    expect(line.points[0]).toEqual(anchor);
    expectDiagonal(line.pixels);

    // Dragging the first point snaps against the second.
    const second = line.points[1];
    line = await dragPoint(page, 0, 0, -200, 15);
    expect(line.points[1]!.timestamp).toBe(second!.timestamp);
    expect(line.points[1]!.value).toBe(second!.value);
    expect(line.points[0]!.value).toBe(second!.value);

    line = await dragPoint(page, 0, 0, 6, -120);
    expect(line.points[1]!.value).toBe(second!.value);
    expect(line.points[0]!.timestamp).toBe(second!.timestamp);
    expect(line.pixels[0]!.x).toBe(line.pixels[1]!.x);

    // Released: the point follows the mouse again, unsnapped.
    line = await dragPoint(page, 0, 0, -200, -60, []);
    expect(line.points[1]!.value).toBe(second!.value);
    expect(Math.abs(line.pixels[0]!.y! - line.pixels[1]!.y! + 60)).toBeLessThan(2);
    expect(line.pixels[1]!.x! - line.pixels[0]!.x!).toBeGreaterThan(150);
  });

  test("undo and redo of a snapped point restore its exact values", async ({ bootedApp: page }) => {
    await page.getByRole("button", { name: "Trend line" }).click();
    await clickAt(page, { x: 300, y: 130 });
    await clickAt(page, { x: 560, y: 260 });
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    const points = async () =>
      (await overlays(page))[0]!.points.map(({ timestamp, value }) => ({ timestamp, value }));
    const free = await points();

    const level = await dragPoint(page, 0, 1, 260, 25);
    expect(level.points[1]!.value).toBe(free[0]!.value);
    const levelPoints = await points();
    const upright = await dragPoint(page, 0, 1, 8, 170);
    expect(upright.points[1]!.timestamp).toBe(free[0]!.timestamp);
    const uprightPoints = await points();

    await page.keyboard.press("Control+z");
    await expect.poll(points).toEqual(levelPoints);
    await page.keyboard.press("Control+z");
    await expect.poll(points).toEqual(free);
    await page.keyboard.press("Control+Shift+z");
    await expect.poll(points).toEqual(levelPoints);
    await page.keyboard.press("Control+Shift+z");
    await expect.poll(points).toEqual(uprightPoints);
  });

  test("Shift on a future bar: placing, dragging, and a vertical without a bar", async ({
    bootedApp: page,
  }) => {
    const FUTURE = 1100;
    const lastBar = await page.evaluate(() =>
      (window as unknown as TestWindow).__chartgenTest.chart().getDataList().at(-1)!.timestamp,
    );

    // A horizontal from a real bar to a future one.
    await shiftDraw(page, "Trend line", { x: 700, y: 130 }, { x: FUTURE, y: 150 });
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    const level = (await overlays(page))[0]!;
    expect(level.points[1]!.value).toBe(level.points[0]!.value);
    const stored = await storedRecords(page);
    expect(stored![0]!.points[1]!.timestamp).toBeGreaterThan(lastBar);

    // A vertical with both points on one future bar: the magnet has no bar to
    // read, so with Ctrl it still follows the mouse.
    await page.getByRole("button", { name: "Trend line" }).click();
    await clickAt(page, { x: FUTURE, y: 130 });
    await page.keyboard.down("Control");
    await page.keyboard.down("Shift");
    await clickAt(page, { x: FUTURE + 6, y: 260 });
    await page.keyboard.up("Shift");
    await page.keyboard.up("Control");
    await expect.poll(async () => (await overlays(page)).length).toBe(2);
    let upright = (await overlays(page))[1]!;
    expect(upright.pixels[1]!.x).toBe(upright.pixels[0]!.x);
    expect(Math.abs(upright.pixels[1]!.y! - 260)).toBeLessThan(1);
    await expect.poll(async () => (await storedRecords(page))?.length).toBe(2);
    const uprightStored = (await storedRecords(page))![1]!.points;
    expect(uprightStored[0]!.timestamp).toBeGreaterThan(lastBar);
    expect(uprightStored[1]!.timestamp).toBe(uprightStored[0]!.timestamp);

    // Not offered an alert, on a future bar either.
    await clickAt(page, { x: upright.pixels[0]!.x!, y: 200 }, "right");
    await expect(drawingMenu(page).getByRole("menuitem", { name: "Remove" })).toBeVisible();
    await expect(drawingMenu(page).getByRole("menuitem", { name: /^Add alert/ })).toHaveCount(0);
    await page.keyboard.press("Escape");

    // Restored, future points hold only a bar index: dragging still snaps to them.
    await reloadApp(page);
    await expect.poll(async () => (await overlays(page)).length).toBe(2);
    upright = (await overlays(page))[1]!;
    expect(upright.pixels[1]!.x).toBe(upright.pixels[0]!.x);
    const dragged = await dragPoint(page, 1, 1, 8, 100);
    expect(dragged.pixels[1]!.x).toBe(dragged.pixels[0]!.x);
    expect(Math.abs(dragged.pixels[1]!.y! - dragged.pixels[0]!.y! - 100)).toBeLessThan(1);
    const flat = await dragPoint(page, 1, 1, 80, 10);
    expect(flat.points[1]!.value).toBe(flat.points[0]!.value);
    expect(flat.pixels[1]!.x!).toBeGreaterThan(flat.pixels[0]!.x!);
  });

  test("Shift leaves other tools alone, placing and dragging a point", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: "Rectangle" }).click();
    await clickAt(page, { x: 300, y: 165 });
    await page.keyboard.down("Shift");
    await clickAt(page, { x: 560, y: 180 });
    await page.keyboard.up("Shift");
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    const [first, second] = (await overlays(page))[0]!.points;
    expect(second!.value).not.toBe(first!.value);

    const dragged = await dragPoint(page, 0, 1, 220, 14);
    expect(dragged.points[1]!.value).not.toBe(first!.value);
    expect(Math.abs(dragged.pixels[1]!.y! - dragged.pixels[0]!.y! - 14)).toBeLessThan(2);
  });

  test("Shift leaves a body drag unsnapped, and the next drag unaffected", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: "Trend line" }).click();
    await clickAt(page, { x: 300, y: 130 });
    await clickAt(page, { x: 560, y: 240 });
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    const before = (await overlays(page))[0]!.pixels;
    const mid = { x: (before[0]!.x! + before[1]!.x!) / 2, y: (before[0]!.y! + before[1]!.y!) / 2 };

    await dragTo(page, mid, { x: mid.x + 60, y: mid.y + 12 }, ["Shift"]);
    const after = (await overlays(page))[0]!.pixels;
    expect(Math.abs(after[0]!.y! - before[0]!.y! - 12)).toBeLessThan(2);
    expect(Math.abs(after[1]!.y! - before[1]!.y! - 12)).toBeLessThan(2);
    expect(Math.abs(after[0]!.x! - before[0]!.x! - 60)).toBeLessThan(10);
    expect(Math.abs(after[1]!.x! - after[0]!.x! - (before[1]!.x! - before[0]!.x!))).toBeLessThan(2);

    // No Shift now: the point lands where the mouse does, near-horizontal or not.
    const next = await dragPoint(page, 0, 1, 200, 15, []);
    expect(next.points[1]!.value).not.toBe(next.points[0]!.value);
    expect(Math.abs(next.pixels[1]!.y! - next.pixels[0]!.y! - 15)).toBeLessThan(2);
  });

  test("no alert is offered on a vertical line", async ({ bootedApp: page }) => {
    await shiftDraw(page, "Trend line", { x: 640, y: 130 }, { x: 648, y: 300 });
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    const x = (await overlays(page))[0]!.pixels[0]!.x!;

    await clickAt(page, { x, y: 200 }, "right");
    await expect(drawingMenu(page).getByRole("menuitem", { name: "Remove" })).toBeVisible();
    await expect(drawingMenu(page).getByRole("menuitem", { name: /^Add alert/ })).toHaveCount(0);
    await page.keyboard.press("Escape");
    await expect(drawingMenu(page)).toHaveCount(0);

    await page.keyboard.press("Alt+KeyA");
    await expect(dialog(page)).toBeVisible();
    await expect(dialog(page).getByLabel("Source").locator("option", { hasText: "Trend line" }))
      .toHaveCount(0);
  });

  test("an alert on a line dragged vertical says so, until it no longer is", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: "Trend line" }).click();
    await clickAt(page, { x: 300, y: 130 });
    await clickAt(page, { x: 560, y: 260 });
    await expect.poll(async () => (await overlays(page)).length).toBe(1);

    await clickAt(page, { x: 430, y: 195 }, "right");
    await drawingMenu(page).getByRole("menuitem", { name: "Add alert on Trend line…" }).click();
    await dialog(page).getByRole("button", { name: "Create" }).click();
    await page.getByRole("tab", { name: /^Alerts/ }).click();
    await expect(alertRows(page)).toHaveCount(1);
    await expect(alertRows(page).locator("[data-role=status]")).not.toContainText("inactive");

    const upright = await dragPoint(page, 0, 1, 8, 170);
    expect(upright.points[1]!.timestamp).toBe(upright.points[0]!.timestamp);
    await expect(alertRows(page)).toHaveAttribute("data-status", "inactive");
    await expect(alertRows(page).locator("[data-role=status]")).toHaveText(
      "inactive: line is vertical",
    );

    // Editing it works, shows its source, and does not call the line gone.
    await alertRows(page).hover();
    await alertRows(page).getByRole("button", { name: "Edit alert" }).click();
    await expect(dialog(page).locator("[data-role=alert-source]")).toContainText("Trend line");
    await dialog(page).getByLabel("Condition").selectOption("crossing_up");
    await dialog(page).getByRole("button", { name: "Save" }).click();
    await expect(dialog(page)).toHaveCount(0);
    await expect(alertRows(page).locator("[data-role=status]")).toHaveText(
      "inactive: line is vertical",
    );

    await dragPoint(page, 0, 1, 200, 60, []);
    await expect(alertRows(page).locator("[data-role=status]")).not.toContainText("inactive");
  });

  test("blur or a hidden tab while a modifier is held restores it", async ({ bootedApp: page }) => {
    for (const fire of [
      () => page.evaluate(() => window.dispatchEvent(new Event("blur"))),
      () => page.evaluate(() => document.dispatchEvent(new Event("visibilitychange"))),
    ]) {
      await page.keyboard.down("Control");
      await page.keyboard.down("Shift");
      await expect(magnetButton(page, "strong")).toBeVisible();
      expect(await shiftHeld(page)).toBe(true);
      await fire();
      await expect(magnetButton(page, "off")).toBeVisible();
      expect(await shiftHeld(page)).toBe(false);
      await page.keyboard.up("Shift");
      await page.keyboard.up("Control");
      await expect(magnetButton(page, "off")).toBeVisible();
      expect(await shiftHeld(page)).toBe(false);
    }
  });

  test("a mouse event resyncs the modifiers", async ({ bootedApp: page }) => {
    const box = (await host(page).boundingBox())!;
    const bare = () =>
      page.evaluate(() =>
        document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true })),
      );

    // A keydown the page never saw (the blur stands in for it): the move catches up.
    await page.keyboard.down("Control");
    await page.keyboard.down("Shift");
    await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await expect(magnetButton(page, "off")).toBeVisible();
    expect(await shiftHeld(page)).toBe(false);
    await page.mouse.move(box.x + 300, box.y + 130);
    await expect(magnetButton(page, "strong")).toBeVisible();
    expect(await shiftHeld(page)).toBe(true);

    // A keyup the page never saw: a move without the modifiers clears them.
    await bare();
    await expect(magnetButton(page, "off")).toBeVisible();
    expect(await shiftHeld(page)).toBe(false);
    await page.keyboard.up("Shift");
    await page.keyboard.up("Control");
  });
});
