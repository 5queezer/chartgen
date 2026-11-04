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
  return import("./watchlist");
}

beforeEach(() => {
  vi.unstubAllGlobals();
});

describe("watchlist store", () => {
  it("uses the default list, expanded, when storage is empty", async () => {
    stubStorage();
    const s = await load();
    expect(s.watchlist()).toEqual([...s.DEFAULT_WATCHLIST]);
    expect(s.watchlistCollapsed()).toBe(false);
  });

  it("falls back to defaults on corrupt or all-invalid data", async () => {
    stubStorage({
      "chartgen.watchlist.symbols": "{not json",
      "chartgen.watchlist.collapsed": '"yes"',
    });
    let s = await load();
    expect(s.watchlist()).toEqual([...s.DEFAULT_WATCHLIST]);
    expect(s.watchlistCollapsed()).toBe(false);

    stubStorage({ "chartgen.watchlist.symbols": '"BTCUSDT"' });
    s = await load();
    expect(s.watchlist()).toEqual([...s.DEFAULT_WATCHLIST]);

    stubStorage({ "chartgen.watchlist.symbols": '["bad symbol!", 3]' });
    s = await load();
    expect(s.watchlist()).toEqual([...s.DEFAULT_WATCHLIST]);
  });

  it("keeps an explicitly empty list", async () => {
    stubStorage({ "chartgen.watchlist.symbols": "[]" });
    expect((await load()).watchlist()).toEqual([]);
  });

  it("normalizes, dedupes and caps the persisted list", async () => {
    const many = Array.from({ length: 60 }, (_, i) => `S${i}`);
    stubStorage({
      "chartgen.watchlist.symbols": JSON.stringify([
        " aapl ",
        "AAPL",
        "bad symbol!",
        "^gspc",
        ...many,
      ]),
      "chartgen.watchlist.collapsed": "true",
    });
    const s = await load();
    expect(s.watchlist()).toHaveLength(s.MAX_WATCHLIST);
    expect(s.watchlist().slice(0, 3)).toEqual(["AAPL", "^GSPC", "S0"]);
    expect(s.watchlistCollapsed()).toBe(true);
  });

  it("add/remove persist; add ignores invalid, duplicate and over-cap symbols", async () => {
    const data = stubStorage({ "chartgen.watchlist.symbols": '["BTCUSDT"]' });
    const s = await load();
    s.addToWatchlist(" ethusdt ");
    s.addToWatchlist("ETHUSDT");
    s.addToWatchlist("no spaces");
    expect(s.watchlist()).toEqual(["BTCUSDT", "ETHUSDT"]);
    expect(data.get("chartgen.watchlist.symbols")).toBe('["BTCUSDT","ETHUSDT"]');

    s.removeFromWatchlist("BTCUSDT");
    expect(s.watchlist()).toEqual(["ETHUSDT"]);
    expect(data.get("chartgen.watchlist.symbols")).toBe('["ETHUSDT"]');

    for (let i = 0; i < 60; i++) s.addToWatchlist(`S${i}`);
    expect(s.watchlist()).toHaveLength(s.MAX_WATCHLIST);
    expect(s.watchlist().at(-1)).toBe("S48");
  });

  it("persists the collapsed state", async () => {
    const data = stubStorage();
    const s = await load();
    s.setWatchlistCollapsed(true);
    expect(s.watchlistCollapsed()).toBe(true);
    expect(data.get("chartgen.watchlist.collapsed")).toBe("true");
  });

  it("sanitizeWatchlist rejects non-arrays", async () => {
    stubStorage();
    const s = await load();
    expect(s.sanitizeWatchlist({ a: 1 })).toBeNull();
    expect(s.sanitizeWatchlist(["x", "X", null])).toEqual(["X"]);
  });
});
