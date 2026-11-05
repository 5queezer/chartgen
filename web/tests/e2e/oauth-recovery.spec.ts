// Sign-in recovery: a failed token exchange must not strand the app on
// `?code=...`. The app scrubs the code, retries the whole flow once by
// itself, and after a second failure stops and offers "Sign in again"
// instead of redirecting in a loop.

import type { Page, Response } from "@playwright/test";

import { test, expect, bootApp, expectChartRendered } from "./fixtures/app";

const isToken = (res: Response) =>
  res.request().method() === "POST" && new URL(res.url()).pathname === "/token";

/** Answer the next `count` token exchanges with a 500; record every status. */
async function failTokenExchanges(page: Page, count: number): Promise<number[]> {
  let left = count;
  await page.route("**/token", async (route) => {
    if (route.request().method() === "POST" && left > 0) {
      left--;
      // CORS header so the app reads the 500 instead of a network error.
      await route.fulfill({
        status: 500,
        headers: { "Access-Control-Allow-Origin": "*" },
        body: "boom",
      });
      return;
    }
    await route.continue();
  });
  const statuses: number[] = [];
  page.on("response", (res) => {
    if (isToken(res)) statuses.push(res.status());
  });
  return statuses;
}

test.describe("sign-in recovery", () => {
  test("a failed token exchange is retried once by itself", async ({ page }) => {
    const statuses = await failTokenExchanges(page, 1);
    const recovered = page.waitForResponse((res) => isToken(res) && res.ok());

    await page.goto("/");
    await recovered;
    await page.waitForURL((url) => !url.searchParams.has("code"));
    await expectChartRendered(page);

    expect(statuses).toEqual([500, 200]);
    // A success clears the retry marker, so a later failure retries again.
    expect(
      await page.evaluate(() => sessionStorage.getItem("chartgen.oauth.retried")),
    ).toBeNull();
  });

  test("a second failure stops with Sign in again, which recovers", async ({
    page,
  }) => {
    const statuses = await failTokenExchanges(page, 2);
    let failed = 0;
    const bothFailed = new Promise<void>((resolve) => {
      page.on("response", (res) => {
        if (isToken(res) && res.status() === 500 && ++failed === 2) resolve();
      });
    });

    await page.goto("/");
    await bothFailed;
    const signIn = page.getByRole("button", { name: "Sign in again" });
    await expect(signIn).toBeVisible();
    await expect(page.getByText(/^sign-in failed: token exchange failed: 500/)).toBeVisible();
    // Stopped: the code is scrubbed and no third exchange was started.
    expect(new URL(page.url()).searchParams.has("code")).toBe(false);
    expect(statuses).toEqual([500, 500]);

    await bootApp(page, () => signIn.click());
    expect(statuses).toEqual([500, 500, 200]);
  });
});
