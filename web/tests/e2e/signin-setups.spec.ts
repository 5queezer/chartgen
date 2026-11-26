// The Setups page at /setups (ADR-0091) on a server with sign-in, against the fake
// providers. Signed out, /setups is the sign-in like any address, and nothing is read.
// Signed in, an account sees its own setups and not another account's.

import type { Page } from "@playwright/test";

import { test, expect } from "./fixtures/app";

/** The bearer token the app last sent to /mcp. */
function tokenOf(page: Page): () => string {
  let token = "";
  page.on("request", (r) => {
    const auth = r.headers()["authorization"];
    if (auth && new URL(r.url()).pathname === "/mcp") token = auth;
  });
  return () => token;
}

/** Make a skip as the signed-in account, as its agent would. */
async function makeSkip(page: Page, token: string, symbol: string): Promise<string> {
  const res = await page.request.post("/mcp", {
    headers: { authorization: token, accept: "application/json, text/event-stream", "content-type": "application/json" },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name: "create_setup", arguments: { symbol, interval: "1h", direction: "long", decision: "skip", thesis: "made-up" } },
    },
  });
  const rpc = (await res.json()) as { result: { isError?: boolean; content: { text: string }[] } };
  expect(rpc.result.isError ?? false, rpc.result.content[0]!.text).toBe(false);
  return (JSON.parse(rpc.result.content[0]!.text) as { id: string }).id;
}

/** Open `path` signed out and sign in through the fake provider. */
async function signInAt(page: Page, path: string, provider: string, user: string): Promise<void> {
  await page.goto(path);
  await expect(page.getByRole("heading", { name: "Sign in to chartgen" })).toBeVisible();
  await page.getByRole("link", { name: `Continue with ${provider}` }).click();
  await page.getByRole("link", { name: `Sign in as ${user}` }).click();
}

test.describe("the Setups page with sign-in", () => {
  test("signed out, /setups is the sign-in like any address, and nothing is read", async ({ page }) => {
    const asked: string[] = [];
    page.on("request", (r) => {
      if (new URL(r.url()).pathname === "/mcp") asked.push(r.url());
    });
    await page.goto("/setups");
    await expect(page.getByRole("heading", { name: "Sign in to chartgen" })).toBeVisible();
    await expect(page.getByTestId("setups-page")).toHaveCount(0);
    expect(asked).toEqual([]);
    // Without a token the tools answer nobody.
    const res = await page.request.post("/mcp", {
      headers: { accept: "application/json, text/event-stream", "content-type": "application/json" },
      data: { jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "list_setups", arguments: {} } },
    });
    expect(res.status()).toBe(401);
  });

  test("an account sees its own setups at /setups, not another account's", async ({ page, browser }) => {
    // Another account, in another browser, makes a setup.
    const other = await browser.newContext();
    const ann = await other.newPage();
    const annToken = tokenOf(ann);
    await signInAt(ann, "/", "Google", "Ann Example");
    await expect.poll(annToken, { timeout: 30_000 }).not.toBe("");
    const theirs = await makeSkip(ann, annToken(), "SOLUSDT");
    await other.close();

    // This account signs in at /setups: the sign-in comes back to the page.
    const token = tokenOf(page);
    await signInAt(page, "/setups", "GitHub", "octo");
    await expect(page.getByTestId("setups-page")).toBeVisible({ timeout: 30_000 });
    await expect(page).toHaveURL(/\/setups$/);
    await expect.poll(token).not.toBe("");
    const mine = await makeSkip(page, token(), "BTCUSDT");
    await page.getByRole("button", { name: "Refresh" }).click();
    await expect(page.locator(`[data-setup-id="${mine}"]`)).toBeVisible();
    await expect(page.locator(`[data-setup-id="${theirs}"]`)).toHaveCount(0);
    // A reload stays on the page, through the sign-in's session.
    await page.reload();
    await expect(page.getByTestId("setups-page")).toBeVisible({ timeout: 30_000 });
    await expect(page.locator(`[data-setup-id="${mine}"]`)).toBeVisible();
    await expect(page.locator(`[data-setup-id="${theirs}"]`)).toHaveCount(0);
  });
});
