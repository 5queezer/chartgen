// Order-flow chart types (ADR-0048) on sample data: Volume candles, Volume footprint,
// TPO, and volume profiles read from finer bars. Numbers are tested in Rust; this is
// what reaches the screen. Charts are read through `window.__chartgenTest`.

import type { Page, Request } from "@playwright/test";

import { test, expect, reloadApp } from "./fixtures/app";

interface PaneInfo {
  pane: number;
  symbol: string;
  flowStatus: string | null;
  flowMode: string;
  barSpace: number | null;
}

const pane = (page: Page, n = 0): Promise<PaneInfo> =>
  page.evaluate(
    (k) => (window as unknown as { __chartgenTest: { panes: () => PaneInfo[] } }).__chartgenTest.panes()[k]!,
    n,
  );

const status = (page: Page) => page.getByTestId("orderflow-status");

async function pick(page: Page, label: string): Promise<void> {
  await page.getByRole("button", { name: /^Chart type:/ }).click();
  await page.getByRole("menuitemradio", { name: label, exact: true }).click();
}

function recordErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(e.message));
  return errors;
}

interface Call {
  name?: string;
  arguments?: Record<string, unknown>;
}

function recordCalls(page: Page, tool: string): Record<string, unknown>[] {
  const calls: Record<string, unknown>[] = [];
  page.on("request", (req: Request) => {
    if (req.method() !== "POST" || !req.url().endsWith("/mcp")) return;
    try {
      const b = req.postDataJSON() as { method?: string; params?: Call };
      if (b.method === "tools/call" && b.params?.name === tool) calls.push(b.params.arguments ?? {});
    } catch {
      // Not JSON: not a tool call.
    }
  });
  return calls;
}

/** The settings a painter draws with, from the chart itself. */
const painterExtend = (page: Page, name: string) =>
  page.evaluate((reg) => {
    const c = (window as unknown as {
      __chartgenTest: {
        chart: () => {
          getIndicatorByPaneId: (p: string, n: string) => { extendData?: Record<string, unknown> } | null;
        };
      };
    }).__chartgenTest.chart();
    const ind = c.getIndicatorByPaneId("candle_pane", reg);
    if (!ind) return null;
    const ext = ind.extendData as { settings: unknown; bars?: Map<number, unknown>; profiles?: Map<number, unknown> };
    return { settings: ext.settings, count: (ext.bars ?? ext.profiles)!.size };
  }, name);

const FOOTPRINT = "chartgen__footprint";
const TPO = "chartgen__tpo";
const TABLE = "chartgen__orderflow_table";

const hasTablePane = (page: Page) =>
  page.evaluate((reg) => {
    const c = (window as unknown as {
      __chartgenTest: { chart: () => { getIndicatorByPaneId: (p: string, n: string) => unknown } };
    }).__chartgenTest.chart();
    return !!c.getIndicatorByPaneId("orderflow_table_pane", reg);
  }, TABLE);

const tableIsLast = (page: Page) =>
  page.evaluate(() => {
    const c = (window as unknown as {
      __chartgenTest: {
        chart: () => {
          getSize: (p?: string) => { top: number } | null;
          getIndicatorByPaneId: () => Map<string, unknown>;
        };
      };
    }).__chartgenTest.chart();
    const tops = [...c.getIndicatorByPaneId().keys()].map((p) => c.getSize(p)?.top ?? 0);
    return c.getSize("orderflow_table_pane")?.top === Math.max(...tops);
  });

/** Pixels the candle pane painted of colors close to `rgb`, at any opacity. */
const paintedLike = (page: Page, rgb: [number, number, number]) =>
  page.evaluate(([r, g, b]) => {
    const c = (window as unknown as {
      __chartgenTest: { chart: () => { getDom: (p: string, k: string) => HTMLElement } };
    }).__chartgenTest.chart();
    const canvas = c.getDom("candle_pane", "main").querySelector("canvas")!;
    const { data } = canvas.getContext("2d")!.getImageData(0, 0, canvas.width, canvas.height);
    let n = 0;
    for (let i = 0; i < data.length; i += 4) {
      const a = data[i + 3]! / 255;
      if (a < 0.1) continue;
      // Canvas data is not premultiplied on read: compare the colors as they are.
      if (Math.abs(data[i]! - r!) < 24 && Math.abs(data[i + 1]! - g!) < 24 && Math.abs(data[i + 2]! - b!) < 24) n++;
    }
    return n;
  }, rgb);

