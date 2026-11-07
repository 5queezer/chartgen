// What this browser can do about notifications with chartgen closed (ADR-0045), and
// who tells the user about a fire. Pure, so that every state can be tested.

import type { ServerAlert } from "../types/responses";

export type PushSupport = "ok" | "unsupported" | "install";

export interface PushEnv {
  serviceWorker: boolean;
  pushManager: boolean;
  notification: boolean;
  ios: boolean;
  standalone: boolean;
}

/** iOS only allows push for a web app on the Home Screen, so there it can be fixed. */
export function pushSupport(env: PushEnv): PushSupport {
  if (env.ios && !env.standalone) return "install";
  return env.serviceWorker && env.pushManager && env.notification ? "ok" : "unsupported";
}

export function readPushEnv(): PushEnv {
  if (typeof window === "undefined" || typeof navigator === "undefined") {
    return { serviceWorker: false, pushManager: false, notification: false, ios: false, standalone: false };
  }
  const nav = navigator as Navigator & { standalone?: boolean };
  return {
    serviceWorker: "serviceWorker" in navigator,
    pushManager: "PushManager" in window,
    notification: "Notification" in window,
    // iPadOS reports itself as a Mac, one with a touch screen.
    ios:
      /iPad|iPhone|iPod/.test(navigator.userAgent) ||
      (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1),
    standalone: nav.standalone === true || window.matchMedia?.("(display-mode: standalone)").matches,
  };
}

export function base64UrlToBytes(text: string): Uint8Array<ArrayBuffer> {
  const normal = text.replace(/-/g, "+").replace(/_/g, "/");
  const raw = atob(normal.padEnd(normal.length + ((4 - (normal.length % 4)) % 4), "="));
  return Uint8Array.from(raw, (c) => c.charCodeAt(0));
}

export function bytesToBase64Url(bytes: ArrayBuffer | ArrayBufferView): string {
  const view =
    bytes instanceof ArrayBuffer
      ? new Uint8Array(bytes)
      : new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return btoa(String.fromCharCode(...view))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

/**
 * How long a push service keeps a message for a device that is off, and how long a
 * delivery waits on the server (`TTL_SECS`, `EVENT_MAX_AGE_SECS`).
 */
const PUSH_MAX_AGE_MS = 3_600_000;

/**
 * Who tells the user about a fire of `a`. An alert that notifies this device has one
 * voice, the service worker, which always shows its notification. So the tab says
 * nothing about it, unless the push did not get through, and until the server knows, it
 * waits rather than risk both. A service accepting a push is not the device getting it,
 * and one that was off for longer than the message lives never will: a fire first seen
 * that late is reported here, because a second report beats a silent alarm.
 *
 * `device` is this browser's id with the server. One user can have several, which fare
 * differently, so this one's outcome is what counts: a device the fire did not go to has
 * no push coming, and one still waiting has its answer pending. Not knowing the id yet,
 * the last outcome across devices is all there is to go by.
 */
export function pushReport(
  a: ServerAlert,
  pushOnHere: boolean,
  now: number,
  device: string | null = null,
): "report" | "quiet" | "wait" {
  if (!pushOnHere || !a.push) return "report";
  const fired = a.last_triggered_at;
  if (fired === null || now - fired > PUSH_MAX_AGE_MS) return "report";
  if (device !== null) {
    const mine = a.last_push_devices[device];
    if (mine === undefined) return "report";
    if (mine === null) return "wait";
    return mine.error === null ? "quiet" : "report";
  }
  if (a.last_push_at === null || a.last_push_at < fired) return "wait";
  return a.last_push_error === null ? "quiet" : "report";
}
