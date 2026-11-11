// The operator page at /admin (ADR-0063), in a real browser against the fake
// providers. The sign-in project's server names the fake GitHub user as its one
// operator. Everyone else gets the app at /admin, as at any other address, and
// from the server what an unknown path gets.

import type { Page, Response } from "@playwright/test";

import { test, expect, expectChartRendered } from "./fixtures/app";

const operatorPage = (page: Page) => page.getByTestId("operator-page");
const overviewAnswer = (page: Page) =>
  page.waitForResponse((r: Response) => new URL(r.url()).pathname === "/admin/overview");

/** Open /admin signed out and sign in through the fake provider. */
async function openAdminAs(page: Page, provider: string, user: string): Promise<Response> {
  await page.goto("/admin");
  await expect(page.getByRole("heading", { name: "Sign in to chartgen" })).toBeVisible();
  const answer = overviewAnswer(page);
  await page.getByRole("link", { name: `Continue with ${provider}` }).click();
  await page.getByRole("link", { name: `Sign in as ${user}` }).click();
  return answer;
}

/** What the server says to this page for a path that does not exist. */
const unknownPath = (page: Page) =>
  page.evaluate(async () => {
    const res = await fetch("/admin/zzzzzzzz", { headers: { Accept: "application/json" } });
    return { status: res.status, body: await res.text() };
  });

test.describe("the operator page", () => {
  test("the operator opens /admin and sees the server, the users and the clients", async ({
    page,
  }) => {
    const answer = await openAdminAs(page, "GitHub", "octo");
    expect(answer.status()).toBe(200);
    expect(answer.headers()["cache-control"]).toBe("no-store");
    expect(answer.headers()["access-control-allow-origin"]).toBeUndefined();

    await expect(operatorPage(page)).toBeVisible();
    await expect(page).toHaveURL(/\/admin$/);
    await expect(page.getByTestId("operator-server")).toContainText("sign in");
    const me = page.getByTestId("operator-users").locator('[data-user="github:1001"]');
    await expect(me).toContainText("octo");
    await expect(me).toContainText("GitHub");
    await expect(page.getByTestId("operator-clients").locator("tbody tr").first()).toBeVisible();
    // Read-only: the two buttons refresh and leave, and nothing else can be pressed.
    await expect(operatorPage(page).getByRole("button")).toHaveText(["Refresh", "Back to the chart"]);

    // A reload stays on the page; leaving it is the app at `/`.
    await page.reload();
    await expect(operatorPage(page)).toBeVisible();
    await page.getByRole("button", { name: "Back to the chart" }).click();
    await expectChartRendered(page);
    await expect(page).toHaveURL((url) => url.pathname === "/");
    await expect(operatorPage(page)).toHaveCount(0);
    // And the account menu says which id to name as operator.
    await page.getByTestId("user-menu").click();
    await expect(page.getByTestId("user-id")).toHaveText("github:1001");
  });

  test("another user gets the app at /admin, and from the server what an unknown path gets", async ({
    page,
  }) => {
    const answer = await openAdminAs(page, "Google", "Ann Example");
    await expectChartRendered(page);
    await expect(page).toHaveURL((url) => url.pathname === "/");
    await expect(operatorPage(page)).toHaveCount(0);
    await expect(page.getByText(/operator|forbidden|not allowed/i)).toHaveCount(0);

    // The short "no" of the server, with nothing in it, as for a path that does not exist.
    const unknown = await unknownPath(page);
    expect(unknown).toEqual({ status: 404, body: "" });
    expect(answer.status()).toBe(404);
    expect(answer.headers()["content-length"]).toBe("0");
  });

  test("an application's token of the operator gets what an unknown path gets", async ({
    page,
    request,
  }) => {
    // Signed in as the operator in the web app, to approve an application.
    await page.goto("/");
    await page.getByRole("link", { name: "Continue with GitHub" }).click();
    await page.getByRole("link", { name: "Sign in as octo" }).click();
    await expectChartRendered(page);

    const redirect = "https://application.example/cb";
    const registered = await request.post("/register", {
      data: { redirect_uris: [redirect], client_name: "An application" },
    });
    const { client_id } = (await registered.json()) as { client_id: string };
    const verifier = "v".repeat(64);
    const challenge = await page.evaluate(async (v) => {
      const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(v));
      return btoa(String.fromCharCode(...new Uint8Array(digest)))
        .replace(/\+/g, "-")
        .replace(/\//g, "_")
        .replace(/=+$/, "");
    }, verifier);
    await page.goto(
      `/authorize?response_type=code&client_id=${client_id}&redirect_uri=${encodeURIComponent(redirect)}&code_challenge=${challenge}&code_challenge_method=S256&state=s`,
    );
    // The application's site does not exist: the code is read from where the browser
    // is sent, and it does not matter that nothing answers there.
    const sent = page.waitForRequest((r) => r.url().startsWith(redirect));
    await page.getByRole("button", { name: /approve/i }).click();
    const code = new URL((await sent).url()).searchParams.get("code")!;
    const exchanged = await request.post("/token", {
      form: {
        grant_type: "authorization_code",
        code,
        redirect_uri: redirect,
        client_id,
        code_verifier: verifier,
      },
    });
    const { access_token } = (await exchanged.json()) as { access_token: string };
    expect(access_token).toBeTruthy();

    const headers = {
      Authorization: `Bearer ${access_token}`,
      Accept: "application/json",
      "Sec-Fetch-Site": "same-origin",
    };
    const asked = await request.get("/admin/overview", { headers });
    const unknown = await request.get("/admin/zzzzzzzz", { headers });
    expect(asked.status()).toBe(404);
    expect({ status: asked.status(), body: await asked.text() }).toEqual({
      status: unknown.status(),
      body: await unknown.text(),
    });
  });

  test("signed out, /admin is the sign-in like any address, and nothing is asked", async ({
    page,
  }) => {
    const asked: string[] = [];
    page.on("request", (r) => {
      if (new URL(r.url()).pathname === "/admin/overview") asked.push(r.url());
    });
    await page.goto("/admin");
    await expect(page.getByRole("heading", { name: "Sign in to chartgen" })).toBeVisible();
    expect(asked).toEqual([]);
    const unknown = await page.request.get("/admin/zzzzzzzz", {
      headers: { Accept: "application/json" },
    });
    const overview = await page.request.get("/admin/overview", {
      headers: { Accept: "application/json" },
    });
    expect({ status: overview.status(), body: await overview.text() }).toEqual({
      status: unknown.status(),
      body: await unknown.text(),
    });
  });
});
