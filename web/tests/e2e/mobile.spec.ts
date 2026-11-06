// The compact shell and touch input, in device emulation (Chromium's mobile
// mode: not a real device, no real on-screen keyboard). Set MOBILE_SHOTS_DIR to
// also write screenshots there.

import { devices, type BrowserContextOptions, type Page } from "@playwright/test";

import { test, expect, bootApp, gotoApp } from "./fixtures/app";

/** A device's context options; the suite only runs Chromium. */
const device = (name: string): BrowserContextOptions => {
  const { defaultBrowserType: _browser, ...options } = devices[name]!;
  return options;
};
const touchPhone360: BrowserContextOptions = {
  viewport: { width: 360, height: 640 },
  isMobile: true,
  hasTouch: true,
};

/** Every device on the compact shell. */
const COMPACT = [
  { name: "iPhone 13", shot: "phone-portrait", use: device("iPhone 13") },
  { name: "iPhone 13 landscape", shot: "phone-landscape", use: device("iPhone 13 landscape") },
  { name: "Pixel 7", shot: "pixel-portrait", use: device("Pixel 7") },
  { name: "Pixel 7 landscape", shot: "pixel-landscape", use: device("Pixel 7 landscape") },
  { name: "360x640 phone", shot: "small-phone", use: touchPhone360 },
  { name: "iPad Mini", shot: "tablet-portrait", use: device("iPad Mini") },
];

const PANELS = ["Watchlist", "Alerts", "Data Window", "Object Tree"];

const shot = async (page: Page, name: string) => {
  const dir = process.env.MOBILE_SHOTS_DIR;
  if (dir) await page.screenshot({ path: `${dir}/${name}.png` });
};

/** Boot at a width where the status text is hidden: wait for the bars instead. */
async function bootCompact(page: Page): Promise<void> {
  const exchanged = page.waitForResponse(
    (res) => res.request().method() === "POST" && new URL(res.url()).pathname === "/token",
  );
  await page.goto("/");
  expect((await exchanged).ok()).toBe(true);
  await page.waitForURL((url) => !url.searchParams.has("code"));
  await expect.poll(() => barCount(page), { timeout: 20_000 }).toBeGreaterThan(0);
  await expect(page.locator(".pane-header").first()).toBeVisible();
}

const barCount = (page: Page) =>
  page.evaluate(() => {
    const t = (window as unknown as { __chartgenTest?: { series: () => { bars: unknown[] } | null } })
      .__chartgenTest;
    return t?.series()?.bars.length ?? 0;
  });

const drawings = (page: Page) =>
  page.evaluate(() =>
    (window as unknown as { __chartgenDrawings: () => unknown[] }).__chartgenDrawings().length,
  );

type ChartProbe = {
  getVisibleRange: () => { from: number; to: number };
  getBarSpace: () => number;
};
const chartState = (page: Page) =>
  page.evaluate(() => {
    const c = (window as unknown as { __chartgenTest: { chart: () => ChartProbe } }).__chartgenTest.chart();
    return { from: c.getVisibleRange().from, space: c.getBarSpace() };
  });

/** The chart host's size once the layout has settled. */
async function expectRealChart(page: Page): Promise<void> {
  await expect
    .poll(() =>
      page.evaluate(() => {
        const host = document.querySelector(".chart-host")!.getBoundingClientRect();
        const canvas = document.querySelector<HTMLCanvasElement>(".chart-host canvas")!;
        return (
          host.width >= window.innerWidth * 0.9 &&
          host.height >= 200 &&
          host.bottom <= window.innerHeight &&
          canvas.getBoundingClientRect().width >= host.width * 0.8
        );
      }),
    )
    .toBe(true);
}

/** Heights of the price pane and of the sub-panes, top down. */
const paneHeights = (page: Page) =>
  page.evaluate(() => {
    const c = (
      window as unknown as {
        __chartgenTest: {
          chart: () => {
            getSize: (id: string) => { height: number } | null;
            getIndicatorByPaneId: () => Map<string, unknown>;
          };
        };
      }
    ).__chartgenTest.chart();
    const subs = [...c.getIndicatorByPaneId().keys()].filter((id) => id !== "candle_pane");
    return {
      candle: c.getSize("candle_pane")!.height,
      subs: subs.map((id) => c.getSize(id)!.height),
    };
  });

