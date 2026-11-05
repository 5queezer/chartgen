// Drawing tools — draw, persist, restore, select/delete, context menu,
// undo and the toolbar's armed/lock/hide states.

import type { Page } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

const KEY = "chartgen.drawings.BTCUSDT";
const A = { x: 300, y: 160 };
const B = { x: 560, y: 260 };
// Right of the last bar, still inside the plot area.
const FUTURE = { x: 1100, y: 200 };
const MID = { x: (A.x + B.x) / 2, y: (A.y + B.y) / 2 };

// The coordinates above assume the chart is as wide as a 1280 px viewport
// without the right-side watchlist; widen the viewport by the panel's
// 320 px (w-80) so FUTURE stays inside the plot.
test.use({ viewport: { width: 1280 + 320, height: 720 } });

const host = (page: Page) => page.locator(".chart-container");

/**
 * Click with real mouse movement first: klinecharts only advances a
 * drawing on mouse-move + click, not on a teleporting click.
 */
async function clickAt(
  page: Page,
  at: { x: number; y: number },
  button: "left" | "right" = "left",
): Promise<void> {
  const box = await host(page).boundingBox();
  if (!box) throw new Error("chart container not laid out");
  await page.mouse.move(box.x + at.x, box.y + at.y, { steps: 5 });
  await page.mouse.down({ button });
  await page.mouse.up({ button });
  // klinecharts treats two clicks in quick succession as a double click.
  await page.waitForTimeout(700);
}

/** Overlays alive on the chart, via the DEV-only hook in DrawingToolbar. */
const overlays = (page: Page) =>
  page.evaluate(
    () =>
      (
        window as unknown as {
          __chartgenDrawings: () => {
            name: string;
            lock: boolean;
            visible: boolean;
            offRange: boolean;
            paneId: string;
            styles: { polygon?: Record<string, unknown> } | null;
            points: { timestamp?: number; value?: number }[];
          }[];
        }
      ).__chartgenDrawings(),
  );

const stored = (page: Page) =>
  page.evaluate((k) => {
    const raw = localStorage.getItem(k);
    return raw === null ? null : (JSON.parse(raw) as Record<string, unknown>[]);
  }, KEY);

async function drawTrendLine(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Trend line" }).click();
  await clickAt(page, A);
  await clickAt(page, B);
  await expect.poll(() => overlays(page)).toHaveLength(1);
}

