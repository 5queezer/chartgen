// Bar replay (ADR-0023, web guide "Bar replay"). The suite's server runs
// with CHARTGEN_DATA_SOURCE=sample: BTCUSDT is pushed (ADR-0021) every
// ~500 ms while live, or polled every 2 s on the polling path. The chart is
// observed through the DEV-only `window.__chartgenTest` hook (App.tsx) and
// the MCP request log.

import type { Page, Request } from "@playwright/test";

import { test, expect, gotoApp, openSidebarTab } from "./fixtures/app";

interface ToolCall {
  name: string;
  args: { bars?: number; end_time?: number; indicators?: unknown[] };
}

/** tools/call of an MCP request, or null for anything else. */
function toolCall(req: Request): ToolCall | null {
  if (req.method() !== "POST" || !req.url().endsWith("/mcp")) return null;
  try {
    const b = req.postDataJSON() as {
      method?: string;
      params?: { name?: string; arguments?: ToolCall["args"] };
    };
    return b.method === "tools/call" && b.params?.name
      ? { name: b.params.name, args: b.params.arguments ?? {} }
      : null;
  } catch {
    return null;
  }
}

/** A live update request: a tail poll or a push subscription. */
const isLive = (c: ToolCall) =>
  c.name === "subscribe_chart" ||
  (c.name === "generate_chart" && c.args.bars === 2 && c.args.end_time === undefined);
const isUnsubscribe = (c: ToolCall) => c.name === "unsubscribe_chart";

function recordCalls(page: Page): ToolCall[] {
  const calls: ToolCall[] = [];
  page.on("request", (req) => {
    const c = toolCall(req);
    if (c) calls.push(c);
  });
  return calls;
}

interface Hook {
  chart: () => {
    getDataList: () => Array<{ timestamp: number }>;
    convertFromPixel: (
      c: Array<{ x: number }>,
      finder: { paneId: string },
    ) => Array<{ dataIndex?: number }>;
    getDom: () => HTMLElement | null;
  } | null;
  series: () => { bars: Array<{ t: number; c: number }> } | null;
  shownSeries: () => {
    bars: Array<{ t: number }>;
    indicators: Record<string, { hbars?: unknown[]; lines?: Array<{ values: unknown[] }> }>;
  } | null;
  replay: () => { mode: string; playing: boolean; cursorTime: number | null };
}

/** Timestamps klinecharts holds. */
const shownTimes = (page: Page) =>
  page.evaluate(() =>
    ((window as unknown as { __chartgenTest: Hook }).__chartgenTest
      .chart()
      ?.getDataList() ?? []).map((d) => d.timestamp),
  );

/** Bar times (s) of the live feed's full series. */
const liveTimes = (page: Page) =>
  page.evaluate(
    () =>
      (window as unknown as { __chartgenTest: Hook }).__chartgenTest
        .series()
        ?.bars.map((b) => b.t) ?? [],
  );

/** Close of the live feed's newest bar: moves with every applied tick. */
const liveClose = (page: Page) =>
  page.evaluate(
    () => (window as unknown as { __chartgenTest: Hook }).__chartgenTest.series()?.bars.at(-1)?.c ?? null,
  );

const drawings = (page: Page) =>
  page.evaluate(
    () => (window as unknown as { __chartgenDrawings: () => unknown[] }).__chartgenDrawings().length,
  );

const trendLine = (page: Page) => page.getByRole("button", { name: "Trend line" });

const replayState = (page: Page) =>
  page.evaluate(() => (window as unknown as { __chartgenTest: Hook }).__chartgenTest.replay());

const shownCount = async (page: Page) => (await shownTimes(page)).length;

