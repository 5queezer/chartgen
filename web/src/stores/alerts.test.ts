import { beforeEach, describe, expect, it, vi } from "vitest";

function stubStorage(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial));
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => void data.set(k, v),
  });
  return data;
}

// The store reads localStorage at import time (see watchlist.test.ts).
async function load() {
  vi.resetModules();
  return import("./alerts");
}

const KEY = "chartgen.alerts.list";
const valid = {
  id: "a1",
  symbol: "btcusdt",
  condition: "crossing",
  price: 100,
  trigger: "once",
  active: true,
  createdAt: 1,
};

beforeEach(() => {
  vi.unstubAllGlobals();
});

describe("sanitizeAlerts", () => {
  it("normalizes symbols and drops invalid entries and duplicate ids", async () => {
    stubStorage();
    const { sanitizeAlerts } = await load();
    const out = sanitizeAlerts([
      valid,
      { ...valid }, // duplicate id
      { ...valid, id: "bad-cond", condition: "nope" },
      { ...valid, id: "bad-price", price: -1 },
      { ...valid, id: "nan", price: Number.NaN },
      { ...valid, id: "bad-sym", symbol: "!!" },
      { ...valid, id: "bad-trigger", trigger: "twice" },
      { ...valid, id: "bad-active", active: "yes" },
      "junk",
      null,
    ]);
    expect(out).toHaveLength(1);
    expect(out?.[0]).toMatchObject({ id: "a1", symbol: "BTCUSDT" });
    expect(sanitizeAlerts({})).toBeNull();
  });

  it("keeps optional fields only when valid", async () => {
    stubStorage();
    const { sanitizeAlerts } = await load();
    const [a, b] = sanitizeAlerts([
      { ...valid, message: "hi", lastTriggeredAt: 7 },
      { ...valid, id: "a2", message: 5, lastTriggeredAt: "x" },
    ])!;
    expect(a).toMatchObject({ message: "hi", lastTriggeredAt: 7 });
    expect(b).not.toHaveProperty("message");
    expect(b).not.toHaveProperty("lastTriggeredAt");
  });

  it("caps at MAX_ALERTS", async () => {
    stubStorage();
    const { sanitizeAlerts, MAX_ALERTS } = await load();
    const many = Array.from({ length: MAX_ALERTS + 10 }, (_, i) => ({
      ...valid,
      id: `id${i}`,
    }));
    expect(sanitizeAlerts(many)).toHaveLength(MAX_ALERTS);
  });
});

describe("alerts store", () => {
  it("starts empty on missing or corrupt storage", async () => {
    stubStorage();
    expect((await load()).alerts()).toEqual([]);
    stubStorage({ [KEY]: "{not json" });
    expect((await load()).alerts()).toEqual([]);
  });

  it("loads valid entries from storage", async () => {
    stubStorage({ [KEY]: JSON.stringify([valid, { id: "x" }]) });
    const s = await load();
    expect(s.alerts().map((a) => a.id)).toEqual(["a1"]);
  });

  it("adds, persists, updates, pauses and removes", async () => {
    const data = stubStorage();
    const s = await load();
    const a = s.addAlert({
      symbol: "aapl",
      condition: "greater",
      price: 10,
      trigger: "every",
      message: "  look  ",
    })!;
    expect(a).toMatchObject({ symbol: "AAPL", active: true, message: "look" });
    expect(JSON.parse(data.get(KEY)!)).toHaveLength(1);

    s.updateAlert(a.id, { price: 12, message: "" });
    expect(s.alerts()[0]).toMatchObject({ price: 12 });
    expect(s.alerts()[0]).not.toHaveProperty("message");

    s.setAlertActive(a.id, false);
    expect(s.alerts()[0]!.active).toBe(false);
    s.moveAlert(a.id, 13);
    expect(s.alerts()[0]).toMatchObject({ price: 13, active: false });

    s.removeAlert(a.id);
    expect(s.alerts()).toEqual([]);
    expect(JSON.parse(data.get(KEY)!)).toEqual([]);
  });

  it("rejects invalid prices and the cap", async () => {
    stubStorage();
    const s = await load();
    const input = {
      symbol: "AAPL",
      condition: "crossing" as const,
      price: 1,
      trigger: "once" as const,
    };
    expect(s.addAlert({ ...input, price: 0 })).toBeNull();
    expect(s.addAlert({ ...input, symbol: "!" })).toBeNull();
    for (let i = 0; i < s.MAX_ALERTS; i++) s.addAlert(input);
    expect(s.addAlert(input)).toBeNull();
    expect(s.alerts()).toHaveLength(s.MAX_ALERTS);
  });
});

