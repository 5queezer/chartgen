// Shared Playwright fixtures for the chartgen E2E suite.
//
// The `bootedApp` fixture walks the app through its cold-start path:
// visit `/`, follow the OAuth PKCE redirect back to the app, let
// the chart auto-load, and wait for the chart canvas to render. Specs that need a
// ready-to-use app depend on it; specs that need to observe the boot
// itself (smoke.spec.ts) skip it and drive `page` directly.
//
// Every test gets a fresh browser context, so localStorage and
// sessionStorage start empty: no OAuth `client_id`/`client_secret` pair
// leaks in from an earlier test.

import { expect, test as base, type Page } from "@playwright/test";

interface AppFixtures {
  bootedApp: Page;
}

/**
 * Wait until the klinecharts canvas inside `.chart-host` has a
 * non-zero-sized canvas rendered by the library. We intentionally do
 * not sample pixels here — the sample_data fallback can produce mostly
 * empty bars and `getImageData` becomes a flakiness generator. A
 * rendered canvas + at least one `.pane-header` is strong enough.
 */
export async function expectChartRendered(page: Page): Promise<void> {
  const chartHost = page.locator(".chart-host");
  await expect(chartHost).toBeVisible();

  // klinecharts mounts its canvas inside `.chart-container`.
  const canvas = chartHost.locator("canvas").first();
  await expect(canvas).toBeVisible({ timeout: 15_000 });

  // klinecharts sizes the canvas via an internal resize-observer tick,
  // so the width/height can still be zero immediately after the element
  // becomes visible. Poll until both dimensions are positive.
  await expect
    .poll(
      async () => {
        const size = await canvas.evaluate((el: HTMLCanvasElement) => ({
          width: el.width,
          height: el.height,
        }));
        return size.width > 0 && size.height > 0;
      },
      { timeout: 15_000 },
    )
    .toBe(true);

  // Wait for the status line to settle into "loaded" so downstream
  // assertions (pane header text, indicator data) see a stable DOM.
  await expect(page.locator("text=loaded").first()).toBeVisible({
    timeout: 20_000,
  });

  // Per ADR-0006: the price pane and any indicator pane each get a
  // `.pane-header` strip. With default indicators there should be at
  // least one.
  await expect(page.locator(".pane-header").first()).toBeVisible();
}

/**
 * Run `navigate` and wait until the app has booted: OAuth done and chart
 * rendered. Every document load runs the whole PKCE redirect again (the
 * access token lives in memory only), so a reload is a boot too: two
 * document loads of the Vite dev app plus the token exchange.
 *
 * The waits are on the boot's own events (the token exchange response,
 * then the URL losing `?code`), bounded only by the test timeout. A fixed
 * window per step is a guess at page-load time that a loaded machine
 * easily exceeds.
 */
export async function bootApp(
  page: Page,
  navigate: () => Promise<unknown>,
): Promise<void> {
  const exchanged = page.waitForResponse(
    (res) =>
      res.request().method() === "POST" &&
      new URL(res.url()).pathname === "/token",
  );
  await navigate();
  const token = await exchanged;
  expect(token.ok(), `token exchange answered ${token.status()}`).toBe(true);
  // The app scrubs `?code`/`?state` from the URL once it holds the token.
  await page.waitForURL((url) => !url.searchParams.has("code"));
  // The chart loads on its own once auth is ready.
  await expectChartRendered(page);
}

/** Open `path` and wait for the booted app (OAuth done, chart rendered). */
export function gotoApp(page: Page, path = "/"): Promise<void> {
  return bootApp(page, () => page.goto(path));
}

/** Reload and wait for the booted app (OAuth done, chart rendered). */
export function reloadApp(page: Page): Promise<void> {
  return bootApp(page, () => page.reload());
}

export const test = base.extend<AppFixtures>({
  bootedApp: async ({ page }, use) => {
    await gotoApp(page);
    await use(page);
  },
});

export { expect } from "@playwright/test";
