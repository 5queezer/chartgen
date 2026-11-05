// Live tail polling and history paging (ADR-0009, web guide "Live
// updates"). The suite's server runs with CHARTGEN_DATA_SOURCE=sample:
// bars are a pure function of (symbol, open time), so pages join exactly,
// and the forming bar's close moves every wall-clock second. BTCUSDT is
// a 24/7 symbol, so the tail is polled every 2 s.
//
// The chart is observed through the DEV-only `window.__chartgenTest`
// hook (App.tsx) and the MCP request log.
//
// These specs cover the polling path, so server push (ADR-0021) is off:
// every subscribe_chart answers "not pushable", as for a Yahoo symbol.
// realtime-push.spec.ts covers push and its fallback to polling.

import type { Page, Request } from "@playwright/test";

import { test, expect, gotoApp, disablePush } from "./fixtures/app";

test.use({ push: false });

interface ChartArgs {
  ticker?: string;
  timeframe?: string;
  bars?: number;
  warmup?: number;
  end_time?: number;
}

/** generate_chart arguments of an MCP request, or null for anything else. */
function chartArgs(req: Request): ChartArgs | null {
  if (req.method() !== "POST" || !req.url().endsWith("/mcp")) return null;
  try {
    const b = req.postDataJSON() as {
      method?: string;
      params?: { name?: string; arguments?: ChartArgs };
    };
    return b.method === "tools/call" && b.params?.name === "generate_chart"
      ? (b.params.arguments ?? {})
      : null;
  } catch {
    return null;
  }
}

const isTail = (a: ChartArgs | null) => a?.bars === 2 && a.end_time === undefined;

/** Every generate_chart call from now on. */
function recordCalls(page: Page): ChartArgs[] {
  const calls: ChartArgs[] = [];
  page.on("request", (req) => {
    const a = chartArgs(req);
    if (a) calls.push(a);
  });
  return calls;
}

interface Hook {
  chart: () => {
    getDataList: () => Array<{ timestamp: number; close: number }>;
    convertToPixel: (
      point: { timestamp: number },
      finder: { paneId: string },
    ) => { x?: number };
  } | null;
  series: () => {
    interval: string | null;
    bars: Array<{ t: number; c: number }>;
    indicators: Record<string, { lines?: Array<{ values: Array<number | null> }> }>;
  } | null;
}

const timestamps = (page: Page) =>
  page.evaluate(() =>
    ((window as unknown as { __chartgenTest: Hook }).__chartgenTest
      .chart()
      ?.getDataList() ?? []).map((d) => d.timestamp),
  );

const lastClose = (page: Page) =>
  page.evaluate(
    () =>
      (window as unknown as { __chartgenTest: Hook }).__chartgenTest
        .chart()
        ?.getDataList()
        .at(-1)?.close ?? null,
  );

/** Pixel x of the bar opening at `timestamp`. */
const xOf = (page: Page, timestamp: number) =>
  page.evaluate((ts) => {
    const c = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!;
    return c.convertToPixel({ timestamp: ts }, { paneId: "candle_pane" }).x ?? null;
  }, timestamp);

function expectStrictlyIncreasing(ts: number[]): void {
  expect(ts.length).toBeGreaterThan(0);
  const bad = ts.findIndex((t, i) => i > 0 && t <= ts[i - 1]!);
  expect(bad, `timestamps out of order or duplicated at ${bad}`).toBe(-1);
}

/** Drag the chart rightwards (towards older bars). */
async function dragRight(page: Page): Promise<void> {
  const box = (await page.locator(".chart-container").boundingBox())!;
  const y = box.y + box.height * 0.3;
  await page.mouse.move(box.x + 80, y);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width - 120, y, { steps: 8 });
  await page.mouse.up();
}

function setVisibility(page: Page, state: "hidden" | "visible") {
  return page.evaluate((s) => {
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => s,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  }, state);
}

