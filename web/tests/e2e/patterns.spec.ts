// Pattern indicators (ADR-0046): candlestick labels with their tooltip, auto chart
// patterns with a status, auto fib levels, and alerts on all three.
//
// The sample source follows the wall clock, so nothing here assumes which pattern
// is where: labels and shapes are read from the chart's own series, then found on
// the canvas. Set PATTERN_SHOTS_DIR to also write screenshots there.

import type { Page } from "@playwright/test";

import { test, expect, gotoApp } from "./fixtures/app";

const shot = async (page: Page, name: string) => {
  const dir = process.env.PATTERN_SHOTS_DIR;
  if (dir) await page.screenshot({ path: `${dir}/${name}.png` });
};

test.use({ viewport: { width: 1600, height: 800 } });

const CANDLE_PANE = "candle_pane";

/** Open the app with exactly these indicators. */
async function openWith(page: Page, entries: unknown[], style?: string): Promise<void> {
  await page.addInitScript(
    ([list, chartStyle]) => {
      localStorage.setItem("chartgen.indicators.active", JSON.stringify(list));
      if (chartStyle) localStorage.setItem("chartgen.chartType", chartStyle as string);
    },
    [entries, style] as const,
  );
  await gotoApp(page);
}

interface Mark {
  x: number;
  y: number;
  above: boolean;
  tooltip: string;
  key?: string;
  bias?: string;
  text?: string;
}

/** A chart pixel of bar `x` at price `y`, in page coordinates. */
async function pixelOf(page: Page, x: number, y: number) {
  const at = await page.evaluate(
    ([bar, price, pane]) => {
      const t = window as unknown as {
        __chartgenTest: {
          chart: () => {
            convertToPixel: (
              p: { dataIndex: number; value: number },
              f: { paneId: string },
            ) => { x: number; y: number };
          };
        };
      };
      return t.__chartgenTest.chart().convertToPixel(
        { dataIndex: bar as number, value: price as number },
        { paneId: pane as string },
      );
    },
    [x, y, CANDLE_PANE] as const,
  );
  const box = await page.locator(".chart-container").boundingBox();
  return { x: box!.x + at.x, y: box!.y + at.y };
}

/** The indicator as the chart holds it. */
const held = <T>(page: Page, key: string, read: string) =>
  page.evaluate(
    ([k, r]) => {
      const w = window as unknown as { __chartgenTest: { series: () => { indicators: Record<string, unknown>; bars: unknown[] } | null } };
      const s = w.__chartgenTest.series();
      const ind = s?.indicators[k as string] as Record<string, unknown> | undefined;
      return (new Function("ind", "bars", `return ${r}`) as (i: unknown, b: unknown) => unknown)(
        ind,
        s?.bars,
      ) as T;
    },
    [key, read] as const,
  );

/** The last-but-a-few visible labels with no other label on their bar and side. */
async function loneLabel(page: Page, key: string): Promise<Mark> {
  const labels = await held<Mark[]>(page, key, "(ind && ind.labels) || []");
  const closed = labels.filter((l) => !(l as Mark & { tentative?: boolean }).tentative);
  const lone = closed.filter(
    (l) => closed.filter((o) => o.x === l.x && o.above === l.above).length === 1,
  );
  expect(lone.length).toBeGreaterThan(0);
  return lone[Math.max(0, lone.length - 4)]!;
}

/** Hover the middle of a label's box. */
async function hoverLabel(page: Page, l: Mark): Promise<void> {
  const px = await pixelOf(page, l.x, l.y);
  await page.mouse.move(px.x, px.y + (l.above ? -12 : 12), { steps: 3 });
}

const dialog = (page: Page) => page.getByRole("dialog");
const rows = (page: Page) => page.locator("[data-alert-id]");
const toasts = (page: Page) => page.locator("[data-toast]");
const tooltip = (page: Page) => page.getByTestId("pattern-tooltip");

/** Inject prices in one synchronous step, as alerts-sources.spec.ts does. */
const feed = (page: Page, prices: number[]) =>
  page.evaluate((ps) => {
    const w = window as unknown as {
      __chartgenTest: { feedPrice: (s: string, p: number) => void };
    };
    for (const p of ps) w.__chartgenTest.feedPrice("BTCUSDT", p);
  }, prices);

