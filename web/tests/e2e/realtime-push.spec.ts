// Real-time chart push (ADR-0021, web guide "Live updates"). The suite's
// server runs with CHARTGEN_DATA_SOURCE=sample, whose push source ticks
// the forming candle every ~500 ms; tail polling would see it move every
// 2 s at best. The chart is observed through the DEV-only
// `window.__chartgenTest` hook (App.tsx) and the MCP request log.

import type { Page, Request } from "@playwright/test";

import { test, expect } from "./fixtures/app";

interface ToolCall {
  name?: string;
  arguments?: Record<string, unknown>;
}

/** The tools/call of an MCP request, or null for anything else. */
function toolCall(req: Request): ToolCall | null {
  if (req.method() !== "POST" || !req.url().endsWith("/mcp")) return null;
  try {
    const b = req.postDataJSON() as { method?: string; params?: ToolCall };
    return b.method === "tools/call" ? (b.params ?? null) : null;
  } catch {
    return null;
  }
}

const isTailPoll = (c: ToolCall | null) =>
  c?.name === "generate_chart" && c.arguments?.bars === 2;

/** Every tools/call from now on. */
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
    getDataList: () => Array<{ timestamp: number; close: number }>;
    convertToPixel: (
      point: { timestamp: number },
      finder: { paneId: string },
    ) => { x?: number };
  } | null;
  series: () => { bars: Array<{ t: number }> } | null;
}

const lastClose = (page: Page) =>
  page.evaluate(
    () =>
      (window as unknown as { __chartgenTest: Hook }).__chartgenTest
        .chart()
        ?.getDataList()
        .at(-1)?.close ?? null,
  );

const timestamps = (page: Page) =>
  page.evaluate(() =>
    ((window as unknown as { __chartgenTest: Hook }).__chartgenTest
      .chart()
      ?.getDataList() ?? []).map((d) => d.timestamp),
  );

/** Pixel x of the bar opening at `timestamp`. */
const xOf = (page: Page, timestamp: number) =>
  page.evaluate((ts) => {
    const c = (window as unknown as { __chartgenTest: Hook }).__chartgenTest.chart()!;
    return c.convertToPixel({ timestamp: ts }, { paneId: "candle_pane" }).x ?? null;
  }, timestamp);

/** Drag the chart rightwards (towards older bars). */
async function dragRight(page: Page): Promise<void> {
  const box = (await page.locator(".chart-container").boundingBox())!;
  const y = box.y + box.height * 0.3;
  await page.mouse.move(box.x + 80, y);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width - 120, y, { steps: 8 });
  await page.mouse.up();
}

test("pushed ticks move the chart faster than polling, with no tail polls", async ({
  bootedApp: page,
}) => {
  const badge = page.getByTestId("live-status");
  await expect(badge).toHaveAttribute("data-state", "push");
  await expect(badge).toHaveText("live");
  const calls = recordCalls(page);

  // Within one 2 s poll period, at least three distinct closes.
  const closes = new Set<number | null>();
  const end = Date.now() + 2_000;
  while (Date.now() < end) {
    closes.add(await lastClose(page));
    await page.waitForTimeout(100);
  }
  closes.delete(null);
  expect(closes.size).toBeGreaterThanOrEqual(3);

  // Two more poll periods: still not a single tail poll.
  await page.waitForTimeout(4_000);
  expect(calls.filter(isTailPoll)).toEqual([]);
  await expect(badge).toHaveAttribute("data-state", "push");
});

test("a chart change resubscribes with the new chart's arguments", async ({
  bootedApp: page,
}) => {
  const calls = recordCalls(page);
  await page.getByRole("button", { name: "5m", exact: true }).click();
  await expect
    .poll(() => calls.filter((c) => c.name === "unsubscribe_chart").length)
    .toBeGreaterThan(0);
  await expect
    .poll(() => calls.find((c) => c.name === "subscribe_chart")?.arguments)
    .toMatchObject({ ticker: "BTCUSDT", timeframe: "5m", warmup: 300, chart_style: "candles" });
  await expect(page.getByTestId("live-status")).toHaveAttribute("data-state", "push");
});