/** The price pane has at least `share` of the panes' height, and no sub-pane has vanished. */
async function expectPriceShare(page: Page, share: number): Promise<void> {
  await expect
    .poll(async () => {
      const { candle, subs } = await paneHeights(page);
      return candle / (candle + subs.reduce((a, b) => a + b, 0)) >= share && subs.every((h) => h >= 30);
    })
    .toBe(true);
}

/** Nothing scrolls the page sideways, measured against the device's own width. */
async function expectNoOverflow(page: Page): Promise<void> {
  const width = page.viewportSize()!.width;
  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollWidth))
    .toBeLessThanOrEqual(width);
}

/** Fully on screen, above `bottom` (the top of the keyboard, by default the screen's end). */
async function expectInView(page: Page, name: string | RegExp, bottom?: number): Promise<void> {
  const button = page.getByRole("button", { name, exact: typeof name === "string" }).last();
  await expect(button).toBeVisible();
  const box = await button.boundingBox();
  const size = page.viewportSize()!;
  expect(box, String(name)).not.toBeNull();
  expect(box!.x, String(name)).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width, String(name)).toBeLessThanOrEqual(size.width);
  expect(box!.y + box!.height, String(name)).toBeLessThanOrEqual(bottom ?? size.height);
}

/**
 * What the footer row shows: no two controls overlap (where the footer scrolls,
 * a control counts only for its visible part), none reaches the dock, and each
 * dock icon sits in the middle of its button.
 */
async function footerProblems(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const footerEl = document.querySelector("footer")!;
    const footer = footerEl.getBoundingClientRect();
    const clips = getComputedStyle(footerEl).overflowX !== "visible";
    const label = (e: Element) => e.getAttribute("aria-label") ?? e.textContent?.trim() ?? e.tagName;
    const visible = (e: Element) => (e as HTMLElement).offsetParent !== null;
    const controls = [...document.querySelectorAll("footer button, footer select")]
      .filter(visible)
      .map((e) => {
        const r = e.getBoundingClientRect();
        return { name: label(e), left: r.left, right: clips ? Math.min(r.right, footer.right) : r.right, top: r.top, bottom: r.bottom };
      })
      .filter((r) => r.right > r.left);
    const dock = [...document.querySelectorAll('nav[aria-label="Panels"] button')].map((e) => {
      const r = e.getBoundingClientRect();
      const icon = e.querySelector("svg")!.getBoundingClientRect();
      return { name: `dock ${label(e)}`, left: r.left, right: r.right, top: r.top, bottom: r.bottom, icon };
    });
    const problems: string[] = [];
    const all = [...controls, ...dock];
    for (let i = 0; i < all.length; i++) {
      for (let j = i + 1; j < all.length; j++) {
        const [a, b] = [all[i]!, all[j]!];
        if (a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom) {
          problems.push(`${a.name} overlaps ${b.name}`);
        }
      }
    }
    for (const d of dock) {
      const off = d.icon.left + d.icon.width / 2 - (d.left + d.right) / 2;
      if (Math.abs(off) > 1) problems.push(`${d.name} icon is ${off}px off center`);
    }
    return problems;
  });
}

const dock = (page: Page, name: string) =>
  page.getByRole("navigation", { name: "Panels" }).getByRole("button", { name, exact: true });

/** One finger or two, through the browser's real touch pipeline. */
async function touch(page: Page) {
  const cdp = await page.context().newCDPSession(page);
  type Point = { x: number; y: number };
  const send = (type: "touchStart" | "touchMove" | "touchEnd", points: Point[]) =>
    cdp.send("Input.dispatchTouchEvent", {
      type,
      touchPoints: points.map((p, id) => ({ ...p, id })),
    });
  return {
    start: (...points: Point[]) => send("touchStart", points),
    move: (...points: Point[]) => send("touchMove", points),
    end: () => send("touchEnd", []),
  };
}

/** A point in the price pane, as a fraction of the chart's area. */
async function inChart(page: Page, fx = 0.5, fy = 0.35) {
  const box = await page.locator(".chart-container").boundingBox();
  if (!box) throw new Error("chart container not laid out");
  return { x: box.x + box.width * fx, y: box.y + box.height * fy };
}

