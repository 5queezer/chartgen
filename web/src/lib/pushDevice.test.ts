import { describe, expect, it } from "vitest";

import type { ServerAlert } from "../types/responses";
import {
  base64UrlToBytes,
  bytesToBase64Url,
  pushReport,
  pushSupport,
  type PushEnv,
} from "./pushDevice";

const DESKTOP: PushEnv = {
  serviceWorker: true,
  pushManager: true,
  notification: true,
  ios: false,
  standalone: false,
};

describe("pushSupport", () => {
  it("is fine on a desktop or Android browser with the APIs", () => {
    expect(pushSupport(DESKTOP)).toBe("ok");
  });

  it("says what to do on iOS when not installed, even with the APIs there", () => {
    expect(pushSupport({ ...DESKTOP, ios: true })).toBe("install");
    expect(
      pushSupport({ ...DESKTOP, ios: true, serviceWorker: false, pushManager: false }),
    ).toBe("install");
  });

  it("works on iOS once installed", () => {
    expect(pushSupport({ ...DESKTOP, ios: true, standalone: true })).toBe("ok");
  });

  it("is unsupported when any API is missing", () => {
    for (const missing of ["serviceWorker", "pushManager", "notification"] as const) {
      expect(pushSupport({ ...DESKTOP, [missing]: false })).toBe("unsupported");
    }
  });
});

describe("base64url", () => {
  it("round-trips bytes that plain base64 would write with + and /", () => {
    const bytes = new Uint8Array([251, 255, 254, 0, 1]);
    const text = bytesToBase64Url(bytes);
    expect(text).toBe("-__-AAE");
    expect([...base64UrlToBytes(text)]).toEqual([...bytes]);
  });

  it("reads an ArrayBuffer as a browser's getKey gives it", () => {
    expect(bytesToBase64Url(new Uint8Array([1, 2, 3]).buffer)).toBe("AQID");
  });
});

describe("pushReport", () => {
  const FIRED = 1_000_000;
  const alert = (over: Partial<ServerAlert>): ServerAlert =>
    ({
      push: true,
      last_triggered_at: FIRED,
      last_push_at: FIRED + 500,
      last_push_error: null,
      ...over,
    }) as ServerAlert;

  it("leaves the tab to report without push on this device, or on the alert", () => {
    expect(pushReport(alert({}), false, FIRED)).toBe("report");
    expect(pushReport(alert({ push: false }), true, FIRED)).toBe("report");
  });

  it("stays quiet once the push got through", () => {
    expect(pushReport(alert({}), true, FIRED + 1000)).toBe("quiet");
  });

  it("waits while the server has no outcome for this fire", () => {
    expect(pushReport(alert({ last_push_at: null }), true, FIRED)).toBe("wait");
    expect(pushReport(alert({ last_push_at: FIRED - 1 }), true, FIRED)).toBe("wait");
  });

  it("reports when the push failed, or no outcome can come any more", () => {
    expect(pushReport(alert({ last_push_error: "HTTP 500" }), true, FIRED + 1000)).toBe("report");
    expect(pushReport(alert({ last_push_at: null }), true, FIRED + 3_600_001)).toBe("report");
  });

  it("reports a fire first seen after the message's life, though the service took it", () => {
    expect(pushReport(alert({}), true, FIRED + 3_600_000)).toBe("quiet");
    expect(pushReport(alert({}), true, FIRED + 3_600_001)).toBe("report");
  });
});
