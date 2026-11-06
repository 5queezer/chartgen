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

  it("keeps a scale mode the user picked while comparing", async () => {
    stubStorage({ "chartgen.layout.scaleMode": '"log"' });
    const { layout, compare } = await load();
    compare.addCompare("ETHUSDT", "BTCUSDT");
    layout.setScaleMode("normal");
    compare.removeCompare("ETHUSDT");
    expect(layout.scaleMode()).toBe("normal");
  });

  it("does not leak the restore target into a layout switch", async () => {
    stubStorage({ "chartgen.layout.scaleMode": '"log"' });
    const { layout, compare } = await load();
    compare.addCompare("ETHUSDT", "BTCUSDT");
    // A layout with its own compare list and percent scale is applied.
    layout.setScaleMode("percentage");
    compare.setCompareSymbols(["SOLUSDT"]);
    compare.removeCompare("SOLUSDT");
    expect(layout.scaleMode()).toBe("normal");
  });

  it("takes the restore target it is given, whatever the list was", async () => {
    stubStorage({ "chartgen.layout.scaleMode": '"log"' });
    const { layout, compare } = await load();
    compare.addCompare("ETHUSDT", "BTCUSDT");
    compare.setCompareSymbols(["ETHUSDT"], "normal");
    compare.removeCompare("ETHUSDT");
    expect(layout.scaleMode()).toBe("normal");

    compare.addCompare("ETHUSDT", "BTCUSDT");
    layout.setScaleMode("percentage");
    compare.setCompareSymbols(["SOLUSDT"], "log");
    compare.removeCompare("SOLUSDT");
    expect(layout.scaleMode()).toBe("log");
  });

  it("falls back to the default scale for a layout saved without a target", async () => {
    stubStorage({ "chartgen.layout.scaleMode": '"log"' });
    const { layout, compare } = await load();
    compare.addCompare("ETHUSDT", "BTCUSDT");
    compare.setCompareSymbols(["ETHUSDT"]);
    compare.removeCompare("ETHUSDT");
    expect(layout.scaleMode()).toBe("normal");
  });

  it("keeps each symbol's color slot when another is removed", async () => {
    stubStorage();
    const { compare } = await load();
    for (const s of ["A", "B", "C"]) compare.addCompare(s, "BTCUSDT");
    compare.removeCompare("A");
    expect(compare.compareEntries()).toEqual([
      { symbol: "B", slot: 1 },
      { symbol: "C", slot: 2 },
    ]);
    compare.addCompare("D", "BTCUSDT");
    expect(compare.compareEntries().at(-1)).toEqual({ symbol: "D", slot: 0 });
    compare.setCompareSymbols(["C", "E"]);
    expect(compare.compareEntries()).toEqual([
      { symbol: "C", slot: 2 },
      { symbol: "E", slot: 0 },
    ]);
  });

  it("restores slots from storage and rebuilds invalid ones", async () => {
    stubStorage({
      "chartgen.layout.compare": '["A","B"]',
      "chartgen.layout.compareSlots": "[3,1]",
    });
    expect((await load()).compare.compareEntries().map((e) => e.slot)).toEqual([
      3, 1,
    ]);
    stubStorage({
      "chartgen.layout.compare": '["A","B"]',
      "chartgen.layout.compareSlots": "[1,1]",
    });
    expect((await load()).compare.compareEntries().map((e) => e.slot)).toEqual([
      0, 1,
    ]);
  });
});