for (const d of COMPACT) {
  test.describe(d.name, () => {
    test.use(d.use);

    test("the chart fills the screen, and every panel opens and closes", async ({ page }) => {
      await bootCompact(page);
      await expect(page.locator("html")).toHaveClass(/compact/);
      await expectRealChart(page);
      await expectNoOverflow(page);
      expect(await footerProblems(page)).toEqual([]);
      // Sidebar and drawing tools are out of the chart's way until asked for.
      await expect(page.getByRole("complementary", { name: "Sidebar" })).toHaveCount(0);
      await expect(page.getByRole("button", { name: "Trend line" })).toBeHidden();
      await shot(page, d.shot);

      const before = await page.locator(".chart-host").boundingBox();
      for (const name of PANELS) {
        await dock(page, name).tap();
        const sheet = page.getByRole("complementary", { name: "Sidebar" });
        await expect(sheet).toBeVisible();
        await expect(page.getByRole("tab", { name })).toHaveAttribute("aria-selected", "true");
        await expectNoOverflow(page);
        const box = (await sheet.boundingBox())!;
        expect(box.x + box.width).toBeLessThanOrEqual(page.viewportSize()!.width);
        expect(box.y + box.height).toBeLessThanOrEqual(page.viewportSize()!.height);
        // A sheet lies over the chart: it never resizes it.
        expect(await page.locator(".chart-host").boundingBox()).toEqual(before);
        if (name === "Watchlist") await shot(page, `${d.shot}-sheet`);
        await page.getByRole("button", { name: "Close panel" }).tap();
        await expect(sheet).toHaveCount(0);
      }

      await dock(page, "Draw").tap();
      await expect(page.getByRole("button", { name: "Trend line" })).toBeVisible();
      await expectNoOverflow(page);
      await shot(page, `${d.shot}-draw`);
      await dock(page, "Draw").tap();
      await expect(page.getByRole("button", { name: "Trend line" })).toBeHidden();
    });

    test("dialogs fit, with their buttons in view", async ({ page }) => {
      await bootCompact(page);
      const dialog = page.getByRole("dialog");
      const close = async () => {
        await page.keyboard.press("Escape");
        await expect(dialog).toHaveCount(0);
      };

      await page.getByRole("button", { name: /^Symbol:/ }).tap();
      await expect(page.getByPlaceholder(/Search symbol/)).toBeInViewport({ ratio: 1 });
      await expectNoOverflow(page);
      await close();

      await page.getByRole("button", { name: "Indicators" }).tap();
      await expect(page.getByPlaceholder(/Search indicators/)).toBeInViewport({ ratio: 1 });
      await expectNoOverflow(page);
      await close();

      await page.getByTestId("toolbar-more").tap();
      await page.getByRole("menuitem", { name: "Chart settings" }).tap();
      await expect(dialog).toBeVisible();
      await expectInView(page, "OK");
      await expectNoOverflow(page);
      await shot(page, `${d.shot}-settings`);
      await close();

      await dock(page, "Alerts").tap();
      await page.getByRole("button", { name: "Create alert" }).tap();
      await expect(dialog).toBeVisible();
      await expectInView(page, "Create");
      await expectNoOverflow(page);
      await shot(page, `${d.shot}-alert`);
      await close();
      await page.getByRole("button", { name: "Close panel" }).tap();

      await page.getByTestId("goto-date").tap();
      await expectInView(page, "OK");
      await close();

      await page.getByRole("button", { name: /^Layout:/ }).tap();
      await page.getByRole("menuitem", { name: /^Rename/ }).tap();
      await expectInView(page, "OK");
      await page.getByRole("button", { name: "Cancel" }).tap();
      await expect(dialog).toHaveCount(0);
    });
  });
}

for (const name of ["iPhone 13 landscape", "Pixel 7 landscape", "iPhone SE landscape"]) {
  test.describe(name, () => {
    test.use(device(name));

    test("the price pane gets most of the height, and rotating keeps it right", async ({ page }) => {
      await bootCompact(page);
      await expectPriceShare(page, 0.6);
      await shot(page, `${name.replaceAll(" ", "-")}-panes`);
      const { width, height } = page.viewportSize()!;
      const sideways = (await paneHeights(page)).subs;

      // Upright the sub-panes grow back: to their usual heights, except on a
      // phone so small that upright is a short screen too.
      await page.setViewportSize({ width: height, height: width });
      if (name === "iPhone SE landscape") {
        await expect
          .poll(async () => (await paneHeights(page)).subs.every((h, i) => h > sideways[i]!))
          .toBe(true);
        await expectPriceShare(page, 0.6);
        // On a bigger screen its panes prove what they were asked to be.
        await page.setViewportSize({ width: 800, height: 1000 });
        await expect.poll(async () => (await paneHeights(page)).subs).toEqual([120, 120]);
      } else {
        await expect.poll(async () => (await paneHeights(page)).subs).toEqual([120, 120]);
      }

      await page.setViewportSize({ width, height });
      await expect.poll(async () => (await paneHeights(page)).subs).toEqual(sideways);
      await expectPriceShare(page, 0.6);
    });
  });
}