const BULL: [number, number, number] = [0x26, 0xa6, 0x9a];
const BEAR: [number, number, number] = [0xef, 0x53, 0x50];
const POC: [number, number, number] = [0xff, 0x98, 0x00];

async function openSettings(page: Page) {
  await page.getByTestId("orderflow-settings-open").click();
  const dialog = page.getByTestId("orderflow-settings");
  await expect(dialog).toBeVisible();
  return dialog;
}

async function closeSettings(page: Page) {
  await page.getByTestId("orderflow-settings").getByRole("button", { name: "Close" }).click();
}

test.describe("volume candles", () => {
  test("draw with widths that follow volume, and give the candles back", async ({ bootedApp: page }) => {
    const errors = recordErrors(page);
    await pick(page, "Volume candles");
    await expect
      .poll(() =>
        page.evaluate(() => {
          const t = (window as unknown as {
            __chartgenTest: { chart: () => { getIndicatorByPaneId: (p: string, n: string) => { extendData?: { type?: string } } | null } };
          }).__chartgenTest;
          return t.chart().getIndicatorByPaneId("candle_pane", "chartgen__series")?.extendData?.type;
        }),
      )
      .toBe("volume_candles");
    await expect.poll(() => paintedLike(page, BULL)).toBeGreaterThan(50);
    // Widen the bars: at the default zoom every candle would be the same few pixels.
    await page.evaluate(() => {
      (window as unknown as { __chartgenTest: { chart: () => { setBarSpace: (n: number) => void } } }).__chartgenTest
        .chart()
        .setBarSpace(40);
    });
    await page.waitForTimeout(300);
    const widths = await page.evaluate(() => {
      const c = (window as unknown as {
        __chartgenTest: {
          chart: () => {
            getDataList: () => { high: number; low: number; open: number; close: number; volume: number }[];
            getVisibleRange: () => { from: number; to: number };
            getDom: (p: string, k: string) => HTMLElement;
            convertToPixel: (p: { dataIndex: number; value: number }, f: { paneId: string }) => { x: number; y: number };
          };
        };
      }).__chartgenTest.chart();
      const canvas = c.getDom("candle_pane", "main").querySelector("canvas")!;
      const ctx = canvas.getContext("2d")!;
      const ratio = canvas.width / canvas.clientWidth;
      const list = c.getDataList();
      const { from, to } = c.getVisibleRange();
      const out: { volume: number; width: number }[] = [];
      for (let i = from; i < Math.min(to, list.length - 1); i++) {
        const b = list[i]!;
        const p = c.convertToPixel({ dataIndex: i, value: (b.open + b.close) / 2 }, { paneId: "candle_pane" });
        const row = ctx.getImageData(0, Math.round(p.y * ratio), canvas.width, 1).data;
        const center = Math.round(p.x * ratio);
        const on = (x: number) => row[x * 4 + 3]! > 200;
        if (!on(center)) continue;
        let a = center;
        let z = center;
        while (a > 0 && on(a - 1)) a--;
        while (z < canvas.width - 1 && on(z + 1)) z++;
        out.push({ volume: b.volume, width: (z - a + 1) / ratio });
      }
      return out;
    });
    expect(widths.length).toBeGreaterThan(5);
    const sorted = [...widths].sort((a, b) => a.volume - b.volume);
    expect(sorted.at(-1)!.width).toBeGreaterThan(sorted[0]!.width + 4);

    await pick(page, "Candles");
    await expect.poll(() => page.evaluate(() => {
      const t = (window as unknown as {
        __chartgenTest: { chart: () => { getIndicatorByPaneId: (p: string, n: string) => unknown } };
      }).__chartgenTest;
      return !!t.chart().getIndicatorByPaneId("candle_pane", "chartgen__series");
    })).toBe(false);
    expect(errors).toEqual([]);
  });
});

