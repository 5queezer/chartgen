// Shared Playwright fixtures for the chartgen E2E suite.
//
// The `bootedApp` fixture walks the app through its cold-start path:
// visit `/`, follow the OAuth PKCE redirect back to the app, fire a
// Load, and wait for the chart canvas to render. Specs that need a
// ready-to-use app depend on it; specs that need to observe the boot
// itself (smoke.spec.ts) skip it and drive `page` directly.
//
// localStorage is cleared for every test — chartgen's OAuth store
// persists `client_id`/`client_secret` there, and a stale pair causes
// weird cross-test behavior when the backing server restarts.

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

export const test = base.extend<AppFixtures>({
  bootedApp: async ({ page }, use) => {
    // Visit once to establish the origin so we can reset storage. The
    // MCP SDK writes to localStorage via the dynamic-client-registration
    // cache, so this also prevents stale `client_id` leaking across tests.
    await page.goto("/");
    const clearError = await page.evaluate(() => {
      try {
        localStorage.clear();
        sessionStorage.clear();
        return null;
      } catch (err) {
        return err instanceof Error ? err.message : String(err);
      }
    });
    if (clearError !== null) {
      // Surface the failure rather than silently continuing with
      // whatever leftover state the previous test left behind.
      console.warn(`[fixture] failed to reset browser storage: ${clearError}`);
    }

    await page.goto("/");

    // The first paint triggers the OAuth redirect chain: /authorize on
    // the backend round-trips to / with ?code=...&state=..., which the
    // app consumes in onMount. Wait for the post-redirect "ready" or
    // "loaded" status, not the URL — the app scrubs code/state from the
    // URL once the exchange finishes.
    await page.waitForURL(/\/(?:\?.*)?$/, { timeout: 30_000 });
    await expect(page.locator("text=authenticating...")).toHaveCount(0, {
      timeout: 30_000,
    });

    // Click Load to trigger the initial chart fetch.
    await page.getByRole("button", { name: "Load" }).click();
    await expectChartRendered(page);

    await use(page);
  },
});

export { expect } from "@playwright/test";
