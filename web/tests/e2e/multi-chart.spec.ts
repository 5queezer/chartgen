// Multi-chart grid (ADR-0043): grid switching, per-chart isolation, routing to
// the active chart, saved grids, old layouts, live data in each chart, the
// compact layout and clean unmounts. Charts are read through the DEV-only
// `window.__chartgenTest.panes()` hook in App.tsx.

import type { Page } from "@playwright/test";

import { test, expect } from "./fixtures/app";

interface PaneInfo {
  pane: number;
  symbol: string;
  timeframe: string;
  indicators: unknown[];
  invert: boolean;
  reverse: boolean | null;
  candleIndicators: string[];
  candleHeight: number | null;
  stackHeight: number | null;
  staleCanvases: number;
  bars: number;
  version: number | null;
  updateKind: string | null;
  poll: string;
  drawings: number;
}

const panes = (page: Page): Promise<PaneInfo[]> =>
  page.evaluate(() =>
    (window as unknown as { __chartgenTest: { panes: () => PaneInfo[] } }).__chartgenTest.panes(),
  );

const paneCell = (page: Page, n: number) =>
  page.locator(`[data-testid=chart-pane][data-pane="${n}"]`);

/** Press a chart at a spot of its plot that holds no legend or control. */
async function activate(page: Page, n: number): Promise<void> {
  const box = await paneCell(page, n).boundingBox();
  if (!box) throw new Error(`pane ${n} not laid out`);
  await page.mouse.click(box.x + box.width * 0.4, box.y + box.height * 0.35);
  await expect(paneCell(page, n)).toHaveAttribute("data-active", "");
}

async function chooseGrid(page: Page, id: string): Promise<void> {
  await page.getByTestId("grid-menu").click();
  await page.getByTestId(`grid-option-${id}`).click();
}

/** Every chart of the grid holds bars of its current request. */
async function expectLoaded(page: Page, count: number): Promise<void> {
  await expect
    .poll(async () => {
      const list = await panes(page);
      return list.length === count && list.every((p) => p.bars > 0);
    }, { timeout: 30_000 })
    .toBe(true);
}

async function reloadGrid(page: Page, count: number): Promise<void> {
  const exchanged = page.waitForResponse(
    (res) => res.request().method() === "POST" && new URL(res.url()).pathname === "/token",
  );
  await page.reload();
  await exchanged;
  await page.waitForURL((url) => !url.searchParams.has("code"));
  await expectLoaded(page, count);
}

/** Drag a chart to older bars until it has paged in history. */
async function pageBack(page: Page, n: number): Promise<number> {
  const box = await paneCell(page, n).boundingBox();
  if (!box) throw new Error(`pane ${n} not laid out`);
  const y = box.y + box.height * 0.3;
  const start = (await panes(page))[n]!.bars;
  await expect
    .poll(
      async () => {
        await page.mouse.move(box.x + 40, y);
        await page.mouse.down();
        await page.mouse.move(box.x + box.width * 0.8, y, { steps: 6 });
        await page.mouse.up();
        return (await panes(page))[n]!.bars;
      },
      { timeout: 20_000 },
    )
    .toBeGreaterThan(start);
  return (await panes(page))[n]!.bars;
}

const row = (page: Page, sym: string) =>
  page.getByRole("table", { name: "Watchlist" }).locator(`[role=row][data-symbol="${sym}"]`);

async function pickSymbol(page: Page, sym: string): Promise<void> {
  await row(page, sym).click();
}

const chip = (page: Page, name: string) =>
  page.getByRole("button", { name: `Remove ${name}`, exact: true });

/** Click with real mouse movement first (see drawing.spec.ts). */
async function drawClick(page: Page, n: number, fx: number, fy: number): Promise<void> {
  const box = await paneCell(page, n).boundingBox();
  if (!box) throw new Error(`pane ${n} not laid out`);
  await page.mouse.move(box.x + box.width * fx, box.y + box.height * fy, { steps: 5 });
  await page.mouse.down();
  await page.mouse.up();
  await page.waitForTimeout(700);
}

function recordErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(e.message));
  return errors;
}

