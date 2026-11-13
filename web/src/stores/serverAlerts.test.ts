import { beforeEach, describe, expect, it, vi } from "vitest";

import { ServerAlertChangePayloadSchema, ServerAlertsPayloadSchema } from "../types/responses";
import type { ServerAlert } from "../types/responses";
import {
  editsInDialog,
  newFires,
  serverAlertError,
  serverAlertStatus,
  serverAlertText,
} from "./serverAlerts";

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
  push: false,
  last_push_at: null,
  last_push_status: null,
  last_push_error: null,
  last_push_devices: {},
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
    const rsi = { name: "rsi", params: { length: 14 }, line: "RSI", compare: "level" as const, value: 31.2 };
    expect(serverAlertText({ ...alert, condition: "less", level: 25, indicator: rsi })).toBe(
      "BTCUSDT RSI less than 25",
    );
    const ema = { ...rsi, name: "ma", line: "EMA 50", compare: "close" as const };
    expect(serverAlertText({ ...alert, condition: "crossing_up", level: null, indicator: ema })).toBe(
      "BTCUSDT close crossing up EMA 50",
    );
    // An alert at every close of its interval, as an agent makes one (ADR-0069).
    const atClose = { ...alert, interval: "4h", evaluate_on: "close" as const, condition: "bar_close" as const, level: null };
    expect(serverAlertText(atClose)).toBe("BTCUSDT 4h bar close");
    expect(ServerAlertsPayloadSchema.parse({ enabled: true, alerts: [atClose] }).alerts[0]?.condition).toBe(
      "bar_close",
    );
    // The dialog edits price alerts only.
    expect([alert, atClose, { ...alert, condition: "less" as const, indicator: rsi }].map(editsInDialog)).toEqual([
      true,
      false,
      false,
    ]);
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
    // An indicator alert, as `ServerAlerts::view` shows one that compares the close.
    const onEma = {
      ...alert,
      evaluate_on: "close",
      level: null,
      indicator: { name: "ma", params: { type: "ema", length: 50 }, line: "EMA 50", compare: "close", value: null },
    };
    expect(ServerAlertsPayloadSchema.parse({ enabled: true, alerts: [onEma] }).alerts[0]?.indicator?.line).toBe(
      "EMA 50",
    );
    expect(
      ServerAlertsPayloadSchema.safeParse({
        enabled: true,
        alerts: [{ ...onEma, indicator: { ...onEma.indicator, compare: "above" } }],
      }).success,
    ).toBe(false);
    expect(
      ServerAlertsPayloadSchema.safeParse({
        enabled: true,
        alerts: [{ ...alert, condition: "sideways" }],
      }).success,
    ).toBe(false);
  });
});

