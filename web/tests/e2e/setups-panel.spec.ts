// Setups panel (ADR-0071): the signed-in owner's setups, read with the app's own token.
// Setups are made here through the MCP endpoint with that token, as an agent signed in
// as the same owner would make them. One made from another browser, which is another
// OAuth client and so another owner, is not shown. The hint on a win below 0 R and the
// totals are checked on answers the test puts in place of the server's.

import { readFileSync } from "node:fs";

import type { Page } from "@playwright/test";

import { test, expect, gotoApp, openSidebarTab, reloadApp } from "./fixtures/app";

/** One of the answers the server's tests write (src/setup_service/tests.rs). */
const fixture = (name: string): Record<string, unknown> =>
  JSON.parse(
    readFileSync(new URL(`../../../tests/fixtures/${name}`, import.meta.url), "utf8"),
  ) as Record<string, unknown>;
const answers = fixture("setup_responses.json");
const record = fixture("track_record_responses.json");

type Json = Record<string, unknown>;

/** The bearer token the app last sent to /mcp. */
function tokenOf(page: Page): () => string {
  let token = "";
  page.on("request", (r) => {
    const auth = r.headers()["authorization"];
    if (auth && new URL(r.url()).pathname === "/mcp") token = auth;
  });
  return () => token;
}

/** Call a tool as the app's owner, as an agent with the same sign-in would. */
async function tool(page: Page, token: string, name: string, args: Json): Promise<Json> {
  const res = await page.request.post("/mcp", {
    headers: {
      authorization: token,
      accept: "application/json, text/event-stream",
      "content-type": "application/json",
    },
    data: { jsonrpc: "2.0", id: 1, method: "tools/call", params: { name, arguments: args } },
  });
  expect(res.ok(), `${name} answered HTTP ${res.status()}`).toBe(true);
  const rpc = (await res.json()) as { result: { isError?: boolean; content: { text: string }[] } };
  const text = rpc.result.content[0]!.text;
  expect(rpc.result.isError ?? false, text).toBe(false);
  return JSON.parse(text) as Json;
}

async function lastPrice(page: Page, token: string, symbol: string): Promise<number> {
  const q = (await tool(page, token, "get_quotes", { symbols: [symbol] })) as {
    quotes: { last: number }[];
  };
  return q.quotes[0]!.last;
}

/** A long on `symbol` 1h with a drawing and a tag, below the price. */
async function makeTake(page: Page, token: string, symbol: string): Promise<Json> {
  const p = await lastPrice(page, token, symbol);
  return tool(page, token, "create_setup", {
    symbol,
    interval: "1h",
    direction: "long",
    thesis: "a retest of the range low",
    entry: { low: +(p * 0.97).toFixed(2), high: +(p * 0.98).toFixed(2) },
    invalidation: +(p * 0.95).toFixed(2),
    targets: [+(p * 1.05).toFixed(2)],
    expires_at: Date.now() + 86_400_000,
    tags: ["strategy:panel-test"],
    drawings: [{ tool: "horizontalStraightLine", points: [{ time: Date.now() - 3 * 3_600_000, snap: "low" }] }],
  });
}

const rows = (page: Page) => page.locator("[data-setup-id]");
const row = (page: Page, id: unknown) => page.locator(`[data-setup-id="${String(id)}"]`);

/** The chart's own header names the symbol and the interval it shows. */
const showing = (page: Page, symbol: string, interval: string) =>
  expect(page.locator(".pane-header").first()).toContainText(new RegExp(`${symbol}\\s*·\\s*${interval}`));

const storedDrawings = (page: Page, symbol: string) =>
  page.evaluate(
    (key) => JSON.parse(localStorage.getItem(key) ?? "[]") as { name: string; points: { timestamp: number; value: number }[] }[],
    `chartgen.drawings.${symbol}`,
  );

