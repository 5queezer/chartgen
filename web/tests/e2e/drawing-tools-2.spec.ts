// Measure tools, fib extension, pitchfork and long/short positions: place,
// list in the Object Tree, persist across a reload, and draw the right
// labels for known points (read through the DEV-only __chartgenDrawings
// hook, since canvas text is not in the DOM).
// Set DRAWING_SHOTS_DIR to also write a screenshot of all of them there.

import type { Page } from "@playwright/test";

import { test, expect, pickTool, reloadApp, openSidebarTab } from "./fixtures/app";
import {
  dateRangeLabel,
  fibExtensionLevels,
  formatAmount,
  positionLabels,
  priceRangeLabel,
  shortPositionLabels,
} from "../../src/lib/drawingMath";

const KEY = "chartgen.drawings.BTCUSDT";

test.use({ viewport: { width: 1280 + 320, height: 720 } });

const host = (page: Page) => page.locator(".chart-container");

interface Live {
  id: string;
  name: string;
  points: { timestamp?: number; dataIndex?: number; value: number }[];
  labels?: string[];
  precision: number;
  pixels: { x: number; y: number }[];
}

const overlays = (page: Page) =>
  page.evaluate(
    () => (window as unknown as { __chartgenDrawings: () => Live[] }).__chartgenDrawings(),
  );

const stored = (page: Page) =>
  page.evaluate((k) => {
    const raw = localStorage.getItem(k);
    return raw === null ? null : (JSON.parse(raw) as { name: string; points: { timestamp: number; value: number }[] }[]);
  }, KEY);

type At = { x: number; y: number };

async function move(page: Page, at: At): Promise<void> {
  const box = await host(page).boundingBox();
  if (!box) throw new Error("chart container not laid out");
  await page.mouse.move(box.x + at.x, box.y + at.y, { steps: 5 });
}

async function clickAt(page: Page, at: At): Promise<void> {
  await move(page, at);
  await page.mouse.down();
  await page.mouse.up();
  // klinecharts treats two clicks in quick succession as a double click.
  await page.waitForTimeout(700);
}

async function place(page: Page, tool: string, clicks: At[]): Promise<void> {
  await pickTool(page, tool);
  for (const c of clicks) await clickAt(page, c);
}

interface Case {
  name: string;
  tool: string;
  clicks: At[];
  points: number;
  /** Expected labels from the live points. */
  expected: (o: Live) => string[];
}

/** Bar interval: all drawings here are placed on the default timeframe. */
const barLabel = (o: Live) => {
  const [a, b] = o.points;
  const ms = Math.abs(b!.timestamp! - a!.timestamp!);
  // "N bar(s), span": N follows from the span; check the span text exactly.
  const m = /^(\d+) bars?, /.exec(o.labels?.find((l) => /bars?, /.test(l)) ?? "");
  return dateRangeLabel(Number(m?.[1] ?? 0), ms);
};

const CASES: Case[] = [
  {
    name: "cgPriceRange",
    tool: "Price range",
    clicks: [{ x: 250, y: 300 }, { x: 330, y: 180 }],
    points: 2,
    expected: (o) => [priceRangeLabel(o.points[0]!.value, o.points[1]!.value, o.precision)],
  },
  {
    name: "cgDateRange",
    tool: "Date range",
    clicks: [{ x: 400, y: 150 }, { x: 560, y: 230 }],
    points: 2,
    expected: (o) => [barLabel(o)],
  },
  {
    name: "cgDatePriceRange",
    tool: "Date and price range",
    clicks: [{ x: 620, y: 300 }, { x: 760, y: 180 }],
    points: 2,
    expected: (o) => [priceRangeLabel(o.points[0]!.value, o.points[1]!.value, o.precision), barLabel(o)],
  },
  {
    name: "cgFibExt",
    tool: "Fib extension (trend-based)",
    clicks: [{ x: 200, y: 300 }, { x: 280, y: 200 }, { x: 340, y: 260 }],
    points: 3,
    expected: (o) =>
      fibExtensionLevels(o.points[0]!.value, o.points[1]!.value, o.points[2]!.value).map(
        ({ level, value }) => `${level} (${formatAmount(value, o.precision)})`,
      ),
  },
  {
    name: "cgPitchfork",
    tool: "Pitchfork",
    clicks: [{ x: 800, y: 300 }, { x: 860, y: 160 }, { x: 950, y: 290 }],
    points: 3,
    expected: () => [],
  },
  {
    name: "cgLong",
    tool: "Long position",
    clicks: [{ x: 1000, y: 220 }],
    points: 3,
    expected: (o) => {
      const [e, t, s] = o.points.map((p) => p.value) as [number, number, number];
      const l = positionLabels("long", e, t, s, o.precision);
      return [l.target, l.stop, l.ratio];
    },
  },
  {
    name: "cgShort",
    tool: "Short position",
    clicks: [{ x: 60, y: 130 }],
    points: 3,
    expected: (o) => {
      const [e, t, s] = o.points.map((p) => p.value) as [number, number, number];
      const l = positionLabels("short", e, t, s, o.precision);
      return [l.target, l.stop, l.ratio];
    },
  },
];