for (const name of ["iPhone 13 landscape", "iPhone SE landscape"]) {
  test.describe(name, () => {
    test.use(device(name));

    test("a colour popover from a low swatch stays on screen", async ({ page }) => {
      await bootCompact(page);
      await page.locator(".pane-header-interactive", { hasText: "RSI" }).tap();
      await page.getByRole("button", { name: /^Settings for RSI/ }).first().tap();
      await page.getByRole("dialog").getByRole("tab", { name: "Style" }).tap();
      const swatch = page.getByRole("dialog").getByTestId("color-swatch").last();
      await swatch.scrollIntoViewIfNeeded();
      await swatch.tap();
      const popover = page.locator('[data-popper-positioner] [role="dialog"]');
      await expect(popover).toBeVisible();
      await expect(popover).toBeInViewport({ ratio: 1 });
      await expect(popover.getByRole("slider", { name: "Opacity" })).toBeInViewport({ ratio: 1 });
      const size = page.viewportSize()!;
      expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBeLessThanOrEqual(size.height);
    });

    for (const settle of [0, 30, 200]) {
      test(`pane heights do not depend on the resize's timing (${settle} ms)`, async ({ page }) => {
        await bootCompact(page);
        const sideways = (await paneHeights(page)).subs;
        const { width, height } = page.viewportSize()!;
        // Tall, then back to a screen too short for those heights.
        await page.setViewportSize({ width: 800, height: 1000 });
        await expect.poll(async () => (await paneHeights(page)).subs).toEqual([120, 120]);
        if (settle) await page.waitForTimeout(settle);
        await page.setViewportSize({ width, height });
        if (settle) await page.waitForTimeout(settle);
        await expect.poll(async () => (await paneHeights(page)).subs).toEqual(sideways);
      });
    }
  });
}

for (const name of ["iPhone 13 landscape", "Pixel 7 landscape"]) {
  test.describe(name, () => {
    test.use(device(name));

    test("the footer fits beside the dock, without the time zone", async ({ page }) => {
      await bootCompact(page);
      await expect(page.getByTestId("tz-select")).toBeHidden();
      for (const control of await page.locator("footer button").all()) {
        const box = (await control.boundingBox())!;
        const footer = (await page.locator("footer").boundingBox())!;
        expect(box.x + box.width).toBeLessThanOrEqual(footer.x + footer.width);
      }
    });
  });
}