/** Enter replay and click the middle of the chart; returns the picked bar's time (ms). */
async function startInMiddle(page: Page): Promise<number> {
  await page.getByTestId("toolbar-replay").click();
  await expect(page.getByTestId("replay-bar")).toHaveAttribute("data-mode", "select");
  const box = (await page.locator(".chart-container").boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height * 0.3;
  await page.mouse.move(x, y);
  await expect(page.getByTestId("replay-cutline").locator("div").first()).toBeVisible();
  // The bar under the mouse, as klinecharts maps it.
  const picked = await page.evaluate((px) => {
    const c = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!;
    const rect = c.getDom()!.getBoundingClientRect();
    const [p] = c.convertFromPixel([{ x: px - rect.left }], { paneId: "candle_pane" });
    return c.getDataList()[p!.dataIndex!]!.timestamp;
  }, x);
  await page.mouse.click(x, y);
  await expect(page.getByTestId("replay-bar")).toHaveAttribute("data-mode", "active");
  return picked;
}

/**
 * Pick a start, step, play and pause, then jump to real time, checking that
 * nothing live reaches the chart meanwhile. `badge`: the live badge state
 * of this path ("push" or "live").
 */
async function roundTrip(page: Page, badge: "push" | "live"): Promise<void> {
  const calls = recordCalls(page);
  await expect(page.getByTestId("live-status")).toHaveAttribute("data-state", badge);
  // Ticks move the newest close while live.
  const before = await liveClose(page);
  await expect.poll(() => liveClose(page), { timeout: 6_000 }).not.toBe(before);
  const fullCount = await shownCount(page);

  const picked = await startInMiddle(page);
  // A push subscription is ended at once.
  if (badge === "push") await expect.poll(() => calls.filter(isUnsubscribe).length).toBe(1);
  const liveDuringReplay = calls.filter(isLive).length;
  const closeDuringReplay = await liveClose(page);
  await expect.poll(async () => (await shownTimes(page)).at(-1)).toBe(picked);
  const startCount = await shownCount(page);
  expect(startCount).toBeLessThan(fullCount);
  expect((await replayState(page)).cursorTime).toBe(picked / 1000);
  // No live data in the toolbar badge while replaying.
  await expect(page.getByTestId("live-status")).toHaveCount(0);

  // One step adds exactly the next bar, by button and by Shift+→.
  const times = await liveTimes(page);
  const at = times.indexOf(picked / 1000);
  await page.getByTestId("replay-step").click();
  await expect.poll(() => shownCount(page)).toBe(startCount + 1);
  expect((await shownTimes(page)).at(-1)).toBe(times[at + 1]! * 1000);
  await page.keyboard.press("Shift+ArrowRight");
  await expect.poll(() => shownCount(page)).toBe(startCount + 2);
  expect((await shownTimes(page)).at(-1)).toBe(times[at + 2]! * 1000);

  // Play at 10 bars/s advances; pause stops.
  await page.getByTestId("replay-speed").click();
  await page.getByRole("menuitemradio", { name: /^10x/ }).click();
  await expect(page.getByTestId("replay-speed")).toHaveText("10x");
  await page.getByTestId("replay-play").click();
  await expect(page.getByTestId("replay-play")).toHaveAttribute("aria-label", "Pause");
  await expect.poll(() => shownCount(page), { timeout: 5_000 }).toBeGreaterThan(startCount + 5);
  await page.keyboard.press("Shift+ArrowDown");
  await expect(page.getByTestId("replay-play")).toHaveAttribute("aria-label", "Play");
  const paused = await shownCount(page);
  await page.waitForTimeout(800);
  expect(await shownCount(page)).toBe(paused);
  // Contiguous bars, the same as the live series up to the cursor.
  const shown = await shownTimes(page);
  expect(shown).toEqual(times.slice(0, shown.length).map((t) => t * 1000));

  // Two poll periods and more since entering: no poll, no subscription, and
  // no pushed tick applied to the live series.
  await page.waitForTimeout(2_500);
  expect(calls.filter(isLive).length).toBe(liveDuringReplay);
  expect(await liveClose(page)).toBe(closeDuringReplay);

  // Jump to real time: the whole series again, and live resumes.
  await page.getByTestId("replay-realtime").click();
  await expect(page.getByTestId("replay-bar")).toHaveCount(0);
  await expect.poll(async () => (await shownTimes(page)).length).toBeGreaterThanOrEqual(fullCount);
  expect(await shownTimes(page)).toEqual((await liveTimes(page)).map((t) => t * 1000));
  await expect
    .poll(() => calls.filter(isLive).length, { timeout: 6_000 })
    .toBeGreaterThan(liveDuringReplay);
  if (badge === "push") {
    expect(calls.filter(isLive).at(-1)!.name).toBe("subscribe_chart");
  }
  await expect(page.getByTestId("live-status")).toHaveAttribute("data-state", badge, {
    timeout: 10_000,
  });
  await expect.poll(() => liveClose(page), { timeout: 6_000 }).not.toBe(closeDuringReplay);
}

test.describe("bar replay on the polling path", () => {
  test.use({ push: false });
  test("no polls while replaying; polling resumes", async ({ bootedApp: page }) => {
    await roundTrip(page, "live");
  });
});

test.describe("bar replay", () => {
  test("pick a start, step, play and pause, then jump to real time", async ({
    bootedApp: page,
  }) => {
    await roundTrip(page, "push");
  });

  test("Esc cancels an armed drawing tool first, then exits", async ({ bootedApp: page }) => {
    await startInMiddle(page);
    await trendLine(page).click();
    await expect(trendLine(page)).toHaveAttribute("aria-pressed", "true");
    await page.keyboard.press("Escape");
    await expect(trendLine(page)).toHaveAttribute("aria-pressed", "false");
    expect((await replayState(page)).mode).toBe("active");
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("replay-bar")).toHaveCount(0);
  });

  test("picking a start never draws", async ({ bootedApp: page }) => {
    await startInMiddle(page);
    // Select bar cancels an armed tool.
    await trendLine(page).click();
    await page.getByTestId("replay-select-bar").click();
    await expect(trendLine(page)).toHaveAttribute("aria-pressed", "false");
    // A tool armed while picking does not get the pick click as a point.
    await trendLine(page).click();
    const box = (await page.locator(".chart-container").boundingBox())!;
    await page.mouse.move(box.x + box.width * 0.4, box.y + box.height * 0.3, { steps: 5 });
    await page.mouse.click(box.x + box.width * 0.4, box.y + box.height * 0.3);
    await expect(page.getByTestId("replay-bar")).toHaveAttribute("data-mode", "active");
    await page.waitForTimeout(700);
    await page.mouse.move(box.x + box.width * 0.2, box.y + box.height * 0.5, { steps: 5 });
    await page.waitForTimeout(300);
    expect(await drawings(page)).toBe(0);
  });

  test("the Data Window shows the replay bar, never a later one", async ({ bootedApp: page }) => {
    await openSidebarTab(page, "Data Window");
    const dwClose = async () => Number(await page.getByTestId("dw-c").textContent());
    const liveTime = await page.getByTestId("dw-time").textContent();
    await startInMiddle(page);
    // At rest (mouse off the chart) it shows the last bar of the frame.
    await page.mouse.move(5, 5);
    await expect(page.getByTestId("dw-time")).not.toHaveText(liveTime!);
    const frameClose = () =>
      page.evaluate(
        () =>
          (window as unknown as { __chartgenTest: Hook }).__chartgenTest.shownSeries()!.bars.at(-1)! as unknown as {
            c: number;
          },
      );
    await expect.poll(async () => Math.abs((await dwClose()) - (await frameClose()).c)).toBeLessThan(0.01);
    // A step moves it on by exactly that bar.
    await page.keyboard.press("Shift+ArrowRight");
    await expect.poll(async () => Math.abs((await dwClose()) - (await frameClose()).c)).toBeLessThan(0.01);
  });

  test("missing history shows an empty frame and the failure", async ({ bootedApp: page }) => {
    await startInMiddle(page);
    // 500 1m bars (8 h) end after the cursor (the middle of the 1h view, days
    // back), and history pages fail.
    await page.route("**/mcp", (route) =>
      toolCall(route.request())?.args.end_time !== undefined
        ? route.fulfill({ status: 500, body: "history down" })
        : route.continue(),
    );
    await page.getByRole("button", { name: "1m", exact: true }).click();
    await expect(page.getByTestId("replay-notice")).toHaveText(
      "Could not load the history before the replay start",
      { timeout: 15_000 },
    );
    expect(await shownTimes(page)).toEqual([]);
    expect((await replayState(page)).cursorTime).toBeNull();
  });

  test("Esc exits and paging older history keeps the cursor", async ({ bootedApp: page }) => {
    const picked = await startInMiddle(page);
    const before = await liveTimes(page);
    // Drag towards older bars until a page lands.
    const box = (await page.locator(".chart-container").boundingBox())!;
    const y = box.y + box.height * 0.3;
    for (let i = 0; i < 40 && (await liveTimes(page)).length === before.length; i++) {
      await page.mouse.move(box.x + 80, y);
      await page.mouse.down();
      await page.mouse.move(box.x + box.width - 120, y, { steps: 8 });
      await page.mouse.up();
      await page.waitForTimeout(100);
    }
    await expect.poll(async () => (await liveTimes(page)).length).toBe(before.length + 500);
    // Dragging is not picking: the cursor stayed, the page is in front of it.
    expect((await replayState(page)).cursorTime).toBe(picked / 1000);
    await expect.poll(async () => (await shownTimes(page)).at(-1)).toBe(picked);
    const shown = await shownTimes(page);
    expect(shown[0]).toBe((await liveTimes(page))[0]! * 1000);
    expect(shown.length).toBe(before.indexOf(picked / 1000) + 1 + 500);
    // A step still adds one bar after the prepend.
    await page.keyboard.press("Shift+ArrowRight");
    await expect.poll(() => shownCount(page)).toBe(shown.length + 1);

    await page.keyboard.press("Escape");
    await expect(page.getByTestId("replay-bar")).toHaveCount(0);
    expect((await replayState(page)).mode).toBe("off");
    await expect.poll(async () => (await shownTimes(page)).length).toBe((await liveTimes(page)).length);
  });

  test("drawings work and a timeframe change keeps the cursor time", async ({
    bootedApp: page,
  }) => {
    const picked = await startInMiddle(page);
    const box = (await page.locator(".chart-container").boundingBox())!;
    // A trend line across bars left of the cursor. klinecharts takes a
    // click soon after the start pick for a double click: wait it out.
    await page.waitForTimeout(700);
    await trendLine(page).click();
    for (const [fx, fy] of [
      [0.2, 0.3],
      [0.4, 0.4],
    ] as const) {
      await page.mouse.move(box.x + box.width * fx, box.y + box.height * fy, { steps: 5 });
      await page.mouse.down();
      await page.mouse.up();
      await page.waitForTimeout(700);
    }
    await expect.poll(() => drawings(page)).toBe(1);
    // Drawing clicks are not start picks.
    expect((await replayState(page)).cursorTime).toBe(picked / 1000);
    await page.keyboard.press("Shift+ArrowRight");
    expect(await drawings(page)).toBe(1);
    const cursor = (await replayState(page)).cursorTime!;

    // 15m bars: the frame ends with the last 15m bar closed by the close of
    // the cursor's 1h bar.
    await page.getByRole("button", { name: "15m", exact: true }).click();
    await expect
      .poll(async () => (await shownTimes(page)).at(-1), { timeout: 15_000 })
      .toBe(cursor * 1000 + 45 * 60_000);
    expect(await replayState(page)).toMatchObject({ mode: "active", cursorTime: cursor + 45 * 60 });
    const times = await shownTimes(page);
    expect(times.at(-1)! - times.at(-2)!).toBe(15 * 60_000);

    await page.getByTestId("replay-close").click();
    await expect(page.getByTestId("replay-bar")).toHaveCount(0);
    expect(await drawings(page)).toBe(1);
  });

  test("Select date starts at the bar of that date", async ({ bootedApp: page }) => {
    await page.getByTestId("toolbar-replay").click();
    const times = await liveTimes(page);
    // A bar 100 bars back, as a UTC date and time.
    const target = times.at(-100)!;
    await page.getByTestId("tz-select").selectOption("utc");
    await page.getByTestId("replay-select-date").click();
    const d = new Date(target * 1000);
    const pad = (n: number) => String(n).padStart(2, "0");
    await page.getByLabel("Date", { exact: true }).fill(
      `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())}`,
    );
    await page.getByLabel("Time (optional)").fill(`${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}`);
    await page.getByRole("button", { name: "OK" }).click();
    await expect(page.getByTestId("replay-bar")).toHaveAttribute("data-mode", "active");
    await expect.poll(async () => (await shownTimes(page)).at(-1)).toBe(target * 1000);
  });

  test("repainting indicators are recomputed on the server up to the cursor", async ({
    page,
  }) => {
    await page.addInitScript(() =>
      localStorage.setItem("chartgen.indicators.active", JSON.stringify(["rsi", "vpvr"])),
    );
    await gotoApp(page);
    const calls = recordCalls(page);

    const picked = await startInMiddle(page);
    // One recompute of just the profile, ending right after the cursor bar.
    await expect
      .poll(() => calls.filter((c) => c.args.end_time !== undefined).length)
      .toBeGreaterThan(0);
    const rc = calls.find((c) => c.args.end_time !== undefined)!;
    expect(rc.args.indicators).toEqual(["vpvr"]);
    expect(rc.args.end_time).toBe(picked + 3_600_000);
    // The profile comes from it: hbars present once it landed.
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as unknown as { __chartgenTest: Hook }).__chartgenTest.shownSeries()
              ?.indicators.vpvr?.hbars?.length ?? 0,
        ),
      )
      .toBeGreaterThan(0);
    // ...and drawn: the profile's indicator sits on the candle pane.
    await expect
      .poll(() =>
        page.evaluate(() => {
          const c = (
            window as unknown as {
              __chartgenTest: { chart: () => { getIndicatorByPaneId: (p: string, n: string) => unknown } };
            }
          ).__chartgenTest.chart();
          return c?.getIndicatorByPaneId("candle_pane", "chartgen_vpvr") != null;
        }),
      )
      .toBe(true);
  });

  test("pattern indicators show what the cursor knew, not the live chart (ADR-0046)", async ({
    page,
  }) => {
    await page.addInitScript(() =>
      localStorage.setItem(
        "chartgen.indicators.active",
        JSON.stringify([{ name: "auto_fib", params: { depth: 5, deviation: 1 } }, "chart_patterns"]),
      ),
    );
    await gotoApp(page);
    const calls = recordCalls(page);
    await startInMiddle(page);
    await expect
      .poll(() => calls.filter((c) => c.args.end_time !== undefined).length)
      .toBeGreaterThan(0);
    // The fib plots come from a recompute that ends at the cursor: every level
    // starts on a bar of the frame, and its plot holds only that level.
    await expect
      .poll(() =>
        page.evaluate(() => {
          const s = (
            window as unknown as {
              __chartgenTest: {
                shownSeries: () => {
                  bars: unknown[];
                  indicators: Record<
                    string,
                    {
                      levels?: Array<{ line: number; x: number; y: number }>;
                      lines?: Array<{ values: Array<number | null> }>;
                    }
                  >;
                } | null;
              };
            }
          ).__chartgenTest.shownSeries();
          const fib = s?.indicators.auto_fib;
          if (!fib?.levels?.length) return "no levels";
          for (const l of fib.levels) {
            if (l.x >= s!.bars.length) return "a level from the future";
            const v = fib.lines![l.line]!.values;
            if (v.length !== s!.bars.length) return "a plot past the cursor";
            if (v.some((y, i) => (i < l.x ? y !== null : y !== l.y))) return "another swing's values";
          }
          return "ok";
        }),
      )
      .toBe("ok");
  });
});
