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
