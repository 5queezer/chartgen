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

  it("accepts heikin_ashi and maps chart types to server styles", async () => {
    stubStorage({ "chartgen.layout.chartType": '"heikin_ashi"' });
    const s = await load();
    expect(s.chartType()).toBe("heikin_ashi");
    expect(s.chartStyleOf("heikin_ashi")).toBe("heikin_ashi");
    expect(s.chartStyleOf("hollow")).toBe("candles");
    expect(s.chartStyleOf("candles")).toBe("candles");
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
    // `/` would add a path segment to the upstream URL.
    stubStorage({ "chartgen.layout.symbol": '"../../X"' });
    const s = await load();
    expect(s.symbol()).toBe("BTCUSDT");
    expect(s.normalizeSymbol("BTC/USDT")).toBeNull();
  });

  it("ignores blank or invalid symbols", async () => {
    stubStorage();
    const s = await load();
    s.setSymbol("   ");
    s.setSymbol("no spaces");
    expect(s.symbol()).toBe("BTCUSDT");
  });

  it("defaults the price scale to normal + auto-fit", async () => {
    stubStorage();
    const s = await load();
    expect(s.scaleMode()).toBe("normal");
    expect(s.autoScale()).toBe(true);
  });

  it("ignores a stored autoScale key: auto-fit always starts on", async () => {
    const data = stubStorage({ "chartgen.layout.autoScale": "false" });
    const s = await load();
    expect(s.autoScale()).toBe(true);
    s.setAutoScale(false);
    expect(data.get("chartgen.layout.autoScale")).toBe("false"); // untouched leftover
    expect(s.autoScale()).toBe(false);
  });

  it("restores a valid scale mode and rejects invalid values", async () => {
    stubStorage({ "chartgen.layout.scaleMode": '"log"' });
    expect((await load()).scaleMode()).toBe("log");
    stubStorage({ "chartgen.layout.scaleMode": '"sqrt"' });
    expect((await load()).scaleMode()).toBe("normal");
  });

  it("persists the scale mode", async () => {
    const data = stubStorage();
    const s = await load();
    s.setScaleMode("percentage");
    expect(data.get("chartgen.layout.scaleMode")).toBe('"percentage"');
  });

  it("validates the stored time zone and persists changes", async () => {
    stubStorage({ "chartgen.layout.timeZone": '"Mars/Base"' });
    expect((await load()).timeZone()).toBe("local");
    const data = stubStorage({ "chartgen.layout.timeZone": '"Europe/Berlin"' });
    const s = await load();
    expect(s.timeZone()).toBe("Europe/Berlin");
    s.setTimeZone("utc");
    expect(data.get("chartgen.layout.timeZone")).toBe('"utc"');
    s.setTimeZone("nope");
    expect(s.timeZone()).toBe("utc");
  });

  it("validates the stored theme and persists changes", async () => {
    const data = stubStorage({ "chartgen.layout.theme": '"light"' });
    let s = await load();
    expect(s.theme()).toBe("light");
    s.setTheme("dark");
    expect(data.get("chartgen.layout.theme")).toBe('"dark"');

    stubStorage({ "chartgen.layout.theme": '"neon"' });
    s = await load();
    expect(s.theme()).toBe("dark");
  });

  it("defaults to the OS color scheme when nothing is stored", async () => {
    stubStorage();
    vi.stubGlobal("matchMedia", (q: string) => ({
      matches: q === "(prefers-color-scheme: light)",
    }));
    const s = await load();
    expect(s.theme()).toBe("light");
  });
});
