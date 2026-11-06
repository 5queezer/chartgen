// Browser push for server alerts (ADR-0045). No real push service is contacted: the
// browser's PushManager is replaced so that it hands out a subscription at a fake service
// in this file, the one origin playwright.config.ts allows, and only because the server
// runs on the sample source. The keys live here too, so what arrives is decrypted with
// the subscription's own keys. The service worker is the real one.

import { createDecipheriv, createECDH, hkdfSync, randomBytes } from "node:crypto";
import { createServer, type Server } from "node:http";

import type { BrowserContext, Page, Request } from "@playwright/test";

import { test, expect, gotoApp } from "./fixtures/app";

const CHARTGEN_PORT = Number(process.env.CHARTGEN_PORT ?? 9315);
const SERVICE_PORT = CHARTGEN_PORT - 1_000;
const ALERTS_MCP = `http://127.0.0.1:${CHARTGEN_PORT - 2_000}/mcp`;

interface Received {
  url: string;
  headers: Record<string, string | string[] | undefined>;
  body: Buffer;
}

let service: Server;
let pushes: Received[] = [];
let reply = 201;

test.beforeAll(async () => {
  service = createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => chunks.push(c));
    req.on("end", () => {
      if (req.method !== "POST" || !req.url?.startsWith("/push/")) {
        res.writeHead(404).end();
        return;
      }
      pushes.push({ url: req.url, headers: req.headers, body: Buffer.concat(chunks) });
      res.writeHead(reply).end();
    });
  });
  await new Promise<void>((resolve) => service.listen(SERVICE_PORT, "127.0.0.1", resolve));
});

test.afterAll(async () => {
  await new Promise((resolve) => service.close(resolve));
});

test.beforeEach(() => {
  pushes = [];
  reply = 201;
});

/** What a browser's push subscription holds, with the private half it never shows. */
function newDevice() {
  const ecdh = createECDH("prime256v1");
  ecdh.generateKeys();
  const auth = randomBytes(16);
  return {
    ecdh,
    auth,
    endpoint: `http://127.0.0.1:${SERVICE_PORT}/push/${randomBytes(6).toString("hex")}`,
    p256dh: ecdh.getPublicKey().toString("base64url"),
    authText: auth.toString("base64url"),
  };
}
type Device = ReturnType<typeof newDevice>;

/** RFC 8291 from the browser's side, written here and not with the server's code. */
function decrypt(message: Buffer, device: Device): Record<string, unknown> {
  const salt = message.subarray(0, 16);
  const idLen = message[20]!;
  const serverPublic = message.subarray(21, 21 + idLen);
  const sealed = message.subarray(21 + idLen);
  const secret = device.ecdh.computeSecret(serverPublic);
  const info = Buffer.concat([
    Buffer.from("WebPush: info\0"),
    device.ecdh.getPublicKey(),
    serverPublic,
  ]);
  const ikm = Buffer.from(hkdfSync("sha256", secret, device.auth, info, 32));
  const key = (name: string, len: number) =>
    Buffer.from(hkdfSync("sha256", ikm, salt, Buffer.from(`Content-Encoding: ${name}\0`), len));
  const decipher = createDecipheriv("aes-128-gcm", key("aes128gcm", 16), key("nonce", 12));
  decipher.setAuthTag(sealed.subarray(sealed.length - 16));
  const plain = Buffer.concat([
    decipher.update(sealed.subarray(0, sealed.length - 16)),
    decipher.final(),
  ]);
  expect(plain[plain.length - 1]).toBe(2);
  return JSON.parse(plain.subarray(0, -1).toString()) as Record<string, unknown>;
}

