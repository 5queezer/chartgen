// The server keeps a place for everything but its long-lived streams (ADR-0051). A tab
// refused one goes on working: the chart loads and live data falls back to polling.

import { test, expect, expectChartRendered } from "./fixtures/app";

test("a tab refused a stream still shows its chart", async ({ page }) => {
  let refused = 0;
  await page.route("**/mcp", (route) => {
    if (route.request().method() !== "GET") return route.continue();
    refused += 1;
    return route.fulfill({ status: 503, headers: { "Retry-After": "30" }, body: "Too many streams" });
  });
  await page.goto("/");
  await expectChartRendered(page);
  expect(refused).toBeGreaterThan(0);
  await expect(page.getByTestId("chart-load-error")).toHaveCount(0);
  await expect(page.getByText("sign-in failed")).toHaveCount(0);
});