/** Pixels on the chart's canvases near a color. */
const colorCount = (page: Page, rgb: [number, number, number]) =>
  page.evaluate((c) => {
    let n = 0;
    for (const canvas of document.querySelectorAll<HTMLCanvasElement>(".chart-container canvas")) {
      const ctx = canvas.getContext("2d");
      if (!ctx || canvas.width === 0) continue;
      const d = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
      for (let i = 0; i < d.length; i += 4) {
        if (
          d[i + 3]! > 150 &&
          Math.abs(d[i]! - c[0]) < 12 &&
          Math.abs(d[i + 1]! - c[1]) < 12 &&
          Math.abs(d[i + 2]! - c[2]) < 12
        ) {
          n++;
        }
      }
    }
    return n;
  }, rgb);

const BLUE: [number, number, number] = [41, 98, 255];

test.describe("candlestick patterns", () => {
  const entry = { name: "candlestick_patterns", params: { trend: "none" } };

  test("the picker finds them by a pattern's name, and labels appear", async ({ bootedApp: page }) => {
    await page.getByRole("button", { name: /^Indicators/ }).first().click();
    const picker = dialog(page);
    await picker.getByRole("textbox").fill("hammer");
    await picker.getByText("candlestick_patterns", { exact: false }).first().click();
    await expect
      .poll(() => held<number>(page, "candlestick_patterns", "((ind && ind.labels) || []).length"), {
        timeout: 15_000,
      })
      .toBeGreaterThan(0);
  });

  test("labels on the chart, in their colors, with a tooltip that explains them", async ({ page }) => {
    await openWith(page, [entry]);
    await expect
      .poll(() => held<number>(page, "candlestick_patterns", "((ind && ind.labels) || []).length"), {
        timeout: 15_000,
      })
      .toBeGreaterThan(10);
    await expect.poll(() => colorCount(page, BLUE)).toBeGreaterThan(50);
    await shot(page, "candles-all");

    const l = await loneLabel(page, "candlestick_patterns");
    await hoverLabel(page, l);
    await expect(tooltip(page)).toBeVisible();
    await expect(tooltip(page)).toHaveText(l.tooltip);
    await shot(page, "candles-tooltip");

    // Off the label, the tooltip goes.
    const px = await pixelOf(page, l.x, l.y);
    await page.mouse.move(px.x, px.y + (l.above ? 200 : -200), { steps: 2 });
    await expect(tooltip(page)).toHaveCount(0);

    // A label on the forming bar reads differently: outlined, not filled.
    const filled = await colorCount(page, BLUE);
    await page.evaluate(() => {
      const w = window as unknown as {
        __chartgenTest: {
          series: () => { indicators: Record<string, { labels: { bias: string; tentative?: boolean; x: number }[] }> };
        };
      };
      for (const label of w.__chartgenTest.series().indicators.candlestick_patterns!.labels) {
        if (label.bias === "bullish") label.tentative = true;
      }
    });
    await expect.poll(() => colorCount(page, BLUE), { timeout: 10_000 }).toBeLessThan(filled / 2);
    await shot(page, "candles-forming-outlined");
  });

  test("one pattern can be chosen, and the settings say what can change", async ({ page }) => {
    await openWith(page, [entry]);
    const row = page.getByTestId("indicator-legend-candlestick_patterns");
    await row.getByRole("button", { name: /^Settings for/ }).click();
    await expect(dialog(page).getByTestId("repaint-note")).toContainText(
      "can change or vanish until the bar closes",
    );
    await dialog(page).getByLabel("pattern").selectOption("doji");
    await shot(page, "candles-settings");
    await dialog(page).getByRole("button", { name: "OK" }).click();
    await expect
      .poll(async () => {
        const keys = await held<string[]>(page, "candlestick_patterns", "((ind && ind.labels) || []).map(l => l.key)");
        return keys.length > 0 && keys.every((k) => k === "doji");
      }, { timeout: 15_000 })
      .toBe(true);
  });

  test("the Style tab recolors them", async ({ page }) => {
    await openWith(page, [entry]);
    await expect.poll(() => colorCount(page, BLUE), { timeout: 15_000 }).toBeGreaterThan(50);
    const row = page.getByTestId("indicator-legend-candlestick_patterns");
    await row.getByRole("button", { name: /^Settings for/ }).click();
    await dialog(page).getByRole("tab", { name: "Style" }).click();
    await dialog(page).getByTestId("style-marks").getByLabel("Show Patterns").uncheck();
    await dialog(page).getByRole("button", { name: "OK" }).click();
    await expect.poll(() => colorCount(page, BLUE)).toBeLessThan(10);
  });

  test("an alert on a detection fires on a new one, not an old one", async ({ page }) => {
    await openWith(page, [entry]);
    await page
      .getByTestId("indicator-legend-candlestick_patterns")
      .getByRole("button", { name: /^Add alert on/ })
      .click();
    await expect(dialog(page).getByLabel("Source")).toHaveValue(/^event:/);
    // A detection has no condition or level to set, and says when it fires.
    await expect(dialog(page).getByLabel("Condition")).toHaveCount(0);
    await expect(dialog(page).getByLabel("Value", { exact: true })).toHaveCount(0);
    await expect(dialog(page).locator("[data-role=alert-event-note]")).toContainText(
      "Nothing on the forming bar",
    );
    await shot(page, "alert-dialog-event");
    await dialog(page).getByRole("button", { name: "Create" }).click();
    await expect(dialog(page)).toHaveCount(0);
    await page.getByRole("tab", { name: /^Alerts/ }).click();
    await expect(rows(page)).toContainText("Bullish pattern");
    await expect(rows(page)).toHaveAttribute("data-status", "active");

    // Live ticks with the same detection: nothing fires however many there are.
    await feed(page, [100, 200, 100]);
    await expect(rows(page)).toHaveAttribute("data-status", "active");

    // The next tail reports a newer bar.
    // In one step, read at once: a real tail must not get a say in between.
    const fired = await page.evaluate(() => {
      const w = window as unknown as {
        __chartgenTest: {
          feedPrice: (s: string, p: number) => void;
          series: () => { indicators: Record<string, { events: { key: string; t: number | null }[] }> };
        };
      };
      const active = () =>
        (JSON.parse(localStorage.getItem("chartgen.alerts.list")!) as { active: boolean }[])[0]!.active;
      w.__chartgenTest.feedPrice("BTCUSDT", 100);
      const before = active();
      const ev = w.__chartgenTest.series().indicators.candlestick_patterns!.events[0]!;
      ev.t = (ev.t ?? 0) + 3600;
      w.__chartgenTest.feedPrice("BTCUSDT", 100);
      return { before, after: active() };
    });
    expect(fired).toEqual({ before: true, after: false });
    await expect(rows(page)).toHaveAttribute("data-status", "triggered");
    await expect(toasts(page).filter({ hasText: "Bullish pattern" })).toBeVisible();
    await shot(page, "alert-fired");
  });
});