test.describe("live updates", () => {
  test("the forming candle ticks without a reload", async ({ bootedApp: page }) => {
    const calls = recordCalls(page);
    const before = await lastClose(page);
    expect(before).not.toBeNull();
    const countBefore = (await timestamps(page)).length;

    const lastRsi = () =>
      page.evaluate(
        () =>
          (window as unknown as { __chartgenTest: Hook }).__chartgenTest.series()
            ?.indicators.rsi?.lines?.[0]?.values.at(-1) ?? null,
      );
    const rsiBefore = await lastRsi();
    expect(rsiBefore).not.toBeNull();

    await expect
      .poll(() => lastClose(page), { timeout: 6_000, intervals: [250] })
      .not.toBe(before);
    // Indicators move with the forming candle.
    await expect.poll(lastRsi, { timeout: 6_000 }).not.toBe(rsiBefore);
    await expect(page.getByTestId("live-status")).toHaveAttribute("data-state", "live");

    // Polled as a tail with warmup, never as a full reload.
    const tails = calls.filter(isTail);
    expect(tails.length).toBeGreaterThan(0);
    expect(tails[0]).toMatchObject({ ticker: "BTCUSDT", bars: 2, warmup: 300 });
    expect(calls.filter((a) => !isTail(a))).toEqual([]);
    // Same history, updated in place (a 1h rollover is unlikely mid-test).
    expect((await timestamps(page)).length).toBeGreaterThanOrEqual(countBefore);
    expectStrictlyIncreasing(await timestamps(page));
  });

  test("a hidden tab stops polling and catches up when visible", async ({
    bootedApp: page,
  }) => {
    const calls = recordCalls(page);
    await expect.poll(() => calls.filter(isTail).length, { timeout: 6_000 }).toBeGreaterThan(0);

    await setVisibility(page, "hidden");
    await expect(page.getByTestId("live-status")).toHaveAttribute("data-state", "paused", {
      timeout: 4_000,
    });
    const paused = calls.filter(isTail).length;
    await page.waitForTimeout(6_000); // three poll periods
    expect(calls.filter(isTail).length).toBe(paused);

    await setVisibility(page, "visible");
    // Immediately, not after the next 2 s period.
    await expect
      .poll(() => calls.filter(isTail).length, { timeout: 1_000, intervals: [100] })
      .toBeGreaterThan(paused);
    await expect(page.getByTestId("live-status")).toHaveAttribute("data-state", "live");
  });
});

test.describe("history paging", () => {
  test("dragging to the left edge loads older pages in place", async ({
    bootedApp: page,
  }) => {
    const calls = recordCalls(page);
    // Hold pages back a little so the loading state and the view right
    // before the prepend can be observed.
    await page.route("**/mcp", async (route) => {
      if (chartArgs(route.request())?.end_time !== undefined) {
        await new Promise((r) => setTimeout(r, 800));
      }
      return route.continue().catch(() => {});
    });
    const initial = await timestamps(page);
    expect(initial.length).toBe(500);
    const status = page.getByTestId("history-status");

    let pages = 0;
    for (let i = 0; i < 40 && pages < 2; i++) {
      await dragRight(page);
      if (!(await status.isVisible())) continue;
      await expect(status).toHaveAttribute("data-state", "loading");
      const before = await timestamps(page);
      const oldest = before[0]!;
      const x = await xOf(page, oldest);
      await expect.poll(async () => (await timestamps(page)).length).toBe(before.length + 500);
      // Nothing on screen moved: the bar that was oldest is still at the
      // same x, and the new page fills in to its left.
      expect(await xOf(page, oldest)).toBe(x);
      await expect(status).toHaveCount(0);
      pages++;
    }
    expect(pages).toBe(2);

    const ts = await timestamps(page);
    expect(ts.length).toBe(1500);
    expectStrictlyIncreasing(ts);
    // Gapless on the sample source: 1h apart throughout, seams included.
    expect(ts.every((t, i) => i === 0 || t - ts[i - 1]! === 3_600_000)).toBe(true);

    // Each page asks for the bars before the oldest one, with warmup.
    const pageCalls = calls.filter((a) => a.end_time !== undefined);
    expect(pageCalls).toHaveLength(2);
    expect(pageCalls[0]).toMatchObject({ bars: 500, warmup: 300, end_time: initial[0] });
    expect(pageCalls[1]).toMatchObject({ end_time: ts[500] });

    // The series the chart renders from agrees with klinecharts.
    const seriesTs = await page.evaluate(() =>
      (window as unknown as { __chartgenTest: Hook }).__chartgenTest
        .series()!
        .bars.map((b) => b.t * 1000),
    );
    expect(seriesTs).toEqual(ts);

    // The EMA 200 joins the pages without a step (ADR-0032): across each
    // seam it still follows its recursion ema = prev + α (close − prev),
    // which holds exactly inside a page, to within 0.01% of its value.
    // With the 300-bar warmup alone it missed by about 0.05% here.
    const { ema200, closes } = await page.evaluate(() => {
      const s = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.series()!;
      return {
        ema200: s.indicators.ema_stack!.lines![3]!.values,
        closes: s.bars.map((b) => b.c),
      };
    });
    const alpha = 2 / 201;
    for (const s of [500, 1000]) {
      const prev = ema200[s - 1]!;
      const want = prev + alpha * (closes[s]! - prev);
      const miss = Math.abs(ema200[s]! - want) / want;
      expect(miss, `EMA 200 step at seam ${s}`).toBeLessThan(1e-4);
    }
  });

  test("a failed page shows a retry that loads it", async ({ bootedApp: page }) => {
    let fail = true;
    await page.route("**/mcp", (route) => {
      const a = chartArgs(route.request());
      if (fail && a?.end_time !== undefined) {
        return route.fulfill({ status: 500, body: "boom" });
      }
      return route.continue();
    });
    const before = (await timestamps(page)).length;
    const status = page.getByTestId("history-status");
    for (let i = 0; i < 20 && !(await status.isVisible()); i++) await dragRight(page);
    await expect(status).toHaveAttribute("data-state", "error");
    // No retry storm while the error is shown.
    await dragRight(page);
    expect((await timestamps(page)).length).toBe(before);

    fail = false;
    await status.getByRole("button", { name: "Retry" }).click();
    await expect.poll(async () => (await timestamps(page)).length).toBe(before + 500);
    await expect(status).toHaveCount(0);
    expectStrictlyIncreasing(await timestamps(page));
  });
});