test.describe("setups panel", () => {
  test("lists only the owner's setups, open first, and a click opens each on its chart", async ({
    page,
    browser,
  }) => {
    const token = tokenOf(page);
    await gotoApp(page);
    await expect.poll(token).not.toBe("");

    // Another browser is another OAuth client: another owner.
    const other = await browser.newContext();
    const stranger = await other.newPage();
    const strangerToken = tokenOf(stranger);
    await gotoApp(stranger);
    await expect.poll(strangerToken).not.toBe("");
    expect(strangerToken()).not.toBe(token());
    const theirs = await makeTake(stranger, strangerToken(), "SOLUSDT");
    await other.close();

    const take = await makeTake(page, token(), "ETHUSDT");
    const skip = await tool(page, token(), "create_setup", {
      symbol: "BTCUSDT",
      interval: "1h",
      direction: "short",
      decision: "skip",
      thesis: "no edge at the open",
    });

    await openSidebarTab(page, "Setups");
    await expect(rows(page)).toHaveCount(2);
    // Reloaded with the tab remembered, the list is read as soon as the app has signed in,
    // not at the next minute's refresh.
    await reloadApp(page);
    await expect(rows(page)).toHaveCount(2, { timeout: 15_000 });
    await expect(row(page, theirs.id)).toHaveCount(0);
    // The newest first.
    await expect(rows(page).first()).toHaveAttribute("data-setup-id", String(skip.id));
    await expect(row(page, skip.id).locator("[data-role=skip]")).toBeVisible();
    await expect(row(page, take.id).locator("[data-role=tags]")).toHaveText("strategy:panel-test");
    await expect(row(page, take.id).locator("[data-role=status]")).toHaveText("watching");
    // Nothing closed yet: the totals count none, and say what is open.
    await expect(page.locator("[data-role=totals-take] [data-role=count]")).toHaveText("0");
    await expect(page.locator("[data-role=totals-take] [data-role=open]")).toHaveText("1");
    await expect(page.locator("[data-role=totals-skip] [data-role=open]")).toHaveText("1");
    // Nothing is closed, so the closed filter is empty.
    await page.getByRole("group", { name: "Show setups" }).getByRole("button", { name: /^Closed/ }).click();
    await expect(rows(page)).toHaveCount(0);
    await expect(page.locator("[data-role=setups-empty]")).toHaveText("No closed setups yet.");
    await page.getByRole("group", { name: "Show setups" }).getByRole("button", { name: /^Open/ }).click();

    // The take opens on ETHUSDT 1h with its stored line.
    await row(page, take.id).click();
    await showing(page, "ETHUSDT", "1h");
    const line = (take.drawings as { points: { timestamp: number; price: number }[] }[])[0]!.points[0]!;
    await expect
      .poll(() => storedDrawings(page, "ETHUSDT"))
      .toEqual([
        expect.objectContaining({
          name: "horizontalStraightLine",
          points: [{ timestamp: line.timestamp, value: line.price }],
        }),
      ]);
    // Opened again, it is not drawn twice.
    await row(page, take.id).click();
    await page.waitForTimeout(300);
    expect(await storedDrawings(page, "ETHUSDT")).toHaveLength(1);

    // The skip opens on BTCUSDT 1h with the server's bracket as a short position.
    await row(page, skip.id).click();
    await showing(page, "BTCUSDT", "1h");
    await expect
      .poll(() => storedDrawings(page, "BTCUSDT"))
      .toEqual([
        expect.objectContaining({
          name: "cgShort",
          points: [
            { timestamp: skip.created_at, value: (skip.entry as { low: number }).low },
            { timestamp: skip.expires_at, value: (skip.targets as number[])[0] },
            { timestamp: skip.expires_at, value: skip.invalidation },
          ],
        }),
      ]);
    // And it is on the chart: the Object Tree lists it.
    await openSidebarTab(page, "Object Tree");
    await expect(page.getByRole("region", { name: /Object Tree/ }).getByText("Short position")).toBeVisible();
  });

  test("says when a win is below 0 R, filters, and shows the track record's totals", async ({
    page,
  }) => {
    // The server's own answers, with the skip's win made one below 0 R, as a fill past
    // the target leaves it.
    const list = structuredClone(answers.list_setups) as { setups: Json[] };
    const lossWin = list.setups.find((s) => s.status === "won")!;
    lossWin.result_r = -0.12;
    const takes = structuredClone(record.get_track_record) as Json;
    // get_setup of the skip: unfilled the first time it is opened, filled and closed after.
    const before = { ...(structuredClone(answers.create_setup_skip) as Json), id: lossWin.id };
    const reads = [before, lossWin];
    await page.route("**/mcp", async (route) => {
      const body = route.request().postDataJSON() as { id?: unknown; params?: { name?: string; arguments?: Json } } | null;
      const name = body?.params?.name;
      const payload =
        name === "list_setups"
          ? list
          : name === "get_track_record"
            ? takes
            : name === "get_setup"
              ? (reads.shift() ?? lossWin)
              : null;
      if (payload === null) return route.fallback();
      await route.fulfill({
        json: { jsonrpc: "2.0", id: body?.id, result: { content: [{ type: "text", text: JSON.stringify(payload) }] } },
      });
    });
    await gotoApp(page);
    await openSidebarTab(page, "Setups");
    const group = page.getByRole("group", { name: "Show setups" });
    await group.getByRole("button", { name: /^All/ }).click();
    await expect(rows(page)).toHaveCount(3);

    const hinted = row(page, lossWin.id).locator("[data-role=won-at-a-loss]");
    await expect(hinted).toBeVisible();
    await expect(hinted).toHaveAttribute("title", /below 0 R after costs/);
    await expect(row(page, lossWin.id).locator("[data-role=status]")).toHaveText("won");
    await expect(row(page, lossWin.id).locator("[data-role=result]")).toHaveText("−0.12R");
    // Only that row says so.
    await expect(page.locator("[data-role=won-at-a-loss]")).toHaveCount(1);

    // The totals are the tool's, not added up here.
    const total = (takes as { total: { count: number; total_r: number } }).total;
    await expect(page.locator("[data-role=totals-take] [data-role=count]")).toHaveText(String(total.count));
    const r = total.total_r;
    await expect(page.locator("[data-role=totals-take] [data-role=total-r]")).toHaveText(
      `${r < 0 ? "−" : r > 0 ? "+" : ""}${Math.abs(r).toFixed(2)}R`,
    );

    // Opened before and after it moved on, the skip is drawn once, as it is now.
    await row(page, lossWin.id).click();
    await expect.poll(async () => (await storedDrawings(page, "ETHUSDT")).length).toBe(1);
    await row(page, lossWin.id).click();
    const fill = lossWin.fill as { bar_time: number; price: number };
    await expect
      .poll(() => storedDrawings(page, "ETHUSDT"))
      .toEqual([
        expect.objectContaining({
          name: "cgShort",
          points: [
            expect.objectContaining({ timestamp: fill.bar_time, value: fill.price }),
            expect.anything(),
            expect.anything(),
          ],
        }),
      ]);

    await group.getByRole("button", { name: /^Open/ }).click();
    await expect(rows(page)).toHaveCount(0);
    await group.getByRole("button", { name: /^Closed/ }).click();
    await expect(rows(page)).toHaveCount(3);
  });
});