test("losing the push falls back to polling, and push resumes", async ({
  bootedApp: page,
}) => {
  const calls = recordCalls(page);
  // Resubscribe (timeframe change) to learn the subscription and its stream.
  const subscribed = page.waitForResponse((r) => toolCall(r.request())?.name === "subscribe_chart");
  await page.getByRole("button", { name: "5m", exact: true }).click();
  const res = await subscribed;
  const rpc = (await res.json()) as { result: { content: Array<{ text: string }> } };
  const { subscription_id } = JSON.parse(rpc.result.content[0]!.text) as {
    subscription_id: string;
  };
  const headers = await res.request().allHeaders();
  const badge = page.getByTestId("live-status");
  await expect(badge).toHaveAttribute("data-state", "push");

  // While blocked, resubscribing fails like a server without a stream.
  let blocked = true;
  await page.route("**/mcp", (route) => {
    if (blocked && toolCall(route.request())?.name === "subscribe_chart") {
      const { id } = route.request().postDataJSON() as { id: unknown };
      return route.fulfill({
        json: {
          jsonrpc: "2.0",
          id,
          result: { content: [{ type: "text", text: "no open SSE stream" }], isError: true },
        },
      });
    }
    return route.fallback();
  });

  // Kill the subscription behind the client's back: pushes stop.
  const kill = await page.request.post("/mcp", {
    headers: {
      authorization: headers["authorization"]!,
      "mcp-session-id": headers["mcp-session-id"]!,
      accept: "application/json, text/event-stream",
      "content-type": "application/json",
    },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name: "unsubscribe_chart", arguments: { subscription_id } },
    },
  });
  expect(JSON.parse((await kill.json()).result.content[0].text)).toEqual({ unsubscribed: true });

  // The watchdog notices and the feed polls again.
  await expect(badge).toHaveAttribute("data-state", "live", { timeout: 10_000 });
  await expect(badge).toHaveText("polling");
  await expect.poll(() => calls.filter(isTailPoll).length, { timeout: 5_000 }).toBeGreaterThan(0);
  const before = await lastClose(page);
  await expect.poll(() => lastClose(page), { timeout: 6_000 }).not.toBe(before);

  // Once subscribing works again, a retry brings push back and polling stops.
  blocked = false;
  await expect(badge).toHaveAttribute("data-state", "push", { timeout: 20_000 });
  const polls = calls.filter(isTailPoll).length;
  await page.waitForTimeout(4_000);
  // At most one poll that was already in flight when push came back.
  expect(calls.filter(isTailPoll).length).toBeLessThanOrEqual(polls + 1);
});

test("older pages load in place while pushes arrive", async ({ bootedApp: page }) => {
  await expect(page.getByTestId("live-status")).toHaveAttribute("data-state", "push");
  // Hold pages back so pushes land while each one is in flight.
  await page.route("**/mcp", async (route) => {
    if (toolCall(route.request())?.arguments?.end_time !== undefined) {
      await new Promise((r) => setTimeout(r, 1_200));
    }
    return route.fallback();
  });
  const status = page.getByTestId("history-status");
  const closes = new Set<number | null>();

  let pages = 0;
  for (let i = 0; i < 40 && pages < 2; i++) {
    await dragRight(page);
    if (!(await status.isVisible())) continue;
    const before = await timestamps(page);
    const oldest = before[0]!;
    const x = await xOf(page, oldest);
    // Pushes keep ticking while the page is out.
    while (await status.isVisible()) {
      closes.add(await lastClose(page));
      await page.waitForTimeout(100);
    }
    await expect.poll(async () => (await timestamps(page)).length).toBe(before.length + 500);
    // The bar that was oldest has not moved; the page filled in to its left.
    expect(await xOf(page, oldest)).toBe(x);
    pages++;
  }
  expect(pages).toBe(2);
  closes.delete(null);
  expect(closes.size).toBeGreaterThanOrEqual(3);

  // No gaps, no duplicates: 1h apart throughout, seams included.
  const ts = await timestamps(page);
  expect(ts.length).toBe(1500);
  expect(ts.every((t, i) => i === 0 || t - ts[i - 1]! === 3_600_000)).toBe(true);
  const seriesTs = await page.evaluate(() =>
    (window as unknown as { __chartgenTest: Hook }).__chartgenTest
      .series()!
      .bars.map((b) => b.t * 1000),
  );
  expect(seriesTs).toEqual(ts);
  await expect(page.getByTestId("live-status")).toHaveAttribute("data-state", "push");
});