test.describe("in a grid (ADR-0043)", () => {
  test("a chart's marks answer the hover in that chart, not another", async ({ page }) => {
    await openWith(page, [{ name: "candlestick_patterns", params: { trend: "none" } }]);
    // A new chart starts as a copy of the first.
    await page.getByTestId("grid-menu").click();
    await page.getByTestId("grid-option-2h").click();
    const second = page.locator('[data-testid=chart-pane][data-pane="1"]');
    const paneLabels = () =>
      page.evaluate(() => {
        const w = window as unknown as {
          __chartgenTest: {
            paneSeries: (n: number) => { indicators: Record<string, { labels?: Mark[] }> } | null;
          };
        };
        return w.__chartgenTest.paneSeries(1)?.indicators.candlestick_patterns?.labels ?? [];
      });
    await expect.poll(async () => (await paneLabels()).length, { timeout: 30_000 }).toBeGreaterThan(10);

    // Then only the second chart has the indicator.
    const first = page.locator('[data-testid=chart-pane][data-pane="0"]');
    const firstBox = (await first.boundingBox())!;
    await page.mouse.click(firstBox.x + firstBox.width * 0.4, firstBox.y + firstBox.height * 0.35);
    await expect(first).toHaveAttribute("data-active", "");
    await page.getByRole("button", { name: "Remove Candlestick Patterns", exact: true }).first().click();
    await expect
      .poll(() =>
        page.evaluate(() => {
          const w = window as unknown as {
            __chartgenTest: { paneSeries: (n: number) => { indicators: Record<string, unknown> } | null };
          };
          return "candlestick_patterns" in (w.__chartgenTest.paneSeries(0)?.indicators ?? {});
        }),
      )
      .toBe(false);

    // The hook's chart is the active one.
    const box = (await second.boundingBox())!;
    await page.mouse.click(box.x + box.width * 0.4, box.y + box.height * 0.35);
    await expect(second).toHaveAttribute("data-active", "");
    await page.mouse.move(box.x + 5, box.y + 5);

    const all = await paneLabels();
    const closed = all.filter((l) => !(l as Mark & { tentative?: boolean }).tentative);
    const lone = closed.filter(
      (l) => closed.filter((o) => o.x === l.x && o.above === l.above).length === 1,
    );
    const l = lone[Math.max(0, lone.length - 4)]!;
    const at = await page.evaluate(
      ([bar, price]) => {
        const w = window as unknown as {
          __chartgenTest: {
            chart: () => {
              convertToPixel: (
                p: { dataIndex: number; value: number },
                f: { paneId: string },
              ) => { x: number; y: number };
            };
          };
        };
        return w.__chartgenTest
          .chart()
          .convertToPixel({ dataIndex: bar as number, value: price as number }, { paneId: "candle_pane" });
      },
      [l.x, l.y] as const,
    );
    const container = (await second.locator(".chart-container").boundingBox())!;
    const y = container.y + at.y + (l.above ? -12 : 12);
    await page.mouse.move(container.x + at.x, y, { steps: 3 });
    await expect(second.getByTestId("pattern-tooltip")).toHaveText(l.tooltip);
    await shot(page, "grid-tooltip");

    // The same spot of the first chart, which has no marks, says nothing.
    await page.mouse.move(firstBox.x + (container.x - box.x) + at.x, y, { steps: 3 });
    await expect(page.getByTestId("pattern-tooltip")).toHaveCount(0);
  });
});

