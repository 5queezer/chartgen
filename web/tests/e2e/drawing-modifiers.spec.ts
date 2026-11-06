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
  applyNewData: (data: unknown[]) => void;
  overrideOverlay: (o: { id: string; points: Point[] }) => void;
  getOverlayById: (id: string) => { points: Point[] };
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
 * The press waits for a live tick: without it, about 1 run in 40 the first drag after
 * a reload left the point where it was. The chart is still taking its first data then;
 * the exact cause is not pinned down. It is not the tick that used to undo a drag on
 * release, which DrawingToolbar's `refresh` now prevents (tests under "a live tick").
 * One lead: a press on the line's body moves both points, which leaves a vertical line
 * as long as it was. Forcing that took delayed animation frames; 224 drags without
 * this wait, under CPU throttling and long tasks too, all took the point.
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

  test.describe("a live tick during a press", () => {
    /** Fires what a live tick fires: `OnDataReady`, so DrawingToolbar's `refresh`. */
    const tick = (page: Page) =>
      page.evaluate(() => {
        const chart = (window as unknown as TestWindow).__chartgenTest.chart();
        chart.applyNewData(chart.getDataList());
      });

    /** Moves drawing `n` off its committed record; only a `refresh` puts it back. */
    const knockOff = (page: Page, n: number) =>
      page.evaluate((i) => {
        const w = window as unknown as TestWindow & {
          __chartgenDrawings: () => { id: string }[];
        };
        const chart = w.__chartgenTest.chart();
        const { id } = w.__chartgenDrawings()[i]!;
        const points = chart.getOverlayById(id).points.map((p) => ({ ...p, value: 1 }));
        chart.overrideOverlay({ id, points });
      }, n);

    /** A tick still reaches drawing `n`, and places it from its stored record. */
    const expectFollowsTicks = async (page: Page, n: number) => {
      await knockOff(page, n);
      expect((await overlays(page))[n]!.points[0]!.value).toBe(1);
      await tick(page);
      const live = (await overlays(page))[n]!.points;
      const stored = (await storedRecords(page))![n]!.points;
      expect(live.map((p) => p.value)).toEqual(stored.map((p) => p.value));
    };

    /**
     * Two free lines, with the mouse pressed on the end point of the first. `beforePress`
     * runs right before the press, once the mouse is on the point.
     */
    const pressOnLine = async (page: Page, beforePress?: () => Promise<void>) => {
      await page.getByRole("button", { name: "Trend line" }).click();
      await clickAt(page, { x: 300, y: 130 });
      await clickAt(page, { x: 560, y: 260 });
      await page.getByRole("button", { name: "Trend line" }).click();
      await clickAt(page, { x: 700, y: 130 });
      await clickAt(page, { x: 900, y: 260 });
      await expect.poll(async () => (await overlays(page)).length).toBe(2);
      const box = (await host(page).boundingBox())!;
      const [anchor, end] = (await overlays(page))[0]!.pixels;
      await page.mouse.move(box.x + end!.x!, box.y + end!.y!, { steps: 5 });
      await page.waitForTimeout(60);
      await beforePress?.();
      await page.mouse.down();
      return { box, anchor: anchor!, end: end! };
    };

    /** Moves the pressed point, ticks, and asserts the tick left it and the release saved it. */
    const expectDragSurvivesTick = async (
      page: Page,
      { box, anchor }: Awaited<ReturnType<typeof pressOnLine>>,
    ) => {
      await page.mouse.move(box.x + anchor.x! + 200, box.y + anchor.y! + 40, { steps: 8 });
      const moved = (await overlays(page))[0]!.points.map((p) => p.value);
      await tick(page);
      expect((await overlays(page))[0]!.points.map((p) => p.value)).toEqual(moved);
      await page.mouse.up();
      await page.waitForTimeout(100);
      expect((await storedRecords(page))![0]!.points.map((p) => p.value)).toEqual(moved);
    };

    test("pressing takes the focus from a toolbar button, and the press goes on", async ({
      bootedApp: page,
    }) => {
      const magnet = magnetButton(page, "off");
      const press = await pressOnLine(page, () => magnet.focus());
      await expect(magnet).not.toBeFocused();
      await expectDragSurvivesTick(page, press);
    });

    test("a right-button release does not end a left press", async ({ bootedApp: page }) => {
      const press = await pressOnLine(page);
      await page.mouse.up({ button: "right" });
      await expectDragSurvivesTick(page, press);
    });

    /** A line ending on a future bar, with the mouse pressed on that end. */
    const pressOnFuturePoint = async (page: Page) => {
      await page.getByRole("button", { name: "Trend line" }).click();
      await clickAt(page, { x: 700, y: 130 });
      await clickAt(page, { x: 1100, y: 200 });
      await expect.poll(async () => (await overlays(page)).length).toBe(1);
      const lastBar = await page.evaluate(() =>
        (window as unknown as TestWindow).__chartgenTest.chart().getDataList().at(-1)!.timestamp,
      );
      const stored = (await storedRecords(page))![0]!.points;
      expect(stored[1]!.timestamp).toBeGreaterThan(lastBar);
      const box = (await host(page).boundingBox())!;
      const end = (await overlays(page))[0]!.pixels[1]!;
      await page.mouse.move(box.x + end.x!, box.y + end.y!, { steps: 5 });
      await page.waitForTimeout(60);
      await page.mouse.down();
      return { box, end, stored };
    };

    /** Replaces the bars, as a full load does: other bars, and the indices do not shift. */
    const replaceBars = (page: Page) =>
      page.evaluate(() => {
        const chart = (window as unknown as TestWindow).__chartgenTest.chart();
        chart.applyNewData(chart.getDataList().slice(50));
      });

    test("new bars during a press keep the time of a future point", async ({ bootedApp: page }) => {
      const { stored } = await pressOnFuturePoint(page);
      await replaceBars(page);
      await page.mouse.up();
      await page.waitForTimeout(100);
      expect((await storedRecords(page))![0]!.points).toEqual(stored);
    });

    test("after new bars, the press goes on and a tick leaves it alone again", async ({
      bootedApp: page,
    }) => {
      const { box, end } = await pressOnFuturePoint(page);
      await replaceBars(page);
      await page.mouse.move(box.x + end.x! - 60, box.y + end.y! + 30, { steps: 8 });
      const moved = (await overlays(page))[0]!.points[1]!.value;
      await tick(page);
      expect((await overlays(page))[0]!.points[1]!.value).toBe(moved);
      await page.mouse.up();
      await page.waitForTimeout(100);
      expect((await storedRecords(page))![0]!.points[1]!.value).toBe(moved);
    });

    for (const keys of [[], ["Shift"]]) {
      test(`just before the release keeps the drag, keys [${keys.join()}]`, async ({
        bootedApp: page,
      }) => {
        const { box, anchor } = await pressOnLine(page);
        for (const k of keys) await page.keyboard.down(k);
        await page.mouse.move(box.x + anchor.x! + 200, box.y + anchor.y! + 40, { steps: 8 });
        const moved = (await overlays(page))[0]!.points.map((p) => p.value);

        // The other drawing is still refreshed, the pressed one is not.
        await knockOff(page, 1);
        await tick(page);
        expect((await overlays(page))[1]!.points[0]!.value).not.toBe(1);
        expect((await overlays(page))[0]!.points.map((p) => p.value)).toEqual(moved);

        await page.mouse.up();
        for (const k of keys) await page.keyboard.up(k);
        await page.waitForTimeout(100);

        const line = (await overlays(page))[0]!;
        expect(line.points.map((p) => p.value)).toEqual(moved);
        expect((await storedRecords(page))![0]!.points.map((p) => p.value)).toEqual(moved);
        // Shift or not, this is the drag, not the record from before it.
        const dy = line.pixels[1]!.y! - line.pixels[0]!.y!;
        expect(Math.abs(dy - (keys.length > 0 ? 0 : 40))).toBeLessThan(2);

        // Released: it follows ticks again, from the new record.
        await expectFollowsTicks(page, 0);
        expect((await overlays(page))[0]!.points.map((p) => p.value)).toEqual(moved);
      });
    }

    test("a press without a move, then ticks", async ({ bootedApp: page }) => {
      await pressOnLine(page);
      await page.mouse.up();
      await expectFollowsTicks(page, 0);
    });

    test("a release outside the pane, then ticks", async ({ bootedApp: page }) => {
      const { box, end } = await pressOnLine(page);
      const before = (await storedRecords(page))![0]!.points;
      await page.mouse.move(box.x + end.x! + 30, box.y + end.y! - 30, { steps: 4 });
      // Over the drawing toolbar, left of the chart: klinecharts finds no pane there.
      await page.mouse.move(box.x - 20, box.y + end.y!, { steps: 4 });
      await page.mouse.up();
      await expectFollowsTicks(page, 0);
      expect((await storedRecords(page))![0]!.points).toEqual(before);
    });

    /** Pans on empty chart and asserts drawing 0 stayed on its record while the chart scrolled. */
    const expectPanLeavesLine = async (page: Page, box: { x: number; y: number }) => {
      const record = (await storedRecords(page))![0]!.points;
      const from = () =>
        page.evaluate(() =>
          (window as unknown as { __chartgenTest: { chart: () => { getVisibleRange: () => { from: number } } } })
            .__chartgenTest.chart().getVisibleRange().from,
        );
      const before = await from();
      // Far from both lines of `pressOnLine`.
      await page.mouse.move(box.x + 150, box.y + 400, { steps: 4 });
      await page.mouse.down();
      await page.mouse.move(box.x + 330, box.y + 340, { steps: 8 });
      await page.mouse.up();
      await page.waitForTimeout(100);
      expect((await overlays(page))[0]!.points.map((p) => p.value)).toEqual(record.map((p) => p.value));
      expect((await storedRecords(page))![0]!.points).toEqual(record);
      expect(await from()).toBeLessThan(before);
    };

    test("a release outside the pane is discarded at once, and a later pan leaves the drawing", async ({
      bootedApp: page,
    }) => {
      const { box, end } = await pressOnLine(page);
      const before = (await storedRecords(page))![0]!.points;
      await page.mouse.move(box.x + end.x! + 30, box.y + end.y! - 30, { steps: 4 });
      await page.mouse.move(box.x - 20, box.y + end.y!, { steps: 4 });
      await page.mouse.up();
      // No tick needed: right after the release it is back on its record.
      expect((await overlays(page))[0]!.points.map((p) => p.value)).toEqual(before.map((p) => p.value));
      await expectPanLeavesLine(page, box);
    });

    test("a release on a pane separator: a later pan leaves the drawing", async ({ bootedApp: page }) => {
      const { box, end } = await pressOnLine(page);
      await page.mouse.move(box.x + end.x! + 30, box.y + end.y! + 30, { steps: 4 });
      const y = await page.evaluate(() => {
        const chart = (window as unknown as { __chartgenTest: { chart: () => {
          getSize: (id: string, pos: string) => { top: number; height: number };
        } } }).__chartgenTest.chart();
        const pane = chart.getSize("candle_pane", "root");
        return pane.top + pane.height;
      });
      await page.mouse.move(box.x + end.x!, box.y + y, { steps: 4 });
      await page.mouse.up();
      await expectPanLeavesLine(page, box);
    });

    test("a blur during a press: later moves leave the drawing, without a button", async ({
      bootedApp: page,
    }) => {
      const { box, end } = await pressOnLine(page);
      const before = (await storedRecords(page))![0]!.points;
      await page.mouse.move(box.x + end.x! + 30, box.y + end.y! + 30, { steps: 4 });
      await page.evaluate(() => window.dispatchEvent(new Event("blur")));
      expect((await overlays(page))[0]!.points.map((p) => p.value)).toEqual(before.map((p) => p.value));
      // klinecharts no longer thinks the button is down.
      await page.mouse.move(box.x + end.x! + 90, box.y + end.y! + 60, { steps: 4 });
      expect((await overlays(page))[0]!.points.map((p) => p.value)).toEqual(before.map((p) => p.value));
      await page.mouse.up();
      await expectPanLeavesLine(page, box);
    });

    test("a release outside the window, then ticks", async ({ bootedApp: page }) => {
      const { box, end } = await pressOnLine(page);
      await page.mouse.move(box.x + end.x! + 30, box.y + end.y! - 30, { steps: 4 });
      await page.mouse.move(-20, -20, { steps: 4 });
      await page.mouse.up();
      await expectFollowsTicks(page, 0);
    });

    for (const type of ["blur", "touchend", "touchcancel"]) {
      test(`${type} ends the press, without a mouseup`, async ({ bootedApp: page }) => {
        await pressOnLine(page);
        await page.evaluate((t) => window.dispatchEvent(new Event(t)), type);
        await expectFollowsTicks(page, 0);
        await page.mouse.up();
      });
    }

    test("an undo during the press, then ticks", async ({ bootedApp: page }) => {
      await pressOnLine(page);
      await page.keyboard.press("Control+z");
      await expect.poll(async () => (await overlays(page)).length).toBe(1);
      // Still pressed, on a drawing the undo has replaced.
      await expectFollowsTicks(page, 0);
      await page.mouse.up();
      await expectFollowsTicks(page, 0);
    });

    test("an interval change during the press, then ticks", async ({ bootedApp: page }) => {
      const { box, anchor } = await pressOnLine(page);
      await page.mouse.move(box.x + anchor.x! + 200, box.y + anchor.y! + 40, { steps: 8 });
      await page.getByRole("button", { name: "15m", exact: true }).evaluate((b) => (b as HTMLElement).click());
      await expect(page.getByRole("button", { name: "15m", exact: true })).toHaveAttribute("aria-pressed", "true");
      await page.waitForTimeout(300);
      await page.mouse.up();
      await page.waitForTimeout(100);
      await expectFollowsTicks(page, 0);
      await expectFollowsTicks(page, 1);
    });

    test("a symbol change during the press, then ticks", async ({ bootedApp: page }) => {
      const { box, anchor } = await pressOnLine(page);
      await page.mouse.move(box.x + anchor.x! + 200, box.y + anchor.y! + 40, { steps: 8 });
      const row = page.locator('[role=row][data-symbol="ETHUSDT"]');
      await row.evaluate((r) => (r as HTMLElement).click());
      await expect.poll(async () => (await overlays(page)).length).toBe(0);
      await page.mouse.up();

      // Back on the first symbol, its drawings are there and follow ticks.
      await page
        .locator('[role=row][data-symbol="BTCUSDT"]')
        .evaluate((r) => (r as HTMLElement).click());
      await expect.poll(async () => (await overlays(page)).length).toBe(2);
      await expectFollowsTicks(page, 0);
      await expectFollowsTicks(page, 1);
    });
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

  test("a line on one bar of the interval: no alert offered, its alert says so, until the interval changes", async ({
    bootedApp: page,
  }) => {
    const interval = async (name: string) => {
      await page.getByRole("button", { name, exact: true }).click();
      await expect
        .poll(() =>
          page.evaluate(
            () => (window as unknown as { __chartgenTest: { series: () => { interval?: string } | null } })
              .__chartgenTest.series()?.interval,
          ),
        )
        .toBe(name);
      // The chart has the new bars once a drawing's pixels are refreshed from them.
      await page.waitForTimeout(500);
    };
    /** Whether the drawing's menu and the dialog's Source list offer an alert on the line. */
    const offered = async () => {
      const { pixels } = (await overlays(page))[0]!;
      // An extended line on one bar runs through the whole pane, wherever its points are.
      const oneBar = pixels[0]!.x === pixels[1]!.x;
      await clickAt(
        page,
        { x: (pixels[0]!.x! + pixels[1]!.x!) / 2, y: oneBar ? 165 : (pixels[0]!.y! + pixels[1]!.y!) / 2 },
        "right",
      );
      await expect(drawingMenu(page).getByRole("menuitem", { name: "Remove" })).toBeVisible();
      const menu = await drawingMenu(page).getByRole("menuitem", { name: /^Add alert/ }).count();
      await page.keyboard.press("Escape");
      await expect(drawingMenu(page)).toHaveCount(0);
      await page.keyboard.press("Alt+KeyA");
      await expect(dialog(page)).toBeVisible();
      const source = await dialog(page).getByLabel("Source").locator("option", { hasText: "Extended line" }).count();
      await page.keyboard.press("Escape");
      await expect(dialog(page)).toHaveCount(0);
      expect(source).toBe(menu);
      return menu === 1;
    };

    // On 15m, between the :00 and :15 bars of an hour: one bar of 1h, not exactly vertical.
    await interval("15m");
    const [x0, x1] = await page.evaluate(() => {
      const chart = (window as unknown as TestWindow).__chartgenTest.chart() as unknown as {
        getDataList: () => { timestamp: number }[];
        convertToPixel: (p: { dataIndex: number }, f: { paneId: string }) => { x: number };
      };
      const list = chart.getDataList();
      const px = (dataIndex: number) => chart.convertToPixel({ dataIndex }, { paneId: "candle_pane" }).x;
      const i = list.findLastIndex((d, n) => d.timestamp % 3_600_000 === 0 && n < list.length - 2);
      return [px(i), px(i + 1)];
    });
    await pickTool(page, "Extended line");
    await clickAt(page, { x: x0!, y: 100 });
    await clickAt(page, { x: x1!, y: 260 });
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    expect(await offered()).toBe(true);

    // Two alerts on it, one paused.
    await page.getByRole("tab", { name: /^Alerts/ }).click();
    for (const _ of [0, 1]) {
      await page.keyboard.press("Alt+KeyA");
      await dialog(page).getByLabel("Source").selectOption({ label: "Extended line" });
      await dialog(page).getByRole("button", { name: "Create" }).click();
      await expect(dialog(page)).toHaveCount(0);
    }
    await expect(alertRows(page)).toHaveCount(2);
    await alertRows(page).nth(1).hover();
    await alertRows(page).nth(1).getByRole("button", { name: "Pause alert" }).click();
    const live = alertRows(page).nth(0);
    const paused = alertRows(page).nth(1);
    const status = live.locator("[data-role=status]");
    await expect(paused).toHaveAttribute("data-status", "paused");
    await expect(status).not.toContainText("inactive");

    // On 1h no new one is offered, and these are read on 15m: not dead, not called vertical.
    await interval("1h");
    expect(await offered()).toBe(false);
    await expect(status).toHaveText("inactive: open BTCUSDT 15m");
    await interval("15m");
    await expect(status).not.toContainText("inactive");
    await expect(paused).toHaveAttribute("data-status", "paused");
    expect(await offered()).toBe(true);

    // An alert made on 1h before it stopped being offered there: dead on 1h, and says so.
    await page.evaluate(() => {
      const list = JSON.parse(localStorage.getItem("chartgen.alerts.list")!) as {
        source: { interval: string };
      }[];
      for (const a of list) a.source.interval = "1h";
      localStorage.setItem("chartgen.alerts.list", JSON.stringify(list));
    });
    await reloadApp(page);
    await page.getByRole("tab", { name: /^Alerts/ }).click();
    await expect(status).toHaveText("inactive: open BTCUSDT 1h");
    await interval("1h");
    await expect(live).toHaveAttribute("data-status", "inactive");
    await expect(status).toHaveText("inactive: line is vertical");
    await expect(paused).toHaveAttribute("data-status", "paused");

    // The interval changing clears the note, and the paused alert stays paused.
    await interval("15m");
    await expect(status).toHaveText("inactive: open BTCUSDT 1h");
    await expect(paused).toHaveAttribute("data-status", "paused");
    await interval("1h");
    await expect(status).toHaveText("inactive: line is vertical");
    await expect(paused).toHaveAttribute("data-status", "paused");
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