test.describe("multi-chart grid", () => {
  test("switches between 1, 2 horizontal, 2 vertical and 4 charts", async ({ bootedApp: page }, testInfo) => {
    const errors = recordErrors(page);
    const fullCandle = (await panes(page))[0]!.candleHeight;

    await chooseGrid(page, "2h");
    await expectLoaded(page, 2);
    const a = (await paneCell(page, 0).boundingBox())!;
    const b = (await paneCell(page, 1).boundingBox())!;
    expect(b.x).toBeGreaterThanOrEqual(a.x + a.width);
    expect(Math.abs(b.y - a.y)).toBeLessThan(1);
    await page.screenshot({ path: testInfo.outputPath("grid-2h.png") });

    await chooseGrid(page, "2v");
    await expect
      .poll(async () => {
        const [top, bottom] = [await paneCell(page, 0).boundingBox(), await paneCell(page, 1).boundingBox()];
        return bottom!.y >= top!.y + top!.height && Math.abs(bottom!.x - top!.x) < 1;
      })
      .toBe(true);
    await expectLoaded(page, 2);

    await chooseGrid(page, "4");
    await expectLoaded(page, 4);
    // A canvas sized to its own cell, in every chart.
    for (const n of [0, 1, 2, 3]) {
      const cell = (await paneCell(page, n).boundingBox())!;
      const canvas = (await paneCell(page, n).locator("canvas").first().boundingBox())!;
      expect(canvas.width).toBeGreaterThan(cell.width * 0.5);
      expect(canvas.width).toBeLessThanOrEqual(cell.width);
    }
    // The first chart was at full height: its price pane must not end up crushed
    // under sub-panes of their old height, and the new charts draw their candles.
    await expect
      .poll(async () =>
        (await panes(page)).map((p) => (p.candleHeight ?? 0) >= (p.stackHeight ?? 1) * 0.4),
      )
      .toEqual([true, true, true, true]);
    await expect
      .poll(async () => (await panes(page)).map((p) => p.staleCanvases))
      .toEqual([0, 0, 0, 0]);
    await page.waitForTimeout(1500);
    await page.screenshot({ path: testInfo.outputPath("grid-4.png") });

    // Nothing was changed in the charts, so nothing asks.
    let dialogs = 0;
    page.on("dialog", (d) => {
      dialogs++;
      void d.dismiss();
    });
    await chooseGrid(page, "1");
    await expectLoaded(page, 1);
    expect(dialogs).toBe(0);
    // One chart again: the sub-panes get back the height they had.
    await expect.poll(async () => (await panes(page))[0]!.candleHeight).toBe(fullCandle);
    expect(errors).toEqual([]);
  });

  test("symbol, interval and indicators are per chart; the header acts on the active one", async ({
    bootedApp: page,
  }) => {
    await chooseGrid(page, "2h");
    await expectLoaded(page, 2);
    await activate(page, 1);

    await pickSymbol(page, "ETHUSDT");
    await page.getByRole("button", { name: "15m", exact: true }).click();
    await chip(page, "rsi").click();
    await expect.poll(async () => (await panes(page))[1]!.symbol).toBe("ETHUSDT");
    await expect.poll(async () => (await panes(page))[1]!.timeframe).toBe("15m");

    const [first, second] = await panes(page);
    expect(first!.symbol).toBe("BTCUSDT");
    expect(first!.timeframe).toBe("1h");
    expect(first!.indicators.length).toBe(3);
    expect(second!.indicators.length).toBe(2);
    await expect(page.getByRole("button", { name: /^Symbol:/ })).toContainText("ETHUSDT");
    // Each chart's legend says what it shows.
    await expect(paneCell(page, 0).locator(".pane-header").first()).toContainText("BTCUSDT");
    await expect(paneCell(page, 1).locator(".pane-header").first()).toContainText("ETHUSDT");

    // The header follows the active chart.
    await activate(page, 0);
    await expect(page.getByRole("button", { name: /^Symbol:/ })).toContainText("BTCUSDT");
    await expect(chip(page, "rsi")).toBeVisible();
    await activate(page, 1);
    await expect(chip(page, "rsi")).toHaveCount(0);

    // A keyboard shortcut (invert scale) reaches the active chart only.
    await page.keyboard.press("Alt+i");
    await expect.poll(async () => (await panes(page))[1]!.invert).toBe(true);
    expect((await panes(page))[0]!.invert).toBe(false);
    // What each chart's own axis does stays with it, whichever chart is active.
    await expect.poll(async () => (await panes(page))[1]!.reverse).toBe(true);
    expect((await panes(page))[0]!.reverse).toBe(false);
    await activate(page, 0);
    await page.waitForTimeout(100);
    expect((await panes(page))[0]!.reverse).toBe(false);
    expect((await panes(page))[1]!.reverse).toBe(true);
    await activate(page, 1);

    // Same symbol in both: one change of interval still moves one chart.
    await pickSymbol(page, "BTCUSDT");
    await page.getByRole("button", { name: "4h", exact: true }).click();
    await expect.poll(async () => (await panes(page))[1]!.timeframe).toBe("4h");
    expect((await panes(page))[0]!.timeframe).toBe("1h");
  });

  test("charts on one request: a refetch only reloads the chart that asked", async ({ bootedApp: page }) => {
    await chooseGrid(page, "2h");
    await expectLoaded(page, 2);
    const [start] = await panes(page);
    const initial = start!.bars;
    const first = await pageBack(page, 0);
    const second = await pageBack(page, 1);
    expect(first).toBeGreaterThan(initial);
    expect(second).toBeGreaterThan(initial);

    // Pressing the current interval reloads the active chart, here the second.
    await expect(paneCell(page, 1)).toHaveAttribute("data-active", "");
    await page.getByRole("button", { name: "1h", exact: true }).click();
    await expect.poll(async () => (await panes(page))[1]!.bars).toBe(initial);
    await page.waitForTimeout(500);
    expect((await panes(page))[0]!.bars).toBe(first);

    // It keeps working for the chart that was left alone.
    await activate(page, 0);
    await page.getByRole("button", { name: "1h", exact: true }).click();
    await expect.poll(async () => (await panes(page))[0]!.bars).toBe(initial);
  });

  test("each chart registers its own indicators: geometry is kept by registered name", async ({
    bootedApp: page,
  }) => {
    await chooseGrid(page, "2h");
    await expectLoaded(page, 2);
    await expect
      .poll(async () => (await panes(page)).map((p) => p.candleIndicators.includes("chartgen_ema_stack")))
      .toEqual([true, false]);
    expect((await panes(page))[1]!.candleIndicators).toContain("chartgen_p1_ema_stack");
  });

  test("drawing tools act on the active chart, drawings stay with their symbol", async ({
    bootedApp: page,
  }) => {
    await chooseGrid(page, "2h");
    await expectLoaded(page, 2);
    await activate(page, 1);
    await pickSymbol(page, "ETHUSDT");
    await expect.poll(async () => (await panes(page))[1]!.symbol).toBe("ETHUSDT");
    await expectLoaded(page, 2);

    // One rail, the active chart's.
    await expect(page.getByRole("toolbar", { name: "Drawing tools" })).toHaveCount(1);

    await page.keyboard.press("Alt+h");
    await drawClick(page, 1, 0.4, 0.3);
    await expect.poll(async () => (await panes(page))[1]!.drawings).toBe(1);
    expect((await panes(page))[0]!.drawings).toBe(0);
    const stored = await page.evaluate(() => [
      localStorage.getItem("chartgen.drawings.ETHUSDT"),
      localStorage.getItem("chartgen.drawings.BTCUSDT"),
    ]);
    expect(stored[0]).not.toBeNull();
    expect(stored[1]).toBeNull();

    // Undo is the active chart's: pane 0 has nothing to undo and pane 1's stays.
    await activate(page, 0);
    await page.keyboard.press("Control+z");
    expect((await panes(page))[1]!.drawings).toBe(1);
    await activate(page, 1);
    await page.keyboard.press("Control+z");
    await expect.poll(async () => (await panes(page))[1]!.drawings).toBe(0);

    // An armed tool does not follow the press to another chart.
    await page.keyboard.press("Alt+h");
    await drawClick(page, 0, 0.4, 0.3);
    await page.waitForTimeout(300);
    expect((await panes(page))[0]!.drawings).toBe(0);
    expect((await panes(page))[1]!.drawings).toBe(0);
    // Leaving the chart cancelled it: coming back and clicking places nothing.
    await drawClick(page, 1, 0.4, 0.3);
    await page.waitForTimeout(300);
    expect((await panes(page))[1]!.drawings).toBe(0);

    // Two charts on one symbol show its drawings, and neither overwrites the other's.
    await activate(page, 1);
    await pickSymbol(page, "BTCUSDT");
    await expect.poll(async () => (await panes(page))[1]!.symbol).toBe("BTCUSDT");
    await expectLoaded(page, 2);
    await page.keyboard.press("Alt+h");
    await drawClick(page, 1, 0.4, 0.3);
    await expect.poll(async () => (await panes(page))[1]!.drawings).toBe(1);
    await expect.poll(async () => (await panes(page))[0]!.drawings).toBe(1);
    await activate(page, 0);
    await page.keyboard.press("Alt+h");
    await drawClick(page, 0, 0.4, 0.5);
    await expect.poll(async () => (await panes(page))[0]!.drawings).toBe(2);
    await expect.poll(async () => (await panes(page))[1]!.drawings).toBe(2);
  });

  test("a saved grid comes back with each chart; the dirty dot covers it", async ({ bootedApp: page }) => {
    const dot = page.getByTestId("layout-dirty");
    await expect(dot).toHaveCount(0);
    await chooseGrid(page, "2h");
    await expectLoaded(page, 2);
    await expect(dot).toBeVisible();

    await activate(page, 1);
    await pickSymbol(page, "ETHUSDT");
    await page.getByRole("button", { name: "15m", exact: true }).click();
    await page.keyboard.press("Control+s");
    await expect(dot).toHaveCount(0);

    // A change in the second chart alone makes the layout dirty.
    await page.getByRole("button", { name: "4h", exact: true }).click();
    await expect(dot).toBeVisible();
    await page.getByRole("button", { name: "15m", exact: true }).click();
    await expect(dot).toHaveCount(0);

    await reloadGrid(page, 2);
    const [first, second] = await panes(page);
    expect(first!.symbol).toBe("BTCUSDT");
    expect(second!.symbol).toBe("ETHUSDT");
    expect(second!.timeframe).toBe("15m");
    await expect(paneCell(page, 1)).toHaveAttribute("data-active", "");
    await expect(dot).toHaveCount(0);

    // Reverting to a single chart and back through the layout restores the grid.
    await page.getByRole("button", { name: /^Layout:/ }).click();
    await page.getByRole("menuitem", { name: /^Create new/ }).click();
    const input = page.getByRole("textbox", { name: "Layout name" });
    await input.fill("Solo");
    await input.press("Enter");
    await expectLoaded(page, 1);
    await page.getByRole("button", { name: /^Layout:/ }).click();
    await page.getByRole("menuitem", { name: /^Load layout/ }).click();
    await page.getByRole("button", { name: /^Unnamed/ }).click();
    await expectLoaded(page, 2);
    expect((await panes(page))[1]!.symbol).toBe("ETHUSDT");
    await expect(dot).toHaveCount(0);
  });

  test("removing a chart with unsaved changes asks first", async ({ bootedApp: page }) => {
    await chooseGrid(page, "2h");
    await expectLoaded(page, 2);
    await activate(page, 1);
    await pickSymbol(page, "ETHUSDT");
    await expect.poll(async () => (await panes(page))[1]!.symbol).toBe("ETHUSDT");

    page.once("dialog", (d) => void d.dismiss());
    await chooseGrid(page, "1");
    await page.waitForTimeout(200);
    expect((await panes(page)).length).toBe(2);
    expect((await panes(page))[1]!.symbol).toBe("ETHUSDT");

    let message = "";
    page.once("dialog", (d) => {
      message = d.message();
      void d.accept();
    });
    await chooseGrid(page, "1");
    await expectLoaded(page, 1);
    expect(message).toContain("a chart");
    expect((await panes(page))[0]!.symbol).toBe("BTCUSDT");
  });

  test("an old layout loads as one chart", async ({ page, bootedApp: _ }) => {
    const old = {
      version: 1,
      active: "old",
      items: [
        {
          id: "old",
          name: "Old",
          updatedAt: 1,
          state: {
            symbol: "ETHUSDT",
            timeframe: "1h",
            chartType: "candles",
            scaleMode: "normal",
            timeZone: "utc",
            indicators: ["volume"],
            compare: [],
          },
        },
      ],
    };
    await page.evaluate((file) => {
      localStorage.setItem("chartgen.layouts", JSON.stringify(file));
      localStorage.setItem("chartgen.layout.symbol", JSON.stringify("ETHUSDT"));
      localStorage.setItem("chartgen.indicators.active", JSON.stringify(["volume"]));
      localStorage.setItem("chartgen.layout.timeZone", JSON.stringify("utc"));
    }, old);
    await reloadGrid(page, 1);
    await expect(page.getByTestId("layout-name")).toHaveText("Old");
    await expect(page.getByTestId("layout-dirty")).toHaveCount(0);
    await expect(page.getByTestId("grid-menu")).toHaveAttribute("data-grid", "1");
    expect((await panes(page))[0]!.symbol).toBe("ETHUSDT");
  });

  test("live ticks reach every chart", async ({ bootedApp: page }) => {
    const errors = recordErrors(page);
    await chooseGrid(page, "2h");
    await expectLoaded(page, 2);
    await activate(page, 1);
    await pickSymbol(page, "ETHUSDT");
    await expect.poll(async () => (await panes(page))[1]!.symbol).toBe("ETHUSDT");
    await expect
      .poll(async () => (await panes(page)).map((p) => p.poll), { timeout: 20_000 })
      .toEqual(["push", "push"]);

    // Pushed, not polled: the sample source ticks about every 500 ms, far faster
    // than the 2 s tail poll a chart without its ticks would fall back to.
    const before = await panes(page);
    await page.waitForTimeout(5000);
    const after = await panes(page);
    expect(after.map((p) => p.poll)).toEqual(["push", "push"]);
    for (const i of [0, 1]) {
      expect(after[i]!.updateKind).toBe("tail");
      expect(after[i]!.version! - before[i]!.version!).toBeGreaterThanOrEqual(5);
    }
    expect(errors).toEqual([]);
  });

  test("a chart the push limit refuses falls back to polling", async ({ bootedApp: page }) => {
    // Refuse every subscription from here on: as a client at its limit.
    await page.evaluate(() => {
      const realFetch = window.fetch.bind(window);
      window.fetch = (input, init) => {
        const body = typeof init?.body === "string" ? init.body : "";
        if (init?.method === "POST" && body.includes('"subscribe_chart"')) {
          const { id } = JSON.parse(body) as { id: unknown };
          return Promise.resolve(
            new Response(
              JSON.stringify({
                jsonrpc: "2.0",
                id,
                result: { content: [{ type: "text", text: "push limit reached" }], isError: true },
              }),
              { headers: { "content-type": "application/json" } },
            ),
          );
        }
        return realFetch(input, init);
      };
    });
    await chooseGrid(page, "2h");
    await expectLoaded(page, 2);
    await expect.poll(async () => (await panes(page))[1]!.poll, { timeout: 20_000 }).toBe("live");
    const before = (await panes(page))[1]!.version!;
    await expect.poll(async () => (await panes(page))[1]!.version!, { timeout: 15_000 }).toBeGreaterThan(before);
  });

  test("a share link carries the active chart", async ({ bootedApp: page, context }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"], {
      origin: new URL(page.url()).origin,
    });
    await chooseGrid(page, "2h");
    await expectLoaded(page, 2);
    await activate(page, 1);
    await pickSymbol(page, "ETHUSDT");
    await expect.poll(async () => (await panes(page))[1]!.symbol).toBe("ETHUSDT");
    await page.getByRole("button", { name: "Copy link to chart" }).click();
    const link = await page.evaluate(() => navigator.clipboard.readText());
    expect(link).toContain("ETHUSDT");
    expect(link).not.toContain("BTCUSDT");
    expect(link).not.toContain("grid");
  });

  test("opening a link applies it to the active chart and leaves the others", async ({ bootedApp: page }) => {
    await chooseGrid(page, "2h");
    await expectLoaded(page, 2);
    await activate(page, 1);
    const exchanged = page.waitForResponse(
      (res) => res.request().method() === "POST" && new URL(res.url()).pathname === "/token",
    );
    await page.goto("/?symbol=SOLUSDT&interval=4h");
    await exchanged;
    await page.waitForURL((url) => !url.searchParams.has("code"));
    await expectLoaded(page, 2);
    const [first, second] = await panes(page);
    expect(second!.symbol).toBe("SOLUSDT");
    expect(second!.timeframe).toBe("4h");
    expect(first!.symbol).toBe("BTCUSDT");
    expect(first!.timeframe).toBe("1h");
  });

  test("the compact layout shows one chart and keeps the grid", async ({ bootedApp: page }) => {
    await chooseGrid(page, "4");
    await expectLoaded(page, 4);
    await activate(page, 2);
    await pickSymbol(page, "ETHUSDT");
    await expect.poll(async () => (await panes(page))[2]!.symbol).toBe("ETHUSDT");
    await page.keyboard.press("Control+s");
    await expect(page.getByTestId("layout-dirty")).toHaveCount(0);

    await page.setViewportSize({ width: 500, height: 800 });
    await expectLoaded(page, 1);
    const [only] = await panes(page);
    expect(only!.pane).toBe(2);
    expect(only!.symbol).toBe("ETHUSDT");
    await expect(page.getByTestId("grid-menu")).toBeHidden();
    await expect(page.getByTestId("layout-dirty")).toHaveCount(0);
    expect(await page.evaluate(() => localStorage.getItem("chartgen.layout.grid"))).toBe('"4"');

    await page.setViewportSize({ width: 1280, height: 720 });
    await expectLoaded(page, 4);
    expect((await panes(page))[2]!.symbol).toBe("ETHUSDT");
  });

  test("charts unmount clean: no listeners, observers or subscriptions pile up", async ({ page, bootedApp: _ }) => {
    await page.evaluate(() => {
      const w = window as unknown as Record<string, unknown>;
      const counts = { listeners: 0, observers: 0, subscribes: 0, unsubscribes: 0 };
      w.__counts = counts;
      // A set, as removing what is not there is no change.
      const live = new Set<string>();
      const ids = new WeakMap<object, number>();
      let nextId = 0;
      const keyOf = (name: string, type: string, fn: unknown, opts: unknown) => {
        const f = fn as object;
        if (!ids.has(f)) ids.set(f, ++nextId);
        const capture = typeof opts === "boolean" ? opts : !!(opts as AddEventListenerOptions | undefined)?.capture;
        return `${name}:${type}:${ids.get(f)}:${capture}`;
      };
      for (const [name, target] of [["window", window], ["document", document]] as const) {
        const add = target.addEventListener.bind(target);
        const remove = target.removeEventListener.bind(target);
        target.addEventListener = ((type: string, fn: EventListener, opts?: AddEventListenerOptions) => {
          if (fn) live.add(keyOf(name, type, fn, opts));
          counts.listeners = live.size;
          add(type, fn, opts);
        }) as typeof add;
        target.removeEventListener = ((type: string, fn: EventListener, opts?: EventListenerOptions) => {
          if (fn) live.delete(keyOf(name, type, fn, opts));
          counts.listeners = live.size;
          remove(type, fn, opts);
        }) as typeof remove;
      }
      // Elements observed: klinecharts unobserves its canvases rather than disconnecting.
      const RO = window.ResizeObserver;
      window.ResizeObserver = class extends RO {
        private watched = new Set<Element>();
        private track(change: () => void) {
          counts.observers -= this.watched.size;
          change();
          counts.observers += this.watched.size;
        }
        override observe(el: Element, opts?: ResizeObserverOptions) {
          this.track(() => this.watched.add(el));
          super.observe(el, opts);
        }
        override unobserve(el: Element) {
          this.track(() => this.watched.delete(el));
          super.unobserve(el);
        }
        override disconnect() {
          this.track(() => this.watched.clear());
          super.disconnect();
        }
      };
      const realFetch = window.fetch.bind(window);
      window.fetch = (input, init) => {
        const body = typeof init?.body === "string" ? init.body : "";
        const sent = realFetch(input, init);
        if (body.includes('"unsubscribe_chart"')) counts.unsubscribes++;
        else if (body.includes('"subscribe_chart"')) {
          // Only a granted one: the server limits subscribes per minute (ADR-0021).
          void sent.then(async (res) => {
            if ((await res.clone().text()).includes("subscription_id")) counts.subscribes++;
          });
        }
        return sent;
      };
    });
    const counts = () =>
      page.evaluate(() => (window as unknown as { __counts: Record<string, number> }).__counts);

    const cycle = async () => {
      await chooseGrid(page, "4");
      await expectLoaded(page, 4);
      // Pushed, or polled past the server's limits: live either way.
      await expect
        .poll(async () => (await panes(page)).every((p) => p.poll === "push" || p.poll === "live"), {
          timeout: 20_000,
        })
        .toBe(true);
      await chooseGrid(page, "1");
      await expectLoaded(page, 1);
      await expect(page.locator("[data-testid=chart-pane] canvas").first()).toBeVisible();
    };

    await cycle();
    await page.waitForTimeout(500);
    const base = await counts();
    for (let i = 0; i < 3; i++) await cycle();
    await page.waitForTimeout(500);
    const after = await counts();
    expect(after.listeners).toBe(base.listeners);
    expect(after.observers).toBe(base.observers);
    expect(await page.locator("[data-testid=chart-pane]").count()).toBe(1);
    // Every chart removed ended its subscription; the first chart keeps the one from before.
    await expect
      .poll(async () => {
        const c = await counts();
        return c.subscribes > 0 && c.subscribes === c.unsubscribes;
      })
      .toBe(true);
    // Nothing of a removed chart is still asked for.
    const calls: string[] = [];
    page.on("request", (req) => {
      const body = req.postData() ?? "";
      if (body.includes('"generate_chart"') && body.includes('"bars":2')) calls.push(body);
    });
    await page.waitForTimeout(6000);
    expect(calls).toEqual([]);
  });
});
