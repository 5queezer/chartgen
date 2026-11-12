// Price scale: invert (Alt+I) and lock price to bar ratio.

import type { Page } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

type Hook = {
  chart: () => {
    convertFromPixel: (p: { y: number }[], o: { paneId: string }) => { value: number }[];
    getSize: (id: string, w: string) => { height: number } | null;
    getDom: (id: string, w: string) => HTMLElement | null;
    getBarSpace: () => number;
    setBarSpace: (n: number) => void;
    getVisibleRange: () => { realFrom: number; to: number };
    getDataList: () => { high: number; low: number }[];
  } | null;
};

/** Whether every visible bar's high and low is inside the axis. */
const allVisibleInRange = (page: Page) =>
  page.evaluate(() => {
    const c = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!;
    const h = c.getSize("candle_pane", "yAxis")!.height;
    const [a, b] = c.convertFromPixel([{ y: 0 }, { y: h }], { paneId: "candle_pane" });
    const lo = Math.min(a!.value, b!.value);
    const hi = Math.max(a!.value, b!.value);
    // The axis fits realFrom..to; `from` also holds off-screen bars to the left.
    const { realFrom, to } = c.getVisibleRange();
    const bars = c.getDataList().slice(Math.max(0, realFrom), to);
    return bars.length > 0 && bars.every((d) => d.low >= lo - 1e-9 && d.high <= hi + 1e-9);
  });

/** The lock takes its ratio on the next frames: a zoom before that would set the ratio. */
const lockSettled = (page: Page) =>
  page.evaluate(
    () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))),
  );

async function lockViaMenu(page: Page): Promise<void> {
  const box = await page.locator(".chart-host").boundingBox();
  if (!box) throw new Error("chart host has no bounding box");
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2, {
    button: "right",
  });
  await page.getByRole("menuitemcheckbox", { name: /Lock price to bar ratio/ }).click();
  await expect(page.getByRole("menu")).toBeHidden();
  await lockSettled(page);
}

const storage = (page: Page, key: string) =>
  page.evaluate((k) => localStorage.getItem(`chartgen.layout.${k}`), key);

const span = async (page: Page) => {
  const r = await axisRange(page);
  return Math.abs(r.top - r.bottom);
};

const scaleButton = (page: Page, name: string) =>
  page
    .getByRole("group", { name: "Price scale" })
    .getByRole("button", { name, exact: true });

/** Prices at the top and bottom of the candle pane's axis. */
const axisRange = (page: Page) =>
  page.evaluate(() => {
    const c = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!;
    const h = c.getSize("candle_pane", "yAxis")!.height;
    const [top, bottom] = c.convertFromPixel([{ y: 0 }, { y: h }], {
      paneId: "candle_pane",
    });
    return { top: top!.value, bottom: bottom!.value };
  });

const setBarSpace = (page: Page, n: number) =>
  page.evaluate((v) => {
    (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!.setBarSpace(v);
  }, n);

test.describe("lock ratio on a new timeframe", () => {
  test.use({ push: false });

  test("is taken from the fit with the new indicators, whatever they do", async ({
    bootedApp: page,
  }) => {
    // The sample data moves with the clock, so whether the new EMAs sit outside
    // the candles depends on the time of day. Force it: on 5m they sit far below.
    const lowered = 0.6;
    await page.route("**/mcp", async (route) => {
      const body = route.request().postData() ?? "";
      if (!body.includes('"generate_chart"') || !body.includes('"5m"')) {
        return route.continue().catch(() => {});
      }
      const res = await route.fetch();
      const json = (await res.json()) as { result: { content: { text: string }[] } };
      const content = json.result.content[0]!;
      const series = JSON.parse(content.text) as {
        indicators: Record<string, { lines?: { values: (number | null)[] }[] }>;
      };
      for (const line of series.indicators.ema_stack?.lines ?? []) {
        line.values = line.values.map((v) => (v === null ? null : v * lowered));
      }
      content.text = JSON.stringify(series);
      return route.fulfill({ response: res, json });
    });
    await setBarSpace(page, 8);
    await lockViaMenu(page);
    await setBarSpace(page, 20);
    await page.keyboard.press("5");
    await page.keyboard.press("Enter");
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as unknown as { __chartgenTest: { series: () => { interval?: string } | null } })
              .__chartgenTest.series()?.interval,
        ),
      )
      .toBe("5m");
    await page.waitForTimeout(300);
    await expect.poll(() => allVisibleInRange(page)).toBe(true);
  });
});

