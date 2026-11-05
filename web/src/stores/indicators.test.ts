import { afterEach, describe, expect, it, vi } from "vitest";

const KEY = "chartgen.indicators.active";
const DEFAULTS = ["ema_stack", "volume", { name: "rsi", length: 14 }];

function stubStorage(initial?: string) {
  const data = new Map<string, string>();
  if (initial !== undefined) data.set(KEY, initial);
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => void data.set(k, v),
  });
  return data;
}

// The store reads localStorage at module load, so import it fresh per case.
async function loadStore() {
  vi.resetModules();
  return import("./indicators");
}

afterEach(() => vi.unstubAllGlobals());

describe("indicator store load()", () => {
  it("seeds defaults when nothing is stored", async () => {
    stubStorage();
    const { activeIndicators } = await loadStore();
    expect(activeIndicators()).toEqual(DEFAULTS);
  });

  it("falls back to defaults without localStorage", async () => {
    const { activeIndicators } = await loadStore();
    expect(activeIndicators()).toEqual(DEFAULTS);
  });

  it("falls back to defaults on invalid JSON or non-array", async () => {
    stubStorage("{not json");
    expect((await loadStore()).activeIndicators()).toEqual(DEFAULTS);
    stubStorage('{"name":"rsi"}');
    expect((await loadStore()).activeIndicators()).toEqual(DEFAULTS);
  });

  it("respects an explicitly empty stored set", async () => {
    stubStorage("[]");
    expect((await loadStore()).activeIndicators()).toEqual([]);
  });

  it("drops invalid entries and keeps valid ones", async () => {
    stubStorage(
      JSON.stringify(["macd", 5, "", { name: "" }, { name: "rsi", length: 7 }]),
    );
    expect((await loadStore()).activeIndicators()).toEqual([
      "macd",
      { name: "rsi", length: 7 },
    ]);
  });

  it("falls back to defaults when every stored entry is invalid", async () => {
    stubStorage(JSON.stringify([1, null, ""]));
    expect((await loadStore()).activeIndicators()).toEqual(DEFAULTS);
  });
});

describe("indicator store mutations", () => {
  it("adding an active name adds another instance and persists", async () => {
    const data = stubStorage("[]");
    const { addIndicator, activeIndicators } = await loadStore();
    expect(addIndicator("macd")).toBe("macd");
    expect(addIndicator({ name: "macd", fast: 8 })).toBe("macd#2");
    expect(addIndicator("macd")).toBe("macd#3");
    const want = ["macd", { name: "macd", id: "macd#2", fast: 8 }, { name: "macd", id: "macd#3" }];
    expect(activeIndicators()).toEqual(want);
    expect(JSON.parse(data.get(KEY)!)).toEqual(want);
  });

  it("a new instance takes the smallest free key", async () => {
    stubStorage(JSON.stringify(["rsi", { name: "rsi", id: "rsi#3" }]));
    const s = await loadStore();
    expect(s.addIndicator("rsi")).toBe("rsi#2");
    s.removeIndicator("rsi");
    expect(s.addIndicator("rsi")).toBe("rsi");
  });

  it("remove drops only the instance with that key", async () => {
    stubStorage(JSON.stringify(["rsi", { name: "rsi", id: "rsi#2", length: 21 }, "macd"]));
    const { removeIndicator, activeIndicators } = await loadStore();
    removeIndicator("rsi#2");
    expect(activeIndicators()).toEqual(["rsi", "macd"]);
    removeIndicator("rsi");
    expect(activeIndicators()).toEqual(["macd"]);
  });

  it("activeIndicatorsKey keeps a __proto__ key as data", async () => {
    stubStorage();
    const { activeIndicatorsKey } = await loadStore();
    const entry = JSON.parse('{"name":"x","__proto__":{"a":1}}');
    expect(activeIndicatorsKey([entry])).toBe('[{"__proto__":{"a":1},"name":"x"}]');
  });

  it("activeIndicatorsKey sorts object keys but preserves array order", async () => {
    stubStorage();
    const { activeIndicatorsKey } = await loadStore();
    expect(activeIndicatorsKey([{ name: "rsi", length: 14 }])).toBe(
      activeIndicatorsKey([{ length: 14, name: "rsi" }]),
    );
    expect(activeIndicatorsKey(["a", "b"])).not.toBe(
      activeIndicatorsKey(["b", "a"]),
    );
  });
});

