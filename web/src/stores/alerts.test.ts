import { beforeEach, describe, expect, it, vi } from "vitest";

import eventReadings from "../../../tests/fixtures/pattern_event_readings.json";
import semantics from "../../../tests/fixtures/alert_semantics.json";

function stubStorage(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial));
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => void data.set(k, v),
    removeItem: (k: string) => void data.delete(k),
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

describe("the fire rules the server shares (tests/fixtures/alert_semantics.json)", () => {
  for (const c of semantics.cases) {
    it(c.name, async () => {
      stubStorage();
      const { addAlert, alerts, forgetBaseline, processPrice } = await load();
      addAlert({
        symbol: "ETHUSDT",
        condition: c.condition as "crossing",
        trigger: c.trigger as "once",
        price: c.level,
      });
      const fired: number[] = [];
      c.prices.forEach((p, i) => {
        if (p === null) forgetBaseline("ETHUSDT");
        else if (processPrice("ETHUSDT", p).length > 0) fired.push(i);
      });
      expect(fired).toEqual(c.fires);
      expect(alerts()[0]!.active).toBe(c.active);
    });
  }
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

// --- Drawing and indicator alerts (ADR-0034) ---------------------------

const H = 3_600_000;
const DRAWINGS_KEY = "chartgen.drawings.AAPL";
const RSI_SIG = '[{"length":14,"name":"rsi"}]';
const trendLine = {
  id: "d1",
  name: "segment",
  points: [
    { timestamp: 0, value: 100 },
    { timestamp: 10 * H, value: 200 },
  ],
  lock: false,
  visible: true,
};
const hline = {
  id: "h1",
  name: "horizontalStraightLine",
  points: [{ timestamp: 0, value: 150 }],
  lock: false,
  visible: true,
};
const drawingAlert = {
  symbol: "AAPL",
  condition: "crossing_up" as const,
  trigger: "once" as const,
  source: { kind: "drawing" as const, drawingId: "d1", label: "Trend line", interval: "1h" },
};
const hlineAlert = {
  ...drawingAlert,
  source: { kind: "drawing" as const, drawingId: "h1", label: "Horizontal line" },
};
const rsiAlert = {
  symbol: "AAPL",
  condition: "crossing_up" as const,
  trigger: "every" as const,
  source: {
    kind: "indicator" as const,
    interval: "1h",
    instanceKey: "rsi",
    signature: RSI_SIG,
    line: 0,
    name: "RSI (14)",
    against: { kind: "value" as const, value: 70 },
  },
};
const rsiChart = (...values: (number | null)[]) => ({
  chart: { interval: "1h", indicators: { rsi: { lines: [{ values }] } } },
});

/** The chart's tick at bar `h` (hourly bars 0 h … `h`), for sloped drawings. */
async function barTick(h: number, interval = "1h") {
  const { timeAxisOf } = await import("./drawings");
  const axis = timeAxisOf(Array.from({ length: h + 1 }, (_, i) => i * H))!;
  return { time: h * H, chart: { interval, indicators: {}, view: { axis, log: false } } };
}

/** Load the store with indicator signatures as the app sets them. */
async function loadWithRsi() {
  const s = await load();
  s.setIndicatorSignatures({ rsi: RSI_SIG });
  return s;
}

describe("sources: migration and validation", () => {
  it("an old record without a source is a price alert", async () => {
    stubStorage({ [KEY]: JSON.stringify([valid]) });
    const s = await load();
    expect(s.alerts()[0]).not.toHaveProperty("source");
    expect(s.alerts()[0]!.price).toBe(100);
  });

  it("keeps valid sources and drops invalid ones", async () => {
    stubStorage();
    const { sanitizeAlerts } = await load();
    const out = sanitizeAlerts([
      { ...valid, id: "p", source: { kind: "price" } },
      {
        ...valid,
        id: "d",
        price: 0,
        active: false,
        source: drawingAlert.source,
        note: "drawing deleted",
      },
      { ...valid, id: "i", price: 0, source: rsiAlert.source },
      { ...valid, id: "bad-kind", source: { kind: "nope" } },
      { ...valid, id: "bad-drawing", source: { kind: "drawing", label: "x" } },
      {
        ...valid,
        id: "bad-against",
        source: { ...rsiAlert.source, against: { kind: "value", value: "70" } },
      },
      { ...valid, id: "bad-line", source: { ...rsiAlert.source, line: -1 } },
      { ...valid, id: "no-interval", source: { ...rsiAlert.source, interval: "" } },
      { ...valid, id: "no-signature", source: { ...rsiAlert.source, signature: undefined } },
    ])!;
    expect(out.map((a) => a.id)).toEqual(["p", "d", "i"]);
    expect(out[0]).not.toHaveProperty("source");
    expect(out[1]).toMatchObject({ source: drawingAlert.source, note: "drawing deleted" });
    expect(out[2]!.source).toEqual(rsiAlert.source);
  });

  it("keeps a note only on an inactive alert", async () => {
    stubStorage();
    const { sanitizeAlerts } = await load();
    const [a] = sanitizeAlerts([{ ...valid, note: "drawing deleted" }])!;
    expect(a).not.toHaveProperty("note");
  });

  it("an indicator alert can compare with zero or a negative value", async () => {
    stubStorage();
    const s = await load();
    const a = s.addAlert({
      ...rsiAlert,
      source: { ...rsiAlert.source, against: { kind: "value", value: -1.5 } },
    });
    expect(a).not.toBeNull();
    expect(a!.price).toBe(0);
    s.updateAlert(a!.id, { value: 0 });
    expect(s.alerts()[0]!.source).toMatchObject({ against: { kind: "value", value: 0 } });
  });
});

describe("drawing alerts", () => {
  it("fire when the price crosses the line's value at the bar", async () => {
    stubStorage({ [DRAWINGS_KEY]: JSON.stringify([trendLine]) });
    const s = await load();
    s.addAlert(drawingAlert);
    // The line is 150 at bar 5 and 160 at bar 6: 149 → 159 stays below it.
    expect(s.processPrice("AAPL", 149, 1, await barTick(5))).toEqual([]);
    expect(s.processPrice("AAPL", 159, 2, await barTick(6))).toEqual([]);
    expect(s.processPrice("AAPL", 161, 3, await barTick(6))).toHaveLength(1);
    expect(s.alerts()[0]).toMatchObject({ active: false, lastTriggeredAt: 3 });
  });

  it("a sloped line needs the chart at its interval; a horizontal one does not", async () => {
    stubStorage({ [DRAWINGS_KEY]: JSON.stringify([trendLine, hline]) });
    const s = await load();
    const sloped = s.addAlert({ ...drawingAlert, condition: "greater" })!;
    s.addAlert({ ...hlineAlert, condition: "greater" });
    // Only the horizontal one is polled off the chart.
    expect(s.activeAlertSymbols()).toEqual(["AAPL"]);
    // A quotes poll (no chart): the horizontal line fires, the sloped one cannot.
    expect(s.processPrice("AAPL", 1e6, 1).map((a) => a.source)).toEqual([hlineAlert.source]);
    // Another interval on the chart.
    expect(s.processPrice("AAPL", 1e6, 2, await barTick(5, "4h"))).toEqual([]);
    expect(s.inactiveReason(s.alerts().find((a) => a.id === sloped.id)!, null)).toBe(
      "open AAPL 1h",
    );
    expect(s.processPrice("AAPL", 1e6, 3, await barTick(5))).toHaveLength(1);
  });

  it("moving the drawing over the price does not fire it", async () => {
    const data = stubStorage({ [DRAWINGS_KEY]: JSON.stringify([trendLine]) });
    const s = await load();
    s.addAlert(drawingAlert);
    s.processPrice("AAPL", 140, 1, await barTick(5));
    const lower = {
      ...trendLine,
      points: trendLine.points.map((p) => ({ ...p, value: p.value - 50 })),
    };
    data.set(DRAWINGS_KEY, JSON.stringify([lower]));
    expect(s.processPrice("AAPL", 140, 2, await barTick(5))).toEqual([]);
  });

  it("are skipped while the drawing is missing", async () => {
    stubStorage();
    const s = await load();
    s.addAlert({ ...hlineAlert, condition: "greater" });
    expect(s.processPrice("AAPL", 1e9, 1)).toEqual([]);
    expect(s.alerts()[0]!.active).toBe(true);
  });

  it("deleting the drawing disables its alerts with a note; undo restores them", async () => {
    const data = stubStorage({ [DRAWINGS_KEY]: JSON.stringify([trendLine]) });
    const s = await load();
    const a = s.addAlert(drawingAlert)!;
    const other = s.addAlert({ ...drawingAlert, symbol: "MSFT" })!;
    const price = s.addAlert({ symbol: "AAPL", condition: "greater", price: 5, trigger: "once" })!;
    s.syncDrawingAlerts("AAPL", [trendLine]);
    expect(s.alerts().every((x) => x.active)).toBe(true);

    data.delete(DRAWINGS_KEY);
    s.syncDrawingAlerts("AAPL", []);
    const byId = (id: string) => s.alerts().find((x) => x.id === id)!;
    expect(byId(a.id)).toMatchObject({ active: false, note: s.DRAWING_DELETED });
    expect(s.inactiveReason(byId(a.id))).toBe(s.DRAWING_DELETED);
    expect(byId(other.id).active).toBe(true);
    expect(byId(price.id).active).toBe(true);

    // Resuming or editing while the drawing is gone keeps it off, with the note.
    s.setAlertActive(a.id, true);
    expect(byId(a.id)).toMatchObject({ active: false, note: s.DRAWING_DELETED });
    s.updateAlert(a.id, { condition: "crossing" });
    expect(byId(a.id)).toMatchObject({
      active: false,
      note: s.DRAWING_DELETED,
      condition: "crossing",
    });

    // The drawing comes back (undo): the alert is on again.
    data.set(DRAWINGS_KEY, JSON.stringify([trendLine]));
    s.syncDrawingAlerts("AAPL", [trendLine]);
    expect(byId(a.id).active).toBe(true);
    expect(byId(a.id)).not.toHaveProperty("note");
  });

  it("a line dragged vertical disables its alerts with a note, until it stops being vertical", async () => {
    const vertical = {
      ...trendLine,
      points: [trendLine.points[0]!, { ...trendLine.points[1]!, timestamp: trendLine.points[0]!.timestamp }],
    };
    const data = stubStorage({ [DRAWINGS_KEY]: JSON.stringify([trendLine]) });
    const s = await load();
    const a = s.addAlert(drawingAlert)!;
    const paused = s.addAlert(drawingAlert)!;
    s.setAlertActive(paused.id, false);
    const byId = (id: string) => s.alerts().find((x) => x.id === id)!;

    data.set(DRAWINGS_KEY, JSON.stringify([vertical]));
    s.syncDrawingAlerts("AAPL", [vertical]);
    expect(byId(a.id)).toMatchObject({ active: false, note: s.LINE_VERTICAL });
    expect(s.inactiveReason(byId(a.id))).toBe(s.LINE_VERTICAL);
    expect(byId(paused.id)).not.toHaveProperty("note");

    // Resuming or editing while it is vertical keeps it off, with the note.
    s.setAlertActive(a.id, true);
    expect(byId(a.id)).toMatchObject({ active: false, note: s.LINE_VERTICAL });
    s.updateAlert(a.id, { condition: "crossing" });
    expect(byId(a.id)).toMatchObject({ active: false, note: s.LINE_VERTICAL });
    s.setAlertActive(paused.id, true);
    expect(byId(paused.id)).toMatchObject({ active: false, note: s.LINE_VERTICAL });
    s.setAlertActive(paused.id, false);

    // Deleted while vertical says so, and undo brings the vertical note back.
    s.syncDrawingAlerts("AAPL", []);
    expect(byId(a.id)).toMatchObject({ active: false, note: s.DRAWING_DELETED });
    s.syncDrawingAlerts("AAPL", [vertical]);
    expect(byId(a.id)).toMatchObject({ active: false, note: s.LINE_VERTICAL });

    // No longer vertical: on again, and the paused one stays paused.
    data.set(DRAWINGS_KEY, JSON.stringify([trendLine]));
    s.syncDrawingAlerts("AAPL", [trendLine]);
    expect(byId(a.id).active).toBe(true);
    expect(byId(a.id)).not.toHaveProperty("note");
    expect(byId(paused.id).active).toBe(false);
  });

  it("a line on one bar of the alert's interval is a reason in this tab, never stored", async () => {
    // Ten 1-minute bars apart: not vertical by its points.
    const steep = {
      ...trendLine,
      points: [trendLine.points[0]!, { ...trendLine.points[1]!, timestamp: 10 * 60_000 }],
    };
    const data = stubStorage({ [DRAWINGS_KEY]: JSON.stringify([steep]) });
    const s = await load();
    const { timeAxisOf } = await import("./drawings");
    const hourly = timeAxisOf(Array.from({ length: 30 }, (_, i) => i * H));
    const minutes = timeAxisOf(Array.from({ length: 30 }, (_, i) => i * 60_000));
    const ctx = (interval: string, axis: typeof hourly) => ({ symbol: "AAPL", interval, keys: [], axis });
    const a = s.addAlert(drawingAlert)!;
    const paused = s.addAlert(drawingAlert)!;
    s.setAlertActive(paused.id, false);
    const finer = s.addAlert({ ...drawingAlert, source: { ...drawingAlert.source, interval: "1m" } })!;
    const byId = (id: string) => s.alerts().find((x) => x.id === id)!;
    const stored = () => data.get("chartgen.alerts.list");
    const before = stored();

    // Each tab judges by its own chart, and showing a chart stores nothing.
    expect(s.inactiveReason(byId(a.id), ctx("1h", hourly))).toBe(s.LINE_VERTICAL);
    expect(s.inactiveReason(byId(a.id), ctx("1m", minutes))).toBe("open AAPL 1h");
    s.setChartContext(ctx("1h", hourly));
    expect(s.inactiveReason(byId(a.id))).toBe(s.LINE_VERTICAL);
    s.setChartContext(ctx("1m", minutes));
    expect(s.inactiveReason(byId(a.id))).toBe("open AAPL 1h");
    s.setChartContext(ctx("1h", hourly));
    expect(stored()).toBe(before);
    expect(byId(a.id).active).toBe(true);
    expect(byId(a.id)).not.toHaveProperty("note");

    // Paused stays paused, and an alert on another interval is not judged on these bars.
    expect(s.inactiveReason(byId(paused.id))).toBeNull();
    expect(byId(paused.id).active).toBe(false);
    expect(s.inactiveReason(byId(finer.id))).toBe("open AAPL 1m");
    expect(byId(finer.id).active).toBe(true);

    // A save does not store it either, and the reason follows the drawing.
    s.syncDrawingAlerts("AAPL", [steep]);
    expect(stored()).toBe(before);
    data.set(DRAWINGS_KEY, JSON.stringify([trendLine]));
    s.syncDrawingAlerts("AAPL", [trendLine]);
    expect(s.inactiveReason(byId(a.id))).toBeNull();
    expect(stored()).toBe(before);
  });

  it("goes by the chart of the tab that evaluates, when that is another tab", async () => {
    stubStorage({ [DRAWINGS_KEY]: JSON.stringify([trendLine]) });
    const s = await load();
    const a = s.addAlert(drawingAlert)!;
    const price = s.addAlert({ symbol: "AAPL", condition: "crossing", trigger: "once", price: 100 })!;
    const byId = (id: string) => s.alerts().find((x) => x.id === id)!;
    const here = { symbol: "AAPL", interval: "1h", keys: [] };

    s.setEvaluatingChart("AAPL 15m");
    expect(s.inactiveReason(byId(a.id), here)).toBe("open AAPL 1h in the other tab");
    expect(s.inactiveReason(byId(price.id), here)).toBeNull();
    s.setEvaluatingChart("MSFT 1h");
    expect(s.inactiveReason(byId(a.id), here)).toBe("open AAPL 1h in the other tab");
    s.setEvaluatingChart("AAPL 1h");
    expect(s.inactiveReason(byId(a.id), here)).toBeNull();
    expect(s.inactiveReason(byId(a.id), { ...here, interval: "15m" })).toBe("open AAPL 1h");
    s.setEvaluatingChart(null);
    expect(s.inactiveReason(byId(a.id), here)).toBeNull();
  });

  it("a line on one bar of the chart is not evaluated: it neither fires nor re-arms", async () => {
    const steep = {
      ...trendLine,
      points: [trendLine.points[0]!, { ...trendLine.points[1]!, timestamp: 10 * 60_000 }],
    };
    const data = stubStorage({ [DRAWINGS_KEY]: JSON.stringify([trendLine]) });
    const s = await load();
    const { timeAxisOf } = await import("./drawings");
    const view = { axis: timeAxisOf(Array.from({ length: 30 }, (_, i) => i * H))!, log: false };
    const tick = (i: number) => ({ time: i * H, chart: { interval: "1h", indicators: {}, view } });
    s.addAlert({ ...drawingAlert, trigger: "every" });

    // On the line it fires, and an "every" crossing stays armed; now disarm it by hand.
    s.processPrice("AAPL", 100, 0, tick(5));
    expect(s.processPrice("AAPL", 400, 0, tick(6))).toHaveLength(1);
    const stored = JSON.parse(data.get("chartgen.alerts.list")!) as { disarmed?: boolean }[];
    stored[0]!.disarmed = true;
    data.set("chartgen.alerts.list", JSON.stringify(stored));

    // One bar: nothing happens to it, whatever the price does.
    data.set(DRAWINGS_KEY, JSON.stringify([steep]));
    expect(s.processPrice("AAPL", 1, 0, tick(7))).toEqual([]);
    expect(s.processPrice("AAPL", 900, 0, tick(8))).toEqual([]);
    expect(s.alerts()[0]).toMatchObject({ active: true, disarmed: true });
  });

  it("passing through vertical does not re-arm a fired every-time alert", async () => {
    stubStorage({ [DRAWINGS_KEY]: JSON.stringify([trendLine]) });
    const s = await load();
    const { timeAxisOf } = await import("./drawings");
    const view = { axis: timeAxisOf(Array.from({ length: 30 }, (_, i) => i * H))!, log: false };
    // The line reads 150 at this bar.
    const tick = { time: 5 * H, chart: { interval: "1h", indicators: {}, view } };
    s.addAlert({ ...drawingAlert, condition: "greater", trigger: "every" });
    expect(s.processPrice("AAPL", 151, 0, tick)).toHaveLength(1);
    expect(s.alerts()[0]!.disarmed).toBe(true);

    const upright = {
      ...trendLine,
      points: [trendLine.points[0]!, { ...trendLine.points[1]!, timestamp: trendLine.points[0]!.timestamp }],
    };
    s.syncDrawingAlerts("AAPL", [upright]);
    expect(s.alerts()[0]).toMatchObject({ active: false, note: s.LINE_VERTICAL, disarmed: true });
    s.syncDrawingAlerts("AAPL", [trendLine]);
    expect(s.alerts()[0]).toMatchObject({ active: true, disarmed: true });

    // Still above the level: no second firing until the price has been back below.
    expect(s.processPrice("AAPL", 152, 0, tick)).toEqual([]);
    expect(s.processPrice("AAPL", 149, 0, tick)).toEqual([]);
    expect(s.processPrice("AAPL", 151, 0, tick)).toHaveLength(1);
  });

  it("a horizontal tool with two points is never called vertical", async () => {
    // A ray stored from when the tool took two clicks: its level needs neither point's bar.
    const ray = (apart: number) => ({
      ...hline,
      name: "horizontalRayLine",
      points: [hline.points[0]!, { ...hline.points[0]!, timestamp: hline.points[0]!.timestamp + apart }],
    });
    const data = stubStorage({ [DRAWINGS_KEY]: JSON.stringify([ray(0)]) });
    const s = await load();
    const { timeAxisOf } = await import("./drawings");
    const axis = timeAxisOf(Array.from({ length: 30 }, (_, i) => i * H))!;
    const view = { axis, log: false };
    const ctx = { symbol: "AAPL", interval: "1h", keys: [], axis };
    const a = s.addAlert(hlineAlert)!;
    const bound = s.addAlert({ ...hlineAlert, source: { ...hlineAlert.source, interval: "1h" } })!;
    const byId = (id: string) => s.alerts().find((x) => x.id === id)!;

    // Equal times: a save leaves it on, and resuming keeps it on.
    s.syncDrawingAlerts("AAPL", [ray(0)]);
    expect(byId(a.id)).toMatchObject({ active: true });
    expect(byId(a.id)).not.toHaveProperty("note");
    s.setAlertActive(a.id, false);
    s.setAlertActive(a.id, true);
    expect(byId(a.id)).toMatchObject({ active: true });
    expect(byId(a.id)).not.toHaveProperty("note");

    // Within one bar of the chart: no reason, and it fires at its level.
    data.set(DRAWINGS_KEY, JSON.stringify([ray(15 * 60_000)]));
    expect(s.inactiveReason(byId(bound.id), ctx)).toBeNull();
    const tick = { time: 5 * H, chart: { interval: "1h", indicators: {}, view } };
    s.processPrice("AAPL", 149, 0, tick);
    expect(s.processPrice("AAPL", 151, 0, tick).map((f) => f.id).sort()).toEqual([a.id, bound.id].sort());
  });

  it("a paused alert on a deleted drawing gets the note when resumed", async () => {
    stubStorage();
    const s = await load();
    const a = s.addAlert(hlineAlert)!;
    s.setAlertActive(a.id, false);
    s.syncDrawingAlerts("AAPL", []);
    expect(s.alerts()[0]).not.toHaveProperty("note");
    s.setAlertActive(a.id, true);
    expect(s.alerts()[0]).toMatchObject({ active: false, note: s.DRAWING_DELETED });
  });

  it("drawings evicted from storage switch their alerts off", async () => {
    stubStorage();
    const s = await load();
    const drawings = await import("./drawings");
    s.addAlert({ ...hlineAlert, symbol: "S0" });
    for (let i = 0; i <= drawings.MAX_SYMBOLS; i++) {
      drawings.saveDrawings(`S${i}`, [hline as never]);
    }
    expect(s.alerts()[0]).toMatchObject({ active: false, note: s.DRAWING_DELETED });
  });
});

describe("indicator alerts", () => {
  it("cross on the forming bar's values between ticks", async () => {
    stubStorage();
    const s = await loadWithRsi();
    s.addAlert(rsiAlert);
    expect(s.processPrice("AAPL", 1, 1, rsiChart(60, 68))).toEqual([]);
    expect(s.processPrice("AAPL", 1, 2, rsiChart(60, 71))).toHaveLength(1);
    expect(s.processPrice("AAPL", 1, 3, rsiChart(60, 72))).toEqual([]);
    expect(s.processPrice("AAPL", 1, 4, rsiChart(60, 69))).toEqual([]);
    expect(s.processPrice("AAPL", 1, 5, rsiChart(60, 70))).toHaveLength(1);
  });

  it("price against a plot: close crossing the EMA", async () => {
    stubStorage();
    const s = await load();
    s.setIndicatorSignatures({ ema: "ema-sig" });
    s.addAlert({
      ...rsiAlert,
      condition: "crossing",
      trigger: "once",
      source: {
        ...rsiAlert.source,
        instanceKey: "ema",
        signature: "ema-sig",
        name: "EMA 200",
        against: { kind: "price" },
      },
    });
    const ema = (v: number) => ({
      chart: { interval: "1h", indicators: { ema: { lines: [{ values: [v] }] } } },
    });
    expect(s.processPrice("AAPL", 99, 1, ema(100))).toEqual([]);
    expect(s.processPrice("AAPL", 101, 2, ema(100.5))).toHaveLength(1);
  });

  it("are not evaluated without the chart, nor across a gap", async () => {
    stubStorage();
    const s = await loadWithRsi();
    s.addAlert(rsiAlert);
    s.processPrice("AAPL", 1, 1, rsiChart(68));
    // A quotes poll (no series) breaks the continuity.
    s.processPrice("AAPL", 1, 2);
    expect(s.processPrice("AAPL", 1, 3, rsiChart(71))).toEqual([]);
    s.processPrice("AAPL", 1, 4, rsiChart(68));
    s.forgetBaseline("AAPL");
    expect(s.processPrice("AAPL", 1, 5, rsiChart(71))).toEqual([]);
    // After bar replay only the readings are dropped.
    s.processPrice("AAPL", 1, 6, rsiChart(68));
    s.forgetReadings("AAPL");
    expect(s.processPrice("AAPL", 1, 7, rsiChart(71))).toEqual([]);
    // Another interval.
    s.processPrice("AAPL", 1, 8, rsiChart(68));
    expect(
      s.processPrice("AAPL", 1, 9, { chart: { ...rsiChart(71).chart, interval: "4h" } }),
    ).toEqual([]);
  });

  it("do not run on another instance under the same key", async () => {
    stubStorage();
    const s = await loadWithRsi();
    const a = s.addAlert(rsiAlert)!;
    const ctx = { symbol: "AAPL", interval: "1h", keys: ["rsi"] };
    s.processPrice("AAPL", 1, 1, rsiChart(68));
    // RSI (14) removed and RSI (21) added: it gets the same key.
    s.setIndicatorSignatures({ rsi: '[{"length":21,"name":"rsi"}]' });
    expect(s.processPrice("AAPL", 1, 2, rsiChart(71))).toEqual([]);
    expect(s.inactiveReason(s.alerts()[0]!, ctx)).toBe("indicator changed");
    s.setIndicatorSignatures({});
    expect(s.inactiveReason(s.alerts()[0]!, { ...ctx, keys: [] })).toBe(
      "indicator not on the chart",
    );
    s.setIndicatorSignatures({ rsi: RSI_SIG });
    expect(s.inactiveReason(s.alerts().find((x) => x.id === a.id)!, ctx)).toBeNull();
  });

  it("a greater-than alert fires on the first reading", async () => {
    stubStorage();
    const s = await loadWithRsi();
    s.addAlert({
      ...rsiAlert,
      condition: "greater",
      trigger: "once",
      source: { ...rsiAlert.source, against: { kind: "value", value: 0 } },
    });
    expect(s.processPrice("AAPL", 1, 1, rsiChart(null))).toEqual([]);
    expect(s.processPrice("AAPL", 1, 2, rsiChart(55))).toHaveLength(1);
  });

  it("say why they are inactive and are left out of the quotes poll", async () => {
    stubStorage();
    const s = await loadWithRsi();
    const a = s.addAlert(rsiAlert)!;
    expect(s.activeAlertSymbols()).toEqual([]);
    expect(s.inactiveReason(a, null)).toBe("open AAPL 1h");
    expect(s.inactiveReason(a, { symbol: "AAPL", interval: "4h", keys: ["rsi"] })).toBe(
      "open AAPL 1h",
    );
    expect(s.inactiveReason(a, { symbol: "AAPL", interval: "1h", keys: [] })).toBe(
      "indicator not on the chart",
    );
    expect(s.inactiveReason(a, { symbol: "AAPL", interval: "1h", keys: ["rsi"] })).toBeNull();
  });
});

describe("alerts on a detection (ADR-0046)", () => {
  const key = "chart_patterns";
  const H = 3600;
  /** The chart at forming bar `bar` (seconds), with the newest detection at `t`. */
  const tick = (t: number | null, bar: number) => ({
    time: bar * 1000,
    chart: {
      interval: "1h",
      indicators: { [key]: { events: [{ key: "bullish", t }, { key: "bearish", t: null }] } },
    },
  });
  async function withEvent(trigger: "once" | "every" = "every") {
    stubStorage();
    const s = await load();
    s.setIndicatorSignatures({ [key]: "sig" });
    const added = s.addAlert({
      symbol: "BTCUSDT",
      condition: "crossing_up",
      trigger,
      source: {
        kind: "indicator",
        interval: "1h",
        instanceKey: key,
        signature: "sig",
        line: 0,
        name: "Chart Patterns: Bullish pattern confirmed",
        against: { kind: "event", key: "bullish" },
      },
    });
    expect(added).not.toBeNull();
    return s;
  }
  /** Feed `(t, bar)` readings; the indexes that fired. */
  const fires = (
    s: Awaited<ReturnType<typeof withEvent>>,
    readings: Array<[number | null, number]>,
  ) =>
    readings.flatMap(([t, bar], i) =>
      s.processPrice("BTCUSDT", 1, 0, tick(t, bar)).length > 0 ? [i] : [],
    );

  it("an old detection does not fire when the chart loads", async () => {
    const s = await withEvent();
    expect(fires(s, [[100 * H, 500 * H], [100 * H, 500 * H]])).toEqual([]);
  });

  it("each newer detection fires once", async () => {
    const s = await withEvent();
    // Bar 501 closes while bar 502 forms, and then bar 502 on the very next bar:
    // nothing has to fall back first.
    expect(
      fires(s, [
        [100 * H, 501 * H],
        [501 * H, 502 * H],
        [501 * H, 502 * H],
        [502 * H, 503 * H],
      ]),
    ).toEqual([1, 3]);
  });

  it("the first detection ever fires too, if it is new", async () => {
    const s = await withEvent();
    expect(fires(s, [[null, 500 * H], [500 * H, 501 * H]])).toEqual([1]);
  });

  it("a value falling back and returning does not fire: 100, 0, 90", async () => {
    const s = await withEvent();
    expect(fires(s, [[100 * H, 500 * H], [null, 501 * H], [90 * H, 502 * H]])).toEqual([]);
  });

  it("a value falling back and returning does not fire: 100, 90, 100", async () => {
    const s = await withEvent();
    expect(fires(s, [[100 * H, 500 * H], [90 * H, 501 * H], [100 * H, 502 * H]])).toEqual([]);
  });

  it("an old bar never seen before does not fire, the window only just showed it", async () => {
    const s = await withEvent();
    // None so far, then a pattern 400 bars back: the hole in "the first ever".
    expect(fires(s, [[null, 500 * H], [100 * H, 501 * H]])).toEqual([]);
    // Older than the bar last read, though newer than anything seen.
    const t = await withEvent();
    expect(fires(t, [[100 * H, 500 * H], [200 * H, 501 * H]])).toEqual([]);
  });

  it("after falling back, a really new detection still fires", async () => {
    const s = await withEvent();
    expect(
      fires(s, [
        [100 * H, 500 * H],
        [null, 501 * H],
        [501 * H, 502 * H],
      ]),
    ).toEqual([2]);
  });

  it("a catch-up after sleep still fires, once", async () => {
    const s = await withEvent();
    // Asleep from bar 500 to bar 540: a detection on bar 520 happened meanwhile.
    expect(fires(s, [[100 * H, 500 * H], [520 * H, 540 * H], [520 * H, 541 * H]])).toEqual([1]);
  });

  describe("on the real indicator as its window slides (tests/fixtures/pattern_event_readings.json)", () => {
    let firedInAll = 0;
    for (const run of eventReadings.runs) {
      it(`seed ${run.seed}, ${run.pattern}, ${run.side}`, async () => {
        const s = await withEvent();
        const stale: number[] = [];
        let fired = 0;
        let top = -Infinity;
        let c = 0;
        let value: number | null = null;
        let prevBar = 0;
        for (let i = 0; i < run.closes; i++) {
          const change = run.changes[c] as [number, number | null] | undefined;
          if (change && change[0] === i) {
            value = change[1];
            c++;
          }
          // Bar `first + i` has closed, and the next one is forming.
          const bar = run.first + (i + 1) * eventReadings.step;
          const hit = s.processPrice("BTCUSDT", 1, 0, tick(value, bar)).length > 0;
          // No old bar and no bar twice. Without a previous reading, nothing fires.
          if (hit) {
            fired++;
            if (value === null || value <= top || value < prevBar) stale.push(i);
          }
          // Anything else new, on a bar that just closed, must have fired.
          if (i > 0 && !hit && value !== null && value > top && value >= prevBar) stale.push(-i);
          if (value !== null) top = Math.max(top, value);
          prevBar = bar;
        }
        expect(stale).toEqual([]);
        firedInAll += fired;
      });
    }

    it("fired on something", () => {
      expect(firedInAll).toBeGreaterThan(5);
    });
  });

  it("a once alert stops after firing", async () => {
    const s = await withEvent("once");
    expect(
      fires(s, [
        [100 * H, 500 * H],
        [500 * H, 501 * H],
        [501 * H, 502 * H],
      ]),
    ).toEqual([1]);
    expect(s.alerts()[0]!.active).toBe(false);
  });

  it("does not run with another interval or other settings on the chart", async () => {
    const s = await withEvent();
    s.processPrice("BTCUSDT", 1, 0, tick(100 * H, 500 * H));
    const other = { ...tick(500 * H, 501 * H), chart: { ...tick(500 * H, 501 * H).chart, interval: "4h" } };
    expect(s.processPrice("BTCUSDT", 1, 0, other)).toEqual([]);
    // No value on that chart: the baseline starts over.
    expect(s.processPrice("BTCUSDT", 1, 0, tick(501 * H, 502 * H))).toEqual([]);
    s.setIndicatorSignatures({ [key]: "changed" });
    expect(s.processPrice("BTCUSDT", 1, 0, tick(502 * H, 503 * H))).toEqual([]);
  });

  it("survives a reload and rejects a detection without a key", async () => {
    await withEvent();
    const s = await load();
    const [a] = s.alerts();
    expect(a!.source).toMatchObject({ against: { kind: "event", key: "bullish" } });
    expect(
      s.sanitizeAlerts([
        { ...a, id: "bad", source: { ...(a!.source as object), against: { kind: "event" } } },
      ]),
    ).toEqual([]);
  });

  it("says what it watches", async () => {
    const s = await withEvent();
    const { describeAlert } = await import("../lib/alerts");
    expect(describeAlert(s.alerts()[0]!)).toBe("Chart Patterns: Bullish pattern confirmed");
  });
});

describe("alerts on a level plot (ADR-0046)", () => {
  const key = "auto_fib";
  const tick = (y: number, anchor = 1000) => ({
    chart: {
      interval: "1h",
      indicators: {
        [key]: { lines: [{ values: [y] }], levels: [{ line: 0, t: anchor, y }] },
      },
    },
  });

  async function onLevel() {
    stubStorage();
    const s = await load();
    s.setIndicatorSignatures({ [key]: "sig" });
    s.addAlert({
      symbol: "BTCUSDT",
      condition: "crossing",
      trigger: "every",
      source: {
        kind: "indicator",
        interval: "1h",
        instanceKey: key,
        signature: "sig",
        line: 0,
        name: "0.618",
        against: { kind: "price" },
      },
    });
    return s;
  }

  it("a price crossing the level fires", async () => {
    const s = await onLevel();
    s.processPrice("BTCUSDT", 90, 0, tick(100));
    expect(s.processPrice("BTCUSDT", 110, 0, tick(100))).toHaveLength(1);
  });

  it("the level jumping over the price with a new swing does not", async () => {
    const s = await onLevel();
    s.processPrice("BTCUSDT", 90, 0, tick(100));
    // Now under the price, because the swing moved: no crossing by the price.
    expect(s.processPrice("BTCUSDT", 90, 0, tick(80, 2000))).toEqual([]);
    // The new swing's level still works.
    expect(s.processPrice("BTCUSDT", 70, 0, tick(80, 2000))).toHaveLength(1);
  });
});