test.describe("touch on a phone", () => {
  test.use(device("iPhone 13"));

  test("header buttons, dock, ranges and menu items are big enough to tap", async ({ page }) => {
    await bootCompact(page);
    const small = () =>
      page.evaluate(() =>
        [...document.querySelectorAll<HTMLElement>(
          'header button, footer button, nav[aria-label="Panels"] button, [role="menu"] [role^="menuitem"]',
        )]
          .filter((e) => e.offsetParent !== null)
          .filter((e) => {
            const r = e.getBoundingClientRect();
            return r.width < 40 || r.height < 40;
          })
          .map((e) => e.getAttribute("aria-label") ?? e.textContent),
      );
    expect(await small()).toEqual([]);
    await page.getByTestId("toolbar-more").tap();
    await expect(page.getByRole("menuitem").first()).toBeVisible();
    expect(await small()).toEqual([]);
  });

  test("one finger pans and two fingers pinch, without scrolling the page", async ({ page }) => {
    await bootCompact(page);
    await expectRealChart(page);
    const t = await touch(page);

    const start = await chartState(page);
    const from = await inChart(page, 0.3);
    await t.start(from);
    for (let i = 1; i <= 8; i++) await t.move({ x: from.x + i * 15, y: from.y });
    await t.end();
    await expect.poll(async () => (await chartState(page)).from).toBeLessThan(start.from);
    expect(await page.evaluate(() => window.scrollX + window.scrollY)).toBe(0);
    await expect(page.locator(".chart-host")).toHaveCSS("touch-action", "none");
    await expect(page.locator("body")).toHaveCSS("overscroll-behavior-y", "none");

    await page.waitForTimeout(700);
    const space = (await chartState(page)).space;
    const c = await inChart(page);
    await t.start({ x: c.x - 30, y: c.y }, { x: c.x + 30, y: c.y });
    for (let i = 1; i <= 8; i++) {
      await t.move({ x: c.x - 30 - i * 8, y: c.y }, { x: c.x + 30 + i * 8, y: c.y });
    }
    await t.end();
    await expect.poll(async () => (await chartState(page)).space).toBeGreaterThan(space);
  });

  test("a long press shows the crosshair and its readings, not the chart menu", async ({ page }) => {
    await bootCompact(page);
    const time = page.getByTestId("legend-time");
    const lastBar = await time.textContent();

    const t = await touch(page);
    const c = await inChart(page, 0.4);
    await t.start(c);
    // Past the 500 ms of klinecharts and the 700 ms of Kobalte's menu.
    await page.waitForTimeout(900);
    await expect(page.getByRole("menu", { name: "Chart actions" })).toHaveCount(0);
    await expect.poll(() => time.textContent()).not.toBe(lastBar);
    // A drag now moves the crosshair over the bars instead of panning.
    const before = await chartState(page);
    const first = await time.textContent();
    await t.move({ x: c.x - 40, y: c.y });
    await t.move({ x: c.x - 80, y: c.y });
    await expect.poll(() => time.textContent()).not.toBe(first);
    await shot(page, "phone-crosshair");
    await t.end();
    expect((await chartState(page)).from).toBe(before.from);
  });

  test("a tap shows the crosshair and a tap elsewhere hides it", async ({ page }) => {
    await bootCompact(page);
    const time = page.getByTestId("legend-time");
    await expect.poll(() => time.textContent()).not.toBe("");
    const lastBar = await time.textContent();
    const near = await inChart(page, 0.3);
    await page.touchscreen.tap(near.x, near.y);
    await expect.poll(() => time.textContent()).not.toBe(lastBar);
    await page.waitForTimeout(700);
    const far = await inChart(page, 0.7);
    await page.touchscreen.tap(far.x, far.y);
    await expect.poll(() => time.textContent()).toBe(lastBar);
  });

  test("holding a drawing tool opens its list", async ({ page }) => {
    await bootCompact(page);
    await dock(page, "Draw").tap();
    const tool = await page.getByRole("button", { name: "Trend line" }).boundingBox();
    const t = await touch(page);
    await t.start({ x: tool!.x + tool!.width / 2, y: tool!.y + tool!.height / 2 });
    await page.waitForTimeout(600);
    await t.end();
    await expect(page.locator("[data-tool-flyout]")).toBeVisible();
    await expect(page.locator("[data-tool-flyout]")).toBeInViewport({ ratio: 1 });
  });

  test("tapping a legend row gives the controls a mouse gets on hover", async ({ page }) => {
    await bootCompact(page);
    const settings = page.getByRole("button", { name: /^Settings for RSI/ }).first();
    await expect(settings).toBeHidden();
    const row = page.locator(".pane-header-interactive", { hasText: "RSI" });
    await row.tap();
    await expect(settings).toBeVisible();

    // A tap anywhere else closes it again, and the readings come back.
    const c = await inChart(page);
    await page.touchscreen.tap(c.x, c.y);
    await expect(settings).toBeHidden();
    await expect(row.locator(".pane-header-value").first()).toBeVisible();

    await page.waitForTimeout(700);
    await row.tap();
    await settings.tap();
    await expect(page.getByRole("dialog")).toBeVisible();
    await expectInView(page, "OK");
  });

  test("the drawing tools close once armed, and taps place the points", async ({ page }) => {
    await bootCompact(page);
    await dock(page, "Draw").tap();
    await page.getByRole("button", { name: "Trend line" }).tap();
    await expect(page.getByRole("button", { name: "Trend line" })).toBeHidden();
    await expect(dock(page, "Draw")).toHaveAttribute("aria-pressed", "true");

    const box = (await page.locator(".chart-container").boundingBox())!;
    await page.touchscreen.tap(box.x + box.width * 0.3, box.y + box.height * 0.2);
    // klinecharts treats two taps in quick succession as a double tap.
    await page.waitForTimeout(700);
    await page.touchscreen.tap(box.x + box.width * 0.6, box.y + box.height * 0.3);
    await expect.poll(() => drawings(page)).toBe(1);
    await shot(page, "phone-drawing");
  });

  test("dragging a drawing's point past its pane keeps a price, and saves it", async ({ page }) => {
    await bootCompact(page);
    await dock(page, "Draw").tap();
    await page.getByRole("button", { name: "Trend line" }).tap();
    const box = (await page.locator(".chart-container").boundingBox())!;
    await page.touchscreen.tap(box.x + box.width * 0.2, box.y + box.height * 0.1);
    await page.waitForTimeout(700);
    await page.touchscreen.tap(box.x + box.width * 0.5, box.y + box.height * 0.2);
    await expect.poll(() => drawings(page)).toBe(1);
    await page.waitForTimeout(700);

    type Drawing = { points: { value: number }[]; pixels: { x: number; y: number }[] };
    const line = () =>
      page.evaluate(
        () => (window as unknown as { __chartgenDrawings: () => unknown[] }).__chartgenDrawings()[0] as Drawing,
      );
    const stored = () =>
      page.evaluate(() => {
        const key = Object.keys(localStorage).find((k) => k.startsWith("chartgen.drawings"));
        return key ? localStorage.getItem(key) : null;
      });
    const end = (await line()).pixels[1]!;
    const from = { x: box.x + end.x, y: box.y + end.y };
    // The price pane ends here: the next pane's axis is volume, in the thousands.
    const paneBottom = (await paneHeights(page)).candle;
    const start = (await line()).points[1]!.value;
    const t = await touch(page);
    await t.start(from);
    for (let i = 1; i <= 12; i++) {
      await t.move({ x: from.x + i * 5, y: box.y + end.y + ((paneBottom + 60 - end.y) * i) / 12 });
    }
    const moved = (await line()).points[1]!.value;
    expect(moved).not.toBe(start);
    await t.end();
    await page.waitForTimeout(300);

    // Within a price chart's range, where it stopped, and that is what was saved.
    expect(moved).toBeLessThan(start);
    expect((await line()).points[1]!.value).toBe(moved);
    expect(await stored()).toContain(String(moved));
  });

  test("Import layout opens the file picker on touch, and its ghost click stays guarded", async ({ page }) => {
    await bootCompact(page);
    await page.getByRole("button", { name: /^Layout:/ }).tap();
    const chooser = page.waitForEvent("filechooser", { timeout: 5000 });
    await page.getByRole("menuitem", { name: "Import layout…" }).tap();
    await chooser;
  });

  test("a keyboard click after a menu press that never became a click is not dropped", async ({ page }) => {
    await bootCompact(page);
    await page.getByTestId("toolbar-more").tap();
    const item = page.getByRole("menuitem", { name: "Chart settings" });
    await expect(item).toBeVisible();
    await item.dispatchEvent("pointerdown", { pointerType: "touch", bubbles: true });
    await page.keyboard.press("Escape");
    await expect(item).toHaveCount(0);

    const draw = dock(page, "Draw");
    await draw.focus();
    await page.keyboard.press("Enter");
    await expect(draw).toHaveAttribute("aria-pressed", "true");
  });

  test("picking a symbol in the watchlist returns to the chart", async ({ page }) => {
    await bootCompact(page);
    await dock(page, "Watchlist").tap();
    await page.getByRole("row", { name: /ETHUSDT/ }).tap();
    await expect(page.getByRole("button", { name: "Symbol: ETHUSDT. Open symbol search" })).toBeVisible();
    await expect(page.getByRole("complementary", { name: "Sidebar" })).toHaveCount(0);
  });

  test("the replay bar fits and a tap on a bar starts the replay", async ({ page }) => {
    await bootCompact(page);
    await page.getByTestId("toolbar-more").tap();
    await page.getByRole("menuitem", { name: "Bar replay" }).tap();
    for (const id of ["replay-select-bar", "replay-hint", "replay-close"]) {
      await expect(page.getByTestId(id)).toBeInViewport({ ratio: 1 });
    }
    await expectNoOverflow(page);
    await shot(page, "phone-replay");
    const c = await inChart(page, 0.5, 0.2);
    await page.touchscreen.tap(c.x, c.y);
    await expect(page.getByTestId("replay-bar")).toHaveAttribute("data-mode", "active");
    await expect(page.getByTestId("replay-play")).toBeInViewport({ ratio: 1 });
  });

  test("a menu item's tap does not press what opens under the finger", async ({ page }) => {
    await bootCompact(page);
    await page.getByTestId("toolbar-more").tap();
    await page.getByRole("menuitem", { name: "Chart settings" }).tap();
    await page.waitForTimeout(500);
    await expect(page.getByRole("dialog")).toBeVisible();
  });

  test("a dialog's buttons stay above the on-screen keyboard", async ({ page }) => {
    // Emulation has no keyboard: shrink the visual viewport as one would.
    await page.addInitScript(() => {
      const w = window as unknown as { __visible?: number };
      const real = Object.getOwnPropertyDescriptor(VisualViewport.prototype, "height")!;
      Object.defineProperty(VisualViewport.prototype, "height", {
        get() {
          return w.__visible ?? real.get!.call(this);
        },
      });
    });
    await bootCompact(page);
    await dock(page, "Alerts").tap();
    await page.getByRole("button", { name: "Create alert" }).tap();
    await expect(page.getByRole("dialog")).toBeVisible();
    const keyboardTop = 300;
    await page.evaluate((h) => {
      (window as unknown as { __visible?: number }).__visible = h;
      window.visualViewport!.dispatchEvent(new Event("resize"));
    }, keyboardTop);
    await expect
      .poll(async () => {
        const box = (await page.getByRole("dialog").boundingBox())!;
        return box.y + box.height;
      })
      .toBeLessThanOrEqual(keyboardTop);
    await expectInView(page, "Create", keyboardTop);
    await shot(page, "phone-keyboard");
  });

  test("rotating keeps symbol, indicators and drawings, with no reload or refetch", async ({ page }) => {
    await bootCompact(page);
    await page.getByRole("button", { name: /^Symbol:/ }).tap();
    await page.keyboard.type("ETHUSDT");
    await page.keyboard.press("Enter");
    const symbol = page.getByRole("button", { name: "Symbol: ETHUSDT. Open symbol search" });
    await expect(symbol).toBeVisible();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as unknown as { __chartgenTest: { series: () => { symbol: string | null } | null } })
              .__chartgenTest.series()?.symbol,
        ),
      )
      .toBe("ETHUSDT");

    await dock(page, "Draw").tap();
    await page.getByRole("button", { name: "Trend line" }).tap();
    const box = (await page.locator(".chart-container").boundingBox())!;
    await page.touchscreen.tap(box.x + box.width * 0.3, box.y + box.height * 0.2);
    await page.waitForTimeout(700);
    await page.touchscreen.tap(box.x + box.width * 0.6, box.y + box.height * 0.3);
    await expect.poll(() => drawings(page)).toBe(1);

    const legend = () => page.locator(".pane-header-label").allTextContents();
    const labels = await legend();
    await page.evaluate(() => {
      document.querySelector<HTMLElement>(".chart-container canvas")!.dataset.kept = "yes";
    });
    const loads: string[] = [];
    page.on("request", (r) => {
      const body = r.postData() ?? "";
      if (r.isNavigationRequest()) loads.push(r.url());
      // A full load: tail polls and history pages ask for less or for older.
      else if (body.includes('"generate_chart"') && !/"bars":2\b|"end_time"/.test(body)) loads.push(body);
    });

    const natural = (await paneHeights(page)).subs;
    const { width, height } = page.viewportSize()!;
    for (const size of [{ width: height, height: width }, { width, height }]) {
      await page.setViewportSize(size);
      await expectRealChart(page);
      await expectPriceShare(page, size.width > size.height ? 0.6 : 0.4);
      await expectNoOverflow(page);
      await expect(symbol).toBeVisible();
      expect(await footerProblems(page)).toEqual([]);
      expect(await legend()).toEqual(labels);
      expect(await drawings(page)).toBe(1);
    }
    expect(loads).toEqual([]);
    await expect.poll(async () => (await paneHeights(page)).subs).toEqual(natural);
    // Upright again, the time zone is back.
    await expect(page.getByTestId("tz-select")).toBeAttached();
    await expect(page.getByTestId("tz-select")).not.toHaveCSS("display", "none");
    // The same chart, not a new one.
    expect(
      await page.evaluate(
        () => document.querySelector<HTMLElement>(".chart-container canvas")!.dataset.kept,
      ),
    ).toBe("yes");
  });
});