test.describe("chart patterns", () => {
  // Lower thresholds than the defaults: the sample source then always has some.
  const entry = { name: "chart_patterns", params: { depth: 3, deviation: 1, tolerance: 25 } };

  const shapes = (page: Page) =>
    held<
      Array<{
        name: string;
        status: string;
        tooltip: string;
        bias: string;
        label: Mark;
        strokes: Array<{ x1: number; t1: number; x2: number; t2: number }>;
      }>
    >(page, "chart_patterns", "(ind && ind.patterns) || []");

  test("drawn with a label that names the status, and a tooltip", async ({ page }) => {
    await openWith(page, [entry]);
    await expect.poll(async () => (await shapes(page)).length, { timeout: 15_000 }).toBeGreaterThan(0);

    const all = await shapes(page);
    for (const p of all) {
      expect(["forming", "confirmed", "failed"]).toContain(p.status);
      expect(p.strokes.length).toBeGreaterThan(0);
      expect(p.tooltip).toContain(p.name);
    }
    const n = await held<number>(page, "chart_patterns", "bars.length");
    const onScreen = all.filter((p) => p.label.x > n - 150);
    const p = onScreen[0] ?? all[0]!;
    await page.evaluate(
      ([x]) => {
        const w = window as unknown as {
          __chartgenTest: { chart: () => { scrollToDataIndex: (i: number) => void } };
        };
        w.__chartgenTest.chart().scrollToDataIndex(x as number);
      },
      [Math.min(n - 1, p.label.x + 30)],
    );
    await shot(page, "chart-patterns");
    const px = await pixelOf(page, p.label.x, p.label.y);
    await page.mouse.move(px.x, px.y + (p.label.above ? -12 : 12), { steps: 3 });
    await expect(tooltip(page)).toHaveText(p.tooltip);
    await shot(page, "chart-patterns-tooltip");
  });

  test("the outline stays on its bars across live ticks", async ({ page }) => {
    await openWith(page, [entry]);
    await expect.poll(async () => (await shapes(page)).length, { timeout: 15_000 }).toBeGreaterThan(0);
    // A mark on the list as loaded: a tail replaces the list, and with it the mark.
    const marked = () =>
      page.evaluate(() => {
        const w = window as unknown as {
          __chartgenTest: { series: () => { indicators: Record<string, { patterns: unknown[] }> } };
        };
        return "seen" in w.__chartgenTest.series().indicators.chart_patterns!.patterns;
      });
    await page.evaluate(() => {
      const w = window as unknown as {
        __chartgenTest: {
          series: () => { indicators: Record<string, { patterns: Record<string, unknown> }> };
        };
      };
      w.__chartgenTest.series().indicators.chart_patterns!.patterns.seen = true;
    });
    // Tails go by, whose indices run from their own first bar.
    await expect.poll(marked, { timeout: 15_000 }).toBe(false);
    const bars = await held<Array<{ t: number }>>(page, "chart_patterns", "bars");
    const after = await shapes(page);
    expect(after.length).toBeGreaterThan(0);
    let checked = 0;
    for (const p of after) {
      for (const s of p.strokes) {
        if (s.x1 >= 0) {
          expect(bars[s.x1]!.t).toBe(s.t1);
          checked++;
        }
        if (s.x2 >= 0) {
          expect(bars[s.x2]!.t).toBe(s.t2);
          checked++;
        }
      }
    }
    expect(checked).toBeGreaterThan(0);
  });

  test("works on Heikin Ashi bars", async ({ page }) => {
    await openWith(page, [entry, { name: "candlestick_patterns", params: { trend: "none" } }]);
    await page.getByRole("button", { name: /^Chart type/ }).click();
    await page.getByRole("menuitemradio", { name: "Heikin Ashi", exact: true }).click();
    await expect
      .poll(
        () => page.evaluate(() => (window as unknown as { __chartgenTest: { series: () => { chart_style?: string } } }).__chartgenTest.series()?.chart_style),
        { timeout: 15_000 },
      )
      .toBe("heikin_ashi");
    await expect.poll(async () => (await shapes(page)).length, { timeout: 15_000 }).toBeGreaterThan(0);
    await shot(page, "heikin-ashi");
  });
});

