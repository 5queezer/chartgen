// Drawing modifier keys, as in TradingView: Ctrl (Cmd on Mac) held inverts the magnet and
// Shift held snaps a line to horizontal, vertical or 45 degrees, while placing or
// dragging a point.

import type { Page } from "@playwright/test";

import { test as appTest, expect, gotoApp, pickTool, reloadApp, openSidebarTab } from "./fixtures/app";

/** Not Mac, whatever the host: there Cmd inverts the magnet, and these tests hold Ctrl. */
const test = appTest.extend({
  page: async ({ page }, use) => {
    await page.addInitScript(() =>
      Object.defineProperty(navigator, "platform", { get: () => "Linux x86_64", configurable: true }),
    );
    await use(page);
  },
});

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
  getVisibleRange: () => { from: number };
  getSize: (paneId: string, position: string) => { top: number; height: number };
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

/**
 * A trusted window blur. Bringing another page to the front fires none here: Chromium
 * headless keeps both pages visible and focused, also with focus emulation off.
 * Focus moving into an iframe is a real one.
 */
const loseFocus = (page: Page) =>
  page.evaluate(() => {
    const frame = document.createElement("iframe");
    document.body.append(frame);
    frame.contentWindow!.focus();
  });

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

/** Switch the chart's interval and wait for its bars. */
async function showInterval(page: Page, name: string): Promise<void> {
  await page.getByRole("button", { name, exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { __chartgenTest: { series: () => { interval?: string } | null } })
          .__chartgenTest.series()?.interval,
      ),
    )
    .toBe(name);
  // The chart holds them too, not only the feed.
  await expect
    .poll(() =>
      page.evaluate(() => {
        const w = window as unknown as TestWindow & {
          __chartgenTest: { series: () => { bars: { t: number }[] } | null };
        };
        const times = (ts: number[]) => ts.slice(-2).join();
        return (
          times(w.__chartgenTest.chart().getDataList().map((d) => d.timestamp)) ===
          times((w.__chartgenTest.series()?.bars ?? []).map((b) => b.t * 1000))
        );
      }),
    )
    .toBe(true);
}

/** On 15m, an extended line between the :00 and :15 bars of an hour: one bar of 1h, not exactly vertical. */
async function drawLineWithinAnHour(page: Page): Promise<void> {
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
}