describe("processPrice", () => {
  const input = {
    symbol: "AAPL",
    condition: "crossing" as const,
    price: 100,
    trigger: "once" as const,
  };

  it("fires a once alert on the cross and deactivates it", async () => {
    stubStorage();
    const s = await load();
    s.addAlert(input);
    expect(s.processPrice("AAPL", 99, 1)).toEqual([]);
    const fired = s.processPrice("AAPL", 101, 2);
    expect(fired).toHaveLength(1);
    expect(s.alerts()[0]).toMatchObject({ active: false, lastTriggeredAt: 2 });
    expect(s.processPrice("AAPL", 99, 3)).toEqual([]);
  });

  it("ignores other symbols but remembers their prices", async () => {
    stubStorage();
    const s = await load();
    s.addAlert(input);
    s.processPrice("MSFT", 150, 1);
    expect(s.lastPrice("MSFT")).toBe(150);
    expect(s.lastPrice("AAPL")).toBeNull();
    expect(s.alerts()[0]!.active).toBe(true);
  });

  it("does not fire a crossing without a previous price", async () => {
    stubStorage();
    const s = await load();
    s.addAlert(input);
    expect(s.processPrice("AAPL", 101, 1)).toEqual([]);
  });

  it("every-time crossing fires on each cross and stays active", async () => {
    stubStorage();
    const s = await load();
    s.addAlert({ ...input, trigger: "every" });
    s.processPrice("AAPL", 99);
    expect(s.processPrice("AAPL", 101)).toHaveLength(1);
    expect(s.processPrice("AAPL", 102)).toEqual([]);
    expect(s.processPrice("AAPL", 99)).toHaveLength(1);
    expect(s.alerts()[0]!.active).toBe(true);
  });

  it("every-time greater fires once, then re-arms below the level", async () => {
    stubStorage();
    const s = await load();
    s.addAlert({ ...input, condition: "greater", trigger: "every" });
    expect(s.processPrice("AAPL", 101)).toHaveLength(1);
    expect(s.processPrice("AAPL", 102)).toEqual([]);
    expect(s.processPrice("AAPL", 103)).toEqual([]);
    // Exactly on the level re-arms (the condition is strict).
    expect(s.processPrice("AAPL", 100)).toEqual([]);
    expect(s.processPrice("AAPL", 101)).toHaveLength(1);
  });

  it("every-time less mirrors greater", async () => {
    stubStorage();
    const s = await load();
    s.addAlert({ ...input, condition: "less", trigger: "every" });
    expect(s.processPrice("AAPL", 99)).toHaveLength(1);
    expect(s.processPrice("AAPL", 98)).toEqual([]);
    expect(s.processPrice("AAPL", 101)).toEqual([]);
    expect(s.processPrice("AAPL", 99)).toHaveLength(1);
  });

  it("resuming or editing an alert re-arms it", async () => {
    stubStorage();
    const s = await load();
    const a = s.addAlert({ ...input, condition: "greater", trigger: "every" })!;
    expect(s.processPrice("AAPL", 101)).toHaveLength(1);
    expect(s.processPrice("AAPL", 102)).toEqual([]);
    s.updateAlert(a.id, { price: 101 });
    expect(s.processPrice("AAPL", 102)).toHaveLength(1);
  });

  it("skips paused alerts", async () => {
    stubStorage();
    const s = await load();
    const a = s.addAlert({ ...input, condition: "greater" })!;
    s.setAlertActive(a.id, false);
    expect(s.processPrice("AAPL", 101)).toEqual([]);
    expect(s.activeAlertSymbols()).toEqual([]);
  });

  it("ignores non-finite prices", async () => {
    stubStorage();
    const s = await load();
    s.addAlert({ ...input, condition: "less" });
    expect(s.processPrice("AAPL", Number.NaN)).toEqual([]);
    expect(s.lastPrice("AAPL")).toBeNull();
  });
});

