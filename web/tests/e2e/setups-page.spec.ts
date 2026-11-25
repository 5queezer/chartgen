// The Setups page at /setups (ADR-0091), on the server that keeps setups. Setups are
// made through the MCP endpoint with the app's own token, as an agent of the same owner
// would make them; one made from another browser (another OAuth client, so another
// owner) is not shown. Sorting, filters, figures and groupings are checked on answers
// the test puts in place of the server's, made-up setups on the server's own shape.

import { readFileSync } from "node:fs";

import type { Page, Request } from "@playwright/test";

import { test, expect, bootApp, gotoApp, openSidebarTab } from "./fixtures/app";

type Json = Record<string, unknown>;

const fixture = (name: string): Json =>
  JSON.parse(readFileSync(new URL(`../../../tests/fixtures/${name}`, import.meta.url), "utf8")) as Json;
const answers = fixture("setup_responses.json");
const record = fixture("track_record_responses.json");

/** The bearer token the app last sent to /mcp. */
function tokenOf(page: Page): () => string {
  let token = "";
  page.on("request", (r) => {
    const auth = r.headers()["authorization"];
    if (auth && new URL(r.url()).pathname === "/mcp") token = auth;
  });
  return () => token;
}

/** The arguments of every call of `name` the page makes. */
function callsOf(page: Page, name: string): Json[] {
  const calls: Json[] = [];
  page.on("request", (r: Request) => {
    if (r.method() !== "POST" || new URL(r.url()).pathname !== "/mcp") return;
    const body = r.postDataJSON() as { params?: { name?: string; arguments?: Json } } | null;
    if (body?.params?.name === name) calls.push(body.params.arguments ?? {});
  });
  return calls;
}

async function tool(page: Page, token: string, name: string, args: Json): Promise<Json> {
  const res = await page.request.post("/mcp", {
    headers: { authorization: token, accept: "application/json, text/event-stream", "content-type": "application/json" },
    data: { jsonrpc: "2.0", id: 1, method: "tools/call", params: { name, arguments: args } },
  });
  expect(res.ok(), `${name} answered HTTP ${res.status()}`).toBe(true);
  const rpc = (await res.json()) as { result: { isError?: boolean; content: { text: string }[] } };
  const text = rpc.result.content[0]!.text;
  expect(rpc.result.isError ?? false, text).toBe(false);
  return JSON.parse(text) as Json;
}

async function makeTake(page: Page, token: string, symbol: string): Promise<Json> {
  const q = (await tool(page, token, "get_quotes", { symbols: [symbol] })) as { quotes: { last: number }[] };
  const p = q.quotes[0]!.last;
  return tool(page, token, "create_setup", {
    symbol,
    interval: "1h",
    direction: "long",
    thesis: "a retest of the range low",
    entry: { low: +(p * 0.97).toFixed(2), high: +(p * 0.98).toFixed(2) },
    invalidation: +(p * 0.95).toFixed(2),
    targets: [+(p * 1.05).toFixed(2)],
    expires_at: Date.now() + 86_400_000,
    tags: ["strategy:page-test"],
    drawings: [{ tool: "horizontalStraightLine", points: [{ time: Date.now() - 3 * 3_600_000, snap: "low" }] }],
  });
}

/** Open /setups: the app signs in, comes back to `/`, and shows the page. */
async function gotoSetupsPage(page: Page): Promise<void> {
  await page.goto("/setups");
  await expect(page.getByTestId("setups-page")).toBeVisible({ timeout: 30_000 });
  await expect(page).toHaveURL(/\/setups$/);
}

const table = (page: Page, which: "open" | "closed") => page.getByTestId(`setups-${which}`);
const ids = (page: Page, which: "open" | "closed") =>
  table(page, which).locator("[data-setup-id]").evaluateAll((rows) => rows.map((r) => r.getAttribute("data-setup-id")));

const storedDrawings = (page: Page, symbol: string) =>
  page.evaluate(
    (key) => JSON.parse(localStorage.getItem(key) ?? "[]") as { name: string; points: { timestamp: number; value: number }[] }[],
    `chartgen.drawings.${symbol}`,
  );