/** Point the stored drawing alerts at `interval`, as if they had been made there. */
const bindAlertsTo = (page: Page, interval: string) =>
  page.evaluate((iv) => {
    const list = JSON.parse(localStorage.getItem("chartgen.alerts.list")!) as {
      source: { interval: string };
    }[];
    for (const a of list) a.source.interval = iv;
    localStorage.setItem("chartgen.alerts.list", JSON.stringify(list));
  }, interval);

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

  test("two quick clicks at different places place both points", async ({ bootedApp: page }) => {
    const box = (await host(page).boundingBox())!;
    const quickClick = async (at: At) => {
      await page.mouse.move(box.x + at.x, box.y + at.y);
      await page.mouse.down();
      await page.mouse.up();
    };

    await page.getByRole("button", { name: "Trend line" }).click();
    await quickClick({ x: 300, y: 130 });
    await page.waitForTimeout(100);
    await quickClick({ x: 560, y: 260 });
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    const { pixels } = (await overlays(page))[0]!;
    expect(Math.abs(pixels[1]!.x! - 560)).toBeLessThan(10);
    expect(Math.abs(pixels[1]!.y! - 260)).toBeLessThan(1);
    await expect(page.getByRole("button", { name: "Trend line" })).toHaveAttribute("aria-pressed", "false");
    expect((await storedRecords(page))![0]!.points).toHaveLength(2);

    // One step of undo removes it: the extra click makes no extra commit.
    await page.keyboard.press("Control+z");
    await expect.poll(async () => (await overlays(page)).length).toBe(0);

    // A quick press that moves is a pan, not a point.
    await page.getByRole("button", { name: "Trend line" }).click();
    await quickClick({ x: 300, y: 130 });
    await page.mouse.move(box.x + 700, box.y + 200);
    await page.mouse.down();
    await page.mouse.move(box.x + 760, box.y + 240, { steps: 4 });
    await page.mouse.up();
    await page.waitForTimeout(100);
    expect(await overlays(page)).toHaveLength(0);
    await expect(page.getByRole("button", { name: "Trend line" })).toHaveAttribute("aria-pressed", "true");
    await page.keyboard.press("Escape");

    // Three points, all quick.
    await pickTool(page, "Parallel channel");
    await quickClick({ x: 300, y: 130 });
    await page.waitForTimeout(50);
    await quickClick({ x: 560, y: 180 });
    await page.waitForTimeout(50);
    await quickClick({ x: 400, y: 260 });
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    expect((await overlays(page))[0]!.points).toHaveLength(3);
    expect(Math.abs((await overlays(page))[0]!.pixels[2]!.y! - 260)).toBeLessThan(1);
  });

  test.describe("a press that moves while a tool is armed places no point", () => {
    const trendLine = (page: Page) => page.getByRole("button", { name: "Trend line" });
    const visibleFrom = (page: Page) =>
      page.evaluate(() => (window as unknown as TestWindow).__chartgenTest.chart().getVisibleRange().from);

    /** Press at `at`, go `away` px right and come back to `back` px from it, then release. */
    const pressAwayAndBack = async (page: Page, at: At, away: number, back = 0) => {
      const box = (await host(page).boundingBox())!;
      await page.mouse.move(box.x + at.x, box.y + at.y);
      await page.mouse.down();
      await page.mouse.move(box.x + at.x + away, box.y + at.y, { steps: 6 });
      const during = await visibleFrom(page);
      await page.mouse.move(box.x + at.x + back, box.y + at.y, { steps: 6 });
      await page.mouse.up();
      await page.waitForTimeout(100);
      return during;
    };

    /** Nothing placed: still armed, nothing drawn or stored. */
    const expectNothingPlaced = async (page: Page) => {
      expect(await overlays(page)).toHaveLength(0);
      expect(await storedRecords(page)).toBeNull();
      await expect(trendLine(page)).toHaveAttribute("aria-pressed", "true");
    };

    test("a pan and return before the first point", async ({ bootedApp: page }) => {
      await trendLine(page).click();
      const before = await visibleFrom(page);
      const during = await pressAwayAndBack(page, { x: 500, y: 200 }, 120);
      expect(during).toBeLessThan(before);
      await expectNothingPlaced(page);

      // The next clicks are the line's two points, not its second and a new line.
      await clickAt(page, { x: 800, y: 100 });
      await expectNothingPlaced(page);
      await clickAt(page, { x: 900, y: 220 });
      await expect.poll(async () => (await overlays(page)).length).toBe(1);
      const { pixels } = (await overlays(page))[0]!;
      expect(Math.abs(pixels[0]!.y! - 100)).toBeLessThan(1);
      expect(Math.abs(pixels[1]!.y! - 220)).toBeLessThan(1);
    });

    test("a pan and return before the second point", async ({ bootedApp: page }) => {
      await trendLine(page).click();
      await clickAt(page, { x: 300, y: 130 });
      const before = await visibleFrom(page);
      const during = await pressAwayAndBack(page, { x: 700, y: 200 }, 120);
      expect(during).toBeLessThan(before);
      await expectNothingPlaced(page);

      await clickAt(page, { x: 900, y: 260 });
      await expect.poll(async () => (await overlays(page)).length).toBe(1);
      expect(Math.abs((await overlays(page))[0]!.pixels[1]!.y! - 260)).toBeLessThan(1);
    });

    test("a jiggle of 5 px or more, ending within 5 px", async ({ bootedApp: page }) => {
      await trendLine(page).click();
      await clickAt(page, { x: 300, y: 130 });
      await pressAwayAndBack(page, { x: 700, y: 200 }, 6, 1);
      await expectNothingPlaced(page);
    });

    test("a quick click that shifts less than 5 px still places its point", async ({ bootedApp: page }) => {
      const box = (await host(page).boundingBox())!;
      await trendLine(page).click();
      await page.mouse.move(box.x + 300, box.y + 130);
      await page.mouse.down();
      await page.mouse.up();
      await page.mouse.move(box.x + 700, box.y + 200);
      await page.mouse.down();
      await page.mouse.move(box.x + 703, box.y + 200);
      await page.mouse.up();
      await expect.poll(async () => (await overlays(page)).length).toBe(1);
      expect(Math.abs((await overlays(page))[0]!.pixels[1]!.y! - 200)).toBeLessThan(1);
    });

    test("a blur during the press places nothing", async ({ bootedApp: page }) => {
      const box = (await host(page).boundingBox())!;
      await trendLine(page).click();
      await clickAt(page, { x: 300, y: 130 });
      await page.mouse.move(box.x + 700, box.y + 200);
      await page.mouse.down();
      await loseFocus(page);
      await page.waitForTimeout(100);
      await page.mouse.up();
      await page.waitForTimeout(100);
      await expectNothingPlaced(page);
    });

    test("a quick second click pinned from another pane still places its point", async ({
      bootedApp: page,
    }) => {
      // Settled first: right after boot this click can be lost, cause not found.
      await page.waitForTimeout(1500);
      const box = (await host(page).boundingBox())!;
      const hdr = await page.locator(".pane-header", { hasText: "RSI" }).first().boundingBox();
      const paneBottom = await page.evaluate(() => {
        const pane = (window as unknown as TestWindow).__chartgenTest.chart().getSize("candle_pane", "root");
        return pane.top + pane.height;
      });
      await trendLine(page).click();
      await page.mouse.move(box.x + 300, box.y + 130);
      await page.mouse.down();
      await page.mouse.up();
      await page.waitForTimeout(80);
      await page.mouse.move(box.x + 700, hdr!.y + 30);
      await page.mouse.down();
      await page.mouse.up();
      await expect.poll(async () => (await overlays(page)).length).toBe(1);
      const { pixels } = (await overlays(page))[0]!;
      expect(Math.abs(pixels[1]!.x! - 700)).toBeLessThan(10);
      expect(pixels[1]!.y!).toBeLessThan(paneBottom);
    });
  });

  test.describe("a double click while a tool is armed", () => {
    const pressTwice = async (page: Page, at: At, then: At = at) => {
      const box = (await host(page).boundingBox())!;
      await page.mouse.move(box.x + at.x, box.y + at.y);
      await page.mouse.down();
      await page.mouse.up();
      await page.waitForTimeout(60);
      await page.mouse.move(box.x + then.x, box.y + then.y);
      await page.mouse.down();
      await page.mouse.up();
      await page.waitForTimeout(200);
    };

    /** On the chart a double click only places the first point: the tool waits for the second. */
    const firstPointOnly = async (page: Page, tool: string, at: At, then: At) => {
      await page.waitForTimeout(1500);
      await pickTool(page, tool);
      await pressTwice(page, at);
      expect(await overlays(page)).toHaveLength(0);
      expect(await storedRecords(page)).toBeNull();
      await expect(page.getByRole("button", { name: tool })).toHaveAttribute("aria-pressed", "true");
      await page.waitForTimeout(700);
      await clickAt(page, then);
      await expect.poll(async () => (await overlays(page)).length).toBe(1);
      const { pixels } = (await overlays(page))[0]!;
      for (const [i, want] of [at, then].entries()) {
        expect(Math.abs(pixels[i]!.x! - want.x)).toBeLessThan(10);
        expect(Math.abs(pixels[i]!.y! - want.y)).toBeLessThan(1);
      }
      expect(await storedRecords(page)).toHaveLength(1);
    };

    for (const tool of ["Trend line", "Ray", "Extended line"]) {
      test(`${tool} on the chart: the first point only, no zero-length line`, async ({ bootedApp: page }) => {
        await firstPointOnly(page, tool, { x: 500, y: 200 }, { x: 700, y: 120 });
      });
    }

    test("on a future bar: the first point only too", async ({ bootedApp: page }) => {
      await firstPointOnly(page, "Trend line", { x: 1100, y: 200 }, { x: 900, y: 120 });
      const lastBar = await page.evaluate(() =>
        (window as unknown as TestWindow).__chartgenTest.chart().getDataList().at(-1)!.timestamp,
      );
      expect((await storedRecords(page))![0]!.points[0]!.timestamp).toBeGreaterThan(lastBar);
    });

    test("a third quick click, or a slow second one, on the first point places nothing", async ({
      bootedApp: page,
    }) => {
      await page.waitForTimeout(1500);
      const line = page.getByRole("button", { name: "Trend line" });
      await line.click();
      await pressTwice(page, { x: 500, y: 200 });
      await clickAt(page, { x: 500, y: 200 });
      await clickAt(page, { x: 500, y: 200 });
      expect(await overlays(page)).toHaveLength(0);
      await expect(line).toHaveAttribute("aria-pressed", "true");
      await clickAt(page, { x: 700, y: 120 });
      await expect.poll(async () => (await overlays(page)).length).toBe(1);
      expect(Math.abs((await overlays(page))[0]!.pixels[1]!.x! - 700)).toBeLessThan(10);
    });

    test("a second point on the first one's bar at another price still makes a line", async ({
      bootedApp: page,
    }) => {
      await page.getByRole("button", { name: "Trend line" }).click();
      await clickAt(page, { x: 500, y: 200 });
      await clickAt(page, { x: 500, y: 150 });
      await expect.poll(async () => (await overlays(page)).length).toBe(1);
      const { pixels } = (await overlays(page))[0]!;
      expect(pixels[1]!.y!).toBeLessThan(pixels[0]!.y!);
    });

    /** Off the pane klinecharts sends a double click nowhere: it must not become a second click. */
    for (const [name, spot] of [
      ["y-axis", (w: number, _h: number) => ({ x: w - 20, y: 200 })],
      ["x-axis", (_w: number, h: number) => ({ x: 700, y: h - 8 })],
    ] as const) {
      test(`on the ${name}: not sent again, so no drawing`, async ({ bootedApp: page }) => {
        // Settled first: right after boot a quick click can be lost, cause not found.
        await page.waitForTimeout(1500);
        const box = (await host(page).boundingBox())!;
        await page.getByRole("button", { name: "Trend line" }).click();
        await pressTwice(page, spot(box.width, box.height));
        expect(await overlays(page)).toHaveLength(0);
        await expect(page.getByRole("button", { name: "Trend line" })).toHaveAttribute("aria-pressed", "true");
      });
    }

    test("is measured as klinecharts does: 4 px on both axes is two clicks", async ({ bootedApp: page }) => {
      await page.waitForTimeout(1500);
      await page.getByRole("button", { name: "Trend line" }).click();
      await pressTwice(page, { x: 500, y: 200 }, { x: 504, y: 204 });
      await expect.poll(async () => (await overlays(page)).length).toBe(1);
      expect(Math.abs((await overlays(page))[0]!.pixels[1]!.y! - 204)).toBeLessThan(1);
    });
  });

  test("a quick Shift click snaps its point like a slow one", async ({ bootedApp: page }) => {
    const box = (await host(page).boundingBox())!;
    await page.getByRole("button", { name: "Trend line" }).click();
    await page.mouse.move(box.x + 300, box.y + 130);
    await page.mouse.down();
    await page.mouse.up();
    await page.keyboard.down("Shift");
    await page.mouse.move(box.x + 560, box.y + 150);
    await page.mouse.down();
    await page.mouse.up();
    await page.keyboard.up("Shift");
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    const { points } = (await overlays(page))[0]!;
    expect(points[1]!.value).toBe(points[0]!.value);
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

  test("the Meta key does not invert the magnet off Mac", async ({ bootedApp: page }) => {
    await page.keyboard.down("Meta");
    await page.waitForTimeout(100);
    await expect(magnetButton(page, "off")).toHaveAttribute("aria-pressed", "false");
    await page.keyboard.up("Meta");
  });

  test("on Mac, Cmd inverts the magnet and Ctrl does not", async ({ bootedApp: page }) => {
    await page.addInitScript(() =>
      Object.defineProperty(navigator, "platform", { get: () => "MacIntel" }),
    );
    await reloadApp(page);
    await page.keyboard.down("Meta");
    await expect(magnetButton(page, "strong")).toHaveAttribute("aria-pressed", "true");
    await page.keyboard.up("Meta");
    await expect(magnetButton(page, "off")).toHaveAttribute("aria-pressed", "false");
    await page.keyboard.down("Control");
    await page.waitForTimeout(100);
    await expect(magnetButton(page, "off")).toHaveAttribute("aria-pressed", "false");
    await page.keyboard.up("Control");
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
      // Mouse back on the anchor: zero length, so no line. A NaN from the snap would not
      // count as the same point, and would finish one.
      await shiftDraw(page, tool, { x: 1100, y: 130 }, { x: 1100, y: 130 });
      await page.keyboard.press("Escape");
      await expect.poll(async () => (await overlays(page)).length).toBe(3);

      const [level, upright, diagonal] = await overlays(page);
      expect(level!.points[1]!.value).toBe(level!.points[0]!.value);
      expect(level!.points[1]!.timestamp).toBeGreaterThan(level!.points[0]!.timestamp!);
      expect(upright!.points[1]!.timestamp).toBe(upright!.points[0]!.timestamp);
      // A vertical's value follows the mouse.
      expect(Math.abs(upright!.pixels[1]!.y! - 300)).toBeLessThan(1);
      expectDiagonal(diagonal!.pixels);

      // All stored, the vertical one still vertical after a restore.
      await expect.poll(async () => (await storedRecords(page))?.length).toBe(3);
      await reloadApp(page);
      await expect.poll(async () => (await overlays(page)).length).toBe(3);
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

    /**
     * Moves drawing `n` off its committed record; only a `refresh` puts it back. Returns its
     * first value read in the same step, as a load landing later would rightly put it back.
     */
    const knockOff = (page: Page, n: number) =>
      page.evaluate((i) => {
        const w = window as unknown as TestWindow & {
          __chartgenDrawings: () => { id: string }[];
        };
        const chart = w.__chartgenTest.chart();
        const { id } = w.__chartgenDrawings()[i]!;
        const points = chart.getOverlayById(id).points.map((p) => ({ ...p, value: 1 }));
        chart.overrideOverlay({ id, points });
        return chart.getOverlayById(id).points[0]!.value;
      }, n);

    /** A tick still reaches drawing `n`, and places it from its stored record. */
    const expectFollowsTicks = async (page: Page, n: number) => {
      expect(await knockOff(page, n)).toBe(1);
      await tick(page);
      const live = (await overlays(page))[n]!.points;
      const stored = (await storedRecords(page))![n]!.points;
      expect(live.map((p) => p.value)).toEqual(stored.map((p) => p.value));
    };

    const drawTwoLines = async (page: Page) => {
      await page.getByRole("button", { name: "Trend line" }).click();
      await clickAt(page, { x: 300, y: 130 });
      await clickAt(page, { x: 560, y: 260 });
      await page.getByRole("button", { name: "Trend line" }).click();
      await clickAt(page, { x: 700, y: 130 });
      await clickAt(page, { x: 900, y: 260 });
      await expect.poll(async () => (await overlays(page)).length).toBe(2);
    };

    /**
     * Two free lines, with the mouse pressed on the end point of the first. `beforePress`
     * runs right before the press, once the mouse is on the point.
     */
    const pressOnLine = async (page: Page, beforePress?: () => Promise<void>) => {
      await drawTwoLines(page);
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
      // A real tick puts the full bars back, which places the drawing once more: start
      // right after one, so that none comes before the assertions.
      await nextDataTick(page);
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
      // Far from both lines of `pressOnLine`, at the foot of the candles by the pane's own
      // size: a press on the legend row of the pane below would not pan.
      const y = await page.evaluate(() => {
        const chart = (window as unknown as { __chartgenTest: { chart: () => {
          getSize: (id: string, pos: string) => { top: number; height: number };
        } } }).__chartgenTest.chart();
        const pane = chart.getSize("candle_pane", "root");
        return pane.top + pane.height - 40;
      });
      await page.mouse.move(box.x + 150, box.y + y, { steps: 4 });
      await page.mouse.down();
      await page.mouse.move(box.x + 330, box.y + y - 60, { steps: 8 });
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

    test("a real blur during a plain pan ends the pan", async ({ bootedApp: page }) => {
      const box = (await host(page).boundingBox())!;
      const from = () =>
        page.evaluate(() => (window as unknown as TestWindow).__chartgenTest.chart().getVisibleRange().from);
      const start = await from();
      await page.mouse.move(box.x + 500, box.y + 300);
      await page.mouse.down();
      await page.mouse.move(box.x + 740, box.y + 300, { steps: 6 });
      await loseFocus(page);
      await page.waitForTimeout(50);
      const stopped = await from();
      expect(stopped).toBeLessThan(start);
      await page.mouse.move(box.x + 1000, box.y + 300, { steps: 6 });
      expect(await from()).toBe(stopped);
      await page.mouse.up();
      await page.waitForTimeout(100);
      expect(await from()).toBe(stopped);
    });

    test("a real blur with nothing pressed changes nothing", async ({ bootedApp: page }) => {
      await drawTwoLines(page);
      const box = (await host(page).boundingBox())!;
      const stateBefore = {
        range: await page.evaluate(() => (window as unknown as TestWindow).__chartgenTest.chart().getVisibleRange().from),
        stored: await storedRecords(page),
        undo: await page.getByRole("button", { name: "Undo" }).isEnabled(),
      };
      // A tool armed, with one point placed.
      await page.getByRole("button", { name: "Trend line" }).click();
      await clickAt(page, { x: 200, y: 180 });

      await loseFocus(page);
      await page.mouse.move(box.x + 500, box.y + 300, { steps: 4 });
      await page.waitForTimeout(100);

      expect(await page.evaluate(() => (window as unknown as TestWindow).__chartgenTest.chart().getVisibleRange().from)).toBe(stateBefore.range);
      expect(await storedRecords(page)).toEqual(stateBefore.stored);
      expect(await page.getByRole("button", { name: "Undo" }).isEnabled()).toBe(stateBefore.undo);
      await expect(page.getByRole("button", { name: "Trend line" })).toHaveAttribute("aria-pressed", "true");

      // The tool still works: the next click is the line's second point.
      await clickAt(page, { x: 400, y: 280 });
      await expect.poll(async () => (await overlays(page)).length).toBe(3);
      expect(Math.abs((await overlays(page))[2]!.pixels[0]!.y! - 180)).toBeLessThan(1);
      expect(Math.abs((await overlays(page))[2]!.pixels[1]!.y! - 280)).toBeLessThan(1);
    });

    test("a stale press, its release never heard: the next press ends it, and the pan leaves the drawing", async ({
      bootedApp: page,
    }) => {
      await drawTwoLines(page);
      const box = (await host(page).boundingBox())!;
      // What Firefox can leave behind on a release outside the window (klinecharts takes
      // a mouseleave as the release, finds no pane, and no mouseup follows).
      await page.evaluate(() => {
        const w = window as unknown as TestWindow & { __chartgenDrawings: () => { id: string }[] };
        const chart = w.__chartgenTest.chart() as unknown as TestChart & {
          getChartStore: () => { getOverlayStore: () => { setPressedInstanceInfo: (i: unknown) => void } };
        };
        chart.getChartStore().getOverlayStore().setPressedInstanceInfo({
          paneId: "candle_pane",
          instance: chart.getOverlayById(w.__chartgenDrawings()[0]!.id),
          figureType: 1,
          figureKey: "overlay_figure_point_1",
          figureIndex: 1,
          attrsIndex: 0,
        });
      });
      await expectPanLeavesLine(page, box);
    });

    test("a real blur ends the press: it stays discarded, and later moves leave the drawing", async ({
      bootedApp: page,
    }) => {
      const { box, end } = await pressOnLine(page);
      const before = (await storedRecords(page))![0]!.points.map((p) => p.value);
      await page.mouse.move(box.x + end.x! + 30, box.y + end.y! + 30, { steps: 4 });
      expect((await overlays(page))[0]!.points.map((p) => p.value)).not.toEqual(before);

      await loseFocus(page);
      await expect.poll(async () => (await overlays(page))[0]!.points.map((p) => p.value)).toEqual(before);
      await page.mouse.move(box.x + end.x! + 90, box.y + end.y! + 60, { steps: 4 });
      expect((await overlays(page))[0]!.points.map((p) => p.value)).toEqual(before);
      await expectFollowsTicks(page, 0);

      await page.mouse.up();
      await page.waitForTimeout(100);
      expect((await storedRecords(page))![0]!.points.map((p) => p.value)).toEqual(before);
    });

    test.describe("with real touches", () => {
      test.use({ hasTouch: true });

      /** Sends real touch input to the page: `at` in chart pixels, none to lift the finger. */
      const toucher = async (page: Page) => {
        const cdp = await page.context().newCDPSession(page);
        const box = (await host(page).boundingBox())!;
        const send = (type: string, at?: At) =>
          cdp.send("Input.dispatchTouchEvent", {
            type: type as "touchStart",
            touchPoints: at ? [{ x: box.x + at.x, y: box.y + at.y }] : [],
          });
        return {
          box,
          start: (at: At) => send("touchStart", at),
          end: () => send("touchEnd"),
          cancel: () => send("touchCancel"),
          move: async (from: At, to: At) => {
            for (let i = 1; i <= 8; i++) {
              await send("touchMove", {
                x: from.x + ((to.x - from.x) * i) / 8,
                y: from.y + ((to.y - from.y) * i) / 8,
              });
            }
          },
        };
      };

      /** Two lines, the first selected by a tap so its points can be pressed; its end point. */
      const selectFirstLine = async (page: Page, touch: Awaited<ReturnType<typeof toucher>>) => {
        await drawTwoLines(page);
        await page.mouse.move(touch.box.x + 40, touch.box.y + 40);
        const [a, b] = (await overlays(page))[0]!.pixels;
        await touch.start({ x: (a!.x! + b!.x!) / 2, y: (a!.y! + b!.y!) / 2 });
        await touch.end();
        await page.waitForTimeout(100);
        return { x: b!.x!, y: b!.y! };
      };

      test.describe("quick taps while a tool is armed", () => {
        const tap = async (touch: Awaited<ReturnType<typeof toucher>>, at: At) => {
          await touch.start(at);
          await touch.end();
        };

        test("two taps place both points", async ({ bootedApp: page }) => {
          const touch = await toucher(page);
          await page.getByRole("button", { name: "Trend line" }).click();
          await tap(touch, { x: 500, y: 200 });
          await page.waitForTimeout(60);
          await tap(touch, { x: 700, y: 120 });
          await expect.poll(async () => (await overlays(page)).length).toBe(1);
          const { pixels } = (await overlays(page))[0]!;
          expect(Math.abs(pixels[0]!.x! - 500)).toBeLessThan(10);
          expect(Math.abs(pixels[1]!.x! - 700)).toBeLessThan(10);
          expect(Math.abs(pixels[1]!.y! - 120)).toBeLessThan(1);
          expect(await storedRecords(page)).toHaveLength(1);
        });

        test("two taps 30 px apart are two points, as klinecharts measures it", async ({ bootedApp: page }) => {
          const touch = await toucher(page);
          await page.getByRole("button", { name: "Trend line" }).click();
          await tap(touch, { x: 500, y: 200 });
          await page.waitForTimeout(60);
          await tap(touch, { x: 515, y: 215 });
          await expect.poll(async () => (await overlays(page)).length).toBe(1);
          expect(Math.abs((await overlays(page))[0]!.pixels[1]!.y! - 215)).toBeLessThan(1);
        });

        test("a double tap places the first point only", async ({ bootedApp: page }) => {
          const touch = await toucher(page);
          const line = page.getByRole("button", { name: "Trend line" });
          await line.click();
          await tap(touch, { x: 500, y: 200 });
          await page.waitForTimeout(60);
          await tap(touch, { x: 510, y: 205 });
          await page.waitForTimeout(700);
          expect(await overlays(page)).toHaveLength(0);
          await expect(line).toHaveAttribute("aria-pressed", "true");
          await tap(touch, { x: 700, y: 120 });
          await expect.poll(async () => (await overlays(page)).length).toBe(1);
          const { pixels } = (await overlays(page))[0]!;
          expect(Math.abs(pixels[0]!.x! - 500)).toBeLessThan(10);
          expect(Math.abs(pixels[1]!.x! - 700)).toBeLessThan(10);
        });

        test("three taps place all three points of a channel", async ({ bootedApp: page }) => {
          const touch = await toucher(page);
          await pickTool(page, "Parallel channel");
          await tap(touch, { x: 500, y: 200 });
          await page.waitForTimeout(60);
          await tap(touch, { x: 700, y: 120 });
          await page.waitForTimeout(60);
          await tap(touch, { x: 600, y: 260 });
          await expect.poll(async () => (await overlays(page)).length).toBe(1);
          const { pixels } = (await overlays(page))[0]!;
          expect(pixels).toHaveLength(3);
          expect(Math.abs(pixels[2]!.y! - 260)).toBeLessThan(1);
        });

        test("a pan right after the first tap places nothing", async ({ bootedApp: page }) => {
          const touch = await toucher(page);
          const line = page.getByRole("button", { name: "Trend line" });
          await line.click();
          await tap(touch, { x: 500, y: 200 });
          await page.waitForTimeout(60);
          await touch.start({ x: 700, y: 120 });
          await touch.move({ x: 700, y: 120 }, { x: 640, y: 120 });
          await touch.move({ x: 640, y: 120 }, { x: 700, y: 120 });
          await touch.end();
          await page.waitForTimeout(300);
          expect(await overlays(page)).toHaveLength(0);
          await expect(line).toHaveAttribute("aria-pressed", "true");
        });
      });

      test("a touch drag survives a live tick, and the release saves it once", async ({
        bootedApp: page,
      }) => {
        const touch = await toucher(page);
        const end = await selectFirstLine(page, touch);
        const before = (await overlays(page))[0]!.points.map((p) => p.value);

        await touch.start(end);
        await touch.move(end, { x: end.x + 60, y: end.y + 40 });
        const moved = (await overlays(page))[0]!.points.map((p) => p.value);
        expect(moved).not.toEqual(before);
        await tick(page);
        expect((await overlays(page))[0]!.points.map((p) => p.value)).toEqual(moved);
        await touch.end();
        await page.waitForTimeout(100);
        expect((await storedRecords(page))![0]!.points.map((p) => p.value)).toEqual(moved);
        await expectFollowsTicks(page, 0);
        expect((await overlays(page))[0]!.points.map((p) => p.value)).toEqual(moved);

        // One undo step goes back to before the drag.
        await page.keyboard.press("Control+z");
        await expect.poll(async () => (await overlays(page))[0]!.points.map((p) => p.value)).toEqual(before);
      });

      test("a touch released off the panes is discarded, and a later touch pan leaves the drawing", async ({
        bootedApp: page,
      }) => {
        const touch = await toucher(page);
        const end = await selectFirstLine(page, touch);
        const before = (await storedRecords(page))![0]!.points;

        await touch.start(end);
        await touch.move(end, { x: end.x + 30, y: end.y - 30 });
        // Over the drawing toolbar, left of the chart.
        await touch.move({ x: end.x + 30, y: end.y - 30 }, { x: -20, y: end.y });
        await touch.end();
        await expect
          .poll(async () => (await overlays(page))[0]!.points.map((p) => p.value))
          .toEqual(before.map((p) => p.value));

        const from = { x: 150, y: 400 };
        await touch.start(from);
        await touch.move(from, { x: 330, y: 340 });
        await touch.end();
        await page.waitForTimeout(100);
        expect((await overlays(page))[0]!.points.map((p) => p.value)).toEqual(before.map((p) => p.value));
        expect((await storedRecords(page))![0]!.points).toEqual(before);
      });

      test("a cancelled touch is discarded", async ({ bootedApp: page }) => {
        const touch = await toucher(page);
        const end = await selectFirstLine(page, touch);
        const before = (await storedRecords(page))![0]!.points;

        await touch.start(end);
        await touch.move(end, { x: end.x + 60, y: end.y + 40 });
        await touch.cancel();
        await expect
          .poll(async () => (await overlays(page))[0]!.points.map((p) => p.value))
          .toEqual(before.map((p) => p.value));
        await expectFollowsTicks(page, 0);
        expect((await storedRecords(page))![0]!.points).toEqual(before);

        // klinecharts' own press ended too. It ignores mouse events for 500 ms after a touch.
        await page.waitForTimeout(600);
        await expectPanLeavesLine(page, touch.box);
      });
    });

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
    await openSidebarTab(page, /^Alerts/);
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
    const interval = (name: string) => showInterval(page, name);
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
    await drawLineWithinAnHour(page);
    expect(await offered()).toBe(true);

    // Two alerts on it, one paused.
    await openSidebarTab(page, /^Alerts/);
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
    await bindAlertsTo(page, "1h");
    await reloadApp(page);
    await openSidebarTab(page, /^Alerts/);
    await expect(status).toHaveText("inactive: open BTCUSDT 1h");
    await interval("1h");
    await expect(live).toHaveAttribute("data-status", "inactive");
    await expect(status).toHaveText("inactive: line is vertical");
    await expect(paused).toHaveAttribute("data-status", "paused");

    // Another interval gives its own reason, and the paused alert stays paused.
    await interval("15m");
    await expect(status).toHaveText("inactive: open BTCUSDT 1h");
    await expect(paused).toHaveAttribute("data-status", "paused");
    await interval("1h");
    await expect(status).toHaveText("inactive: line is vertical");
    await expect(paused).toHaveAttribute("data-status", "paused");
  });

  test.describe("a stored two-point horizontal ray, from when the tool took two clicks", () => {
    /** Stores a ray with its second point `apart` ms after the first, with alerts on it, on 1h. */
    const seedRay = async (page: Page, apart: number) => {
      await showInterval(page, "1h");
      const level = await page.evaluate((ms) => {
        const bar = (window as unknown as TestWindow).__chartgenTest.chart().getDataList().at(-5)!;
        localStorage.setItem(
          "chartgen.drawings.BTCUSDT",
          JSON.stringify([
            {
              id: "ray",
              name: "horizontalRayLine",
              points: [
                { timestamp: bar.timestamp, value: bar.close },
                { timestamp: bar.timestamp + ms, value: bar.close },
              ],
              lock: false,
              visible: true,
            },
          ]),
        );
        const alert = { symbol: "BTCUSDT", condition: "crossing_up", trigger: "every", active: true, createdAt: 1 };
        const source = { kind: "drawing", drawingId: "ray", label: "Horizontal ray" };
        localStorage.setItem(
          "chartgen.alerts.list",
          JSON.stringify([
            { ...alert, id: "plain", source },
            // No dialog makes this one; it holds the bar check off the horizontal path too.
            { ...alert, id: "bound", source: { ...source, interval: "1h" } },
          ]),
        );
        return bar.close;
      }, apart);
      await reloadApp(page);
      await expect.poll(async () => (await overlays(page)).length).toBe(1);
      await openSidebarTab(page, /^Alerts/);
      return level;
    };

    const statuses = (page: Page) => alertRows(page).locator("[data-role=status]");

    test("on one bar: offered, not called vertical, and it fires", async ({ bootedApp: page }) => {
      const level = await seedRay(page, 15 * 60_000);
      expect((await overlays(page))[0]!.points).toHaveLength(2);
      await expect(statuses(page)).toHaveText(["active", "active"]);

      const { x, y } = (await overlays(page))[0]!.pixels[0]!;
      await clickAt(page, { x: x! + 15, y: y! }, "right");
      await expect(drawingMenu(page).getByRole("menuitem", { name: /^Add alert on Horizontal ray/ })).toBeVisible();
      await page.keyboard.press("Escape");
      await page.keyboard.press("Alt+KeyA");
      await expect(dialog(page).getByLabel("Source").locator("option", { hasText: "Horizontal ray" })).toHaveCount(1);
      await page.keyboard.press("Escape");
      await expect(dialog(page)).toHaveCount(0);

      const fired = await page.evaluate((v) => {
        const hook = (window as unknown as { __chartgenTest: { feedPrice: (s: string, p: number) => void } }).__chartgenTest;
        hook.feedPrice("BTCUSDT", v - 1);
        hook.feedPrice("BTCUSDT", v + 1);
        return (JSON.parse(localStorage.getItem("chartgen.alerts.list")!) as { lastTriggeredAt?: number }[]).map(
          (a) => a.lastTriggeredAt !== undefined,
        );
      }, level);
      expect(fired).toEqual([true, true]);
    });

    test("with equal times: a save does not switch its alerts off", async ({ bootedApp: page }) => {
      await seedRay(page, 0);
      await expect(statuses(page)).toHaveText(["active", "active"]);
      await page.keyboard.press("Alt+KeyA");
      await expect(dialog(page).getByLabel("Source").locator("option", { hasText: "Horizontal ray" })).toHaveCount(1);
      await page.keyboard.press("Escape");
      await expect(dialog(page)).toHaveCount(0);

      await page.getByRole("button", { name: "Lock all drawings" }).click();
      await expect
        .poll(async () => (await storedRecords(page))?.[0] as unknown as { lock: boolean })
        .toMatchObject({ lock: true });
      await expect(statuses(page)).toHaveText(["active", "active"]);
    });
  });

  test("a window does not read a line alert as running while the leading one cannot evaluate it", async ({
    bootedApp: page,
    context,
  }) => {
    await showInterval(page, "1h");
    await page.getByRole("button", { name: "Trend line" }).click();
    await clickAt(page, { x: 300, y: 165 });
    await clickAt(page, { x: 560, y: 260 });
    await expect.poll(async () => (await overlays(page)).length).toBe(1);
    await openSidebarTab(page, /^Alerts/);
    await page.keyboard.press("Alt+KeyA");
    await dialog(page).getByLabel("Source").selectOption({ label: "Trend line" });
    await dialog(page).getByRole("button", { name: "Create" }).click();
    await expect(dialog(page)).toHaveCount(0);

    const other = await context.newPage();
    await gotoApp(other);
    await openSidebarTab(other, /^Alerts/);
    await showInterval(other, "1h");
    const statusIn = (p: Page) => alertRows(p).first().locator("[data-role=status]");
    await expect(other.locator("[data-role=alerts-elsewhere]")).toBeVisible();
    await expect(statusIn(other)).toHaveText("active");

    // The leading window goes elsewhere: nothing evaluates the line now, whatever this one shows.
    await showInterval(page, "15m");
    await expect(statusIn(other)).toHaveText("inactive: open BTCUSDT 1h in the other tab");
    await expect(statusIn(page)).toHaveText("inactive: open BTCUSDT 1h");

    await showInterval(page, "1h");
    await expect(statusIn(other)).toHaveText("active");

    // It stays that way when this window gets the lead: then its own chart is what counts.
    await showInterval(page, "15m");
    await expect(statusIn(other)).toHaveText("inactive: open BTCUSDT 1h in the other tab");
    await page.close();
    await expect(other.locator("[data-role=alerts-elsewhere]")).toHaveCount(0);
    await expect(statusIn(other)).toHaveText("active");
  });

  test("two windows on different intervals each say what holds for them, and write nothing", async ({
    bootedApp: page,
    context,
  }) => {
    // Once widened, the line can sit within the sample price's wobble at some hours and fire.
    // An alert that repeats stays active when it does, and when it last fired is no verdict.
    const stored = () =>
      page.evaluate(() =>
        (JSON.parse(localStorage.getItem("chartgen.alerts.list")!) as { lastTriggeredAt?: number }[]).map(
          ({ lastTriggeredAt: _, ...alert }) => alert,
        ),
      );

    await showInterval(page, "15m");
    await drawLineWithinAnHour(page);
    await openSidebarTab(page, /^Alerts/);
    await page.keyboard.press("Alt+KeyA");
    await dialog(page).getByLabel("Source").selectOption({ label: "Extended line" });
    await dialog(page).getByLabel("Trigger").selectOption("every");
    await dialog(page).getByRole("button", { name: "Create" }).click();
    await expect(dialog(page)).toHaveCount(0);
    await bindAlertsTo(page, "1h");
    await reloadApp(page);
    await openSidebarTab(page, /^Alerts/);
    await showInterval(page, "1h");

    const statusIn = (p: Page) => alertRows(p).first().locator("[data-role=status]");
    await expect(statusIn(page)).toHaveText("inactive: line is vertical");
    const list = await stored();
    const [alert] = list as { active?: boolean; note?: string }[];
    expect(alert!.active).toBe(true);
    expect(alert).not.toHaveProperty("note");

    // The second window, on another interval, shares the storage and not the verdict.
    const other = await context.newPage();
    await gotoApp(other);
    await openSidebarTab(other, /^Alerts/);
    await showInterval(other, "15m");
    await expect(statusIn(other)).toHaveText("inactive: open BTCUSDT 1h");
    await page.waitForTimeout(1500);
    await expect(statusIn(page)).toHaveText("inactive: line is vertical");
    expect(await stored()).toEqual(list);

    // The other window leads from here on, so that its chart is the one evaluated.
    await reloadApp(page);
    await openSidebarTab(page, /^Alerts/);
    await expect(page.locator("[data-role=alerts-elsewhere]")).toBeVisible();

    // Each keeps its own through more changes on the other side.
    await showInterval(other, "1h");
    await expect(statusIn(other)).toHaveText("inactive: line is vertical");
    await showInterval(page, "15m");
    await expect(statusIn(page)).toHaveText("inactive: open BTCUSDT 1h");
    await page.waitForTimeout(1500);
    await expect(statusIn(page)).toHaveText("inactive: open BTCUSDT 1h");
    expect(await stored()).toEqual(list);

    // The line widened here: the other window's reason goes, through the storage event.
    await dragPoint(page, 0, 1, -260, 150, []);
    await expect(statusIn(other)).not.toContainText("inactive");
    await expect(statusIn(page)).toHaveText("inactive: open BTCUSDT 1h");
    expect(await stored()).toEqual(list);

    // Undone on the interval it runs on: this window's reason is back, with no alert changed.
    await showInterval(page, "1h");
    await expect(statusIn(page)).not.toContainText("inactive");
    await page.keyboard.press("Control+z");
    await expect(statusIn(page)).toHaveText("inactive: line is vertical");
    await expect(statusIn(other)).toHaveText("inactive: line is vertical");
    expect(await stored()).toEqual(list);
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

  test("a real blur while a modifier is held restores it", async ({ bootedApp: page }) => {
    await page.keyboard.down("Control");
    await page.keyboard.down("Shift");
    await expect(magnetButton(page, "strong")).toBeVisible();
    expect(await shiftHeld(page)).toBe(true);
    await loseFocus(page);
    await expect(magnetButton(page, "off")).toBeVisible();
    expect(await shiftHeld(page)).toBe(false);
    // The keys come up elsewhere; the page never sees it.
    await page.keyboard.up("Shift");
    await page.keyboard.up("Control");
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
