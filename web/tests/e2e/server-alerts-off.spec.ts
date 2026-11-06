// With server alerts off, which is the default and how production runs (ADR-0042),
// the app must cost the server nothing for them: no `list_server_alerts` at all, and
// one `tools/list` from the tab that leads.

import type { Page, Request } from "@playwright/test";

import { test, expect, gotoApp, reloadApp } from "./fixtures/app";

interface Counts {
  list: number;
  tools: number;
}

function count(page: Page): Counts {
  const counts = { list: 0, tools: 0 };
  page.on("request", (r: Request) => {
    const body = r.postData() ?? "";
    if (body.includes('"list_server_alerts"')) counts.list++;
    if (body.includes('"tools/list"')) counts.tools++;
  });
  return counts;
}

test("one tab asks only tools/list, once", async ({ page }) => {
  const counts = count(page);
  await gotoApp(page);
  await expect.poll(() => counts.tools).toBe(1);
  // Long enough for two rounds of the 5 s poll this must not be running.
  await page.waitForTimeout(11_000);
  expect(counts).toEqual({ list: 0, tools: 1 });
});

test("a second tab asks nothing while the first leads", async ({ page, context }) => {
  const first = count(page);
  await gotoApp(page);
  await expect.poll(() => first.tools).toBe(1);
  const other = await context.newPage();
  const second = count(other);
  await gotoApp(other);
  await other.waitForTimeout(6_000);
  expect(second).toEqual({ list: 0, tools: 0 });
  expect(first.list).toBe(0);
  await other.close();
});

test("the dialog says an alert runs in this browser, and offers no server", async ({ bootedApp: page }) => {
  await page.getByRole("tab", { name: /^Alerts/ }).click();
  await page.getByRole("button", { name: "Create alert" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.locator('[data-role="alert-browser-only"]')).toContainText(
    "does not run alerts",
  );
  await expect(dialog.getByLabel("Run on server")).toHaveCount(0);
  await expect(dialog.getByLabel("Webhook", { exact: true })).toHaveCount(0);
});

test("a focus asks again only after a pause", async ({ page }) => {
  const counts = count(page);
  await gotoApp(page);
  await expect.poll(() => counts.tools).toBe(1);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await page.waitForTimeout(500);
  expect(counts.tools).toBe(1);
});

test("what an earlier server shared does not outlive it being off", async ({ bootedApp: page }) => {
  await page.evaluate(() =>
    localStorage.setItem(
      "chartgen.serverAlerts.list",
      JSON.stringify({ enabled: true, can_create: true, webhooks: true, alerts: [] }),
    ),
  );
  await reloadApp(page);
  await page.getByRole("tab", { name: /^Alerts/ }).click();
  await page.getByRole("button", { name: "Create alert" }).click();
  await expect(page.getByRole("dialog").getByLabel("Run on server")).toHaveCount(0);
  await expect(
    page.getByRole("dialog").locator('[data-role="alert-browser-only"]'),
  ).toContainText("does not run alerts");
});