/** Stand in for the browser's push service, and count what the page asks and shows. */
async function fakeBrowserPush(context: BrowserContext, device: Device): Promise<void> {
  await context.addInitScript(
    ({ endpoint, p256dh, auth }) => {
      const bytes = (text: string) =>
        Uint8Array.from(atob(text.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
      const FLAG = "test.push.subscribed";
      const KEY = "test.push.key";
      const subscription = {
        endpoint,
        get options() {
          const key = localStorage.getItem(KEY);
          return { applicationServerKey: key ? bytes(key).buffer : null };
        },
        getKey: (name: string) => bytes(name === "p256dh" ? p256dh : auth).buffer,
        unsubscribe: async () => {
          localStorage.removeItem(FLAG);
          return true;
        },
      };
      PushManager.prototype.subscribe = async function (options) {
        const key = new Uint8Array(options!.applicationServerKey as ArrayBuffer);
        localStorage.setItem(
          KEY,
          btoa(String.fromCharCode(...key)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""),
        );
        localStorage.setItem(FLAG, "1");
        return subscription as unknown as PushSubscription;
      };
      PushManager.prototype.getSubscription = async function () {
        return (localStorage.getItem(FLAG) ? subscription : null) as unknown as PushSubscription;
      };
      // Headless Chromium reports the permission as denied whatever was granted, so this
      // stands in for it too, starting as a browser does and remembering the answer.
      const PERMISSION = "test.push.permission";
      const seen = { asked: 0, shown: 0 };
      (window as unknown as { pushSeen: typeof seen }).pushSeen = seen;
      class Counted extends window.Notification {
        constructor(title: string, options?: NotificationOptions) {
          super(title, options);
          seen.shown++;
        }
        static override get permission(): NotificationPermission {
          return (localStorage.getItem(PERMISSION) ?? "default") as NotificationPermission;
        }
        static override requestPermission = (async () => {
          seen.asked++;
          localStorage.setItem(PERMISSION, "granted");
          return "granted";
        }) as typeof Notification.requestPermission;
      }
      window.Notification = Counted as typeof Notification;
    },
    { endpoint: device.endpoint, p256dh: device.p256dh, auth: device.authText },
  );
}

const seenInPage = (page: Page) =>
  page.evaluate(() => (window as unknown as { pushSeen: { asked: number; shown: number } }).pushSeen);
const control = (page: Page) => page.locator('[data-role="push-control"]').first();
const dialog = (page: Page) => page.getByRole("dialog");
const rows = (page: Page) => page.locator('[data-alert-id][data-server="true"]');
const toasts = (page: Page) => page.locator("[data-toast]").filter({ hasText: "Alert:" });

/** The page's token, seen on its way to /mcp, for acting with every page closed. */
function bearerOf(page: Page): () => string {
  let bearer = "";
  page.on("request", (r: Request) => {
    const auth = r.headers().authorization;
    if (auth && r.url().endsWith("/mcp")) bearer = auth;
  });
  return () => bearer;
}

async function callTool(bearer: string, name: string, args: unknown): Promise<Record<string, unknown>> {
  const res = await fetch(ALERTS_MCP, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Accept: "application/json",
      Authorization: bearer,
    },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name, arguments: args },
    }),
  });
  const json = (await res.json()) as { result: { content: { text: string }[] } };
  return JSON.parse(json.result.content[0]!.text) as Record<string, unknown>;
}

async function enable(page: Page): Promise<void> {
  await page.getByRole("tab", { name: /^Alerts/ }).click();
  await expect(control(page)).toHaveAttribute("data-state", "off", { timeout: 15_000 });
  await control(page).getByRole("button", { name: "Enable notifications" }).click();
  await expect(control(page)).toHaveAttribute("data-state", "on");
}

/** A level the sample price is far from, so it waits until `update` brings it near. */
async function createWaiting(page: Page, notify = true): Promise<void> {
  await page.getByRole("button", { name: "Create alert" }).click();
  await expect(dialog(page)).toBeVisible();
  await dialog(page).getByLabel("Run on server").check();
  await dialog(page).getByLabel("Condition").selectOption("greater");
  await dialog(page).getByLabel("Price").fill("999999999");
  const box = dialog(page).getByLabel("Send a notification");
  await expect(box).toBeChecked({ checked: await seenOn(page) });
  await box.setChecked(notify);
  await dialog(page).getByRole("button", { name: "Create" }).click();
  await expect(dialog(page)).toBeHidden();
}
const seenOn = (page: Page) => page.evaluate(() => localStorage.getItem("chartgen.push.on") === "1");

