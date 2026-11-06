// One user, two browsers: a phone gets the alert made on the desktop (ADR-0044,
// ADR-0045). Two real browser contexts sign in through the fake provider, each with its
// own OAuth client and push subscription at a fake service in this file, the one origin
// playwright.config.ts allows, and only because the server runs on the sample source.

import { createDecipheriv, createECDH, hkdfSync, randomBytes } from "node:crypto";
import { createServer, type Server } from "node:http";

import type { BrowserContext, Page, Request } from "@playwright/test";

import { test, expect, expectChartRendered } from "./fixtures/app";

const CHARTGEN_PORT = Number(process.env.CHARTGEN_PORT ?? 9315);
const SERVICE_PORT = CHARTGEN_PORT - 5_000;
const MCP = `http://127.0.0.1:${CHARTGEN_PORT - 3_000}/mcp`;

interface Received {
  url: string;
  body: Buffer;
}

let service: Server;
let pushes: Received[] = [];
/** What the fake service answers, by the end of the endpoint. */
let replies: Record<string, number> = {};

test.beforeAll(async () => {
  service = createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => chunks.push(c));
    req.on("end", () => {
      if (req.method !== "POST" || !req.url?.startsWith("/push/")) {
        res.writeHead(404).end();
        return;
      }
      pushes.push({ url: req.url, body: Buffer.concat(chunks) });
      res.writeHead(replies[req.url] ?? 201).end();
    });
  });
  await new Promise<void>((resolve) => service.listen(SERVICE_PORT, "127.0.0.1", resolve));
});

test.afterAll(async () => {
  await new Promise((resolve) => service.close(resolve));
});

test.beforeEach(() => {
  pushes = [];
  replies = {};
});