test.describe("auto fib retracement", () => {
  const entry = { name: "auto_fib", params: { depth: 5, deviation: 1 } };
  type Level = { line: number; x: number; y: number };
  const levels = (page: Page) => held<Level[]>(page, "auto_fib", "(ind && ind.levels) || []");

  test("levels from the last swing, each plot flat from its start", async ({ page }) => {
    await openWith(page, [entry]);
    await expect.poll(async () => (await levels(page)).length, { timeout: 15_000 }).toBe(7);
    const lv = await levels(page);
    const lines = await held<Array<{ label: string; values: Array<number | null> }>>(
      page,
      "auto_fib",
      "ind.lines",
    );
    for (const l of lv) {
      const v = lines[l.line]!.values;
      expect(v.at(-1)).toBeCloseTo(l.y, 6);
      if (l.x > 0) expect(v[l.x - 1]).toBeNull();
      expect(v[Math.max(0, l.x)]).toBeCloseTo(l.y, 6);
    }
    // 0 and 1 are the ends of the swing, 0.5 half way.
    const y = (r: string) => lv[lines.findIndex((p) => p.label === r)]!.y;
    expect(y("0.5")).toBeCloseTo((y("0") + y("1")) / 2, 6);
    await shot(page, "auto-fib");
  });

  test("a level's value and color can be edited", async ({ page }) => {
    await openWith(page, [entry]);
    await expect.poll(async () => (await levels(page)).length, { timeout: 15_000 }).toBe(7);
    const row = page.getByTestId("indicator-legend-auto_fib");
    await row.getByRole("button", { name: /^Settings for/ }).click();
    await expect(dialog(page).getByTestId("repaint-note")).toContainText("redrawn");
    await dialog(page).getByLabel("level2").fill("0.3");
    await dialog(page).getByRole("tab", { name: "Style" }).click();
    await expect(dialog(page).getByLabel("0.618 color")).toBeVisible();
    await expect(dialog(page).getByLabel("Show 0.618")).toBeChecked();
    await shot(page, "auto-fib-style");
    await dialog(page).getByRole("button", { name: "OK" }).click();
    await expect
      .poll(() => held<string>(page, "auto_fib", "ind.lines[1].label"), { timeout: 15_000 })
      .toBe("0.3");
    const lv = await levels(page);
    const lines = await held<Array<{ label: string }>>(page, "auto_fib", "ind.lines");
    const y = (r: string) => lv.find((l) => lines[l.line]!.label === r)!.y;
    expect(y("0.3")).toBeCloseTo(y("0") + (y("1") - y("0")) * 0.3, 6);
  });

  test("an alert fires when the price crosses a level", async ({ page }) => {
    await openWith(page, [entry]);
    await expect.poll(async () => (await levels(page)).length, { timeout: 15_000 }).toBe(7);
    await page.keyboard.press("Alt+KeyA");
    await expect(dialog(page)).toBeVisible();
    const level = dialog(page).getByLabel(/^Level/);
    const value = await level.locator("option", { hasText: ": 0.5" }).getAttribute("value");
    await level.selectOption(value!);
    await dialog(page).getByRole("button", { name: "Create" }).click();
    await page.getByRole("tab", { name: /^Alerts/ }).click();
    await expect(rows(page)).toContainText("Close crossing 0.5");
    await feed(page, [1, 1e9]);
    await expect(rows(page)).toHaveAttribute("data-status", "triggered");
    await expect(toasts(page).filter({ hasText: "Close crossing 0.5" })).toBeVisible();
  });
});
