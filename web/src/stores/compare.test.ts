import { beforeEach, describe, expect, it, vi } from "vitest";

function stubStorage(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial));
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => void data.set(k, v),
  });
  return data;
}

// The stores read localStorage at import time, so each test imports fresh copies.
async function load() {
  vi.resetModules();
  const layout = await import("./layout");
  const compare = await import("./compare");
  return { layout, compare };
}

beforeEach(() => {
  vi.unstubAllGlobals();
});

describe("compare store", () => {
  it("starts empty and falls back on corrupt data", async () => {
    stubStorage({ "chartgen.layout.compare": "{nope" });
    const { compare } = await load();
    expect(compare.compareSymbols()).toEqual([]);
  });

  it("validates, normalises, de-duplicates and caps stored symbols", async () => {
    stubStorage({
      "chartgen.layout.compare": JSON.stringify([
        " ethusdt ",
        "ETHUSDT",
        "bad symbol!",
        42,
        "A",
        "B",
        "C",
        "D",
        "E",
        "F",
      ]),
    });
    const { compare } = await load();
    expect(compare.compareSymbols()).toEqual(["ETHUSDT", "A", "B", "C", "D"]);
  });

  it("switches to percent on the first symbol and restores on the last", async () => {
    const data = stubStorage({ "chartgen.layout.scaleMode": '"log"' });
    const { layout, compare } = await load();
    expect(compare.addCompare("ethusdt", "BTCUSDT")).toBe(true);
    expect(layout.scaleMode()).toBe("percentage");
    expect(compare.addCompare("SOLUSDT", "BTCUSDT")).toBe(true);
    compare.removeCompare("ETHUSDT");
    expect(layout.scaleMode()).toBe("percentage");
    compare.removeCompare("SOLUSDT");
    expect(layout.scaleMode()).toBe("log");
    expect(compare.compareSymbols()).toEqual([]);
    expect(data.get("chartgen.layout.compare")).toBe("[]");
  });

  it("restores the scale mode after a reload", async () => {
    stubStorage({ "chartgen.layout.scaleMode": '"log"' });
    const first = await load();
    first.compare.addCompare("ETHUSDT", "BTCUSDT");
    const data = new Map<string, string>();
    for (const k of ["scaleMode", "compare", "compareRestore"]) {
      data.set(
        `chartgen.layout.${k}`,
        localStorage.getItem(`chartgen.layout.${k}`)!,
      );
    }
    stubStorage(Object.fromEntries(data));
    const { layout, compare } = await load();
    expect(compare.compareSymbols()).toEqual(["ETHUSDT"]);
    compare.removeCompare("ETHUSDT");
    expect(layout.scaleMode()).toBe("log");
  });

  it("rejects the main symbol, duplicates, bad input and a sixth symbol", async () => {
    stubStorage();
    const { compare } = await load();
    expect(compare.addCompare("BTCUSDT", "BTCUSDT")).toBe(false);
    expect(compare.addCompare("", "BTCUSDT")).toBe(false);
    for (const s of ["A", "B", "C", "D", "E"]) {
      expect(compare.addCompare(s, "BTCUSDT")).toBe(true);
    }
    expect(compare.addCompare("A", "BTCUSDT")).toBe(false);
    expect(compare.addCompare("F", "BTCUSDT")).toBe(false);
    expect(compare.compareSymbols()).toHaveLength(5);
  });

  it("setCompareSymbols leaves the scale mode alone", async () => {
    stubStorage();
    const { layout, compare } = await load();
    compare.setCompareSymbols(["ETHUSDT", "junk!!"]);
    expect(compare.compareSymbols()).toEqual(["ETHUSDT"]);
    expect(layout.scaleMode()).toBe("normal");
    compare.setCompareSymbols(undefined);
    expect(compare.compareSymbols()).toEqual([]);
  });
});
