// Price scale: invert (Alt+I) and lock price to bar ratio.

import type { Page } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

type Hook = {
  chart: () => {
    convertFromPixel: (p: { y: number }[], o: { paneId: string }) => { value: number }[];
    getSize: (id: string, w: string) => { height: number } | null;
    getBarSpace: () => number;
    setBarSpace: (n: number) => void;
  } | null;
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
  });
});
