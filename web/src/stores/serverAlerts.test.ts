import { describe, expect, it } from "vitest";

import { ServerAlertChangePayloadSchema, ServerAlertsPayloadSchema } from "../types/responses";
import type { ServerAlert } from "../types/responses";
import { newFires, serverAlertError, serverAlertStatus, serverAlertText } from "./serverAlerts";

const alert: ServerAlert = {
  id: "a",
  symbol: "BTCUSDT",
  interval: "1h",
  evaluate_on: "live",
  condition: "crossing",
  level: 100,
  trigger: "once",
  message: null,
  active: true,
  disarmed: false,
  created_at: 1,
  last_triggered_at: null,
  fire_count: 0,
  webhook: null,
  last_delivery_at: null,
  last_status: null,
  last_error: null,
  feed: "live",
  last_price: 99,
};

describe("newFires", () => {
  it("reports what fired since the seen counts", () => {
    const b = { ...alert, id: "b", fire_count: 2 };
    expect(newFires([alert, b], {}).map((x) => x.id)).toEqual(["b"]);
    expect(newFires([alert, b], { b: 1 }).map((x) => x.id)).toEqual(["b"]);
    expect(newFires([alert, b], { b: 2 })).toEqual([]);
  });
});

describe("serverAlertStatus", () => {
  it("reads like a browser alert's", () => {
    expect(serverAlertStatus(alert)).toBe("active");
    expect(serverAlertStatus({ ...alert, active: false })).toBe("paused");
    expect(serverAlertStatus({ ...alert, active: false, last_triggered_at: 5 })).toBe("triggered");
  });
});

describe("texts", () => {
  it("names the alert and strips the transport's prefix from errors", () => {
    expect(serverAlertText({ ...alert, message: "hi" })).toBe("BTCUSDT crossing 100.00 — hi");
    expect(serverAlertError(new Error("MCP tool error: webhook URL must use https"))).toBe(
      "webhook URL must use https",
    );
  });
});

describe("the wire shape", () => {
  it("parses what the server sends and refuses what it should not", () => {
    const { success } = ServerAlertsPayloadSchema.safeParse({ enabled: false, alerts: [] });
    expect(success).toBe(true);
    expect(ServerAlertsPayloadSchema.parse({ enabled: true, alerts: [alert] }).alerts).toHaveLength(1);
    expect(
      ServerAlertChangePayloadSchema.parse({ ...alert, webhook_secret: "s" }).webhook_secret,
    ).toBe("s");
    expect(ServerAlertsPayloadSchema.safeParse({ enabled: true }).success).toBe(false);
    expect(
      ServerAlertsPayloadSchema.safeParse({
        enabled: true,
        alerts: [{ ...alert, condition: "sideways" }],
      }).success,
    ).toBe(false);
  });
});