describe("persisted re-arm state", () => {
  const input = {
    symbol: "AAPL",
    condition: "greater" as const,
    price: 100,
    trigger: "every" as const,
  };

  it("a fired every-time greater alert stays disarmed across a reload", async () => {
    const data = stubStorage();
    const s = await load();
    s.addAlert(input);
    expect(s.processPrice("AAPL", 101)).toHaveLength(1);
    expect(s.alerts()[0]!.disarmed).toBe(true);

    // Reload: the condition still holds, so it must not fire again.
    const reloaded = await load();
    expect(reloaded.alerts()[0]!.disarmed).toBe(true);
    expect(reloaded.processPrice("AAPL", 102)).toEqual([]);
    expect(reloaded.processPrice("AAPL", 99)).toEqual([]);
    expect(reloaded.alerts()[0]!.disarmed).toBeUndefined();
    expect(JSON.parse(data.get(KEY)!)[0].disarmed).toBeUndefined();
    expect(reloaded.processPrice("AAPL", 101)).toHaveLength(1);
  });

  it("sanitize keeps disarmed only when true", async () => {
    stubStorage();
    const { sanitizeAlerts } = await load();
    const [a, b] = sanitizeAlerts([
      { ...valid, disarmed: true },
      { ...valid, id: "a2", disarmed: "yes" },
    ])!;
    expect(a!.disarmed).toBe(true);
    expect(b).not.toHaveProperty("disarmed");
  });
});

describe("baselines", () => {
  const input = {
    symbol: "AAPL",
    condition: "crossing" as const,
    price: 100,
    trigger: "once" as const,
  };

  it("a forgotten baseline cannot produce a crossing", async () => {
    stubStorage();
    const s = await load();
    s.addAlert(input);
    s.processPrice("AAPL", 99);
    s.forgetBaseline("AAPL");
    expect(s.processPrice("AAPL", 101)).toEqual([]);
    expect(s.lastPrice("AAPL")).toBe(101);
    // The next price has a baseline again.
    expect(s.processPrice("AAPL", 99)).toHaveLength(1);
  });

  it("observePrice records for display only", async () => {
    stubStorage();
    const s = await load();
    s.addAlert(input);
    s.processPrice("AAPL", 99);
    s.observePrice("AAPL", 150);
    expect(s.lastPrice("AAPL")).toBe(150);
    expect(s.processPrice("AAPL", 101)).toHaveLength(1);
  });

  it("resuming an alert forgets the stale baseline", async () => {
    stubStorage();
    const s = await load();
    const a = s.addAlert(input)!;
    s.processPrice("AAPL", 99);
    s.setAlertActive(a.id, false);
    s.setAlertActive(a.id, true);
    expect(s.processPrice("AAPL", 101)).toEqual([]);
  });
});

describe("several tabs", () => {
  const input = {
    symbol: "AAPL",
    condition: "less" as const,
    price: 100,
    trigger: "once" as const,
  };

  it("a mutation keeps alerts another tab stored meanwhile", async () => {
    const data = stubStorage();
    const s = await load();
    s.addAlert(input);
    const other = { ...valid, id: "from-other-tab", symbol: "MSFT" };
    data.set(KEY, JSON.stringify([...JSON.parse(data.get(KEY)!), other]));

    s.addAlert({ ...input, symbol: "NVDA" });
    expect(s.alerts().map((a) => a.id)).toContain("from-other-tab");
    expect(JSON.parse(data.get(KEY)!)).toHaveLength(3);
  });

  it("processPrice works on top of the other tab's state", async () => {
    const data = stubStorage();
    const s = await load();
    s.addAlert(input);
    const stored = JSON.parse(data.get(KEY)!);
    // The other tab paused our alert and added one.
    stored[0].active = false;
    const added = { ...valid, id: "x", symbol: "AAPL", condition: "less" };
    data.set(KEY, JSON.stringify([...stored, added]));

    const fired = s.processPrice("AAPL", 50);
    expect(fired.map((a) => a.id)).toEqual(["x"]);
    const after = JSON.parse(data.get(KEY)!);
    expect(after).toHaveLength(2);
    expect(after[0].active).toBe(false);
  });

  it("a storage event refreshes the list", async () => {
    const data = stubStorage();
    const listeners: Record<string, (e: { key: string | null }) => void> = {};
    vi.stubGlobal("window", {
      addEventListener: (t: string, f: (e: { key: string | null }) => void) =>
        void (listeners[t] = f),
    });
    const s = await load();
    expect(s.alerts()).toEqual([]);
    data.set(KEY, JSON.stringify([valid]));
    listeners.storage!({ key: "unrelated" });
    expect(s.alerts()).toEqual([]);
    listeners.storage!({ key: KEY });
    expect(s.alerts().map((a) => a.id)).toEqual(["a1"]);
    data.delete(KEY);
    listeners.storage!({ key: null });
    expect(s.alerts()).toEqual([]);
  });
});
