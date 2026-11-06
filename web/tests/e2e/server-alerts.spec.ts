// Server alerts (ADR-0042): an alert the server runs, with a webhook, seen from
// the Alerts tab. The webhook goes to a receiver in this file, the one origin
// playwright.config.ts allows, and only because the server runs on the sample
// source. Conditions use "greater than" a level far below the sample price, so
// the alert fires on the feed's next tick.

import { createHmac } from "node:crypto";
import { createServer, type Server } from "node:http";

import type { Page } from "@playwright/test";

import { test, expect, gotoApp, reloadApp } from "./fixtures/app";

const WEBHOOK_PORT = Number(process.env.CHARTGEN_PORT ?? 9315) - 1_000;
const HOOK = `http://127.0.0.1:${WEBHOOK_PORT}/hook`;

interface Received {
  headers: Record<string, string | string[] | undefined>;
  raw: string;
  body: Record<string, unknown>;
}

let server: Server;
let received: Received[] = [];

test.beforeAll(async () => {
  server = createServer((req, res) => {
    let text = "";
    req.on("data", (c: Buffer) => (text += c.toString()));
    req.on("end", () => {
      // Anything else that finds this port is not a delivery.
      if (req.url !== "/hook") {
        res.writeHead(404).end();
        return;
      }
      received.push({
        headers: req.headers,
        raw: text,
        body: JSON.parse(text) as Record<string, unknown>,
      });
      res.writeHead(200).end();
    });
  });
  await new Promise<void>((resolve) => server.listen(WEBHOOK_PORT, "127.0.0.1", resolve));
});

test.afterAll(async () => {
  await new Promise((resolve) => server.close(resolve));
});

test.beforeEach(() => {
  received = [];
});

const dialog = (page: Page) => page.getByRole("dialog");
const alertsTab = (page: Page) => page.getByRole("tab", { name: /^Alerts/ });
const rows = (page: Page) => page.locator('[data-alert-id][data-server="true"]');
const toasts = (page: Page) => page.locator("[data-toast]").filter({ hasText: "Alert:" });

async function openDialog(page: Page): Promise<void> {
  await alertsTab(page).click();
  await page.getByRole("button", { name: "Create alert" }).click();
  await expect(dialog(page)).toBeVisible();
}

/** Fill the dialog for a server alert and submit it. */
async function createServerAlert(
  page: Page,
  opts: { price?: string; condition?: string; webhook?: string; sign?: boolean; trigger?: string } = {},
): Promise<void> {
  await openDialog(page);
  await expect(dialog(page).getByLabel("Run on server")).toBeVisible();
  await dialog(page).getByLabel("Run on server").check();
  await dialog(page).getByLabel("Condition").selectOption(opts.condition ?? "greater");
  await dialog(page).getByLabel("Price").fill(opts.price ?? "0.01");
  if (opts.trigger) await dialog(page).getByLabel("Trigger").selectOption(opts.trigger);
  if (opts.webhook) await dialog(page).getByLabel("Webhook", { exact: true }).fill(opts.webhook);
  if (opts.sign) await dialog(page).getByLabel("Sign webhook").check();
  await dialog(page).getByRole("button", { name: "Create" }).click();
}

test("a server alert fires its webhook once, and the panel shows the result", async ({ bootedApp: page }) => {
  await createServerAlert(page, { webhook: HOOK });
  await expect(dialog(page)).toBeHidden();
  await expect.poll(() => received.length, { timeout: 15_000 }).toBeGreaterThan(0);

  const { body, headers } = received[0]!;
  expect(body.type).toBe("alert.fired");
  expect(body.symbol).toBe("BTCUSDT");
  expect(body.condition).toBe("greater");
  expect(body.level).toBe(0.01);
  expect(body.trigger).toBe("once");
  expect(headers["x-chartgen-event"]).toBe(body.id);
  expect(headers["x-chartgen-signature"]).toBeUndefined();

  const row = rows(page);
  await expect(row).toHaveCount(1);
  await expect(row).toHaveAttribute("data-status", "triggered", { timeout: 15_000 });
  await expect(row.getByText("server", { exact: true })).toBeVisible();
  await expect(row.locator('[data-role="server-detail"]')).toHaveText("webhook HTTP 200");
  await expect(row).toContainText("127.0.0.1");
  await expect(row).not.toContainText("/hook");

  // Reported once, and the browser holds no copy it could evaluate.
  await expect(toasts(page)).toHaveCount(1, { timeout: 15_000 });
  const stored = await page.evaluate(() => localStorage.getItem("chartgen.alerts.list"));
  expect(stored ?? "").not.toContain("0.01");
  expect(received).toHaveLength(1);
});

test("with two tabs open a fire reaches the webhook once and one tab reports it", async ({ bootedApp: page, context }) => {
  const other = await context.newPage();
  await gotoApp(other);
  await createServerAlert(page, { webhook: HOOK });
  await expect.poll(() => received.length, { timeout: 15_000 }).toBe(1);

  await alertsTab(other).click();
  await expect(other.locator('[data-role="alerts-elsewhere"]')).toBeVisible();
  await expect(rows(other)).toHaveAttribute("data-status", "triggered", { timeout: 15_000 });
  await expect(rows(page)).toHaveAttribute("data-status", "triggered", { timeout: 15_000 });
  // The tab holding the lock reports it, and the other stays quiet, a few polls on.
  await expect(toasts(page)).toHaveCount(1, { timeout: 15_000 });
  await other.waitForTimeout(6_000);
  await expect(toasts(other)).toHaveCount(0);
  expect(received).toHaveLength(1);
  await other.close();
});

