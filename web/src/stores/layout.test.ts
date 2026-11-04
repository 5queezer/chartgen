import { beforeEach, describe, expect, it, vi } from "vitest";

function stubStorage(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial));
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => void data.set(k, v),
  });
  return data;
}

// The store reads localStorage at import time, so each test imports a
// fresh copy of the module after stubbing storage.
async function load() {
  vi.resetModules();
  return import("./layout");
}

beforeEach(() => {
  vi.unstubAllGlobals();
});

describe("layout store", () => {
  it("uses defaults when storage is empty", async () => {
    stubStorage();
    const s = await load();
    expect(s.symbol()).toBe("BTCUSDT");
    expect(s.timeframe()).toBe("1h");
    expect(s.chartType()).toBe("candles");
    expect(s.recentSymbols()).toEqual([]);
  });

  it("falls back to defaults on corrupt or invalid data", async () => {
    stubStorage({
      "chartgen.layout.symbol": "{not json",
      "chartgen.layout.timeframe": '"7m"',
      "chartgen.layout.chartType": "42",
      "chartgen.layout.recents": '"nope"',
    });
    const s = await load();
    expect(s.symbol()).toBe("BTCUSDT");
    expect(s.timeframe()).toBe("1h");
    expect(s.chartType()).toBe("candles");
    expect(s.recentSymbols()).toEqual([]);
  });

  it("restores valid persisted values and drops invalid recents", async () => {
    stubStorage({
      "chartgen.layout.symbol": '"AAPL"',
      "chartgen.layout.timeframe": '"4h"',
      "chartgen.layout.chartType": '"area"',
      "chartgen.layout.recents": '["AAPL","MSFT","AAPL",3]',
    });
    const s = await load();
    expect(s.symbol()).toBe("AAPL");
    expect(s.timeframe()).toBe("4h");
    expect(s.chartType()).toBe("area");
    expect(s.recentSymbols()).toEqual(["AAPL", "MSFT"]);
  });

  it("persists changes", async () => {
    const data = stubStorage();
    const s = await load();
    s.setTimeframe("15m");
    s.setChartType("area");
    s.setSymbol(" eth ");
    expect(data.get("chartgen.layout.timeframe")).toBe('"15m"');
    expect(data.get("chartgen.layout.chartType")).toBe('"area"');
    expect(data.get("chartgen.layout.symbol")).toBe('"ETH"');
    expect(s.symbol()).toBe("ETH");
  });

  it("setSymbol does not touch recents; addRecent dedupes, newest first, capped at 10", async () => {
    stubStorage();
    const s = await load();
    s.setSymbol("TYPO");
    expect(s.recentSymbols()).toEqual([]);
    for (let i = 0; i < 12; i++) s.addRecent(`SYM${i}`);
    expect(s.recentSymbols()).toHaveLength(10);
    expect(s.recentSymbols()[0]).toBe("SYM11");
    s.addRecent("SYM5");
    expect(s.recentSymbols()[0]).toBe("SYM5");
    expect(s.recentSymbols().filter((x) => x === "SYM5")).toHaveLength(1);
    expect(s.recentSymbols()).toHaveLength(10);
  });

  it("normalizes persisted symbol and recents on load", async () => {
    stubStorage({
      "chartgen.layout.symbol": '"  eth "',
      "chartgen.layout.recents": JSON.stringify([
        " aapl ",
        "AAPL",
        "bad symbol!",
        "X".repeat(33),
        "^gspc",
        "",
      ]),
    });
    const s = await load();
    expect(s.symbol()).toBe("ETH");
    expect(s.recentSymbols()).toEqual(["AAPL", "^GSPC"]);
  });

  it("rejects a persisted symbol with bad characters or length", async () => {
    stubStorage({ "chartgen.layout.symbol": '"<script>"' });
    expect((await load()).symbol()).toBe("BTCUSDT");
    stubStorage({ "chartgen.layout.symbol": JSON.stringify("A".repeat(40)) });
    expect((await load()).symbol()).toBe("BTCUSDT");
  });

  it("ignores blank or invalid symbols", async () => {
    stubStorage();
    const s = await load();
    s.setSymbol("   ");
    s.setSymbol("no spaces");
    expect(s.symbol()).toBe("BTCUSDT");
  });
});
