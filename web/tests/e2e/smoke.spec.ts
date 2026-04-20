// Cold-start smoke test — ensures the v0.2 happy path still ships:
//   page load → OAuth PKCE dance → Load click → chart renders.
//
// This spec intentionally drives `page` directly instead of pulling in
// the `bootedApp` fixture, because the fixture itself is what we want
// to smoke-test. If this spec passes, other specs can rely on the
// fixture with confidence.

import { test, expect, expectChartRendered } from "./fixtures/app";

test.describe("smoke", () => {
  test("cold boot → OAuth → chart render", async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error") {
        consoleErrors.push(msg.text());
      }
    });
    page.on("pageerror", (err) => {
      consoleErrors.push(`pageerror: ${err.message}`);
    });

    // Start clean so the OAuth flow exercises dynamic registration.
    // If storage.clear() fails the test isn't really cold — surface that
    // so a reviewer knows what they're looking at.
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
      console.warn(`[smoke] failed to reset browser storage: ${clearError}`);
    }

    // Boot. The app will 302 through /authorize and land back with a
    // scrubbed URL once the token exchange completes.
    await page.goto("/");

    // The app clears `?code`/`?state` from the URL after the exchange.
    // Waiting for the status line to leave the authenticating state is
    // the most reliable signal the redirect dance finished.
    await expect(page.locator("text=authenticating...")).toHaveCount(0, {
      timeout: 30_000,
    });

    // Ready state should be visible before the user clicks Load.
    await expect(page.locator("text=ready").first()).toBeVisible({
      timeout: 5_000,
    });

    // Fill ticker + interval and submit.
    // The TextField holds "BTCUSDT" by default; set it explicitly anyway
    // so the test is self-documenting.
    const ticker = page.getByLabel("Ticker");
    await ticker.fill("BTCUSDT");

    await page.getByRole("button", { name: "Load" }).click();

    // During the fetch the status flips through "loading..." into
    // "loaded". We don't need to catch the transient — `expectChartRendered`
    // already waits for the "loaded" state.
    await expectChartRendered(page);

    // Default indicator set seeds RSI, so the RSI pane header should
    // render with the label text.
    const rsiHeader = page.locator(".pane-header").filter({ hasText: "RSI" });
    await expect(rsiHeader.first()).toBeVisible();

    expect(
      consoleErrors,
      `unexpected console errors: ${consoleErrors.join("\n")}`,
    ).toEqual([]);
  });
});
