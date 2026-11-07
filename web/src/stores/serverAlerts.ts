// Alerts that run on the server (ADR-0042). The server is the source of truth,
// so they are not in the browser's own list (`stores/alerts`) and no tab can
// evaluate them or fire them a second time. The tab that leads asks the server, and
// shares the answer with the other tabs through storage. It also remembers there which
// fires the user was already told about, so one fire is reported once, in whichever
// tab leads. A server that does not run them is asked nothing but `tools/list`.

import { createSignal } from "solid-js";

import { CONDITION_LABELS } from "../lib/alerts";
import { pushReport } from "../lib/pushDevice";
import { formatPrice } from "../lib/quote";
import type { McpSession } from "../mcp";
import type {
  CreateServerAlertInput,
  SubscribePushInput,
  UpdateServerAlertInput,
} from "../types/generated-input";
import {
  ServerAlertsPayloadSchema,
  type ServerAlert,
  type ServerAlertChangePayload,
  type ServerAlertsPayload,
} from "../types/responses";

const SEEN_KEY = "chartgen.serverAlerts.seen";
const LIST_KEY = "chartgen.serverAlerts.list";
const DEVICE_KEY = "chartgen.push.device";
const OFF: ServerAlertsPayload = { enabled: false, alerts: [] };

export type ServerAlertStatus = "active" | "triggered" | "paused";

/** As `alertStatus` of a browser alert. */
export function serverAlertStatus(a: ServerAlert): ServerAlertStatus {
  return a.active ? "active" : a.last_triggered_at !== null ? "triggered" : "paused";
}

export function serverAlertText(a: ServerAlert): string {
  const head = `${a.symbol} ${CONDITION_LABELS[a.condition].toLowerCase()} ${formatPrice(a.level)}`;
  return a.message ? `${head} — ${a.message}` : head;
}

/** The alerts in `list` that fired since `seen` (fire counts per id). */
export function newFires(
  list: readonly ServerAlert[],
  seen: Readonly<Record<string, number>>,
): ServerAlert[] {
  return list.filter((a) => a.fire_count > (seen[a.id] ?? 0));
}

/** What the leading tab last shared; validated, as anything read back is. */
function loadShared(): ServerAlertsPayload {
  if (typeof localStorage === "undefined") return OFF;
  try {
    const parsed = ServerAlertsPayloadSchema.safeParse(
      JSON.parse(localStorage.getItem(LIST_KEY) ?? "null"),
    );
    return parsed.success ? parsed.data : OFF;
  } catch {
    return OFF;
  }
}

const [shared, setShared] = createSignal<ServerAlertsPayload>(loadShared());
export const serverAlerts = (): ServerAlert[] => shared().alerts;
export const serverAlertsEnabled = (): boolean => shared().enabled;
/** May this sign-in put an alert on the server? */
export const serverAlertsCanCreate = (): boolean => shared().can_create === true;
/** Does the server send webhooks for this sign-in? */
export const serverWebhooksAllowed = (): boolean => shared().webhooks === true;
/** What it takes to notify this sign-in's devices; null where the server will not. */
export const serverPush = (): ServerAlertsPayload["push"] => shared().push ?? null;

if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key === LIST_KEY || e.key === null) setShared(loadShared());
  });
}

function publish(next: ServerAlertsPayload): void {
  setShared(next);
  try {
    localStorage.setItem(LIST_KEY, JSON.stringify(next));
  } catch {
    // Quota exceeded or private-mode storage: other tabs keep what they have.
  }
}

let session: McpSession | null = null;
export function setServerAlertSession(s: McpSession): void {
  session = s;
}

/** Counts told about in this tab, for when storage fails to keep them. */
const toldHere: Record<string, number> = {};

function readSeen(): Record<string, number> {
  let stored: Record<string, number> = {};
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(SEEN_KEY) ?? "{}");
    if (typeof raw === "object" && raw !== null) stored = raw as Record<string, number>;
  } catch {
    // As if nothing was stored; `toldHere` still holds this tab's.
  }
  const seen = { ...stored };
  for (const [id, n] of Object.entries(toldHere)) seen[id] = Math.max(n, seen[id] ?? 0);
  return seen;
}