test.describe("drawing tools", () => {
  test("rectangle keeps its translucent fill and border after a reload", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: "Rectangle" }).click();
    await clickAt(page, A);
    await clickAt(page, B);
    await expect.poll(() => overlays(page)).toHaveLength(1);
    const polygon = async () => (await overlays(page))[0]?.styles?.polygon;
    const fresh = await polygon();
    expect(fresh).toMatchObject({
      style: "stroke_fill",
      color: "rgba(41, 98, 255, 0.15)",
      borderColor: "#2962ff",
    });

    // Records saved without styles (older versions) must get the defaults too.
    await page.evaluate((k) => {
      const recs = JSON.parse(localStorage.getItem(k) ?? "[]") as Record<string, unknown>[];
      for (const r of recs) delete r.styles;
      localStorage.setItem(k, JSON.stringify(recs));
    }, KEY);
    await reloadApp(page);
    await expect.poll(() => overlays(page)).toHaveLength(1);
    expect(await polygon()).toMatchObject({
      style: "stroke_fill",
      color: "rgba(41, 98, 255, 0.15)",
      borderColor: "#2962ff",
    });
  });

  test("horizontal ray: one click places it", async ({ bootedApp: page }) => {
    await page.getByRole("button", { name: "Horizontal ray" }).click();
    await clickAt(page, A);
    await expect.poll(() => overlays(page)).toHaveLength(1);
    const [o] = await overlays(page);
    expect(o?.points).toHaveLength(1);
    await expect(
      page.getByRole("button", { name: "Horizontal ray" }),
    ).toHaveAttribute("aria-pressed", "false");
    expect(await stored(page)).toHaveLength(1);
  });

  test("a drawing started in the price pane stays there", async ({
    bootedApp: page,
  }) => {
    const hdr = await page
      .locator(".pane-header", { hasText: "RSI" })
      .first()
      .boundingBox();
    const box = await host(page).boundingBox();
    if (!hdr || !box) throw new Error("RSI pane not laid out");
    const inRsi = { x: B.x, y: hdr.y - box.y + 40 };
    await page.getByRole("button", { name: "Fib retracement" }).click();
    await clickAt(page, A);
    await clickAt(page, inRsi);
    await expect.poll(() => overlays(page)).toHaveLength(1);
    expect((await overlays(page))[0]?.paneId).toBe("candle_pane");
  });

  test("trend line: draw, persist, restore, survive timeframe change", async ({
    bootedApp: page,
  }) => {
    await drawTrendLine(page);

    const saved = await stored(page);
    expect(saved).toHaveLength(1);
    expect(saved?.[0]).toMatchObject({ name: "segment", lock: false });
    const points = (saved?.[0] as { points: Record<string, number>[] }).points;
    expect(points).toHaveLength(2);
    for (const p of points) {
      expect(Object.keys(p).sort()).toEqual(["timestamp", "value"]);
    }

    // One drawing per arming: the tool is released after drawing.
    await expect(page.getByRole("button", { name: "Trend line" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );

    await reloadApp(page);
    await expect.poll(() => overlays(page)).toHaveLength(1);

    await page.getByRole("button", { name: "15m", exact: true }).click();
    await expect(
      page.getByRole("group", { name: "Timeframe" }).locator("[aria-pressed=true]"),
    ).toHaveText("15m");
    await expect.poll(() => overlays(page)).toHaveLength(1);
    expect(await stored(page)).toEqual(saved);
  });

  test("select a drawing and press Delete", async ({ bootedApp: page }) => {
    await drawTrendLine(page);
    await clickAt(page, MID);
    await page.keyboard.press("Delete");
    await expect.poll(() => overlays(page)).toHaveLength(0);
    expect(await stored(page)).toBeNull();
  });

  test("restored drawing can be selected and deleted after reload", async ({
    bootedApp: page,
  }) => {
    await drawTrendLine(page);
    await reloadApp(page);
    await expect.poll(() => overlays(page)).toHaveLength(1);
    await clickAt(page, MID);
    await page.keyboard.press("Backspace");
    await expect.poll(() => overlays(page)).toHaveLength(0);
    expect(await stored(page)).toBeNull();
  });

  test("right-click menu removes a drawing", async ({ bootedApp: page }) => {
    await drawTrendLine(page);
    await clickAt(page, MID, "right");
    const menu = page.getByRole("menu", { name: "Drawing actions" });
    await expect(menu).toBeVisible();
    await menu.getByRole("menuitem", { name: "Remove" }).click();
    await expect.poll(() => overlays(page)).toHaveLength(0);
    await expect(menu).toHaveCount(0);
  });

  test("right-click menu locks and unlocks a drawing", async ({
    bootedApp: page,
  }) => {
    await drawTrendLine(page);
    await clickAt(page, MID, "right");
    const menu = page.getByRole("menu", { name: "Drawing actions" });
    await expect(menu.getByRole("menuitem")).toHaveCount(2);
    await menu.getByRole("menuitem", { name: "Lock" }).click();
    expect((await stored(page))?.[0]).toMatchObject({ lock: true });

    // A locked drawing still answers right-click, so it can be unlocked.
    await clickAt(page, MID, "right");
    await menu.getByRole("menuitem", { name: "Unlock" }).click();
    expect((await stored(page))?.[0]).toMatchObject({ lock: false });
  });

  test("right-click while drawing cancels the tool without a menu", async ({
    bootedApp: page,
  }) => {
    const tool = page.getByRole("button", { name: "Trend line" });
    await tool.click();
    await clickAt(page, A);
    await clickAt(page, B, "right");
    await expect(tool).toHaveAttribute("aria-pressed", "false");
    await expect(page.getByRole("menu")).toHaveCount(0);
    await expect.poll(() => overlays(page)).toHaveLength(0);
  });

  test("a point right of the last bar is saved and restored", async ({
    bootedApp: page,
  }) => {
    await page.getByRole("button", { name: "Trend line" }).click();
    await clickAt(page, A);
    await clickAt(page, FUTURE);
    await expect.poll(() => overlays(page)).toHaveLength(1);

    const saved = await stored(page);
    expect(saved).toHaveLength(1);
    const lastBar = await page.evaluate(() => {
      const raw = localStorage.getItem("chartgen.drawings.BTCUSDT") ?? "[]";
      return (JSON.parse(raw) as { points: { timestamp: number }[] }[])[0]!
        .points[0]!.timestamp;
    });
    const future = (saved?.[0] as { points: { timestamp: number }[] }).points[1]!;
    expect(future.timestamp).toBeGreaterThan(lastBar);

    // Reload, then force a re-serialization (lock + unlock): the future
    // timestamp must come back unchanged.
    await reloadApp(page);
    await expect.poll(() => overlays(page)).toHaveLength(1);
    await page.getByRole("button", { name: "Lock all drawings" }).click();
    await page.getByRole("button", { name: "Lock all drawings" }).click();
    expect(
      ((await stored(page))?.[0] as { points: unknown[] }).points[1],
    ).toEqual(future);
  });

  test("a drawing before the loaded data is hidden and never rewritten", async ({
    bootedApp: page,
  }) => {
    const old = [
      {
        name: "segment",
        points: [
          { timestamp: 1000, value: 100 },
          { timestamp: 2000, value: 200 },
        ],
        lock: false,
        visible: true,
      },
    ];
    await page.evaluate(
      (o) => localStorage.setItem("chartgen.drawings.BTCUSDT", JSON.stringify(o)),
      old,
    );
    await reloadApp(page);
    await expect
      .poll(() => overlays(page))
      .toMatchObject([{ offRange: true, visible: false, lock: true }]);

    // Toolbar actions must neither reveal it nor touch its points.
    // The off-range drawing is already hidden, so the label may be either.
    const hideToggle = page.getByRole("button", { name: /^(Hide|Show) all drawings$/ });
    await hideToggle.click();
    await hideToggle.click();
    await page.getByRole("button", { name: "Lock all drawings" }).click();
    await page.getByRole("button", { name: "Lock all drawings" }).click();
    expect(await overlays(page)).toMatchObject([{ visible: false }]);
    expect((await stored(page))?.[0]).toMatchObject({ points: old[0]!.points });
  });

  test("keys are ignored while a dialog is open", async ({
    bootedApp: page,
  }) => {
    await drawTrendLine(page);
    await clickAt(page, MID);
    await page.getByRole("button", { name: /^Symbol:/ }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await page.keyboard.press("Control+z");
    await page.keyboard.press("Escape");
    await expect.poll(() => overlays(page)).toHaveLength(1);
  });

  test("annotation text is edited in a dialog", async ({ bootedApp: page }) => {
    await page.getByRole("button", { name: "Text annotation" }).click();
    await clickAt(page, A);
    await expect.poll(() => overlays(page)).toHaveLength(1);
    const box = (await host(page).boundingBox())!;
    await page.mouse.dblclick(box.x + A.x + 15, box.y + A.y);
    const input = page.getByRole("dialog").getByRole("textbox");
    await input.fill("hello");
    await page.keyboard.press("Enter");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    expect((await stored(page))?.[0]).toMatchObject({ extendData: "hello" });
  });

  test("Esc cancels an armed tool and keyboard shortcuts arm tools", async ({
    bootedApp: page,
  }) => {
    const hline = page.getByRole("button", { name: "Horizontal line" });
    await hline.click();
    await expect(hline).toHaveAttribute("aria-pressed", "true");
    await page.keyboard.press("Escape");
    await expect(hline).toHaveAttribute("aria-pressed", "false");
    await expect.poll(() => overlays(page)).toHaveLength(0);

    await page.keyboard.press("Alt+h");
    await expect(hline).toHaveAttribute("aria-pressed", "true");
    await clickAt(page, A);
    await expect.poll(() => overlays(page)).toHaveLength(1);
    expect((await stored(page))?.[0]).toMatchObject({
      name: "horizontalStraightLine",
    });
  });

  test("Ctrl+Z undoes the last drawing", async ({ bootedApp: page }) => {
    await drawTrendLine(page);
    await page.keyboard.press("Control+z");
    await expect.poll(() => overlays(page)).toHaveLength(0);
    expect(await stored(page)).toBeNull();
  });

  test("hide, lock and remove all", async ({ bootedApp: page }) => {
    await drawTrendLine(page);

    await page.getByRole("button", { name: "Hide all drawings" }).click();
    expect(await overlays(page)).toMatchObject([{ visible: false }]);
    await page.getByRole("button", { name: "Show all drawings" }).click();
    expect(await overlays(page)).toMatchObject([{ visible: true }]);

    await page.getByRole("button", { name: "Lock all drawings" }).click();
    expect(await overlays(page)).toMatchObject([{ lock: true }]);

    page.once("dialog", (d) => void d.accept());
    await page.getByRole("button", { name: "Remove all drawings" }).click();
    await expect.poll(() => overlays(page)).toHaveLength(0);
    expect(await stored(page)).toBeNull();
  });

  test("drawings are stored per symbol", async ({ bootedApp: page }) => {
    await drawTrendLine(page);
    await page.getByRole("button", { name: /^Symbol:/ }).click();
    await page.getByRole("combobox").fill("ETHUSDT");
    await page.keyboard.press("Enter");
    await expect.poll(() => overlays(page)).toHaveLength(0);
    expect(await stored(page)).toHaveLength(1);
  });

  test.describe("styling", () => {
    const bar = (page: Page) => page.getByRole("toolbar", { name: "Drawing style" });
    const style = async (page: Page) =>
      ((await stored(page))?.[0] as { style?: Record<string, unknown> } | undefined)?.style;

    /** Draw a trend line, select it, set red + width 3 + dashed. */
    async function styleLine(page: Page): Promise<void> {
      await drawTrendLine(page);
      await clickAt(page, MID);
      await expect(bar(page)).toBeVisible();
      await bar(page).getByRole("button", { name: "Line color" }).click();
      await page.getByRole("button", { name: "Color #f23645" }).click();
      await page.keyboard.press("Escape");
      await bar(page).getByRole("button", { name: "Width 3" }).click();
      await bar(page).getByRole("button", { name: "Line style dashed" }).click();
      await expect.poll(() => style(page)).toEqual({
        color: "#f23645",
        width: 3,
        dash: "dashed",
      });
    }

    test("color, width and dash persist across reload", async ({
      bootedApp: page,
    }) => {
      await styleLine(page);
      await reloadApp(page);
      await expect.poll(() => overlays(page)).toHaveLength(1);
      expect(await style(page)).toEqual({ color: "#f23645", width: 3, dash: "dashed" });
      await clickAt(page, MID);
      await expect(
        bar(page).getByRole("button", { name: "Width 3" }),
      ).toHaveAttribute("aria-pressed", "true");
    });

    test("bar hides on Escape and is disabled for locked drawings", async ({
      bootedApp: page,
    }) => {
      await drawTrendLine(page);
      await clickAt(page, MID);
      await expect(bar(page)).toBeVisible();
      await page.getByRole("button", { name: "Lock all drawings" }).click();
      await expect(bar(page).getByRole("button", { name: "Width 2" })).toBeDisabled();
      await page.keyboard.press("Escape");
      await expect(bar(page)).toHaveCount(0);
    });

    test("the bar does not cover the legend", async ({ bootedApp: page }) => {
      await drawTrendLine(page);
      await clickAt(page, MID);
      await expect(bar(page)).toBeVisible();
      const b = (await bar(page).boundingBox())!;
      const rows = await page.locator(".pane-header").evaluateAll((els) =>
        els.map((e) => e.getBoundingClientRect().toJSON() as DOMRect),
      );
      for (const r of rows) {
        const apart =
          r.right <= b.x || b.x + b.width <= r.left || r.bottom <= b.y || b.y + b.height <= r.top;
        expect(apart).toBe(true);
      }
    });

    test("undo keeps the drawing selected", async ({ bootedApp: page }) => {
      await styleLine(page);
      await page.keyboard.press("Control+z");
      await expect.poll(() => style(page)).toEqual({ color: "#f23645", width: 3 });
      await expect(bar(page)).toBeVisible();
      await expect(bar(page).getByRole("button", { name: "Width 3" })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
    });

    test("undo and redo buttons", async ({ bootedApp: page }) => {
      const undoBtn = page.getByRole("button", { name: "Undo", exact: true });
      const redoBtn = page.getByRole("button", { name: "Redo", exact: true });
      await expect(undoBtn).toBeDisabled();
      await expect(redoBtn).toBeDisabled();
      await drawTrendLine(page);
      await expect(undoBtn).toBeEnabled();
      await undoBtn.click();
      await expect.poll(() => overlays(page)).toHaveLength(0);
      await expect(redoBtn).toBeEnabled();
      await redoBtn.click();
      await expect.poll(() => overlays(page)).toHaveLength(1);
      await expect(redoBtn).toBeDisabled();
    });

    test("hiding all drawings closes the bar", async ({ bootedApp: page }) => {
      await drawTrendLine(page);
      await clickAt(page, MID);
      await expect(bar(page)).toBeVisible();
      await page.getByRole("button", { name: "Hide all drawings" }).click();
      await expect(bar(page)).toHaveCount(0);
    });

    test("undo reverts style steps, redo reapplies", async ({ bootedApp: page }) => {
      await styleLine(page);
      await page.keyboard.press("Control+z");
      await expect.poll(() => style(page)).toEqual({ color: "#f23645", width: 3 });
      await page.keyboard.press("Control+z");
      await expect.poll(() => style(page)).toEqual({ color: "#f23645" });
      await page.keyboard.press("Control+z");
      await expect.poll(async () => await style(page)).toBeUndefined();
      await page.keyboard.press("Control+Shift+z");
      await expect.poll(() => style(page)).toEqual({ color: "#f23645" });
      await page.keyboard.press("Control+y");
      await expect.poll(() => style(page)).toEqual({ color: "#f23645", width: 3 });
    });

    test("undo after remove restores the drawing with its style", async ({
      bootedApp: page,
    }) => {
      await styleLine(page);
      await page.keyboard.press("Delete");
      await expect.poll(() => overlays(page)).toHaveLength(0);
      await page.keyboard.press("Control+z");
      await expect.poll(() => overlays(page)).toHaveLength(1);
      expect(await style(page)).toEqual({ color: "#f23645", width: 3, dash: "dashed" });
      await page.keyboard.press("Control+Shift+z");
      await expect.poll(() => overlays(page)).toHaveLength(0);
    });
  });
});