test.describe("volume footprint", () => {
  test("renders cells at the widest bars, says how it was made, and survives a reload", async ({
    bootedApp: page,
  }, testInfo) => {
    const errors = recordErrors(page);
    const calls = recordCalls(page, "get_footprint");
    const before = (await pane(page)).barSpace;
    await pick(page, "Volume footprint");
    await expect(status(page)).toContainText("from 1m bars");
    await expect(status(page)).toContainText("taker volume");
    await expect(status(page)).toContainText(/row [\d.]+ auto/);
    await expect.poll(async () => (await pane(page)).barSpace).toBe(50);
    await expect.poll(async () => (await pane(page)).flowMode).toMatch(/^cells/);
    await expect.poll(async () => (await painterExtend(page, FOOTPRINT))?.count ?? 0).toBeGreaterThan(10);
    expect(calls[0]).toMatchObject({ ticker: "BTCUSDT", bars: 60, imbalance: 300, value_area: 70 });
    // Both sides of a bar, and the POC outline, reach the canvas.
    await expect.poll(() => paintedLike(page, BULL)).toBeGreaterThan(100);
    await expect.poll(() => paintedLike(page, BEAR)).toBeGreaterThan(100);
    await expect.poll(() => paintedLike(page, POC)).toBeGreaterThan(30);
    await page.waitForTimeout(500);
    await page.screenshot({ path: testInfo.outputPath("footprint.png") });

    await reloadApp(page);
    await expect(status(page)).toContainText("from 1m bars");
    await pick(page, "Candles");
    await expect(status(page)).toHaveCount(0);
    await expect.poll(async () => (await painterExtend(page, FOOTPRINT))?.count ?? -1).toBe(-1);
    // Back to the zoom it replaced (a reload starts from the default too).
    await expect.poll(async () => (await pane(page)).barSpace).toBe(before);
    expect(errors).toEqual([]);
  });

  test("zoomed out it falls back to candles and says so, and asks for no more data", async ({ bootedApp: page }) => {
    await pick(page, "Volume footprint");
    await expect.poll(async () => (await pane(page)).flowMode).toMatch(/^cells/);
    const calls = recordCalls(page, "get_footprint");
    await page.evaluate(() => {
      (window as unknown as { __chartgenTest: { chart: () => { setBarSpace: (n: number) => void } } }).__chartgenTest
        .chart()
        .setBarSpace(8);
    });
    await expect.poll(async () => (await pane(page)).flowMode).toBe("candles");
    await expect(status(page)).toContainText("Zoom in for footprint cells");
    expect(calls.filter((c) => c.end_time !== undefined)).toEqual([]);
    // Candles really are painted: solid colors, not nothing.
    await expect.poll(() => paintedLike(page, BULL)).toBeGreaterThan(30);
  });

  test("settings apply: what the server computes reloads, what is drawn does not", async ({ bootedApp: page }, testInfo) => {
    await pick(page, "Volume footprint");
    await expect(status(page)).toContainText("from 1m bars");
    const calls = recordCalls(page, "get_footprint");
    const dialog = await openSettings(page);

    await dialog.getByLabel("Imbalance %").fill("150");
    await dialog.getByLabel("Imbalance %").press("Tab");
    await expect.poll(() => calls.at(-1)?.imbalance).toBe(150);
    await dialog.getByLabel("Ticks per row (0: auto)").fill("500");
    await dialog.getByLabel("Ticks per row (0: auto)").press("Tab");
    await expect.poll(() => calls.at(-1)?.row_ticks).toBe(500);

    const fetched = calls.length;
    await dialog.getByLabel("Type").selectOption("delta");
    await dialog.getByLabel("Display").selectOption("profile");
    await dialog.getByLabel("Gradient background").check();
    await dialog.getByLabel("Summary").selectOption("table");
    await closeSettings(page);
    await expect
      .poll(async () => (await painterExtend(page, FOOTPRINT))?.settings)
      .toMatchObject({ type: "delta", display: "profile", gradient: true, summary: "table", imbalance: 150 });
    await expect.poll(() => hasTablePane(page)).toBe(true);
    expect(calls.slice(fetched).filter((c) => c.bars === 60)).toEqual([]);
    await page.waitForTimeout(500);
    await page.screenshot({ path: testInfo.outputPath("footprint-table.png") });

    // Settings last across a reload, and leave no pane when turned off.
    await reloadApp(page);
    await expect.poll(async () => (await painterExtend(page, FOOTPRINT))?.settings).toMatchObject({ type: "delta" });
    // The table is the bottom pane, also when it was created before the sub-panes.
    await expect.poll(() => tableIsLast(page)).toBe(true);
    const again = await openSettings(page);
    await again.getByRole("button", { name: "Reset" }).click();
    await closeSettings(page);
    await expect.poll(() => hasTablePane(page)).toBe(false);
    await expect.poll(async () => (await painterExtend(page, FOOTPRINT))?.settings).toMatchObject({ type: "buy_sell" });
  });

  test("polls the newest bars with the row size of the load", async ({ bootedApp: page }) => {
    const errors = recordErrors(page);
    await pick(page, "Volume footprint");
    await expect(status(page)).toContainText("from 1m bars");
    const calls = recordCalls(page, "get_footprint");
    await expect.poll(() => calls.filter((c) => c.bars === 3).length, { timeout: 15_000 }).toBeGreaterThan(0);
    const tail = calls.find((c) => c.bars === 3)!;
    expect(tail.row_size).toBeGreaterThan(0);
    expect(tail.row_ticks).toBeUndefined();
    // And the interval of the load: its few bars could afford a finer one on a stock.
    expect(tail.intrabar_interval).toBe("1m");
    // The poll sends the row size back, and it is still the auto one.
    await page.waitForTimeout(500);
    await expect(status(page)).toContainText(/row [\d.]+ auto/);
    expect(errors).toEqual([]);
  });

  test("scrolling back pages in older bars", async ({ bootedApp: page }) => {
    await pick(page, "Volume footprint");
    await expect(status(page)).toContainText("from 1m bars");
    const calls = recordCalls(page, "get_footprint");
    const box = (await page.locator(".chart-container").boundingBox())!;
    const y = box.y + box.height * 0.2;
    await expect
      .poll(
        async () => {
          await page.mouse.move(box.x + 60, y);
          await page.mouse.down();
          await page.mouse.move(box.x + box.width * 0.8, y, { steps: 8 });
          await page.mouse.up();
          return calls.filter((c) => c.end_time !== undefined).length;
        },
        { timeout: 20_000 },
      )
      .toBeGreaterThan(0);
    const older = calls.find((c) => c.end_time !== undefined)!;
    expect(older.bars).toBe(60);
    expect(older.row_size).toBeGreaterThan(0);
    await expect.poll(async () => (await painterExtend(page, FOOTPRINT))?.count ?? 0).toBeGreaterThan(60);
  });

  test("on a phone it is one chart and stays usable", async ({ bootedApp: page }) => {
    const errors = recordErrors(page);
    await page.evaluate(() => localStorage.setItem("chartgen.layout.chartType", '"footprint"'));
    await page.setViewportSize({ width: 420, height: 800 });
    await page.reload();
    await expect(status(page)).toContainText("from 1m bars", { timeout: 30_000 });
    await page.getByTestId("orderflow-settings-open").click();
    await expect(page.getByTestId("orderflow-settings")).toBeVisible();
    await closeSettings(page);
    expect(errors).toEqual([]);
  });

  test("a stock says it has no aggressor data", async ({ bootedApp: page }) => {
    await pick(page, "Volume footprint");
    await expect(status(page)).toContainText("taker volume");
    await page
      .getByRole("table", { name: "Watchlist" })
      .locator('[role=row][data-symbol="AAPL"]')
      .click();
    await expect(status(page)).toContainText("close vs open");
  });

  test("each chart of a grid has its own, and nothing fails with four", async ({ bootedApp: page }, testInfo) => {
    const errors = recordErrors(page);
    await pick(page, "Volume footprint");
    await expect(status(page)).toContainText("from 1m bars");
    await page.getByTestId("grid-menu").click();
    await page.getByTestId("grid-option-4").click();
    await expect(status(page)).toHaveCount(4);
    for (const n of [0, 1, 2, 3]) await expect(status(page).nth(n)).toContainText("from 1m bars");
    const cell = page.locator('[data-testid=chart-pane][data-pane="1"]');
    const box = (await cell.boundingBox())!;
    await page.mouse.click(box.x + box.width * 0.4, box.y + box.height * 0.35);
    await pick(page, "TPO");
    await expect(cell.getByTestId("orderflow-status")).toContainText("1D from 30m blocks");
    await expect(page.locator('[data-testid=chart-pane][data-pane="0"]').getByTestId("orderflow-status")).toContainText("from 1m bars");
    await page.waitForTimeout(800);
    await page.screenshot({ path: testInfo.outputPath("grid.png") });
    expect(errors).toEqual([]);
  });
});

