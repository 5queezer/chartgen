// Sign-in through a provider in a real browser (ADR-0044), against the fake
// GitHub and Google of fixtures/fake-provider.mjs. Its project runs a server of
// its own with both providers on. The rest of the suite runs without sign-in.

import type { BrowserContext, Page, Request } from "@playwright/test";

import { test, expect, expectChartRendered, gotoApp, reloadApp } from "./fixtures/app";

const CHARTGEN_PORT = Number(process.env.CHARTGEN_PORT ?? 9315);
const FAKE_ORIGIN = `http://127.0.0.1:${CHARTGEN_PORT - 4_000}`;

const isProvider = (req: Request) => req.url().startsWith(FAKE_ORIGIN);
const chooser = (page: Page) => page.getByRole("heading", { name: "Sign in to chartgen" });
const userMenu = (page: Page) => page.getByTestId("user-menu");

/** From the chooser through the fake provider and back into the loaded app. */
async function signIn(page: Page, provider = "GitHub", user = "octo"): Promise<void> {
  await page.getByRole("link", { name: `Continue with ${provider}` }).click();
  await page.getByRole("link", { name: `Sign in as ${user}` }).click();
  await page.waitForURL((url) => url.pathname === "/" && !url.searchParams.has("code"));
  await expectChartRendered(page);
  await expect(userMenu(page)).toBeVisible();
}

async function bootSignedIn(page: Page): Promise<void> {
  await page.goto("/");
  await expect(chooser(page)).toBeVisible();
  await signIn(page);
}

/** Open a call the app must make, so a dead token shows. */
async function changeTimeframe(page: Page): Promise<void> {
  await page.getByRole("group", { name: "Timeframe" }).locator("[aria-pressed=false]").first().click();
}