async function fire(bearer: string, index = 0): Promise<void> {
  const list = await callTool(bearer, "list_server_alerts", {});
  const alerts = list.alerts as { id: string }[];
  await callTool(bearer, "update_server_alert", { alert_id: alerts[index]!.id, level: 0.01 });
}

test("the permission is asked on the click, not before", async ({ page, context }) => {
  await fakeBrowserPush(context, newDevice());
  await gotoApp(page);
  await page.getByRole("tab", { name: /^Alerts/ }).click();
  await expect(control(page)).toHaveAttribute("data-state", "off", { timeout: 15_000 });
  await page.waitForTimeout(1_000);
  expect((await seenInPage(page)).asked).toBe(0);
  await control(page).getByRole("button", { name: "Enable notifications" }).click();
  await expect(control(page)).toHaveAttribute("data-state", "on");
  expect((await seenInPage(page)).asked).toBe(1);
  // From the site root, and not one a request ever went through.
  const scope = await page.evaluate(async () => {
    const reg = await navigator.serviceWorker.getRegistration("/");
    return reg?.scope;
  });
  expect(new URL(scope!).pathname).toBe("/");
});

test("with every page closed, the fake service gets exactly one push, and it decrypts", async ({
  page,
  context,
}) => {
  const device = newDevice();
  await fakeBrowserPush(context, device);
  const bearer = bearerOf(page);
  await gotoApp(page);
  await enable(page);
  await createWaiting(page);
  await expect(rows(page).getByText("notifies")).toBeVisible();
  const token = bearer();
  expect(token).not.toBe("");

  await page.close();
  expect(context.pages()).toHaveLength(0);
  await fire(token);

  await expect.poll(() => pushes.length, { timeout: 15_000 }).toBe(1);
  const push = pushes[0]!;
  expect(`http://127.0.0.1:${SERVICE_PORT}${push.url}`).toBe(device.endpoint);
  expect(push.headers["content-encoding"]).toBe("aes128gcm");
  expect(String(push.headers.authorization)).toMatch(/^vapid t=.+, k=/);
  expect(push.body.toString("latin1")).not.toContain("BTCUSDT");

  const payload = decrypt(push.body, device);
  expect(payload.type).toBe("alert.fired");
  expect(payload.symbol).toBe("BTCUSDT");
  expect(payload.condition).toBe("greater");
  expect(payload.level).toBe(0.01);
  expect(payload.price as number).toBeGreaterThan(0.01);
  expect(payload.fire_count).toBe(1);

  // Long enough for a second one to have gone out, had there been one.
  await new Promise((resolve) => setTimeout(resolve, 3_000));
  expect(pushes).toHaveLength(1);
  const list = await callTool(token, "list_server_alerts", {});
  const [alert] = list.alerts as { last_push_status: number; fire_count: number }[];
  expect(alert!.last_push_status).toBe(201);
  expect(alert!.fire_count).toBe(1);
});

test("with a tab open the push is the only report, unless it fails", async ({ page, context }) => {
  // Two rounds of the poll, then a failing push's retries.
  test.setTimeout(90_000);
  await fakeBrowserPush(context, newDevice());
  const bearer = bearerOf(page);
  await gotoApp(page);
  await enable(page);

  // Delivered: the tab says nothing.
  await createWaiting(page);
  await fire(bearer(), 0);
  await expect.poll(() => pushes.length, { timeout: 15_000 }).toBe(1);
  await expect(rows(page).first()).toHaveAttribute("data-status", "triggered", { timeout: 15_000 });
  // Two more rounds of the poll, which is what would report it.
  await page.waitForTimeout(11_000);
  await expect(toasts(page)).toHaveCount(0);
  expect((await seenInPage(page)).shown).toBe(0);

  // Not delivered: the tab does tell, once.
  reply = 500;
  await createWaiting(page);
  await fire(bearer(), 1);
  await expect(toasts(page)).toHaveCount(1, { timeout: 40_000 });
  expect((await seenInPage(page)).shown).toBe(1);
  await page.waitForTimeout(6_000);
  expect((await seenInPage(page)).shown).toBe(1);
});