function newDevice() {
  const ecdh = createECDH("prime256v1");
  ecdh.generateKeys();
  const auth = randomBytes(16);
  const path = `/push/${randomBytes(6).toString("hex")}`;
  return {
    ecdh,
    auth,
    path,
    endpoint: `http://127.0.0.1:${SERVICE_PORT}${path}`,
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
  return JSON.parse(plain.subarray(0, -1).toString()) as Record<string, unknown>;
}

/** Stand in for the browser's push service and permission; the page's code is the real one. */
async function fakeBrowserPush(context: BrowserContext, device: Device): Promise<void> {
  await context.addInitScript(
    ({ endpoint, p256dh, auth }) => {
      const bytes = (text: string) =>
        Uint8Array.from(atob(text.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
      const subscription = {
        endpoint,
        get options() {
          const key = localStorage.getItem("test.push.key");
          return { applicationServerKey: key ? bytes(key).buffer : null };
        },
        getKey: (name: string) => bytes(name === "p256dh" ? p256dh : auth).buffer,
        unsubscribe: async () => {
          localStorage.removeItem("test.push.subscribed");
          return true;
        },
      };
      PushManager.prototype.subscribe = async function (options) {
        const key = new Uint8Array(options!.applicationServerKey as ArrayBuffer);
        localStorage.setItem(
          "test.push.key",
          btoa(String.fromCharCode(...key)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""),
        );
        localStorage.setItem("test.push.subscribed", "1");
        return subscription as unknown as PushSubscription;
      };
      PushManager.prototype.getSubscription = async function () {
        return (localStorage.getItem("test.push.subscribed")
          ? subscription
          : null) as unknown as PushSubscription;
      };
      const seen = { shown: 0 };
      (window as unknown as { pushSeen: typeof seen }).pushSeen = seen;
      class Granted extends window.Notification {
        constructor(title: string, options?: NotificationOptions) {
          super(title, options);
          seen.shown++;
        }
        static override get permission(): NotificationPermission {
          return (localStorage.getItem("test.push.permission") ?? "default") as NotificationPermission;
        }
        static override requestPermission = (async () => {
          localStorage.setItem("test.push.permission", "granted");
          return "granted";
        }) as typeof Notification.requestPermission;
      }
      window.Notification = Granted as typeof Notification;
    },
    { endpoint: device.endpoint, p256dh: device.p256dh, auth: device.authText },
  );
}

/** A browser of its own, signed in as `octo`, notifications on. Returns its page and token. */
async function browserOf(newContext: () => Promise<BrowserContext>, device: Device) {
  const context = await newContext();
  await fakeBrowserPush(context, device);
  const page = await context.newPage();
  let bearer = "";
  page.on("request", (r: Request) => {
    const auth = r.headers().authorization;
    if (auth && r.url().endsWith("/mcp")) bearer = auth;
  });
  await page.goto("/");
  await page.getByRole("link", { name: "Continue with GitHub" }).click();
  await page.getByRole("link", { name: "Sign in as octo" }).click();
  await page.waitForURL((url) => url.pathname === "/" && !url.searchParams.has("code"));
  await expectChartRendered(page);
  await page.getByRole("tab", { name: /^Alerts/ }).click();
  const control = page.locator('[data-role="push-control"]').first();
  await expect(control).toHaveAttribute("data-state", "off", { timeout: 15_000 });
  await control.getByRole("button", { name: "Enable notifications" }).click();
  await expect(control).toHaveAttribute("data-state", "on");
  return { context, page, token: () => bearer };
}

async function callTool(bearer: string, name: string, args: unknown): Promise<Record<string, unknown>> {
  const res = await fetch(MCP, {
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

/** Notifications this tab itself has shown: what it does about a fire it must tell of. */
const shown = (page: Page) =>
  page.evaluate(() => (window as unknown as { pushSeen: { shown: number } }).pushSeen.shown);
/** A level far below the sample price is simply greater, at once. */
const firesNow = {
  symbol: "BTCUSDT",
  condition: "greater",
  trigger: "once",
  level: 0.01,
  push: true,
};

test("a phone gets the alert made on the desktop, and each tab goes by its own device", async ({
  browser,
}, info) => {
  // Two cold sign-ins, rounds of the poll, and a failing push's retries.
  test.setTimeout(150_000);
  const newContext = () => browser.newContext({ baseURL: info.project.use.baseURL });
  const laptop = newDevice();
  const handset = newDevice();
  const desktop = await browserOf(newContext, laptop);
  const phone = await browserOf(newContext, handset);

  // Each browser has its own OAuth client, and they are one user with two devices.
  const stored = (page: Page) =>
    page.evaluate(() => localStorage.getItem("chartgen.oauth.client_id"));
  expect(await stored(desktop.page)).not.toBe(await stored(phone.page));
  const list = await callTool(desktop.token(), "list_server_alerts", {});
  expect((list.push as { devices: number }).devices).toBe(2);

  // Made on the desktop: both get it, each readable only with its own keys.
  const made = await callTool(desktop.token(), "create_server_alert", firesNow);
  await expect.poll(() => pushes.length, { timeout: 20_000 }).toBe(2);
  const byDevice = (d: Device) => pushes.find((p) => p.url === d.path);
  expect(decrypt(byDevice(handset)!.body, handset).alert_id).toBe(made.id);
  expect(decrypt(byDevice(laptop)!.body, laptop).alert_id).toBe(made.id);

  // Both got through, so neither tab repeats it.
  await desktop.page.waitForTimeout(11_000);
  expect(await shown(desktop.page)).toBe(0);
  expect(await shown(phone.page)).toBe(0);

  // The phone's push fails and the laptop's does not: only the phone's tab must tell.
  // With one outcome across devices, one of the two would be wrong.
  replies[handset.path] = 500;
  await callTool(desktop.token(), "create_server_alert", firesNow);
  await expect.poll(() => shown(phone.page), { timeout: 40_000 }).toBe(1);
  await desktop.page.waitForTimeout(6_000);
  expect(await shown(desktop.page)).toBe(0);

  // And the other way round.
  replies = { [laptop.path]: 500 };
  await callTool(desktop.token(), "create_server_alert", firesNow);
  await expect.poll(() => shown(desktop.page), { timeout: 40_000 }).toBe(1);
  await phone.page.waitForTimeout(6_000);
  expect(await shown(phone.page)).toBe(1);

  await desktop.context.close();
  await phone.context.close();
});
