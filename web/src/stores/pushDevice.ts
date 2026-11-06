// This browser's notifications for server alerts (ADR-0045). Whether it is on lives in
// the browser's push subscription; a copy is kept in storage so that the first report
// after a load already knows. Permission is only ever asked from a click.

import { createSignal } from "solid-js";

import {
  base64UrlToBytes,
  bytesToBase64Url,
  pushSupport,
  readPushEnv,
} from "../lib/pushDevice";
import { normalizeSymbol, setSymbol } from "./layout";
import {
  serverAlertError,
  serverPush,
  subscribePush,
  unsubscribePush,
} from "./serverAlerts";

const ON_KEY = "chartgen.push.on";

export type PushDeviceState = "unsupported" | "install" | "denied" | "off" | "on";

function loadOn(): boolean {
  try {
    return localStorage.getItem(ON_KEY) === "1";
  } catch {
    return false;
  }
}

const [on, setOnSignal] = createSignal(typeof localStorage !== "undefined" && loadOn());
const [denied, setDenied] = createSignal(
  typeof Notification !== "undefined" && Notification.permission === "denied",
);
const [busy, setBusy] = createSignal(false);
const [error, setError] = createSignal<string | null>(null);
export { busy as pushBusy, error as pushError };

function setOn(next: boolean): void {
  setOnSignal(next);
  try {
    if (next) localStorage.setItem(ON_KEY, "1");
    else localStorage.removeItem(ON_KEY);
  } catch {
    // Not kept: the subscription itself is read again on the next load.
  }
}

/** Does this browser hold a subscription the server knows about? */
export const pushOnHere = (): boolean => on();

export function pushDeviceState(): PushDeviceState {
  const support = pushSupport(readPushEnv());
  if (support !== "ok") return support;
  if (denied()) return "denied";
  return on() ? "on" : "off";
}

function subscriptionArgs(sub: PushSubscription): {
  endpoint: string;
  p256dh: string;
  auth: string;
} {
  const key = (name: PushEncryptionKeyName) => {
    const value = sub.getKey(name);
    if (value === null) throw new Error("The browser gave no push keys.");
    return bytesToBase64Url(value);
  };
  return { endpoint: sub.endpoint, p256dh: key("p256dh"), auth: key("auth") };
}

/** A subscription for `publicKey`, replacing one made for another server key. */
async function subscribed(reg: ServiceWorkerRegistration, publicKey: string) {
  const existing = await reg.pushManager.getSubscription();
  const held = existing?.options.applicationServerKey;
  if (existing && held && bytesToBase64Url(held) === publicKey) return existing;
  await existing?.unsubscribe();
  return reg.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: base64UrlToBytes(publicKey),
  });
}

/** From a click: the only place that asks for permission. */
export async function enablePush(): Promise<boolean> {
  const publicKey = serverPush()?.public_key;
  if (!publicKey) return false;
  setBusy(true);
  setError(null);
  try {
    const permission = await Notification.requestPermission();
    setDenied(permission === "denied");
    if (permission !== "granted") return false;
    const reg = await navigator.serviceWorker.register("/sw.js");
    await navigator.serviceWorker.ready;
    await subscribePush(subscriptionArgs(await subscribed(reg, publicKey)));
    setOn(true);
    return true;
  } catch (err) {
    setError(serverAlertError(err));
    return false;
  } finally {
    setBusy(false);
  }
}

/** Stop here first: if the server cannot be told, its next push answers 410. */
export async function disablePush(): Promise<void> {
  setBusy(true);
  setError(null);
  try {
    const reg = await navigator.serviceWorker.getRegistration("/");
    const sub = await reg?.pushManager.getSubscription();
    await sub?.unsubscribe();
    setOn(false);
    if (sub) await unsubscribePush(sub.endpoint).catch(() => {});
  } catch (err) {
    setError(serverAlertError(err));
  } finally {
    setBusy(false);
  }
}

let synced = false;

/**
 * Once per load, with no prompt: what this browser really holds, told to the server.
 * The browser may have dropped or replaced a subscription with chartgen closed, and only
 * a page can tell the server, so this is where that is noticed.
 */
export async function syncPushDevice(): Promise<void> {
  const publicKey = serverPush()?.public_key;
  if (synced || !publicKey || pushSupport(readPushEnv()) !== "ok") return;
  synced = true;
  setDenied(Notification.permission === "denied");
  try {
    const reg = await navigator.serviceWorker.getRegistration("/");
    const existing = await reg?.pushManager.getSubscription();
    if (!reg || !existing || Notification.permission !== "granted") {
      setOn(false);
      return;
    }
    await subscribePush(subscriptionArgs(await subscribed(reg, publicKey)));
    setOn(true);
  } catch {
    // Not reachable just now: what was known stands.
  }
}

/** Clicking a notification with a tab open: that tab shows the symbol. */
export function installPushClicks(): () => void {
  if (typeof navigator === "undefined" || !("serviceWorker" in navigator)) return () => {};
  const onMessage = (e: MessageEvent<unknown>) => {
    const data = e.data as { type?: unknown; symbol?: unknown } | null;
    if (data?.type !== "chartgen.open-symbol") return;
    const symbol = normalizeSymbol(data.symbol);
    if (symbol !== null) setSymbol(symbol);
  };
  navigator.serviceWorker.addEventListener("message", onMessage);
  return () => navigator.serviceWorker.removeEventListener("message", onMessage);
}