test("an alert that does not notify is still reported by the tab", async ({ page, context }) => {
  await fakeBrowserPush(context, newDevice());
  const bearer = bearerOf(page);
  await gotoApp(page);
  await enable(page);
  await createWaiting(page, false);
  await fire(bearer());
  await expect(toasts(page)).toHaveCount(1, { timeout: 15_000 });
  expect(pushes).toHaveLength(0);
});

test("the real worker shows a push, tagged by its event", async ({ page, context }) => {
  await fakeBrowserPush(context, newDevice());
  await gotoApp(page);
  await enable(page);
  await expect.poll(() => context.serviceWorkers().length).toBe(1);
  // Headless Chromium cannot show one, so this records what the worker asks for.
  await context.serviceWorkers()[0]!.evaluate(() => {
    const shown: unknown[] = [];
    (self as unknown as { shown: unknown[] }).shown = shown;
    const registration = (self as unknown as ServiceWorkerGlobalScope).registration;
    registration.showNotification = async (title, options) => {
      shown.push([title, options?.body, options?.tag, options?.data]);
    };
  });
  const cdp = await context.newCDPSession(page);
  const registered = new Promise<string>((resolve) => {
    cdp.on("ServiceWorker.workerRegistrationUpdated", (e) => {
      const reg = e.registrations.find((r) => !r.isDeleted);
      if (reg) resolve(reg.registrationId);
    });
  });
  await cdp.send("ServiceWorker.enable");
  await cdp.send("ServiceWorker.deliverPushMessage", {
    origin: new URL(page.url()).origin,
    registrationId: await registered,
    data: JSON.stringify({
      id: "event-1",
      symbol: "BTCUSDT",
      condition: "crossing_up",
      level: 70000,
      price: 70012.5,
      message: "take profit",
    }),
  });
  await expect
    .poll(() =>
      context.serviceWorkers()[0]!.evaluate(() => (self as unknown as { shown: unknown[] }).shown),
    )
    .toEqual([
      [
        "BTCUSDT crossing up 70000",
        "Price 70012.5 · take profit",
        "alert-fired-event-1",
        { symbol: "BTCUSDT" },
      ],
    ]);
});

test("a click on a notification, with a tab open, shows that symbol", async ({ bootedApp: page }) => {
  const send = (data: unknown) =>
    page.evaluate((d) => {
      navigator.serviceWorker.dispatchEvent(new MessageEvent("message", { data: d }));
    }, data);
  const shown = (symbol: RegExp) => page.getByRole("button", { name: symbol });
  await expect(shown(/^Symbol:.*BTCUSDT/)).toBeVisible();
  await send({ type: "chartgen.open-symbol", symbol: "<script>" });
  await send({ type: "chartgen.open-symbol", symbol: 42 });
  await send({ type: "something else", symbol: "ETHUSDT" });
  await page.waitForTimeout(300);
  await expect(shown(/^Symbol:.*BTCUSDT/)).toBeVisible();
  await send({ type: "chartgen.open-symbol", symbol: "ETHUSDT" });
  await expect(shown(/^Symbol:.*ETHUSDT/)).toBeVisible({ timeout: 10_000 });
});

test("turning it off stops this device, on the server too", async ({ page, context }) => {
  await fakeBrowserPush(context, newDevice());
  const bearer = bearerOf(page);
  await gotoApp(page);
  await enable(page);
  await createWaiting(page);
  await control(page).getByRole("button", { name: "Turn off" }).click();
  await expect(control(page)).toHaveAttribute("data-state", "off");
  const list = await callTool(bearer(), "list_server_alerts", {});
  expect((list.push as { devices: number }).devices).toBe(0);
  await fire(bearer());
  await expect(rows(page).first().locator('[data-role="server-detail"]')).toHaveText(
    "push skipped: no device has notifications on",
    { timeout: 15_000 },
  );
  expect(pushes).toHaveLength(0);
});

