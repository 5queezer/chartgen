// A redeploy without persistent storage forgets every OAuth client, while
// the browser still holds its `client_id` (ADR-0036). The app must recover by
// itself, so this runs the real production shape: the built app served by a
// real chartgen on one origin, not the Vite dev server of the other specs,
// whose origin differs from the server's and therefore keeps the 400.

import { execFile, spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { promisify } from "node:util";

import type { BrowserContext, Page } from "@playwright/test";

import { test, expect, bootApp, expectChartRendered, gotoApp } from "./fixtures/app";

const LS_CLIENT_ID = "chartgen.oauth.client_id";

let server: ChildProcess;
let tempDir: string;
let origin: string;

function freePort(): Promise<number> {
  return new Promise((done, fail) => {
    const probe = createServer();
    probe.once("error", fail);
    probe.listen(0, () => {
      const { port } = probe.address() as { port: number };
      probe.close(() => done(port));
    });
  });
}

const storedClient = (page: Page) =>
  page.evaluate((key) => localStorage.getItem(key), LS_CLIENT_ID);

/** Give the browser a client the server does not know, for the first load only. */
async function seedStaleClient(page: Page): Promise<void> {
  await page.addInitScript((key) => {
    if (sessionStorage.getItem("e2e.seeded") !== null) return;
    sessionStorage.setItem("e2e.seeded", "1");
    localStorage.setItem(key, "stale-client");
    localStorage.setItem("chartgen.oauth.client_secret", "stale-secret");
  }, LS_CLIENT_ID);
}

/** Give every tab of `context` one shared stale client, before any tab loads. */
async function seedStaleClientForTabs(context: BrowserContext): Promise<void> {
  await context.addInitScript((key) => {
    if (localStorage.getItem("e2e.seeded") !== null) return;
    localStorage.setItem("e2e.seeded", "1");
    localStorage.setItem(key, "stale-client");
    localStorage.setItem("chartgen.oauth.client_secret", "stale-secret");
  }, LS_CLIENT_ID);
}

/** Hold this tab's `/authorize` navigation for `clientId` matching `which`. */
async function holdAuthorize(
  page: Page,
  which: (clientId: string | null) => boolean,
): Promise<{ reached: Promise<string | null>; release: () => void }> {
  let reach = (_clientId: string | null) => {};
  let release = () => {};
  const reached = new Promise<string | null>((done) => (reach = done));
  const released = new Promise<void>((done) => (release = done));
  await page.route("**/authorize?*", async (route) => {
    const clientId = new URL(route.request().url()).searchParams.get("client_id");
    if (which(clientId)) {
      reach(clientId);
      await released;
    }
    await route.continue();
  });
  return { reached, release };
}

/** Wait for this tab's sign-in to settle, and expect a loaded chart. */
async function expectSignedIn(page: Page): Promise<void> {
  const status = page.getByText(/^loaded$|^sign-in failed: /).first();
  await expect(status).toBeVisible({ timeout: 40_000 });
  expect(await status.textContent()).toBe("loaded");
  await expectChartRendered(page);
}

/** Record the status of every `/authorize` answer. */
function authorizeStatuses(page: Page): number[] {
  const statuses: number[] = [];
  page.on("response", (res) => {
    if (new URL(res.url()).pathname === "/authorize") statuses.push(res.status());
  });
  return statuses;
}

test.describe("stale OAuth client", () => {
  test.beforeAll(async ({}, testInfo) => {
    testInfo.setTimeout(300_000);
    const webRoot = resolve(testInfo.project.testDir, "../..");
    tempDir = mkdtempSync(join(tmpdir(), "chartgen-e2e-stale-"));
    const distDir = join(tempDir, "dist");
    await promisify(execFile)(
      "npx",
      ["vite", "build", "--outDir", distDir, "--emptyOutDir"],
      { cwd: webRoot },
    );

    const port = await freePort();
    origin = `http://localhost:${port}`;
    // CI hands every shard a prebuilt binary and no Rust toolchain (ADR-0057).
    const bin = process.env.CHARTGEN_BIN;
    const command = bin ? [bin] : ["cargo", "run", "--"];
    server = spawn(command[0]!, [...command.slice(1), "--serve", "--port", String(port)], {
      cwd: resolve(webRoot, ".."),
      env: {
        ...process.env,
        CHARTGEN_DATA_SOURCE: "sample",
        CHARTGEN_WEB_DIR: distDir,
        CHARTGEN_DB: join(tempDir, "chartgen.db"),
        CHARTGEN_BASE_URL: origin,
      },
      stdio: "ignore",
    });
    await expect
      .poll(
        () =>
          fetch(`${origin}/.well-known/oauth-authorization-server`).then(
            (res) => res.ok,
            () => false,
          ),
        { timeout: 240_000 },
      )
      .toBe(true);
  });

  test.afterAll(() => {
    server?.kill();
    if (tempDir) rmSync(tempDir, { recursive: true, force: true });
  });

  test("re-registers and signs in with no user action", async ({ page }) => {
    await seedStaleClient(page);
    const statuses = authorizeStatuses(page);

    await gotoApp(page, `${origin}/`);

    // Stale id bounced back, then the new id got a code.
    expect(statuses).toEqual([302, 307]);
    const client = await storedClient(page);
    expect(client).not.toBeNull();
    expect(client).not.toBe("stale-client");
    expect(new URL(page.url()).search).toBe("");
    expect(
      await page.evaluate(() => sessionStorage.getItem("chartgen.oauth.retried")),
    ).toBeNull();
  });

  test("a share link opened with a stale client lands on its chart", async ({
    page,
  }) => {
    await seedStaleClient(page);

    await gotoApp(page, `${origin}/?symbol=ETHUSDT&interval=4h&ind=rsi`);

    await expect(page.getByRole("button", { name: /^Symbol:/ })).toContainText(
      "ETHUSDT",
    );
    await expect(
      page.getByRole("group", { name: "Timeframe" }).locator("[aria-pressed=true]"),
    ).toHaveText("4h");
    await expect(page.locator(".pane-header", { hasText: "RSI" }).first()).toBeVisible();
    await expect(page.locator(".pane-header", { hasText: "Volume" })).toHaveCount(0);
    expect(await storedClient(page)).not.toBe("stale-client");
    expect(new URL(page.url()).search).toBe("");
  });

  test("a second failure stops with Sign in again, which registers", async ({
    page,
  }) => {
    await seedStaleClient(page);
    const statuses = authorizeStatuses(page);
    // The re-registration hands out another id the real server does not know.
    await page.route("**/register", (route) =>
      route.fulfill({
        status: 201,
        contentType: "application/json",
        body: JSON.stringify({ client_id: "also-stale", client_secret: "" }),
      }),
    );

    await page.goto(`${origin}/`);

    const signIn = page.getByRole("button", { name: "Sign in again" });
    await expect(signIn).toBeVisible();
    await expect(page.getByText(/^sign-in failed: /)).toBeVisible();
    expect(statuses).toEqual([302, 302]);
    expect(new URL(page.url()).search).toBe("");
    expect(await storedClient(page)).toBeNull();

    await page.unroute("**/register");
    await bootApp(page, () => signIn.click());
    expect(statuses).toEqual([302, 302, 307]);
    expect(await storedClient(page)).not.toBeNull();
    expect(await storedClient(page)).not.toBe("also-stale");
  });

  test("a tab bounced for the old client leaves another tab's sign-in alone", async ({
    context,
  }) => {
    await seedStaleClientForTabs(context);
    test.setTimeout(60_000);
    const [slow, fast] = [await context.newPage(), await context.newPage()];

    // `slow` still has its answer for the old client on the way while `fast`
    // has already registered and is about to get its code.
    const slowStale = await holdAuthorize(slow, (id) => id === "stale-client");
    const fastNew = await holdAuthorize(fast, (id) => id !== "stale-client");
    await slow.goto(`${origin}/`, { waitUntil: "commit" });
    await slowStale.reached;
    await fast.goto(`${origin}/`, { waitUntil: "commit" });
    const fastClient = await fastNew.reached;
    expect(fastClient).not.toBeNull();

    slowStale.release();
    await expectSignedIn(slow);
    fastNew.release();
    await expectSignedIn(fast);

    // Nobody replaced the client that was already good.
    expect(await storedClient(slow)).toBe(fastClient);
  });

  for (const tabs of [2, 3, 5]) {
    test(`${tabs} restored tabs all sign in by themselves`, async ({ context }) => {
      test.setTimeout(60_000);
      await seedStaleClientForTabs(context);
      const signedInAs: (string | null)[] = [];
      context.on("request", (req) => {
        if (new URL(req.url()).pathname === "/token") {
          signedInAs.push(new URLSearchParams(req.postData() ?? "").get("client_id"));
        }
      });
      const pages = await Promise.all(
        Array.from({ length: tabs }, () => context.newPage()),
      );

      await Promise.all(pages.map((p) => p.goto(`${origin}/`, { waitUntil: "commit" })));

      await Promise.all(pages.map((p) => expectSignedIn(p)));

      // Every tab is the client this browser will use after a reload.
      expect(signedInAs).toHaveLength(tabs);
      expect(new Set(signedInAs)).toEqual(new Set([await storedClient(pages[0]!)]));
    });
  }

  test("without surviving sessionStorage it stops instead of looping", async ({
    page,
  }) => {
    // An embedded webview may lose sessionStorage on navigation, so the state
    // can never match.
    await page.addInitScript(() => {
      const items = new Map<string, string>();
      Object.defineProperty(window, "sessionStorage", {
        value: {
          getItem: (k: string) => items.get(k) ?? null,
          setItem: (k: string, v: string) => items.set(k, v),
          removeItem: (k: string) => items.delete(k),
        },
      });
      localStorage.setItem("chartgen.oauth.client_id", "stale-client");
      localStorage.setItem("chartgen.oauth.client_secret", "stale-secret");
    });
    const statuses = authorizeStatuses(page);

    await page.goto(`${origin}/`);

    await expect(page.getByRole("button", { name: "Sign in again" })).toBeVisible();
    expect(statuses).toEqual([302]);
  });

  test("a crafted link keeps the stored client", async ({ page }) => {
    await gotoApp(page, `${origin}/`);
    const client = await storedClient(page);
    expect(client).not.toBeNull();

    await page.goto(`${origin}/?error=invalid_client&state=forged`);

    // A harmless stop, with nothing touched and no restart.
    const signIn = page.getByRole("button", { name: "Sign in again" });
    await expect(signIn).toBeVisible();
    expect(new URL(page.url()).search).toBe("");
    expect(await storedClient(page)).toBe(client);

    await bootApp(page, () => signIn.click());
    expect(await storedClient(page)).toBe(client);
  });
});
