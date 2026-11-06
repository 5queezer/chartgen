// Alerts that run on the server (ADR-0042). The server is the source of truth,
// so they are not in the browser's own list (`stores/alerts`) and no tab can
// evaluate them or fire them a second time. This store mirrors what the server
// says, and remembers which fires the user was already told about, in storage
// shared by the tabs, so one fire is reported once, in whichever tab leads.

import { createSignal } from "solid-js";

import { CONDITION_LABELS } from "../lib/alerts";
import { formatPrice } from "../lib/quote";
import type { McpSession } from "../mcp";
import type {
  CreateServerAlertInput,
  UpdateServerAlertInput,
} from "../types/generated-input";
import type { ServerAlert, ServerAlertChangePayload } from "../types/responses";

const SEEN_KEY = "chartgen.serverAlerts.seen";

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

const [serverAlerts, setServerAlerts] = createSignal<ServerAlert[]>([]);
export { serverAlerts };
const [serverAlertsEnabled, setServerAlertsEnabled] = createSignal(false);
export { serverAlertsEnabled };

let session: McpSession | null = null;
export function setServerAlertSession(s: McpSession): void {
  session = s;
}

function readSeen(): Record<string, number> {
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(SEEN_KEY) ?? "{}");
    return typeof raw === "object" && raw !== null ? (raw as Record<string, number>) : {};
  } catch {
    return {};
  }
}

/**
 * The fires nobody was told about yet, marked as told. Read fresh each time:
 * another tab may have led until now. Ids that are gone are forgotten.
 */
export function takeNewFires(): ServerAlert[] {
  const list = serverAlerts();
  const fired = newFires(list, readSeen());
  try {
    localStorage.setItem(
      SEEN_KEY,
      JSON.stringify(Object.fromEntries(list.map((a) => [a.id, a.fire_count]))),
    );
  } catch {
    // Quota exceeded or private-mode storage: best-effort only.
  }
  return fired;
}

/** Ask the server. False when it could not answer; the last list stays. */
export async function refreshServerAlerts(): Promise<boolean> {
  if (!session) return false;
  try {
    const payload = await session.callListServerAlerts();
    setServerAlertsEnabled(payload.enabled);
    setServerAlerts(payload.alerts);
    return true;
  } catch {
    return false;
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
  setServerAlerts((list) =>
    list.some((x) => x.id === alert.id)
      ? list.map((x) => (x.id === alert.id ? alert : x))
      : [...list, alert],
  );
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

export async function deleteServerAlert(id: string): Promise<void> {
  await connected().callDeleteServerAlert({ alert_id: id });
  setServerAlerts((list) => list.filter((a) => a.id !== id));
}