test.describe("price-axis drag scaling", () => {
  test.use({ hasTouch: true, isMobile: true, viewport: { width: 390, height: 844 } });

  test("a vertical touch drag scales the price axis without panning the chart", async ({
    bootedApp: page,
  }) => {
    const before = await span(page);
    const visibleBefore = await page.evaluate(() => {
      const c = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!;
      return c.getVisibleRange();
    });
    const box = await page.locator(".chart-host").boundingBox();
    if (!box) throw new Error("chart host has no bounding box");
    const x = box.x + box.width - 20;
    const y = box.y + box.height / 3;
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchStart",
      touchPoints: [{ x, y }],
    });
    for (let step = 1; step <= 5; step++) {
      await cdp.send("Input.dispatchTouchEvent", {
        type: "touchMove",
        touchPoints: [{ x, y: y + step * 8 }],
      });
    }
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });

    await expect.poll(() => span(page)).toBeGreaterThan(before * 1.05);
    const visibleAfter = await page.evaluate(() => {
      const c = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!;
      return c.getVisibleRange();
    });
    expect(visibleAfter).toEqual(visibleBefore);
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "false");
  });

  test("a touch drag keeps scaling after crossing the right axis edge", async ({ bootedApp: page }) => {
    const before = await span(page);
    const rect = await page.evaluate(() => {
      const c = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!;
      const { left, top, height } = c.getDom("candle_pane", "yAxis")!.getBoundingClientRect();
      return { left, top, height };
    });
    const x = rect.left + 10;
    const y = rect.top + rect.height / 3;
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchStart",
      touchPoints: [{ x, y }],
    });
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [{ x: x - 25, y: y + 40 }],
    });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });

    await expect.poll(() => span(page)).toBeGreaterThan(before * 1.05);
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "false");
  });

  test("releasing a non-tracked touch does not end axis scaling", async ({ bootedApp: page }) => {
    const box = await page.locator(".chart-host").boundingBox();
    if (!box) throw new Error("chart host has no bounding box");
    const x = box.x + box.width - 20;
    const y = box.y + box.height / 3;
    const cdp = await page.context().newCDPSession(page);
    await page.evaluate(() => {
      const c = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!;
      const axis = c.getDom("candle_pane", "yAxis")!;
      const w = window as unknown as { axisReleases: number };
      w.axisReleases = 0;
      axis.addEventListener("mouseup", () => w.axisReleases++);
    });
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchStart",
      touchPoints: [{ x, y, id: 0 }],
    });
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchStart",
      touchPoints: [
        { x, y, id: 0 },
        { x: box.x + box.width / 2, y, id: 1 },
      ],
    });
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchEnd",
      touchPoints: [{ x: box.x + box.width / 2, y, id: 1 }],
    });
    await expect.poll(() => page.evaluate(() => (window as unknown as { axisReleases: number }).axisReleases)).toBe(0);

    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchEnd",
      touchPoints: [{ x, y, id: 0 }],
    });

    await expect.poll(() => page.evaluate(() => (window as unknown as { axisReleases: number }).axisReleases)).toBe(1);
  });

  test("touch move releases auto and lock even after pointer scaling stops", async ({ bootedApp: page }) => {
    await lockViaMenu(page);
    const box = await page.locator(".chart-host").boundingBox();
    if (!box) throw new Error("chart host has no bounding box");
    const x = box.x + box.width - 20;
    const y = box.y + box.height / 3;
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchStart",
      touchPoints: [{ x, y }],
    });
    await page.evaluate(() => {
      const c = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!;
      c.getDom("candle_pane", "yAxis")!.dispatchEvent(
        new PointerEvent("pointermove", { bubbles: true, buttons: 0 }),
      );
    });
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [{ x, y: y + 40 }],
    });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });

    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "false");
    await expect.poll(() => storage(page, "lockRatio")).toBe("false");
  });

  test("double-tap after a touch drag restores automatic price scaling", async ({ bootedApp: page }) => {
    const box = await page.locator(".chart-host").boundingBox();
    if (!box) throw new Error("chart host has no bounding box");
    const x = box.x + box.width - 20;
    const y = box.y + box.height / 3;
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchStart",
      touchPoints: [{ x, y }],
    });
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [{ x, y: y + 40 }],
    });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "false");

    for (let tap = 0; tap < 2; tap++) {
      await cdp.send("Input.dispatchTouchEvent", {
        type: "touchStart",
        touchPoints: [{ x, y }],
      });
      await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    }

    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "true");
    await expect.poll(() => allVisibleInRange(page)).toBe(true);
  });

  test("a tap before a drag does not make the next tap restore auto", async ({ bootedApp: page }) => {
    const box = await page.locator(".chart-host").boundingBox();
    if (!box) throw new Error("chart host has no bounding box");
    const x = box.x + box.width - 20;
    const y = box.y + box.height / 3;
    const cdp = await page.context().newCDPSession(page);
    const tap = async () => {
      await cdp.send("Input.dispatchTouchEvent", {
        type: "touchStart",
        touchPoints: [{ x, y }],
      });
      await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    };
    await tap();
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchStart",
      touchPoints: [{ x, y }],
    });
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [{ x, y: y + 40 }],
    });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await tap();

    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "false");
  });

  test("active axis touch release ends scaling while another touch remains", async ({
    bootedApp: page,
  }) => {
    const box = await page.locator(".chart-host").boundingBox();
    if (!box) throw new Error("chart host has no bounding box");
    const x = box.x + box.width - 20;
    const y = box.y + box.height / 3;
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchStart",
      touchPoints: [{ x, y, id: 0 }],
    });
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchStart",
      touchPoints: [
        { x, y, id: 0 },
        { x: box.x + box.width / 2, y, id: 1 },
        { x: box.x + box.width / 2, y: y + 20, id: 2 },
      ],
    });
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchEnd",
      touchPoints: [{ x, y, id: 0 }],
    });
    const before = await span(page);

    await page.mouse.move(x, y + 40);
    await page.waitForTimeout(200);

    // The live sample candle can move the auto-fitted span slightly; real scaling changes it by orders of magnitude more.
    expect(Math.abs((await span(page)) - before)).toBeLessThan(0.001);
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "true");
  });

  test("touch cancel clears a pending tap before the next tap", async ({ bootedApp: page }) => {
    const box = await page.locator(".chart-host").boundingBox();
    if (!box) throw new Error("chart host has no bounding box");
    const x = box.x + box.width - 20;
    const y = box.y + box.height / 3;
    const cdp = await page.context().newCDPSession(page);
    await scaleButton(page, "auto").click();
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "false");
    // Native tap/cancel/tap can synthesize dblclick; this test isolates lastTap.
    await page.evaluate(() => {
      const c = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!;
      c.getDom("candle_pane", "yAxis")!.addEventListener(
        "dblclick",
        (event) => event.stopImmediatePropagation(),
        { capture: true },
      );
    });
    const tap = async () => {
      await cdp.send("Input.dispatchTouchEvent", {
        type: "touchStart",
        touchPoints: [{ x, y }],
      });
      await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    };
    await tap();
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchStart",
      touchPoints: [{ x, y }],
    });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchCancel", touchPoints: [] });
    await tap();

    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "false");
  });

  test("touch cancel ends scaling before a mouse move without a pressed button", async ({
    bootedApp: page,
  }) => {
    const box = await page.locator(".chart-host").boundingBox();
    if (!box) throw new Error("chart host has no bounding box");
    const x = box.x + box.width - 20;
    const y = box.y + box.height / 3;
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchStart",
      touchPoints: [{ x, y }],
    });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchCancel", touchPoints: [] });
    const before = await span(page);

    await page.mouse.move(x, y + 40);
    await page.waitForTimeout(200);

    // The live sample candle can move the auto-fitted span slightly; real scaling changes it by orders of magnitude more.
    expect(Math.abs((await span(page)) - before)).toBeLessThan(0.001);
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "true");
  });
});