test.describe("TPO", () => {
  test("renders profiles, takes its settings and says how it was made", async ({ bootedApp: page }, testInfo) => {
    const errors = recordErrors(page);
    const calls = recordCalls(page, "get_tpo");
    await pick(page, "TPO");
    await expect(status(page)).toContainText("1D from 30m blocks");
    await expect(status(page)).toContainText(/row [\d.]+ auto/);
    await expect.poll(async () => (await painterExtend(page, TPO))?.count ?? 0).toBeGreaterThan(3);
    expect(calls[0]).toMatchObject({ period: "1D", block: "30m", periods: 10, value_area: 70, ib_blocks: 2 });
    // A poll asks for the period before too: its last block may have changed.
    await expect.poll(() => calls.filter((c) => c.periods === 2).length, { timeout: 40_000 }).toBeGreaterThan(0);
    await expect.poll(() => paintedLike(page, POC)).toBeGreaterThan(30);
    await page.waitForTimeout(500);
    await page.screenshot({ path: testInfo.outputPath("tpo.png") });

    const dialog = await openSettings(page);
    await dialog.getByLabel("Block size").selectOption("1h");
    await expect.poll(() => calls.at(-1)?.block).toBe("1h");
    await expect(status(page)).toContainText("1D from 1h blocks");
    // A month is too many 5m blocks: it says what it did instead.
    await dialog.getByLabel("Block size").selectOption("5m");
    await dialog.getByLabel("Period").selectOption("1M");
    await expect(status(page)).toContainText("1M from 15m blocks (shorter ones cannot cover it)");
    await dialog.getByLabel("Block size").selectOption("1h");
    await dialog.getByLabel("Period").selectOption("1W");
    await expect.poll(() => calls.at(-1)?.period).toBe("1W");
    await dialog.getByLabel("Sessions (HH:MM-HH:MM, commas)").fill("00:00-08:00, 08:00-16:00");
    await dialog.getByLabel("Sessions (HH:MM-HH:MM, commas)").press("Tab");
    await expect.poll(() => calls.at(-1)?.sessions).toEqual(["00:00-08:00", "08:00-16:00"]);
    await dialog.getByLabel("Sessions (HH:MM-HH:MM, commas)").fill("");
    await dialog.getByLabel("Sessions (HH:MM-HH:MM, commas)").press("Tab");
    await dialog.getByLabel("Period").selectOption("1D");

    await dialog.getByLabel("Split by blocks").check();
    await dialog.getByLabel("Draw as").selectOption("blocks");
    await dialog.getByLabel("Volume profile").check();
    await dialog.getByLabel("Summary pane").check();
    await closeSettings(page);
    await expect
      .poll(async () => (await painterExtend(page, TPO))?.settings)
      .toMatchObject({ splitByBlocks: true, style: "blocks", showProfile: true, summary: true });
    await expect.poll(() => hasTablePane(page)).toBe(true);
    await page.waitForTimeout(500);
    await page.screenshot({ path: testInfo.outputPath("tpo-options.png") });
    expect(errors).toEqual([]);
  });

  test("a bad session says what failed and the chart stays usable", async ({ bootedApp: page }) => {
    await pick(page, "TPO");
    await expect(status(page)).toContainText("1D from 30m blocks");
    const dialog = await openSettings(page);
    await dialog.getByLabel("Sessions (HH:MM-HH:MM, commas)").fill("nonsense");
    await dialog.getByLabel("Sessions (HH:MM-HH:MM, commas)").press("Tab");
    await closeSettings(page);
    await expect(status(page)).toContainText("Order flow failed");
    await expect(status(page)).toContainText("HH:MM");
    // Candles, not an empty chart.
    await expect.poll(() => paintedLike(page, BULL)).toBeGreaterThan(30);
  });

  test("in bar replay it shows candles and says so", async ({ bootedApp: page }) => {
    await pick(page, "TPO");
    await expect(status(page)).toContainText("1D from 30m blocks");
    await page.getByRole("button", { name: "Bar replay" }).click();
    await expect(status(page)).toContainText("off in replay");
    await expect.poll(async () => (await painterExtend(page, TPO))?.count).toBe(0);
    await expect.poll(() => paintedLike(page, BULL)).toBeGreaterThan(30);
  });
});