test.describe("drawing tools 2", () => {
  for (const c of CASES) {
    test(`${c.tool}: place, list, label, persist`, async ({ bootedApp: page }) => {
      await place(page, c.tool, c.clicks);
      await expect.poll(() => overlays(page)).toHaveLength(1);
      const [o] = await overlays(page);
      expect(o?.name).toBe(c.name);
      expect(o?.points).toHaveLength(c.points);
      await expect(page.getByRole("button", { name: c.tool, exact: true })).toHaveAttribute(
        "aria-pressed",
        "false",
      );

      // Labels (poll: the canvas draws on the next frame).
      await expect.poll(async () => (await overlays(page))[0]?.labels ?? []).toEqual(
        c.expected(o!),
      );

      // Object Tree lists it under the tool's name.
      await openSidebarTab(page, "Object Tree");
      await expect(page.locator("[data-testid^=tree-drawing-]")).toHaveCount(1);
      await expect(page.locator("[data-testid^=tree-drawing-]")).toContainText(c.tool);

      // Persisted, and identical after a reload.
      const before = await stored(page);
      expect(before).toHaveLength(1);
      expect(before?.[0]?.name).toBe(c.name);
      expect(before?.[0]?.points).toHaveLength(c.points);
      await reloadApp(page);
      await expect.poll(() => overlays(page)).toHaveLength(1);
      expect(await stored(page)).toEqual(before);
      await expect.poll(async () => (await overlays(page))[0]?.labels ?? []).toEqual(
        c.expected((await overlays(page))[0]!),
      );
    });
  }

  test("long position: dragging the target updates the risk/reward ratio", async ({
    bootedApp: page,
  }) => {
    await place(page, "Long position", [{ x: 400, y: 260 }]);
    await expect.poll(() => overlays(page)).toHaveLength(1);
    const first = (await overlays(page))[0]!;
    const [e, t, s] = first.points.map((p) => p.value) as [number, number, number];
    // Sized from the visible price span: reward 10% of it, stop half of that.
    const size = await page.evaluate(() =>
      (window as unknown as { __chartgenPositionSize: () => { reward: number } }).__chartgenPositionSize(),
    );
    expect(Math.abs((t - e) / size.reward - 1)).toBeLessThan(0.02);
    expect(e - s).toBeCloseTo((t - e) / 2, 6);
    await expect.poll(async () => (await overlays(page))[0]?.labels?.[2]).toBe(
      "Risk/Reward Ratio: 2.00",
    );

    // Hover the zone first so the handles become active, then drag the target.
    const target = first.pixels[1]!;
    const entry = first.pixels[0]!;
    await move(page, { x: target.x - 10, y: (entry.y + target.y) / 2 });
    await move(page, target);
    const box = (await host(page).boundingBox())!;
    await page.mouse.down();
    await page.mouse.move(box.x + target.x, box.y + target.y - 30, { steps: 8 });
    await page.mouse.up();

    await expect
      .poll(async () => (await overlays(page))[0]?.points[1]?.value ?? 0)
      .toBeGreaterThan(t);
    const [e2, t2, s2] = (await overlays(page))[0]!.points.map((p) => p.value) as [number, number, number];
    expect(e2).toBeCloseTo(e, 6);
    expect(s2).toBeCloseTo(s, 6);
    const expected = positionLabels("long", e2, t2, s2, first.precision);
    await expect.poll(async () => (await overlays(page))[0]?.labels).toEqual([
      expected.target,
      expected.stop,
      expected.ratio,
    ]);
    expect(expected.ratio).not.toBe("Risk/Reward Ratio: 2.00");
    // Target and stop still share the right edge.
    const [, tp, sp] = (await overlays(page))[0]!.pixels;
    expect(tp!.x).toBe(sp!.x);
  });

  test("all tools together (screenshot)", async ({ bootedApp: page }) => {
    for (const c of CASES) {
      await place(page, c.tool, c.clicks);
    }
    await expect.poll(() => overlays(page)).toHaveLength(CASES.length);
    expect((await stored(page))?.map((r) => r.name)).toEqual(CASES.map((c) => c.name));
    await move(page, { x: 20, y: 20 });
    const dir = process.env.DRAWING_SHOTS_DIR;
    if (dir) await page.screenshot({ path: `${dir}/all-tools.png` });
  });

  test("toolbar fits at 1280x720, in both themes", async ({ bootedApp: page }) => {
    await page.setViewportSize({ width: 1280, height: 720 });
    const dir = process.env.DRAWING_SHOTS_DIR;
    for (const scheme of ["dark", "light"] as const) {
      await page.emulateMedia({ colorScheme: scheme });
      await reloadApp(page);
      // The footer stays on screen, the tools section scrolls only as a fallback.
      for (const name of ["Remove all drawings", "Undo", "Lock all drawings"]) {
        const box = await page.getByRole("button", { name }).boundingBox();
        expect(box, name).not.toBeNull();
        expect(box!.y + box!.height, name).toBeLessThanOrEqual(720);
      }
      const bar = page.getByRole("toolbar", { name: "Drawing tools" });
      expect(await bar.evaluate((el) => el.scrollHeight <= el.clientHeight)).toBe(true);
      await page.getByRole("button", { name: "Measure tools" }).click();
      await expect(page.getByRole("menuitem")).toHaveCount(3);
      if (dir) await page.screenshot({ path: `${dir}/toolbar-${scheme}.png` });
      await page.keyboard.press("Escape");
    }
  });

  test("group flyouts: keyboard, last-used tool, Esc, shortcuts", async ({ bootedApp: page }) => {
    // Chevron opens the menu on the group's current tool; arrows move, Enter arms.
    await page.getByRole("button", { name: "Lines tools" }).click();
    await expect(page.getByRole("menuitem", { name: "Trend line", exact: true })).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(page.getByRole("menuitem", { name: "Ray", exact: true })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("menu")).toHaveCount(0);
    const ray = page.getByRole("button", { name: "Ray", exact: true });
    await expect(ray).toHaveAttribute("aria-pressed", "true");

    // Esc inside the flyout closes only the flyout.
    await page.getByRole("button", { name: "Lines tools" }).click();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu")).toHaveCount(0);
    await expect(ray).toHaveAttribute("aria-pressed", "true");
    await page.keyboard.press("Escape");
    await expect(ray).toHaveAttribute("aria-pressed", "false");

    // The group remembers the last tool across a reload.
    await reloadApp(page);
    await expect(page.getByRole("button", { name: "Ray", exact: true })).toBeVisible();

    // Alt+T arms the trend line and the group follows.
    await page.keyboard.press("Alt+KeyT");
    await expect(page.getByRole("button", { name: "Trend line", exact: true })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  test("press and hold on a group button opens its flyout", async ({ bootedApp: page }) => {
    const btn = page.getByRole("button", { name: "Price range", exact: true });
    const box = (await btn.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await expect(page.getByRole("menuitem", { name: "Date range", exact: true })).toBeVisible();
    await page.mouse.up();
    // The release is not a click: nothing got armed.
    await expect(page.getByRole("menuitem", { name: "Date range", exact: true })).toBeVisible();
    await expect(btn).toHaveAttribute("aria-pressed", "false");
  });
});

// Position labels at any zoom (fitPositionLabels in drawingMath, collision with
// other positions in drawingOverlays). Positions are seeded in pixels of the view
// on screen, so their size does not depend on the sample data's clock.
const POS_LABEL_H = 12 + 2 * 3;

interface Seed {
  kind: "long" | "short";
  /** Bars before the last bar on screen, of the left and right edge. */
  from: number;
  to: number;
  /** Heights of the reward and risk zone, in px. */
  reward: number;
  risk: number;
}

type TestChart = {
  getDataList: () => { timestamp: number }[];
  getVisibleRange: () => { realTo: number };
  getSize: (pane: string) => { height: number } | null;
  convertFromPixel: (c: { y: number }[], f: { paneId: string }) => { value: number }[];
  setBarSpace: (space: number) => void;
};

async function seedPositions(page: Page, seeds: Seed[]): Promise<void> {
  await page.evaluate(
    ({ key, seeds }) => {
      const c = (window as unknown as { __chartgenTest: { chart: () => TestChart } }).__chartgenTest.chart();
      const list = c.getDataList();
      const last = Math.min(c.getVisibleRange().realTo, list.length) - 1;
      const mid = c.getSize("candle_pane")!.height / 2;
      const value = (y: number) => c.convertFromPixel([{ y }], { paneId: "candle_pane" })[0]!.value;
      const records = seeds.map((s, i) => {
        // y grows downwards: a long's target is above the entry.
        const up = s.kind === "long" ? -1 : 1;
        return {
          id: `seed-${i}`,
          name: s.kind === "long" ? "cgLong" : "cgShort",
          points: [
            { timestamp: list[last - s.from]!.timestamp, value: value(mid) },
            { timestamp: list[last - s.to]!.timestamp, value: value(mid + up * s.reward) },
            { timestamp: list[last - s.to]!.timestamp, value: value(mid - up * s.risk) },
          ],
          lock: false,
          visible: true,
        };
      });
      localStorage.setItem(key, JSON.stringify(records));
    },
    { key: KEY, seeds },
  );
  await reloadApp(page);
  await expect.poll(async () => (await overlays(page)).length).toBe(seeds.length);
}

const texts = (o: Live) => {
  const [e, t, s] = o.points.map((p) => p.value) as [number, number, number];
  const kind = o.name === "cgLong" ? "long" : "short";
  return {
    full: positionLabels(kind, e, t, s, o.precision),
    short: shortPositionLabels(kind, e, t, s, o.precision),
  };
};

/** Box size on screen: width, and heights of the reward and risk zone. */
const boxOf = (o: Live) => {
  const [e, t, s] = o.pixels as [At, At, At];
  return { width: Math.abs(t.x - e.x), reward: Math.abs(t.y - e.y), risk: Math.abs(s.y - e.y) };
};

test.describe("position labels at any zoom", () => {
  test("a low box drops the ratio and keeps target and stop; zoomed out, no full text", async ({
    bootedApp: page,
  }) => {
    // As in the zoomed-out report: a short about as low as one label, with a thin stop zone.
    await seedPositions(page, [{ kind: "short", from: 60, to: 20, reward: 20, risk: 4 }]);
    const [o] = await overlays(page);
    const box = boxOf(o!);
    // The premise, with margin: the scale may move a little on the reload.
    expect(box.reward + box.risk).toBeGreaterThanOrEqual(POS_LABEL_H + 2);
    expect(box.risk).toBeLessThanOrEqual(POS_LABEL_H / 2 - 2);
    expect(box.width).toBeGreaterThan(250);
    const { full, short } = texts(o!);
    await expect.poll(async () => (await overlays(page))[0]?.labels).toEqual([full.target, full.stop]);

    // Zoomed out the box is 40 px wide: room for no full text, at most the short ones.
    await page.evaluate(() =>
      (window as unknown as { __chartgenTest: { chart: () => TestChart } }).__chartgenTest.chart().setBarSpace(1),
    );
    await expect.poll(async () => boxOf((await overlays(page))[0]!).width).toBeLessThanOrEqual(41);
    const allowed = new Set([short.target, short.stop, short.ratio]);
    await expect
      .poll(async () => ((await overlays(page))[0]?.labels ?? []).every((l) => allowed.has(l)))
      .toBe(true);
  });

  test("a large box keeps all three full labels", async ({ bootedApp: page }) => {
    await seedPositions(page, [{ kind: "long", from: 60, to: 20, reward: 80, risk: 40 }]);
    const [o] = await overlays(page);
    const box = boxOf(o!);
    expect(Math.min(box.reward, box.risk)).toBeGreaterThan(POS_LABEL_H);
    const { full } = texts(o!);
    await expect.poll(async () => (await overlays(page))[0]?.labels).toEqual([
      full.target,
      full.stop,
      full.ratio,
    ]);
  });

  test("labels of positions close together do not overlap", async ({ bootedApp: page }) => {
    // Two positions on top of each other, and a third apart from them.
    const big = { kind: "long", reward: 80, risk: 40 } as const;
    await seedPositions(page, [
      { ...big, from: 60, to: 25 },
      { ...big, from: 60, to: 25 },
      { ...big, from: 130, to: 95 },
    ]);
    const shown = async () => (await overlays(page)).map((o) => o.labels?.length ?? 0);
    // Of the two on top of each other one draws its labels and the other none; the third all.
    await expect.poll(async () => (await shown()).slice(0, 2).sort()).toEqual([0, 3]);
    expect((await shown())[2]).toBe(3);
  });
});
