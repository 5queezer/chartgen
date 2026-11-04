import { afterEach, describe, expect, it, vi } from "vitest";

const KEY = "chartgen.indicators.active";
const DEFAULTS = ["ema_stack", { name: "rsi", length: 14 }];

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
  it("add is idempotent by name and persists", async () => {
    const data = stubStorage("[]");
    const { addIndicator, activeIndicators } = await loadStore();
    addIndicator("macd");
    addIndicator({ name: "macd", fast: 8 });
    expect(activeIndicators()).toEqual(["macd"]);
    expect(JSON.parse(data.get(KEY)!)).toEqual(["macd"]);
  });

  it("remove drops entries by name regardless of form", async () => {
    stubStorage();
    const { removeIndicator, activeIndicators } = await loadStore();
    removeIndicator("rsi");
    expect(activeIndicators()).toEqual(["ema_stack"]);
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
    s.toggleIndicatorHidden(1);
    expect(JSON.parse(data.get(KEY)!)).toEqual([
      "ema_stack",
      { name: "rsi", length: 14, hidden: true },
    ]);
    const s2 = await loadStore();
    expect(s2.isHidden(s2.activeIndicators()[1]!)).toBe(true);
  });

  it("updates params by index, keeping name and visibility", async () => {
    stubStorage();
    const s = await loadStore();
    s.toggleIndicatorHidden(1);
    s.updateIndicatorParams(1, { length: 21 });
    expect(s.activeIndicators()[1]).toEqual({
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