describe("asking the server", () => {
  const LIST = "chartgen.serverAlerts.list";

  function stubStorage(initial: Record<string, string> = {}, opts = { failWrites: false }) {
    const data = new Map(Object.entries(initial));
    vi.stubGlobal("localStorage", {
      getItem: (k: string) => data.get(k) ?? null,
      setItem: (k: string, v: string) => {
        if (opts.failWrites) throw new Error("quota");
        data.set(k, v);
      },
      removeItem: (k: string) => void data.delete(k),
    });
    return data;
  }

  /** The store reads storage at import time. */
  async function load() {
    vi.resetModules();
    return import("./serverAlerts");
  }

  function session(listed: boolean, payload: unknown = { enabled: true, alerts: [] }) {
    return {
      serverAlertsListed: vi.fn(async () => listed),
      callListServerAlerts: vi.fn(async () => payload),
    };
  }

  beforeEach(() => {
    vi.unstubAllGlobals();
  });

  it("asks only tools/list while the tool is not listed", async () => {
    stubStorage();
    const store = await load();
    const s = session(false);
    store.setServerAlertSession(s as never);
    expect(await store.refreshServerAlerts()).toBe(false);
    expect(await store.refreshServerAlerts()).toBe(false);
    expect(s.serverAlertsListed).toHaveBeenCalledTimes(2);
    expect(s.callListServerAlerts).not.toHaveBeenCalled();
    expect(store.serverAlertsEnabled()).toBe(false);
  });

  it("stops polling when the server says it is off, even if the tool is listed", async () => {
    stubStorage();
    const store = await load();
    const s = session(true, { enabled: false, alerts: [] });
    store.setServerAlertSession(s as never);
    expect(await store.refreshServerAlerts()).toBe(false);
    expect(store.serverAlertsEnabled()).toBe(false);
  });

  it("polls the list while on, without asking for the tool again, and shares it", async () => {
    const data = stubStorage();
    const store = await load();
    const payload = { enabled: true, can_create: true, webhooks: false, alerts: [alert] };
    const s = session(true, payload);
    store.setServerAlertSession(s as never);
    expect(await store.refreshServerAlerts()).toBe(true);
    expect(await store.refreshServerAlerts()).toBe(true);
    expect(s.serverAlertsListed).toHaveBeenCalledTimes(1);
    expect(s.callListServerAlerts).toHaveBeenCalledTimes(2);
    expect(store.serverAlertsCanCreate()).toBe(true);
    expect(store.serverWebhooksAllowed()).toBe(false);
    expect(JSON.parse(data.get(LIST)!).alerts[0].id).toBe("a");
  });

  it("a tab that does not ask reads what the leading tab shared, validated", async () => {
    stubStorage({ [LIST]: JSON.stringify({ enabled: true, can_create: true, alerts: [alert] }) });
    const store = await load();
    expect(store.serverAlerts().map((a) => a.id)).toEqual(["a"]);
    expect(store.serverAlertsEnabled()).toBe(true);

    stubStorage({ [LIST]: JSON.stringify({ enabled: true, alerts: [{ id: "bad" }] }) });
    const bad = await load();
    expect(bad.serverAlerts()).toEqual([]);
    expect(bad.serverAlertsEnabled()).toBe(false);
  });

  it("keeps what it knows when the server cannot answer", async () => {
    stubStorage({ [LIST]: JSON.stringify({ enabled: true, alerts: [alert] }) });
    const store = await load();
    const s = session(true);
    s.callListServerAlerts.mockRejectedValue(new Error("down"));
    store.setServerAlertSession(s as never);
    expect(await store.refreshServerAlerts()).toBe(true);
    expect(store.serverAlerts()).toHaveLength(1);
  });

  it("reports a fire once even when storage refuses to keep the count", async () => {
    const fired = { ...alert, fire_count: 1 };
    stubStorage({ [LIST]: JSON.stringify({ enabled: true, alerts: [fired] }) }, { failWrites: true });
    const store = await load();
    expect(store.takeNewFires().map((a) => a.id)).toEqual(["a"]);
    expect(store.takeNewFires()).toEqual([]);
  });

  describe("with push on this device", () => {
    const FIRED = 1_000_000;
    const fired = { ...alert, fire_count: 1, push: true, last_triggered_at: FIRED };
    const open = (a: ServerAlert) => {
      const data = stubStorage({ [LIST]: JSON.stringify({ enabled: true, alerts: [a] }) });
      return { data, store: load() };
    };

    it("says nothing about a fire the push reported, and marks it told", async () => {
      const { data, store } = open({ ...fired, last_push_at: FIRED + 1 });
      const s = await store;
      expect(s.takeNewFires(true, FIRED + 2)).toEqual([]);
      expect(JSON.parse(data.get("chartgen.serverAlerts.seen")!)).toEqual({ a: 1 });
      expect(s.takeNewFires(true, FIRED + 3)).toEqual([]);
    });

    it("waits for the outcome, then reports a push that failed, once", async () => {
      const { data, store } = open(fired);
      const s = await store;
      expect(s.takeNewFires(true, FIRED + 1)).toEqual([]);
      expect(JSON.parse(data.get("chartgen.serverAlerts.seen")!)).toEqual({ a: 0 });
      const failed = { ...fired, last_push_at: FIRED + 2, last_push_error: "HTTP 500" };
      data.set(LIST, JSON.stringify({ enabled: true, alerts: [failed] }));
      const again = await load();
      expect(again.takeNewFires(true, FIRED + 3).map((a) => a.id)).toEqual(["a"]);
      expect(again.takeNewFires(true, FIRED + 4)).toEqual([]);
    });

    it("reports as before when this device has push off", async () => {
      const { store } = open({ ...fired, last_push_at: FIRED + 1 });
      expect((await store).takeNewFires(false).map((a) => a.id)).toEqual(["a"]);
    });
  });

  it("reports a fire once across tabs through storage", async () => {
    const fired = { ...alert, fire_count: 1 };
    const data = stubStorage({ [LIST]: JSON.stringify({ enabled: true, alerts: [fired] }) });
    const first = await load();
    expect(first.takeNewFires()).toHaveLength(1);
    expect(JSON.parse(data.get("chartgen.serverAlerts.seen")!)).toEqual({ a: 1 });
    const second = await load();
    expect(second.takeNewFires()).toEqual([]);
  });
});
