// Cold-start smoke test — ensures the v0.2 happy path still ships:
//   page load → OAuth PKCE dance → auto-load → chart renders.
//
// This spec intentionally uses plain `page` instead of the `bootedApp`
// fixture, so it can watch the boot itself (console errors, MCP
// handshakes) from the first request on. If this spec passes, other
// specs can rely on the fixture with confidence.

import { test, expect, gotoApp } from "./fixtures/app";

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

    // Count MCP `initialize` handshakes: all boot queries must share one
    // connection (a leak shows up as one extra initialize per query).
    let initializeCount = 0;
    page.on("request", (req) => {
      if (
        req.method() !== "POST" ||
        !new URL(req.url()).pathname.endsWith("/mcp")
      )
        return;
      try {
        if (JSON.parse(req.postData() ?? "").method === "initialize")
          initializeCount++;
      } catch {
        // non-JSON or batch body; not an initialize request.
      }
    });

    // Boot from a fresh context (empty storage), so the OAuth flow
    // exercises dynamic registration. The app 302s through /authorize,
    // lands back with ?code, exchanges it and scrubs the URL; the chart
    // then loads without any button click.
    await gotoApp(page);

    // Default indicator set seeds RSI, so the RSI pane header should
    // render with the label text.
    const rsiHeader = page.locator(".pane-header").filter({ hasText: "RSI" });
    await expect(rsiHeader.first()).toBeVisible();

    expect(initializeCount, "MCP initialize requests during boot").toBe(1);

    expect(
      consoleErrors,
      `unexpected console errors: ${consoleErrors.join("\n")}`,
    ).toEqual([]);
  });
});