test("switching timeframe mid-poll leaves no bars of the old one", async ({
  bootedApp: page,
}) => {
  // Hold 1h tail polls back so their responses land after the switch.
  await page.route("**/mcp", async (route) => {
    const a = chartArgs(route.request());
    if (isTail(a) && a?.timeframe === "1h") {
      await new Promise((r) => setTimeout(r, 1_500));
    }
    return route.continue().catch(() => {});
  });
  const inFlight = page.waitForRequest((r) => {
    const a = chartArgs(r);
    return isTail(a) && a?.timeframe === "1h";
  });
  await inFlight;
  await page.getByRole("button", { name: "5m", exact: true }).click();

  const interval = () =>
    page.evaluate(
      () => (window as unknown as { __chartgenTest: Hook }).__chartgenTest.series()?.interval,
    );
  await expect.poll(interval, { timeout: 15_000 }).toBe("5m");
  // Let the held 1h response and a few 5m polls land.
  await page.waitForTimeout(4_000);
  expect(await interval()).toBe("5m");
  const ts = await timestamps(page);
  expectStrictlyIncreasing(ts);
  expect(ts.every((t, i) => i === 0 || t - ts[i - 1]! === 300_000)).toBe(true);
  // And live on the new timeframe.
  const before = await lastClose(page);
  await expect.poll(() => lastClose(page), { timeout: 6_000 }).not.toBe(before);
});

const interval = (page: Page) =>
  page.evaluate(
    () => (window as unknown as { __chartgenTest: Hook }).__chartgenTest.series()?.interval,
  );

/** Record every state the live badge shows, into window.__liveStates. */
function recordLiveStates(page: Page) {
  return page.evaluate(() => {
    const w = window as unknown as { __liveStates: string[] };
    w.__liveStates = [];
    new MutationObserver(() => {
      const s = document.querySelector("[data-testid=live-status]")?.getAttribute("data-state");
      if (s) w.__liveStates.push(s);
    }).observe(document.body, { subtree: true, childList: true, attributes: true });
  });
}

const liveStates = (page: Page) =>
  page.evaluate(() => (window as unknown as { __liveStates: string[] }).__liveStates);