test.describe("the Setups page", () => {
  test("shows the owner's setups only, and a setup opens in the drawer and on its chart", async ({ page, browser }) => {
    const token = tokenOf(page);
    await gotoApp(page);
    await expect.poll(token).not.toBe("");

    // Another browser is another OAuth client: another owner.
    const other = await browser.newContext();
    const stranger = await other.newPage();
    const strangerToken = tokenOf(stranger);
    await gotoApp(stranger);
    await expect.poll(strangerToken).not.toBe("");
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

    const lists = callsOf(page, "list_setups");
    await gotoSetupsPage(page);
    await expect(table(page, "open").locator("[data-setup-id]")).toHaveCount(2);
    await expect(page.locator(`[data-setup-id="${String(theirs.id)}"]`)).toHaveCount(0);
    // Newest first.
    expect(await ids(page, "open")).toEqual([skip.id, take.id]);
    // Only the fields the tables show: the rest comes with get_setup for one setup.
    expect(lists.at(-1)).toEqual(expect.objectContaining({ limit: 500, fields: expect.arrayContaining(["status", "result_r"]) }));
    expect(lists.at(-1)!.fields).not.toContain("thesis");
    // The server's open risk: one take, long.
    await expect(page.locator("[data-role=risk-long]")).toContainText("1R");
    await expect(page.getByTestId("setups-risk")).toContainText("ETHUSDT");

    // The drawer reads the whole setup.
    await page.locator(`[data-setup-id="${String(take.id)}"]`).click();
    const drawer = page.getByTestId("setup-detail");
    await expect(drawer.locator("[data-role=thesis]")).toHaveText("a retest of the range low");
    await expect(drawer).toContainText("strategy:page-test");

    // Open chart: the app at `/`, on ETHUSDT 1h with the setup's line.
    await bootApp(page, () => drawer.getByRole("link", { name: "Open chart" }).click());
    await expect(page).toHaveURL((url) => url.pathname === "/" && !url.searchParams.has("setup"));
    await expect(page.locator(".pane-header").first()).toContainText(/ETHUSDT\s*·\s*1h/);
    const line = (take.drawings as { points: { timestamp: number; price: number }[] }[])[0]!.points[0]!;
    await expect
      .poll(() => storedDrawings(page, "ETHUSDT"))
      .toEqual([expect.objectContaining({ name: "horizontalStraightLine", points: [{ timestamp: line.timestamp, value: line.price }] })]);

    // The tab beside the chart lists this symbol's setups, and links to the page.
    await openSidebarTab(page, "Setups");
    await expect(page.locator("[role=region][aria-label=Setups] [data-setup-id]")).toHaveCount(1);
    await expect(page.locator(`[role=region][aria-label=Setups] [data-setup-id="${String(take.id)}"]`)).toBeVisible();
    await page.locator("[data-role=setups-page-link]").click();
    await expect(page.getByTestId("setups-page")).toBeVisible({ timeout: 30_000 });
  });

  test("sorts, filters, and shows the track record's figures and groupings as the tool answers", async ({ page }) => {
    // The server's own setups, and two made up on their shape: a won take and an open one.
    const base = structuredClone(answers.list_setups) as { setups: Json[] };
    const [cancelled, skipWon, stopped] = base.setups as [Json, Json, Json];
    const won = {
      ...structuredClone(stopped),
      id: "setup-000011",
      symbol: "SOLUSDT",
      status: "won",
      result_r: 2.2,
      net: "gain",
      tags: ["trigger:example-b"],
      // Made at noon UTC on 2 January, after the server's own ones on the 1st.
      created_at: 1767355200000,
      exit: { price: 1, bar_time: 1767400000000 },
      scored_through: 1767400000000,
    };
    const open = {
      ...structuredClone(stopped),
      id: "setup-000012",
      status: "watching",
      result_r: null,
      net: null,
      fill: null,
      exit: null,
      tags: ["trigger:example-a"],
      created_at: 1767441600000,
    };
    const list = { ...base, total: 5, setups: [open, won, ...base.setups] };
    const skips = { ...structuredClone(record.get_track_record), total: { ...(record.get_track_record as { total: Json }).total, total_r: -0.5 } };
    const trackCalls: Json[] = [];
    await page.route("**/mcp", async (route) => {
      const body = route.request().postDataJSON() as { id?: unknown; params?: { name?: string; arguments?: Json } } | null;
      const name = body?.params?.name;
      const args = body?.params?.arguments ?? {};
      if (name === "get_track_record") trackCalls.push(args);
      const payload =
        name === "list_setups"
          ? list
          : name === "get_track_record"
            ? args.group_by === "skip_reason"
              ? record.get_track_record_by_skip_reason
              : args.group_by === "tag"
                ? record.get_track_record_by_tag
                : args.group_by
                  ? { ...(record.get_track_record_grouped as Json), group_by: args.group_by, feature: args.feature ?? null }
                  : args.decision === "skip"
                    ? skips
                    : record.get_track_record
          : null;
      if (payload === null) return route.fallback();
      await route.fulfill({
        json: { jsonrpc: "2.0", id: body?.id, result: { content: [{ type: "text", text: JSON.stringify(payload) }] } },
      });
    });
    await gotoSetupsPage(page);

    // The figures are the tool's, for takes and for skips.
    await expect(page.locator("[data-role=taken] [data-role=total-r]")).toHaveText("+0.61R");
    await expect(page.locator("[data-role=skipped] [data-role=total-r]")).toHaveText("−0.50R");

    // Open and closed apart; closed ones by when they ended, the latest first.
    expect(await ids(page, "open")).toEqual([open.id]);
    expect(await ids(page, "closed")).toEqual([won.id, stopped.id, cancelled.id, skipWon.id]);
    const result = table(page, "closed").getByRole("button", { name: /^Result/ });
    await result.click();
    expect(await ids(page, "closed")).toEqual([won.id, skipWon.id, cancelled.id, stopped.id]);
    await expect(table(page, "closed").locator("th").filter({ hasText: "Result" })).toHaveAttribute("aria-sort", "descending");
    await result.click();
    expect(await ids(page, "closed")).toEqual([stopped.id, cancelled.id, skipWon.id, won.id]);

    // Filters: a symbol, then a decision; the record is asked with the filters it takes.
    const filters = page.getByRole("group", { name: "Filters" });
    await filters.getByLabel("Symbol").selectOption("ETHUSDT");
    await expect.poll(() => ids(page, "closed")).toEqual([cancelled.id, skipWon.id]); // still by result, rising
    await expect(table(page, "open")).toHaveCount(0);
    await expect.poll(() => trackCalls.some((a) => a.symbol === "ETHUSDT" && a.decision === "take")).toBe(true);
    await filters.getByLabel("Decision").selectOption("skip");
    await expect.poll(() => ids(page, "closed")).toEqual([skipWon.id]);
    await filters.getByLabel("Tag").selectOption("trigger:example-b");
    await expect(table(page, "closed")).toHaveCount(0);
    await filters.getByRole("button", { name: "Clear" }).click();
    await expect.poll(() => ids(page, "closed")).toHaveLength(4);
    await filters.getByLabel("Made from").fill("2026-01-02");
    await expect.poll(() => ids(page, "open")).toEqual([open.id]);
    await expect.poll(() => ids(page, "closed")).toEqual([won.id]);
    await filters.getByRole("button", { name: "Clear" }).click();

    // The track record grouped: what the tool answers, per group.
    const breakdown = page.getByTestId("setups-record");
    await expect(breakdown.locator("[data-group=All] [data-role=total-r]")).toHaveText("+0.61R");
    await page.getByLabel("Group by").selectOption("skip_reason");
    await expect(breakdown.locator('[data-group="example-reason"] [data-role=total-r]')).toHaveText("+1.77R");
    await expect(breakdown.locator('[data-group="(none)"] [data-role=total-r]')).toHaveText("−1.15R");
    expect(trackCalls.at(-1)).toEqual({ group_by: "skip_reason" });
    // By tag: the families first, then one family's tags.
    await page.getByLabel("Group by").selectOption("tag");
    await expect.poll(() => trackCalls.at(-1)).toEqual({ group_by: "tag", tag_prefixes: ["trigger:", "strategy:"] });
    await page.getByLabel("Tags to group by").selectOption("trigger:…");
    await expect
      .poll(() => trackCalls.at(-1))
      .toEqual({ group_by: "tag", tag_prefixes: ["trigger:example-a", "trigger:example-b"] });
    await expect(breakdown.locator('[data-group="trigger:example-b"]')).toBeVisible();
  });
});