test.describe("tablet rotation crosses the breakpoint", () => {
  test.use(device("iPad Mini landscape"));

  test("landscape is the desktop layout, portrait the compact one", async ({ page }) => {
    await gotoApp(page);
    await shot(page, "tablet-landscape");
    const sidebar = page.getByRole("complementary", { name: "Sidebar" });
    await expect(sidebar).toBeVisible();
    await expect(page.locator("html")).not.toHaveClass(/compact/);
    await expect(page.getByRole("navigation", { name: "Panels" })).toHaveCount(0);
    await expectNoOverflow(page);
    // Touch targets, in a layout that otherwise is the desktop one.
    const more = await page.getByTestId("toolbar-more").boundingBox();
    expect(more!.height).toBeGreaterThanOrEqual(40);

    await page.locator(".chart-container").evaluate((el) => {
      el.dataset.kept = "yes";
    });
    await page.setViewportSize({ width: 768, height: 1024 });
    await expect(page.locator("html")).toHaveClass(/compact/);
    await expect(sidebar).toHaveCount(0);
    await expectRealChart(page);
    await expectNoOverflow(page);

    // The open sheet does not leak into the column's stored state, nor back.
    await dock(page, "Alerts").tap();
    await expect(sidebar).toBeVisible();
    await page.getByRole("button", { name: "Close panel" }).tap();
    await page.setViewportSize({ width: 1024, height: 768 });
    await expect(sidebar).toBeVisible();
    await expect(page.getByRole("tab", { name: "Alerts" })).toHaveAttribute("aria-selected", "true");
    await expect(page.locator(".chart-container")).toHaveAttribute("data-kept", "yes");
  });
});