test.describe("refetch of the current key", () => {
  test("a re-pick is adopted and the chart stays live", async ({ bootedApp: page }) => {
    // Page in some history so the reload is visible as a reset to 500 bars.
    for (let i = 0; i < 20 && (await timestamps(page)).length <= 500; i++) {
      await dragRight(page);
    }
    await expect.poll(async () => (await timestamps(page)).length).toBe(1000);

    await recordLiveStates(page);
    await page.getByRole("button", { name: "1h", exact: true }).click();
    await expect.poll(async () => (await timestamps(page)).length).toBe(500);
    const before = await lastClose(page);
    await expect.poll(() => lastClose(page), { timeout: 6_000 }).not.toBe(before);
    await expect(page.getByTestId("live-status")).toHaveAttribute("data-state", "live");
    expect(await liveStates(page)).not.toContain("retrying");
  });

  test("after a refetch, switching back to a cached timeframe shows its candles", async ({
    bootedApp: page,
  }) => {
    const calls = recordCalls(page);
    await page.getByRole("button", { name: "1h", exact: true }).click();
    await expect.poll(() => calls.filter((a) => a.bars === 500).length).toBe(1);
    await page.getByRole("button", { name: "5m", exact: true }).click();
    await expect.poll(() => interval(page), { timeout: 15_000 }).toBe("5m");
    await page.getByRole("button", { name: "1h", exact: true }).click();
    await expect.poll(() => interval(page), { timeout: 15_000 }).toBe("1h");

    const ts = await timestamps(page);
    expect(ts.length).toBe(500);
    expectStrictlyIncreasing(ts);
    expect(ts.every((t, i) => i === 0 || t - ts[i - 1]! === 3_600_000)).toBe(true);
    // The cached 1h load is adopted, not refetched, and it ticks.
    expect(calls.filter((a) => a.bars === 500 && a.timeframe === "1h")).toHaveLength(1);
    const before = await lastClose(page);
    await expect.poll(() => lastClose(page), { timeout: 6_000 }).not.toBe(before);
  });
});

test("a catch-up after missed bars keeps the scrolled-back view", async ({ page }) => {
  await disablePush(page);
  // The full load arrives 10 bars short, and tail polls are held until the
  // view is scrolled back: the first poll then finds a gap and catches up
  // with a longer tail, more bars than are appended one by one.
  const MISSING = 10;
  let releaseTails!: () => void;
  const tailsHeld = new Promise<void>((r) => (releaseTails = r));
  await page.route("**/mcp", async (route) => {
    const a = chartArgs(route.request());
    if (isTail(a)) {
      await tailsHeld;
      return route.continue().catch(() => {});
    }
    if (a?.bars !== 500 || a.end_time !== undefined) return route.continue();
    const res = await route.fetch();
    const rpc = (await res.json()) as {
      result: { content: Array<{ type: string; text: string }> };
    };
    const s = JSON.parse(rpc.result.content[0]!.text);
    const n = s.bars.length - MISSING;
    s.bars = s.bars.slice(0, n);
    for (const ind of Object.values(s.indicators) as Array<Record<string, any>>) {
      for (const l of [...(ind.lines ?? []), ...(ind.histogram ?? [])]) {
        l.values = l.values.slice(0, n);
      }
      for (const f of ind.fills ?? []) {
        f.y1 = f.y1.slice(0, n);
        f.y2 = f.y2.slice(0, n);
      }
      if (ind.signals) ind.signals = ind.signals.filter((g: { x: number }) => g.x < n);
      if (ind.divlines) {
        ind.divlines = ind.divlines.filter(
          (d: { x1: number; x2: number }) => Math.max(d.x1, d.x2) < n,
        );
      }
    }
    rpc.result.content[0]!.text = JSON.stringify(s);
    return route.fulfill({ response: res, json: rpc });
  });
  const calls = recordCalls(page);

  await gotoApp(page);
  expect((await timestamps(page)).length).toBe(500 - MISSING);

  await dragRight(page);
  const anchor = (await timestamps(page))[300]!;
  const x = await xOf(page, anchor);
  releaseTails();

  await expect.poll(async () => (await timestamps(page)).length).toBe(500);
  expect(calls.filter(isTail).length).toBeGreaterThan(0);
  expect(calls.some((a) => a.end_time === undefined && a.bars! > 2 && a.bars! < 500)).toBe(true);
  expect(await xOf(page, anchor)).toBe(x);
  const ts = await timestamps(page);
  expectStrictlyIncreasing(ts);
  expect(ts.every((t, i) => i === 0 || t - ts[i - 1]! === 3_600_000)).toBe(true);
});