describe("indicator store visibility and params", () => {
  it("keeps repeated names on load, numbering the instances", async () => {
    stubStorage(
      JSON.stringify([
        { name: "rsi", length: 9 },
        "macd",
        "rsi",
        { name: "macd", fast: 5, hidden: true },
      ]),
    );
    expect((await loadStore()).activeIndicators()).toEqual([
      { name: "rsi", length: 9 },
      "macd",
      { name: "rsi", id: "rsi#2" },
      { name: "macd", id: "macd#2", fast: 5, hidden: true },
    ]);
  });

  it("keeps valid ids and repairs malformed or repeated ones on load", async () => {
    stubStorage(
      JSON.stringify([
        { name: "rsi", id: "rsi#5", length: 21 },
        { name: "rsi", id: "rsi#5" },
        { name: "rsi", id: "macd#2" },
        { name: "rsi", id: "rsi" },
        { name: "rsi", id: 7 },
      ]),
    );
    const s = await loadStore();
    expect(s.activeIndicators().map(s.instanceKey)).toEqual([
      "rsi#5",
      "rsi",
      "rsi#2",
      "rsi#3",
      "rsi#4",
    ]);
    expect(s.activeIndicators()[0]).toEqual({ name: "rsi", id: "rsi#5", length: 21 });
    expect(s.activeIndicators()[1]).toBe("rsi");
  });

  it("edits one instance by index and keeps its id", async () => {
    stubStorage(JSON.stringify(["rsi", { name: "rsi", id: "rsi#2" }]));
    const s = await loadStore();
    s.updateIndicatorParams(1, { length: 21 });
    s.toggleIndicatorHidden(1);
    expect(s.activeIndicators()).toEqual([
      "rsi",
      { name: "rsi", id: "rsi#2", length: 21, hidden: true },
    ]);
    s.toggleIndicatorHidden(1);
    s.updateIndicatorParams(1, {});
    expect(s.activeIndicators()[1]).toEqual({ name: "rsi", id: "rsi#2" });
  });

  it("toWire keeps the instance id of every visible instance", async () => {
    stubStorage();
    const s = await loadStore();
    expect(
      s.toWire([
        { name: "rsi", length: 14 },
        { name: "rsi", id: "rsi#2", length: 21, style: { plots: {} } },
        { name: "rsi", id: "rsi#3", hidden: true },
      ]),
    ).toEqual([
      { name: "rsi", length: 14 },
      { name: "rsi", id: "rsi#2", length: 21 },
    ]);
  });

  it("paramLabels tells repeated instances apart with their defaults", async () => {
    stubStorage();
    const { paramLabels } = await loadStore();
    const defaults = { rsi: { length: 14 }, macd: { fast: 12, slow: 26 } };
    const entries = ["rsi", { name: "rsi", id: "rsi#2", length: 21 }, { name: "macd", fast: 8 }];
    expect([...paramLabels(entries, defaults)]).toEqual([
      ["rsi", "(14)"],
      ["rsi#2", "(21)"],
      ["macd", "(8)"],
    ]);
    expect([...paramLabels(entries, defaults, true)]).toEqual([
      ["rsi", "(14)"],
      ["rsi#2", "(21)"],
      ["macd", undefined],
    ]);
  });

  it("loads the pre-ADR-0013 format unchanged", async () => {
    stubStorage(JSON.stringify(["macd", { name: "rsi", length: 9 }]));
    const s = await loadStore();
    expect(s.activeIndicators()).toEqual(["macd", { name: "rsi", length: 9 }]);
    expect(s.toWire(s.activeIndicators())).toEqual([
      "macd",
      { name: "rsi", length: 9 },
    ]);
  });

  it("persists hidden state and reloads it", async () => {
    const data = stubStorage();
    const s = await loadStore();
    s.toggleIndicatorHidden(2);
    expect(JSON.parse(data.get(KEY)!)).toEqual([
      "ema_stack",
      "volume",
      { name: "rsi", length: 14, hidden: true },
    ]);
    const s2 = await loadStore();
    expect(s2.isHidden(s2.activeIndicators()[2]!)).toBe(true);
  });

  it("updates params by index, keeping name and visibility", async () => {
    stubStorage();
    const s = await loadStore();
    s.toggleIndicatorHidden(2);
    s.updateIndicatorParams(2, { length: 21 });
    expect(s.activeIndicators()[2]).toEqual({
      name: "rsi",
      length: 21,
      hidden: true,
    });
  });

  it("moves between bare name and object form as params/hidden change", async () => {
    stubStorage();
    const s = await loadStore();
    s.updateIndicatorParams(0, { fast: 5 });
    expect(s.activeIndicators()[0]).toEqual({ name: "ema_stack", fast: 5 });
    s.updateIndicatorParams(0, {});
    expect(s.activeIndicators()[0]).toBe("ema_stack");
    s.toggleIndicatorHidden(0);
    expect(s.activeIndicators()[0]).toEqual({
      name: "ema_stack",
      hidden: true,
    });
    s.toggleIndicatorHidden(0);
    expect(s.activeIndicators()[0]).toBe("ema_stack");
  });

  it("ignores out-of-range indexes", async () => {
    stubStorage();
    const s = await loadStore();
    const before = s.activeIndicators();
    s.updateIndicatorParams(9, { x: 1 });
    s.toggleIndicatorHidden(9);
    expect(s.activeIndicators()).toEqual(before);
  });

  it("toWire excludes hidden entries and strips client-only fields", async () => {
    stubStorage();
    const s = await loadStore();
    const wire = s.toWire([
      "a",
      { name: "b", hidden: true },
      { name: "c", length: 3, hidden: false },
      { name: "d", length: 4 },
    ]);
    expect(wire).toEqual(["a", { name: "c", length: 3 }, { name: "d", length: 4 }]);
    expect(JSON.stringify(wire)).not.toContain("hidden");
  });
});