test.describe("sign-in", () => {
  test.describe.configure({ timeout: 120_000 });

  // This project's Vite is cold, and global-setup warms the other one.
  test.beforeAll(async ({ browser }, info) => {
    info.setTimeout(240_000);
    const page = await browser.newPage({ baseURL: info.project.use.baseURL });
    page.setDefaultTimeout(180_000);
    page.setDefaultNavigationTimeout(180_000);
    await page.goto("/");
    await expect(chooser(page)).toBeVisible({ timeout: 180_000 });
    await page.close();
  });

  test("the user menu is the header's control size", async ({ page }) => {
    await bootSignedIn(page);
    const menu = (await userMenu(page).boundingBox())!;
    expect(menu.height).toBe(27);
    expect(menu.y).toBe(5);
  });

  test("the chooser leads through a provider to the app, showing who signed in", async ({ page }) => {
    await page.goto("/");
    await expect(chooser(page)).toBeVisible();
    await expect(page.getByRole("link", { name: "Continue with GitHub" })).toBeVisible();
    await expect(page.getByRole("link", { name: "Continue with Google" })).toBeVisible();

    await signIn(page);

    await expect(userMenu(page)).toHaveAttribute("aria-label", "Account: octo");
    await userMenu(page).click();
    await expect(page.getByText("Signed in with GitHub")).toBeVisible();
    await expect(page.getByTestId("user-name")).toHaveText("octo");
  });

  test("Google is a user of its own", async ({ page }) => {
    await page.goto("/");
    await signIn(page, "Google", "Ann Example");
    await userMenu(page).click();
    await expect(page.getByText("Signed in with Google")).toBeVisible();
    await expect(page.getByTestId("user-name")).toHaveText("Ann Example");
  });

  test("a reload takes no provider, the session is enough", async ({ page }) => {
    await bootSignedIn(page);
    const visits: string[] = [];
    page.on("request", (req) => {
      if (isProvider(req)) visits.push(req.url());
    });

    await reloadApp(page);

    await expect(userMenu(page)).toBeVisible();
    expect(visits).toEqual([]);
  });

  test("signing out ends at a page of its own, and signing in takes the chooser again", async ({
    page,
    context,
  }) => {
    await bootSignedIn(page);
    await userMenu(page).click();
    await page.getByRole("menuitem", { name: "Sign out" }).click();

    await expect(page.getByRole("heading", { name: "You are signed out" })).toBeVisible();
    expect(new URL(page.url()).pathname).toBe("/oauth/signed-out");
    const session = (await context.cookies()).find((c) => c.name.includes("chartgen_session"));
    expect(session).toBeUndefined();

    await page.getByRole("link", { name: "Sign in again" }).click();
    await expect(chooser(page)).toBeVisible();
  });

  test("a sign-out takes the other tabs along, so none signs back in", async ({ context }) => {
    const first = await context.newPage();
    await bootSignedIn(first);
    const second = await context.newPage();
    await gotoApp(second);

    await first.bringToFront();
    await userMenu(first).click();
    await first.getByRole("menuitem", { name: "Sign out" }).click();

    await expect(first.getByRole("heading", { name: "You are signed out" })).toBeVisible();
    await expect(second.getByRole("heading", { name: "You are signed out" })).toBeVisible();
    const cookies = await context.cookies();
    expect(cookies.find((c) => c.name.includes("chartgen_session"))).toBeUndefined();
  });

  test("a share link survives the whole round trip", async ({ page }) => {
    await page.goto("/?symbol=ETHUSDT&interval=4h&ind=rsi");
    await expect(chooser(page)).toBeVisible();
    await signIn(page);

    await expect(page.getByRole("button", { name: /^Symbol:/ })).toContainText("ETHUSDT");
    await expect(
      page.getByRole("group", { name: "Timeframe" }).locator("[aria-pressed=true]"),
    ).toHaveText("4h");
    await expect(page.locator(".pane-header", { hasText: "RSI" }).first()).toBeVisible();
    expect(new URL(page.url()).search).toBe("");
  });

  test("a denied login offers a retry that works", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("link", { name: "Continue with GitHub" }).click();
    await page.getByRole("link", { name: "Deny" }).click();

    await expect(page.getByRole("heading", { name: "Sign-in cancelled" })).toBeVisible();
    await page.getByRole("link", { name: "Try again" }).click();

    await expect(chooser(page)).toBeVisible();
    await signIn(page);
  });

  test("a provider error offers a retry too", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("link", { name: "Continue with Google" }).click();
    await page.getByRole("link", { name: "Answer with a bad code" }).click();

    await expect(page.getByRole("heading", { name: "Sign-in failed" })).toBeVisible();
    await expect(page.getByText("Google refused this sign-in")).toBeVisible();
    await page.getByRole("link", { name: "Try again" }).click();
    await signIn(page, "Google", "Ann Example");
  });

  test("cancelling returns to the app, which waits at Sign in again", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("link", { name: "Continue with GitHub" }).click();
    await page.getByRole("link", { name: "Deny" }).click();
    await page.getByRole("link", { name: "Cancel" }).click();

    await page.waitForURL((url) => url.pathname === "/" && !url.searchParams.has("error"));
    await expect(page.getByText("sign-in failed: you cancelled it")).toBeVisible();
    await expect(userMenu(page)).toHaveCount(0);
    // It stays there: no redirect on its own.
    await page.waitForTimeout(500);
    expect(new URL(page.url()).pathname).toBe("/");

    await page.getByRole("button", { name: "Sign in again" }).click();
    await expect(chooser(page)).toBeVisible();
  });

  test("a stale token signs in again by itself while the session lives", async ({ page }) => {
    await bootSignedIn(page);
    let left = 1;
    await page.route("**/mcp", async (route) => {
      if (route.request().method() === "POST" && left > 0) {
        left--;
        await route.fulfill({ status: 401, body: "{}" });
        return;
      }
      await route.continue();
    });
    const visits: string[] = [];
    page.on("request", (req) => {
      if (isProvider(req)) visits.push(req.url());
    });
    const exchanged = page.waitForResponse(
      (res) => res.request().method() === "POST" && new URL(res.url()).pathname === "/token",
    );

    await changeTimeframe(page);
    await exchanged;
    await page.waitForURL((url) => !url.searchParams.has("code"));

    await expectChartRendered(page);
    await expect(userMenu(page)).toBeVisible();
    expect(visits).toEqual([]);
  });

  test("a stale token without a session lands on sign-in, not a dead end", async ({ page }) => {
    await bootSignedIn(page);
    // Signed out behind the app's back, so no other tab is told.
    await page.evaluate(() => fetch("/oauth/logout", { method: "POST" }));

    await changeTimeframe(page);

    await expect(chooser(page)).toBeVisible();
    await signIn(page);
  });

  test("a client the server forgot recovers, signed in and with no provider", async ({ page }) => {
    await bootSignedIn(page);
    const visits: string[] = [];
    page.on("request", (req) => {
      if (isProvider(req)) visits.push(req.url());
    });
    await page.evaluate(() => {
      localStorage.setItem("chartgen.oauth.client_id", "stale-client");
      localStorage.setItem("chartgen.oauth.client_secret", "stale-secret");
    });

    await reloadApp(page);

    await expect(userMenu(page)).toBeVisible();
    const stored = await page.evaluate(() => localStorage.getItem("chartgen.oauth.client_id"));
    expect(stored).not.toBe("stale-client");
    expect(visits).toEqual([]);
  });

  test("the app itself is not asked for consent, and a look-alike from elsewhere is", async ({
    page,
    context,
  }) => {
    // From the first sign-in through the provider, whose callback looks cross-site, to a
    // reload through a live session: the app's own /authorize, in a real browser.
    const consent: string[] = [];
    page.on("request", (req) => {
      if (new URL(req.url()).pathname === "/oauth/consent") consent.push(req.url());
    });
    await bootSignedIn(page);
    expect(consent).toEqual([]);
    await reloadApp(page);
    expect(consent).toEqual([]);

    const origin = new URL(page.url()).origin;
    const res = await context.request.post("/register", {
      data: { redirect_uris: [`${origin}/`], client_name: "chartgen-web" },
    });
    const { client_id } = (await res.json()) as { client_id: string };
    const params = new URLSearchParams({
      response_type: "code",
      client_id,
      redirect_uri: `${origin}/`,
      code_challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
      code_challenge_method: "S256",
      state: "attacker-state",
    });
    // A link on someone else's page: the browser says cross-site.
    await page.goto(`${FAKE_ORIGIN}/client/cb`);
    await page.evaluate((target) => window.location.assign(target), `${origin}/authorize?${params}`);
    await expect(page.getByRole("heading", { name: "Approve this application?" })).toBeVisible();
    expect(new URL(page.url()).searchParams.has("code")).toBe(false);

    // A link from outside any page, such as a mail: the browser says none.
    await page.goto(`${origin}/authorize?${params}`);
    await expect(page.getByRole("heading", { name: "Approve this application?" })).toBeVisible();
    expect(new URL(page.url()).searchParams.has("code")).toBe(false);
  });

  test.describe("a third-party client", () => {
    const CLIENT_REDIRECT = `${FAKE_ORIGIN}/client/cb`;

    /** A client with a hostile name, and the address its user is sent to. */
    async function thirdParty(context: BrowserContext): Promise<string> {
      const res = await context.request.post("/register", {
        data: {
          redirect_uris: [CLIENT_REDIRECT],
          client_name: "<img src=x onerror=alert(1)>Totally Claude",
        },
      });
      const { client_id } = (await res.json()) as { client_id: string };
      const params = new URLSearchParams({
        response_type: "code",
        client_id,
        redirect_uri: CLIENT_REDIRECT,
        code_challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
        code_challenge_method: "S256",
        state: "client-state",
      });
      return `/authorize?${params}`;
    }

    test("is shown with its redirect host, escaped, and approval reaches it", async ({
      page,
      context,
    }) => {
      await bootSignedIn(page);
      const address = await thirdParty(context);

      await page.goto(address);

      await expect(page.getByRole("heading", { name: "Approve this application?" })).toBeVisible();
      await expect(page.getByTestId("redirect-host")).toHaveText("127.0.0.1");
      await expect(page.getByText("<img src=x onerror=alert(1)>Totally Claude")).toBeVisible();
      await expect(page.locator("img")).toHaveCount(0);

      // A real form POST followed by a redirect off the origin: this is what a
      // form-action in the page's policy would break.
      await page.getByRole("button", { name: "Approve" }).click();
      await page.waitForURL((url) => url.href.startsWith(CLIENT_REDIRECT));
      const back = new URL(page.url());
      expect(back.searchParams.get("code")).toBeTruthy();
      expect(back.searchParams.get("state")).toBe("client-state");

      // Approved once: the next authorization goes straight through.
      await page.goto(address);
      await page.waitForURL((url) => url.href.startsWith(CLIENT_REDIRECT));
      expect(new URL(page.url()).searchParams.get("code")).toBeTruthy();
    });

    test("hears a denial, with no code", async ({ page, context }) => {
      await bootSignedIn(page);
      await page.goto(await thirdParty(context));
      await page.getByRole("button", { name: "Deny" }).click();

      await page.waitForURL((url) => url.href.startsWith(CLIENT_REDIRECT));
      const back = new URL(page.url());
      expect(back.searchParams.get("error")).toBe("access_denied");
      expect(back.searchParams.get("code")).toBeNull();
    });
  });

  test.describe("compact layout", () => {
    test.use({ viewport: { width: 360, height: 740 }, hasTouch: true, isMobile: true });

    test("the sign-in pages fit, and the user menu signs out by touch", async ({ page }) => {
      await page.goto("/");
      await expect(chooser(page)).toBeVisible();
      const fits = () =>
        page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth);
      expect(await fits()).toBe(true);

      // Not `signIn`: this narrow, the status text it waits for is hidden, and the
      // account is in the More tools menu instead of taking the symbol's room.
      await page.getByRole("link", { name: "Continue with GitHub" }).click();
      await page.getByRole("link", { name: "Sign in as octo" }).click();
      await page.waitForURL((url) => url.pathname === "/" && !url.searchParams.has("code"));
      await expect(page.locator(".pane-header").first()).toBeVisible();
      await expect(userMenu(page)).toBeHidden();
      expect(await fits()).toBe(true);
      await expect(page.getByRole("button", { name: /^Layout:/ })).toBeVisible();
      const symbol = page.getByRole("button", { name: /^Symbol:/ }).getByText("BTCUSDT");
      expect(await symbol.evaluate((e) => e.scrollWidth <= e.clientWidth)).toBe(true);

      await page.getByTestId("toolbar-more").tap();
      await expect(page.getByText("Signed in with GitHub")).toBeVisible();
      await expect(page.getByTestId("user-name")).toHaveText("octo");
      await page.getByRole("menuitem", { name: "Sign out" }).tap();
      await expect(page.getByRole("heading", { name: "You are signed out" })).toBeVisible();
    });
  });
});
