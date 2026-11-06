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

import type { Page } from "@playwright/test";

import { test, expect, bootApp, gotoApp } from "./fixtures/app";

const LS_CLIENT_ID = "chartgen.oauth.client_id";

let server: ChildProcess;
let distDir: string;
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
    distDir = mkdtempSync(join(tmpdir(), "chartgen-e2e-dist-"));
    await promisify(execFile)(
      "npx",
      ["vite", "build", "--outDir", distDir, "--emptyOutDir"],
      { cwd: webRoot },
    );

    const port = await freePort();
    origin = `http://localhost:${port}`;
    server = spawn("cargo", ["run", "--", "--serve", "--port", String(port)], {
      cwd: resolve(webRoot, ".."),
      env: {
        ...process.env,
        CHARTGEN_DATA_SOURCE: "sample",
        CHARTGEN_WEB_DIR: distDir,
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
    if (distDir) rmSync(distDir, { recursive: true, force: true });
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

  test("a crafted link keeps the stored client", async ({ page }) => {
    await gotoApp(page, `${origin}/`);
    const client = await storedClient(page);
    expect(client).not.toBeNull();

    await gotoApp(page, `${origin}/?error=invalid_client&state=forged`);

    expect(await storedClient(page)).toBe(client);
  });
});
