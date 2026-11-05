// Price scale: invert (Alt+I) and lock price to bar ratio.

import type { Page } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

type Hook = {
  chart: () => {
    convertFromPixel: (p: { y: number }[], o: { paneId: string }) => { value: number }[];
    getSize: (id: string, w: string) => { height: number } | null;
    getBarSpace: () => number;
    setBarSpace: (n: number) => void;
    getVisibleRange: () => { from: number; to: number };
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
    const { from, to } = c.getVisibleRange();
    const bars = c.getDataList().slice(from, to);
    return bars.length > 0 && bars.every((d) => d.low >= lo - 1e-9 && d.high <= hi + 1e-9);
  });

async function lockViaMenu(page: Page): Promise<void> {
  const box = await page.locator(".chart-host").boundingBox();
  if (!box) throw new Error("chart host has no bounding box");
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2, {
    button: "right",
  });
  await page.getByRole("menuitemcheckbox", { name: /Lock price to bar ratio/ }).click();
  await expect(page.getByRole("menu")).toBeHidden();
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
    await setBarSpace(page, 20);
    await expect.poll(() => allVisibleInRange(page)).toBe(false);
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