test.describe("price scale invert and lock ratio", () => {
  test("Alt+I inverts the axis, persists over reload, toggles back", async ({
    bootedApp: page,
  }) => {
    const before = await axisRange(page);
    expect(before.top).toBeGreaterThan(before.bottom);

    await page.keyboard.press("Alt+KeyI");
    await expect(scaleButton(page, "inv")).toHaveAttribute("aria-pressed", "true");
    await expect
      .poll(async () => {
        const r = await axisRange(page);
        return r.top < r.bottom;
      })
      .toBe(true);
    expect(
      await page.evaluate(() => localStorage.getItem("chartgen.layout.invertScale")),
    ).toBe("true");

    await reloadApp(page);
    await expect(scaleButton(page, "inv")).toHaveAttribute("aria-pressed", "true");
    await expect
      .poll(async () => {
        const r = await axisRange(page);
        return r.top < r.bottom;
      })
      .toBe(true);

    await page.keyboard.press("Alt+KeyI");
    await expect
      .poll(async () => {
        const r = await axisRange(page);
        return r.top > r.bottom;
      })
      .toBe(true);
  });

  test("the crosshair price label follows the inverted axis", async ({
    bootedApp: page,
  }) => {
    const label = async () => {
      const box = await page.locator(".chart-host").boundingBox();
      if (!box) throw new Error("chart host has no bounding box");
      const y = (frac: number) => box.y + box.height * frac;
      const x = box.x + box.width * 0.5;
      const read = async (frac: number) => {
        await page.mouse.move(x, y(frac));
        // The crosshair label is drawn on canvas; read the price under the
        // cursor through the same conversion klinecharts uses for it.
        return page.evaluate((py) => {
          const c = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!;
          return c.convertFromPixel([{ y: py }], { paneId: "candle_pane" })[0]!.value;
        }, box.height * frac);
      };
      return { top: await read(0.2), low: await read(0.6) };
    };
    const normal = await label();
    expect(normal.top).toBeGreaterThan(normal.low);
    await page.keyboard.press("Alt+KeyI");
    await expect.poll(async () => (await label()).top < (await label()).low).toBe(true);
  });

  test("Invert scale in the chart context menu", async ({ bootedApp: page }) => {
    const box = await page.locator(".chart-host").boundingBox();
    if (!box) throw new Error("chart host has no bounding box");
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2, {
      button: "right",
    });
    await page.getByRole("menuitemcheckbox", { name: /Invert scale/ }).click();
    await expect(scaleButton(page, "inv")).toHaveAttribute("aria-pressed", "true");
  });

  test("lock ratio: zooming in horizontally shrinks the price range proportionally", async ({
    bootedApp: page,
  }) => {
    const box = await page.locator(".chart-host").boundingBox();
    if (!box) throw new Error("chart host has no bounding box");
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2, {
      button: "right",
    });
    await page.getByRole("menuitemcheckbox", { name: /Lock price to bar ratio/ }).click();
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "false");
    await lockSettled(page);

    const span = async () => {
      const r = await axisRange(page);
      return r.top - r.bottom;
    };
    await setBarSpace(page, 8);
    await expect.poll(span).toBeGreaterThan(0);
    const base = await span();
    await setBarSpace(page, 16);
    await expect
      .poll(async () => Math.abs((await span()) / base - 0.5) < 0.03)
      .toBe(true);
    await setBarSpace(page, 8);
    await expect
      .poll(async () => Math.abs((await span()) / base - 1) < 0.03)
      .toBe(true);

    expect(
      await page.evaluate(() => localStorage.getItem("chartgen.layout.lockRatio")),
    ).toBe("true");

    // Turning auto back on releases the lock.
    await scaleButton(page, "auto").click();
    expect(
      await page.evaluate(() => localStorage.getItem("chartgen.layout.lockRatio")),
    ).toBe("false");
    // The default gap is back: the span at the original zoom is the original.
    await setBarSpace(page, 8);
    await expect
      .poll(async () => Math.abs((await span()) / base - 1) < 0.03)
      .toBe(true);
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "true");
  });

  test("lock survives a reload without cropping the chart", async ({ bootedApp: page }) => {
    const unlocked = await span(page);
    await lockViaMenu(page);
    await reloadApp(page);
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "false");
    await expect
      .poll(async () => Math.abs((await span(page)) / unlocked - 1) < 0.05)
      .toBe(true);
    expect(await allVisibleInRange(page)).toBe(true);
  });

  test("zooming out far widens the range (gaps of 1 or more)", async ({
    bootedApp: page,
  }) => {
    await setBarSpace(page, 8);
    await lockViaMenu(page);
    await expect.poll(() => span(page)).toBeGreaterThan(0);
    const base = await span(page);
    await setBarSpace(page, 2);
    await expect
      .poll(async () => Math.abs((await span(page)) / base - 4) < 0.2)
      .toBe(true);
  });

  test("a new timeframe starts from a fresh fit, not the old ratio", async ({
    bootedApp: page,
  }) => {
    await setBarSpace(page, 8);
    await lockViaMenu(page);
    await expect.poll(() => span(page)).toBeGreaterThan(0);
    const base = await span(page);
    // Whether this crops any bars depends on the data, which moves with the
    // clock; the lock holding does not: 2.5 times the bar width, 0.4 the range.
    await setBarSpace(page, 20);
    await expect
      .poll(async () => Math.abs((await span(page)) / base - 0.4) < 0.03)
      .toBe(true);
    await page.keyboard.press("5");
    await page.keyboard.press("Enter");
    await expect.poll(() => allVisibleInRange(page), { timeout: 15_000 }).toBe(true);
  });

  test("dragging the axis releases the lock", async ({ bootedApp: page }) => {
    await lockViaMenu(page);
    const box = await page.locator(".chart-host").boundingBox();
    if (!box) throw new Error("chart host has no bounding box");
    const x = box.x + box.width - 20;
    const y = box.y + box.height / 3;
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x, y + 40, { steps: 5 });
    await page.mouse.up();
    await expect.poll(() => storage(page, "lockRatio")).toBe("false");
    // Manual scaling: auto stays off.
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "false");
  });

  test("a press that ends away from the window does not release the lock on the next hover", async ({
    bootedApp: page,
  }) => {
    await lockViaMenu(page);
    const box = await page.locator(".chart-host").boundingBox();
    if (!box) throw new Error("chart host has no bounding box");
    const x = box.x + box.width - 20;
    const y = box.y + box.height / 3;
    // Press on the axis, then the window loses focus and the release goes elsewhere:
    // the page sees a pointerdown and never the pointerup.
    await page.mouse.move(x, y);
    await page.evaluate(
      ([px, py]) => {
        const target = document.elementFromPoint(px!, py!)!;
        target.dispatchEvent(
          new PointerEvent("pointerdown", { bubbles: true, buttons: 1, clientX: px, clientY: py }),
        );
        window.dispatchEvent(new Event("blur"));
      },
      [x, y],
    );
    // Back over the axis with no button down.
    await page.mouse.move(x, y + 20, { steps: 5 });
    await page.waitForTimeout(200);
    expect(await storage(page, "lockRatio")).toBe("true");
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "false");
  });

  test("Cancel in the settings dialog undoes invert and lock", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: "Chart settings" }).click();
    const dialog = page.getByRole("dialog", { name: "Settings" });
    await dialog.getByRole("tab", { name: "Scales" }).click();
    await dialog.getByLabel("Invert scale").check();
    await dialog.getByLabel("Lock price to bar ratio").check();
    // The modal dialog hides the rest of the page from the role tree.
    expect(await storage(page, "invertScale")).toBe("true");
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toBeHidden();
    await expect(scaleButton(page, "inv")).toHaveAttribute("aria-pressed", "false");
    await expect(scaleButton(page, "auto")).toHaveAttribute("aria-pressed", "true");
    expect(await storage(page, "lockRatio")).toBe("false");
    expect(await storage(page, "invertScale")).toBe("false");
  });

  test("inverted Columns grow from the top of the pane", async ({ bootedApp: page }) => {
    await page.getByRole("button", { name: /^Chart type/ }).click();
    await page.getByRole("menuitemradio", { name: "Columns", exact: true }).click();
    // Half-transparent column pixels in the first rows of the pane's canvases.
    const topRows = () =>
      page.evaluate(() => {
        const label = document.querySelector(".pane-header-label");
        if (!label) return 0;
        const y = label.getBoundingClientRect().bottom + 20;
        let n = 0;
        for (const canvas of document.querySelectorAll<HTMLCanvasElement>(
          ".chart-container canvas",
        )) {
          const rect = canvas.getBoundingClientRect();
          if (rect.width < 300 || y < rect.top || y >= rect.bottom) continue;
          const ctx = canvas.getContext("2d");
          if (!ctx || canvas.width === 0) continue;
          const d = ctx.getImageData(0, 0, canvas.width, 6).data;
          for (let i = 0; i < d.length; i += 4) {
            if (d[i + 3]! >= 120 && d[i + 3]! <= 135) n++;
          }
        }
        return n;
      });
    await expect.poll(async () => (await span(page)) > 0).toBe(true);
    await page.waitForTimeout(500);
    expect(await topRows()).toBeLessThan(50);
    await page.keyboard.press("Alt+KeyI");
    await expect.poll(topRows).toBeGreaterThan(300);
  });
});