test.describe("a mouse window", () => {
  for (const width of [360, 400, 600]) {
    test(`resized to ${width} px keeps a real chart, and comes back unchanged`, async ({ page }) => {
      await gotoApp(page);
      const desktop = await paneHeights(page);
      // Short as well as narrow: the panes shrink, and come back below.
      await page.setViewportSize({ width, height: 380 });
      await expectPriceShare(page, 0.6);
      await page.setViewportSize({ width, height: 700 });
      await expectRealChart(page);
      await expectNoOverflow(page);
      await dock(page, "Watchlist").click();
      await expect(page.getByRole("complementary", { name: "Sidebar" })).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(page.getByRole("complementary", { name: "Sidebar" })).toHaveCount(0);

      await page.setViewportSize({ width: 1280, height: 720 });
      await expect(page.getByRole("complementary", { name: "Sidebar" })).toBeVisible();
      await expect(page.getByRole("navigation", { name: "Panels" })).toHaveCount(0);
      await expect.poll(() => paneHeights(page)).toEqual(desktop);
    });
  }

  test("leaving a short compact window restores the panes straight away", async ({ page }) => {
    await gotoApp(page);
    const desktop = await paneHeights(page);
    await page.setViewportSize({ width: 600, height: 380 });
    await expectPriceShare(page, 0.6);
    await page.setViewportSize({ width: 1280, height: 720 });
    await expect.poll(() => paneHeights(page)).toEqual(desktop);
  });

  test("keeps the desktop layout at desktop sizes", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 720 });
    await gotoApp(page);
    await expect(page.locator("html")).not.toHaveClass(/compact/);
    const rect = (selector: string) =>
      page.locator(selector).first().evaluate((e) => {
        const r = e.getBoundingClientRect();
        return { x: r.x, width: r.width };
      });
    expect(await rect('[aria-label="Drawing tools"]')).toEqual({ x: 0, width: 48 });
    expect(await rect(".chart-host")).toEqual({ x: 48, width: 912 });
    expect(await rect('aside[aria-label="Sidebar"]')).toEqual({ x: 960, width: 320 });
    expect((await page.locator("header").boundingBox())!.height).toBe(52);
    await expect(page.getByRole("navigation", { name: "Panels" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: /^Hide EMA/ }).first()).toBeVisible();
    await expect(page.locator(".chart-host")).toHaveCSS("min-height", "400px");

    // The chart menu still opens from the mouse.
    await page.locator(".chart-container").click({ button: "right", position: { x: 300, y: 160 } });
    await expect(page.getByRole("menu", { name: "Chart actions" })).toBeVisible();
    await page.keyboard.press("Escape");

    // The boot helper still works through a reload at this size.
    await bootApp(page, () => page.reload());
  });
});