/**
 * The fires this tab should tell the user about, marked as told; those a push reports
 * are marked too, and those it may yet report stay for the next look (`pushReport`).
 * Read fresh each time: another tab may have led until now. Ids that are gone are
 * forgotten.
 */
export function takeNewFires(pushOnHere = false, now = Date.now()): ServerAlert[] {
  const list = serverAlerts();
  const seen = readSeen();
  const pending = newFires(list, seen);
  const device = thisPushDevice();
  const report = new Map(pending.map((a) => [a.id, pushReport(a, pushOnHere, now, device)]));
  const fired = pending.filter((a) => report.get(a.id) === "report");
  const told = list.map((a) => [
    a.id,
    report.get(a.id) === "wait" ? (seen[a.id] ?? 0) : a.fire_count,
  ]) as [string, number][];
  for (const [id, count] of told) toldHere[id] = count;
  try {
    localStorage.setItem(SEEN_KEY, JSON.stringify(Object.fromEntries(told)));
  } catch {
    // Quota exceeded or private-mode storage: `toldHere` keeps this tab from repeating.
  }
  return fired;
}

/**
 * Ask the server, and share what it says. True while it runs alerts, so polling goes
 * on. While they are off only `tools/list` is asked, which has no cost to speak of;
 * when it could not answer, what was known stands.
 */
export async function refreshServerAlerts(): Promise<boolean> {
  if (!session) return false;
  try {
    if (!serverAlertsEnabled() && !(await session.serverAlertsListed())) {
      publish(OFF);
      return false;
    }
    const payload = await session.callListServerAlerts();
    publish(payload);
    return payload.enabled;
  } catch {
    return serverAlertsEnabled();
  }
}

function connected(): McpSession {
  if (!session) throw new Error("Not connected yet.");
  return session;
}

/** The server's reason, without the transport's prefix. */
export function serverAlertError(err: unknown): string {
  const text = err instanceof Error ? err.message : String(err);
  return text.replace(/^MCP tool error: /, "");
}

/** Put the alert in the list as the server now has it. The secret stays out. */
function merge(a: ServerAlertChangePayload): void {
  const { webhook_secret: _, ...alert } = a;
  const now = shared();
  publish({
    ...now,
    alerts: now.alerts.some((x) => x.id === alert.id)
      ? now.alerts.map((x) => (x.id === alert.id ? alert : x))
      : [...now.alerts, alert],
  });
}

export async function createServerAlert(
  args: CreateServerAlertInput,
): Promise<ServerAlertChangePayload> {
  const created = await connected().callCreateServerAlert(args);
  merge(created);
  return created;
}

export async function updateServerAlert(
  args: UpdateServerAlertInput,
): Promise<ServerAlertChangePayload> {
  const updated = await connected().callUpdateServerAlert(args);
  merge(updated);
  return updated;
}

/** This browser's id with the server, as `subscribe_push` last answered. */
function thisPushDevice(): string | null {
  try {
    return localStorage.getItem(DEVICE_KEY);
  } catch {
    return null;
  }
}

function rememberPushDevice(device: string | null): void {
  try {
    if (device === null) localStorage.removeItem(DEVICE_KEY);
    else localStorage.setItem(DEVICE_KEY, device);
  } catch {
    // Private-mode storage: the last outcome across devices stands in.
  }
}

/** Tell the server about this browser, and keep the count and id it answers. */
export async function subscribePush(args: SubscribePushInput): Promise<void> {
  const { devices, device } = await connected().callSubscribePush(args);
  rememberPushDevice(device);
  const now = shared();
  if (now.push) publish({ ...now, push: { ...now.push, devices } });
}

export async function unsubscribePush(endpoint: string): Promise<void> {
  const { devices } = await connected().callUnsubscribePush({ endpoint });
  rememberPushDevice(null);
  const now = shared();
  if (now.push) publish({ ...now, push: { ...now.push, devices } });
}

export async function deleteServerAlert(id: string): Promise<void> {
  await connected().callDeleteServerAlert({ alert_id: id });
  const now = shared();
  publish({ ...now, alerts: now.alerts.filter((a) => a.id !== id) });
}