test("a price fed to the browser does not fire a server alert", async ({ bootedApp: page }) => {
  // Far above the sample price, so the server's own feed never fires it.
  await createServerAlert(page, { price: "9000000", condition: "crossing_up" });
  await expect(rows(page)).toHaveAttribute("data-status", "active");
  await page.evaluate(() => {
    const w = window as unknown as {
      __chartgenTest: { feedPrice: (s: string, p: number) => void };
    };
    w.__chartgenTest.feedPrice("BTCUSDT", 8_999_999);
    w.__chartgenTest.feedPrice("BTCUSDT", 9_000_001);
  });
  await page.waitForTimeout(500);
  await expect(toasts(page)).toHaveCount(0);
  await expect(rows(page)).toHaveAttribute("data-status", "active");
});

test("a signed webhook shows its secret once and signs with it", async ({ bootedApp: page }) => {
  await createServerAlert(page, { webhook: HOOK, sign: true });
  const shown = dialog(page).locator('[data-role="webhook-secret"]');
  await expect(shown).toBeVisible();
  const secret = (await shown.textContent())!;
  expect(secret).toMatch(/^[0-9a-f]{64}$/);
  await dialog(page).getByRole("button", { name: "Done" }).click();

  await expect.poll(() => received.length, { timeout: 15_000 }).toBe(1);
  const { headers, raw } = received[0]!;
  const timestamp = headers["x-chartgen-timestamp"] as string;
  expect(headers["x-chartgen-signature"]).toBe(
    `sha256=${createHmac("sha256", secret).update(`${timestamp}.${raw}`).digest("hex")}`,
  );

  await reloadApp(page);
  await alertsTab(page).click();
  await expect(rows(page)).toHaveCount(1);
  const html = await page.content();
  expect(html).not.toContain(secret);
});

test("the dialog refuses an unsafe webhook and keeps the alert uncreated", async ({ bootedApp: page }) => {
  for (const url of [
    "https://127.0.0.1/hook",
    "https://169.254.169.254/latest/meta-data/",
    "https://[::1]/",
    "http://example.com/",
    "https://example.com:8443/",
  ]) {
    await page.keyboard.press("Escape");
    await createServerAlert(page, { webhook: url });
    await expect(dialog(page).getByRole("alert")).toContainText("webhook URL");
    await dialog(page).getByRole("button", { name: "Cancel" }).click();
    await expect(dialog(page)).toBeHidden();
  }
  await expect(rows(page)).toHaveCount(0);
  expect(received).toHaveLength(0);
});

test("drawing and indicator alerts say they stay in the browser", async ({ bootedApp: page }) => {
  await openDialog(page);
  await expect(dialog(page).getByLabel("Run on server")).toBeVisible();
  await expect(dialog(page).locator('[data-role="alert-browser-only"]')).toHaveCount(0);
  const source = dialog(page).getByLabel("Source");
  const plot = source.locator("optgroup[label=Indicators] option").first();
  await source.selectOption((await plot.getAttribute("value"))!);
  await expect(dialog(page).getByLabel("Run on server")).toHaveCount(0);
  await expect(dialog(page).locator('[data-role="alert-browser-only"]')).toContainText(
    "run in this browser",
  );
});

test("pause, edit and delete act on the server, and survive a reload", async ({ bootedApp: page }) => {
  await createServerAlert(page, { price: "9000000", condition: "crossing_up", trigger: "every" });
  const row = rows(page);
  await expect(row).toHaveAttribute("data-status", "active");

  await row.hover();
  await row.getByRole("button", { name: "Pause alert" }).click();
  await expect(row).toHaveAttribute("data-status", "paused");

  await row.hover();
  await row.getByRole("button", { name: "Edit alert" }).click();
  await expect(dialog(page).getByLabel("Run on server")).toHaveCount(0);
  await dialog(page).getByLabel("Price").fill("8000000");
  await dialog(page).getByRole("button", { name: "Save" }).click();
  await expect(dialog(page)).toBeHidden();
  await expect(row).toHaveAttribute("data-status", "active");
  await expect(row.locator('[data-role="description"]')).toContainText("8000000");

  await reloadApp(page);
  await alertsTab(page).click();
  await expect(rows(page)).toHaveCount(1);
  await expect(rows(page)).toHaveAttribute("data-status", "active");

  await rows(page).hover();
  await rows(page).getByRole("button", { name: "Delete alert" }).click();
  await expect(rows(page)).toHaveCount(0);
  await reloadApp(page);
  await alertsTab(page).click();
  await page.waitForTimeout(1_000);
  await expect(rows(page)).toHaveCount(0);
});

test("a fire is reported once across a reload", async ({ bootedApp: page }) => {
  await createServerAlert(page);
  await expect(toasts(page)).toHaveCount(1, { timeout: 15_000 });
  await reloadApp(page);
  await alertsTab(page).click();
  await expect(rows(page)).toHaveAttribute("data-status", "triggered");
  await page.waitForTimeout(1_500);
  await expect(toasts(page)).toHaveCount(0);
});