test.describe("volume profiles from finer bars", () => {
  test("only the chart's load asks for them, not a page or a tail", async ({ page, bootedApp: _ }) => {
    await page.evaluate(() =>
      localStorage.setItem("chartgen.indicators.active", JSON.stringify(["volume_profile"])),
    );
    const calls = recordCalls(page, "generate_chart");
    await reloadApp(page);
    await expect(page.getByTestId("indicator-legend-volume_profile")).toContainText("from");
    const load = calls.find((c) => c.bars === 500 && c.end_time === undefined);
    expect(load?.profile_intrabars).toBe(true);
    await page.evaluate(() =>
      (window as unknown as { __chartgenTest: { chart: () => { setBarSpace: (n: number) => void } } }).__chartgenTest
        .chart()
        .setBarSpace(1),
    );
    await expect.poll(() => calls.filter((c) => c.end_time !== undefined).length, { timeout: 20_000 }).toBeGreaterThan(0);
    for (const later of calls.filter((c) => c !== load && c.indicators !== undefined)) {
      if (later.end_time !== undefined || later.bars !== 500) expect(later.profile_intrabars).toBeUndefined();
    }
  });

  test("say which, and the new ones reach the chart", async ({ page, bootedApp: _ }) => {
    await page.evaluate(() =>
      localStorage.setItem(
        "chartgen.indicators.active",
        JSON.stringify(["volume_profile", "anchored_vp", "fixed_range_vp", "session_vp"]),
      ),
    );
    await reloadApp(page);
    for (const key of ["volume_profile", "anchored_vp", "fixed_range_vp"]) {
      await expect(page.getByTestId(`indicator-legend-${key}`)).toContainText("from");
      await expect(page.getByTestId(`indicator-legend-${key}`)).toContainText("bars");
    }
    // Its live updates spread each bar's volume, so its load does too, and says nothing.
    await expect(page.getByTestId("indicator-legend-session_vp")).not.toContainText("from");
    const hbars = await page.evaluate(() => {
      const s = (window as unknown as {
        __chartgenTest: { series: () => { indicators: Record<string, { hbars?: unknown[]; profile_interval?: string }> } };
      }).__chartgenTest.series();
      return Object.fromEntries(
        Object.entries(s.indicators).map(([k, v]) => [k, [v.hbars?.length ?? 0, v.profile_interval]]),
      );
    });
    expect(hbars.anchored_vp![0]).toBeGreaterThan(0);
    expect(hbars.fixed_range_vp![0]).toBeGreaterThan(0);
    expect(hbars.volume_profile![1]).toBeTruthy();

    // A session holds its levels to its last bar, and nothing joins it to the next.
    const session = await page.evaluate(() => {
      const t = (window as unknown as {
        __chartgenTest: {
          series: () => { indicators: Record<string, { lines: { values: (number | null)[] }[] }> };
          chart: () => {
            getIndicatorByPaneId: (p: string, n: string) => {
              figures: { key: string; styles?: (d: unknown, ind: unknown, s: unknown) => { color?: string } }[];
            } | null;
          };
        };
      }).__chartgenTest;
      const ind = t.chart().getIndicatorByPaneId("candle_pane", "chartgen_session_vp")!;
      const figure = ind.figures.find((f) => f.key === "line0")!;
      const color = (current: number, next: number) =>
        figure.styles!(
          { current: { indicatorData: { line0: current } }, next: { indicatorData: { line0: next } } },
          ind,
          {},
        ).color;
      const values = t.series().indicators.session_vp!.lines[0]!.values;
      return { nulls: values.filter((v) => v === null).length, same: color(1, 1), jump: color(1, 2) };
    });
    expect(session.nulls).toBe(0);
    expect(session.jump).toBe("transparent");
    expect(session.same).not.toBe("transparent");

    // A session profile lasts through live updates: on master it fell out of them.
    await page.waitForTimeout(3000);
    expect(
      await page.evaluate(
        () =>
          (window as unknown as { __chartgenTest: { series: () => { indicator_errors?: unknown } } }).__chartgenTest.series()
            .indicator_errors ?? null,
      ),
    ).toBeNull();
  });
});