test("a reload keeps it on and tells the server again, with no prompt", async ({ page, context }) => {
  await fakeBrowserPush(context, newDevice());
  await gotoApp(page);
  await enable(page);
  await gotoApp(page);
  await page.getByRole("tab", { name: /^Alerts/ }).click();
  await expect(control(page)).toHaveAttribute("data-state", "on", { timeout: 15_000 });
  expect((await seenInPage(page)).asked).toBe(0);
});

test("a blocked permission says so, and offers no button", async ({ page, context }) => {
  await context.addInitScript(() => {
    Object.defineProperty(Notification, "permission", { get: () => "denied" });
  });
  await gotoApp(page);
  await page.getByRole("tab", { name: /^Alerts/ }).click();
  await expect(control(page)).toHaveAttribute("data-state", "denied", { timeout: 15_000 });
  await expect(control(page)).toContainText("blocked");
  await expect(control(page).getByRole("button")).toHaveCount(0);
});

test("a browser with no push says so", async ({ page, context }) => {
  await context.addInitScript(() => {
    delete (window as unknown as { PushManager?: unknown }).PushManager;
  });
  await gotoApp(page);
  await page.getByRole("tab", { name: /^Alerts/ }).click();
  await expect(control(page)).toHaveAttribute("data-state", "unsupported", { timeout: 15_000 });
  await expect(control(page).getByRole("button")).toHaveCount(0);
});

test("the worker and manifest are at the root", async ({ page }) => {
  await gotoApp(page);
  const manifest = await page.request.get("/manifest.webmanifest");
  expect(manifest.ok()).toBe(true);
  const parsed = (await manifest.json()) as { display: string; start_url: string };
  expect(parsed.display).toBe("standalone");
  expect(parsed.start_url).toBe("/");
  const worker = await page.request.get("/sw.js");
  expect(worker.ok()).toBe(true);
  expect(worker.headers()["content-type"]).toContain("javascript");
  await expect(page.locator('link[rel="manifest"]')).toHaveCount(1);

  // Every icon asked for exists, as a PNG of the size it claims. On a clean checkout, as
  // in CI, this is what notices one that was never committed.
  const touch = await page.locator('link[rel="apple-touch-icon"]').getAttribute("href");
  const manifestIcons = (parsed as unknown as { icons: { src: string; sizes: string }[] }).icons;
  const icons = [...manifestIcons, { src: touch!, sizes: "180x180" }];
  expect(manifestIcons.map((i) => i.sizes)).toEqual(expect.arrayContaining(["192x192", "512x512"]));
  for (const { src, sizes } of icons) {
    const res = await page.request.get(src);
    expect(res.ok(), src).toBe(true);
    const png = await res.body();
    expect(png.subarray(1, 4).toString(), src).toBe("PNG");
    expect(`${png.readUInt32BE(16)}x${png.readUInt32BE(20)}`, src).toBe(sizes);
  }
});

test.describe("on an iPhone, not installed", () => {
  test.use({
    viewport: { width: 390, height: 844 },
    hasTouch: true,
    isMobile: true,
    userAgent:
      "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1",
  });

  test("the compact layout says to add it to the Home Screen, and fits", async ({ page }) => {
    const exchanged = page.waitForResponse(
      (res) => res.request().method() === "POST" && new URL(res.url()).pathname === "/token",
    );
    await page.goto("/");
    expect((await exchanged).ok()).toBe(true);
    await page.waitForURL((url) => !url.searchParams.has("code"));
    await page.getByRole("navigation", { name: "Panels" }).getByRole("button", { name: "Alerts", exact: true }).tap();
    await expect(control(page)).toHaveAttribute("data-state", "install", { timeout: 15_000 });
    await expect(control(page)).toContainText("Add to Home Screen");
    await expect(control(page).getByRole("button")).toHaveCount(0);
    const box = (await control(page).boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(390);
  });
});
