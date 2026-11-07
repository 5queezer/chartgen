// The server keeps a place for everything but its long-lived streams (ADR-0051). A tab
// refused one goes on working: the chart loads and live data falls back to polling.

import { test, expect, gotoApp } from "./fixtures/app";

test("a tab refused a stream still shows its chart", async ({ page }) => {
  let refused = 0;
  await page.route("**/mcp", (route) => {
    if (route.request().method() !== "GET") return route.continue();
    refused += 1;
    return route.fulfill({ status: 503, headers: { "Retry-After": "30" }, body: "Too many streams" });
  });
  await gotoApp(page);
  expect(refused).toBeGreaterThan(0);
  await expect(page.getByTestId("chart-load-error")).toHaveCount(0);
  await expect(page.getByText("sign-in failed")).toHaveCount(0);
});

test("a tab refused a stream comes back to it when told to, and to pushed data", async ({ page }) => {
  let refusals = 0;
  const subscribed: string[] = [];
  page.on("response", async (res) => {
    if (!res.url().endsWith("/mcp") || res.request().method() !== "POST") return;
    if (!res.request().postData()?.includes('"subscribe_chart"')) return;
    const text = await res.text().catch(() => "");
    if (text.includes("subscription_id") || text.includes("subscriptionId")) subscribed.push(text);
  });
  await page.route("**/mcp", (route) => {
    if (route.request().method() !== "GET" || refusals >= 3) return route.continue();
    refusals += 1;
    return route.fulfill({ status: 503, headers: { "Retry-After": "1" }, body: "Too many streams" });
  });
  await gotoApp(page);
  await expect.poll(() => refusals, { timeout: 15_000 }).toBe(3);
  await expect.poll(() => subscribed.length, { timeout: 45_000 }).toBeGreaterThan(0);
});

test("a chart refused for a moment comes by itself, with no error shown", async ({ page }) => {
  let refusals = 0;
  await page.route("**/mcp", (route) => {
    const call = route.request();
    if (call.method() !== "POST" || !call.postData()?.includes('"generate_chart"') || refusals >= 2) {
      return route.continue();
    }
    refusals += 1;
    return route.fulfill({
      status: 503,
      headers: { "Retry-After": "1" },
      body: "Too many tool calls are running",
    });
  });
  await gotoApp(page);
  expect(refusals).toBe(2);
  await expect(page.getByTestId("chart-load-error")).toHaveCount(0);
});
