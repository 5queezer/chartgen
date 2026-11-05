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

const sym = (symbol: string) => ({ kind: "symbol", symbol });
const stored = (data: Map<string, string>) =>
  JSON.parse(data.get("chartgen.watchlist.v2") ?? "null");

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
    expect(stored(data).lists[0].items).toEqual([sym("BTCUSDT"), sym("ETHUSDT")]);

    s.removeFromWatchlist("BTCUSDT");
    expect(s.watchlist()).toEqual(["ETHUSDT"]);
    expect(stored(data).lists[0].items).toEqual([sym("ETHUSDT")]);

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

  it("migrates the old single list into 'Watchlist' and keeps the old key", async () => {
    const data = stubStorage({ "chartgen.watchlist.symbols": '["aapl","MSFT"]' });
    const s = await load();
    expect(s.watchlists()).toHaveLength(1);
    expect(s.activeList().name).toBe("Watchlist");
    expect(s.watchlist()).toEqual(["AAPL", "MSFT"]);
    expect(s.activeItems().some((i) => i.kind === "section")).toBe(false);
    expect(data.get("chartgen.watchlist.symbols")).toBe('["aapl","MSFT"]');
    s.addToWatchlist("NVDA");
    expect(stored(data).version).toBe(2);
    expect(stored(data).lists[0].name).toBe("Watchlist");
  });

  it("v2 data wins over the old key and survives a reload", async () => {
    const data = stubStorage({ "chartgen.watchlist.symbols": '["OLD"]' });
    let s = await load();
    const id = s.createList("Tech")!;
    s.addToWatchlist("nvda");
    s.addSection("Chips");
    s.setFlag("NVDA", "green");
    const doc = data.get("chartgen.watchlist.v2")!;
    stubStorage({ "chartgen.watchlist.v2": doc });
    s = await load();
    expect(s.watchlists().map((l) => l.name)).toEqual(["Watchlist", "Tech"]);
    expect(s.activeListId()).toBe(id);
    expect(s.watchlist()).toEqual(["NVDA"]);
    expect(s.activeItems()[1]).toMatchObject({ kind: "section", name: "Chips" });
    expect(s.flags()).toEqual({ NVDA: "green" });
  });

  it("falls back to a fresh list on corrupt v2 data and sanitizes bad entries", async () => {
    stubStorage({ "chartgen.watchlist.v2": '{"version":2,"lists":[]}' });
    let s = await load();
    expect(s.watchlist()).toEqual([...s.DEFAULT_WATCHLIST]);

    stubStorage({
      "chartgen.watchlist.v2": JSON.stringify({
        version: 2,
        active: "nope",
        lists: [
          { id: "a", name: "  ", items: [] },
          { id: "b", name: "B", items: [sym("x"), sym("X"), { kind: "bogus" }, 5] },
          { id: "b", name: "dup", items: [] },
        ],
        flags: { X: "red", Y: "pink", GONE: "blue" },
      }),
    });
    s = await load();
    expect(s.watchlists().map((l) => l.id)).toEqual(["b"]);
    expect(s.activeListId()).toBe("b");
    expect(s.watchlist()).toEqual(["X"]);
    expect(s.flags()).toEqual({ X: "red" });
  });

  it("creates, renames, duplicates and deletes lists", async () => {
    stubStorage();
    const s = await load();
    const first = s.activeListId();
    expect(s.createList("  ")).toBeNull();
    const second = s.createList("Second")!;
    expect(s.activeListId()).toBe(second);
    expect(s.watchlist()).toEqual([]);
    s.addToWatchlist("AAPL");
    s.renameList(second, "Renamed");
    s.renameList(second, " ");
    expect(s.activeList().name).toBe("Renamed");

    const copy = s.duplicateList(second)!;
    expect(s.activeListId()).toBe(copy);
    expect(s.activeList().name).toBe("Renamed copy");
    expect(s.watchlist()).toEqual(["AAPL"]);
    s.addToWatchlist("MSFT");
    s.setActiveList(second);
    expect(s.watchlist()).toEqual(["AAPL"]);

    expect(s.deleteList(second)).toBe(true);
    expect(s.activeListId()).toBe(first);
    expect(s.deleteList(copy)).toBe(true);
    expect(s.deleteList(first)).toBe(false);
    expect(s.watchlists()).toHaveLength(1);
  });

  it("caps lists and symbols per list", async () => {
    stubStorage();
    const s = await load();
    while (s.createList(`L${s.watchlists().length}`) !== null);
    expect(s.watchlists()).toHaveLength(s.MAX_LISTS);
    expect(s.duplicateList(s.activeListId())).toBeNull();
    for (let i = 0; i < 60; i++) s.addToWatchlist(`S${i}`);
    expect(s.watchlist()).toHaveLength(s.MAX_WATCHLIST);
    // Another list has its own budget.
    s.setActiveList(s.watchlists()[0]!.id);
    s.addToWatchlist("S0");
    expect(s.watchlist()).toContain("S0");
  });

  it("sections: add, rename, collapse, remove keeps symbols", async () => {
    stubStorage();
    const s = await load();
    s.setActiveList(s.createList("L")!);
    s.addToWatchlist("A");
    const id = s.addSection(" Tech ")!;
    s.addToWatchlist("B");
    expect(s.addSection("")).toBeNull();
    s.renameSection(id, "Chips");
    s.toggleSection(id);
    expect(s.activeItems()[1]).toMatchObject({ name: "Chips", collapsed: true });
    s.removeSection(id);
    expect(s.activeItems().map((i) => i.kind)).toEqual(["symbol", "symbol"]);
    for (let i = 0; i < 25; i++) s.addSection(`S${i}`);
    expect(s.activeItems().filter((i) => i.kind === "section")).toHaveLength(
      s.MAX_SECTIONS,
    );
  });

  const order = (s: Awaited<ReturnType<typeof load>>) =>
    s.activeItems().map((i) => (i.kind === "symbol" ? i.symbol : `[${i.name}]`));
  const sectionId = (s: Awaited<ReturnType<typeof load>>, at: number) =>
    (s.activeItems()[at] as { id: string }).id;

  async function sectioned() {
    stubStorage();
    const s = await load();
    s.setActiveList(s.createList("L")!);
    for (const x of ["A", "B"]) s.addToWatchlist(x);
    s.addSection("X");
    for (const x of ["C", "D"]) s.addToWatchlist(x);
    s.addSection("Y");
    s.addToWatchlist("E");
    expect(order(s)).toEqual(["A", "B", "[X]", "C", "D", "[Y]", "E"]);
    return s;
  }

  it("moves a symbol under a section and between positions", async () => {
    const s = await sectioned();
    s.moveItem(0, 6); // A before E (under Y)
    expect(order(s)).toEqual(["B", "[X]", "C", "D", "[Y]", "A", "E"]);
    s.moveItem(5, 0); // A to the top
    expect(order(s)).toEqual(["A", "B", "[X]", "C", "D", "[Y]", "E"]);
    s.moveItem(0, 7); // to the very end
    expect(order(s)).toEqual(["B", "[X]", "C", "D", "[Y]", "E", "A"]);
    s.moveItem(6, 6); // no-op
    expect(order(s)).toEqual(["B", "[X]", "C", "D", "[Y]", "E", "A"]);
  });

  it("moves a section together with its symbols", async () => {
    const s = await sectioned();
    s.moveItem(2, 5); // X block right before Y: no change
    expect(order(s)).toEqual(["A", "B", "[X]", "C", "D", "[Y]", "E"]);
    s.moveItem(5, 2); // Y block before X
    expect(order(s)).toEqual(["A", "B", "[Y]", "E", "[X]", "C", "D"]);
    s.moveItem(2, 7); // Y block to the end
    expect(order(s)).toEqual(["A", "B", "[X]", "C", "D", "[Y]", "E"]);
    // Dropping a section onto a symbol snaps to that block's header.
    s.moveItem(5, 4);
    expect(order(s)).toEqual(["A", "B", "[Y]", "E", "[X]", "C", "D"]);
  });

  it("moveItemBy steps over rows hidden in a collapsed section", async () => {
    const s = await sectioned();
    s.moveItemBy(1, 1); // B down: past header X -> top of X
    expect(order(s)).toEqual(["A", "[X]", "B", "C", "D", "[Y]", "E"]);
    s.moveItemBy(2, -1); // B back up
    expect(order(s)).toEqual(["A", "B", "[X]", "C", "D", "[Y]", "E"]);
    s.toggleSection(sectionId(s, 2));
    s.moveItemBy(1, 1); // B down over collapsed X (and C, D) to the top of Y
    expect(order(s)).toEqual(["A", "[X]", "C", "D", "[Y]", "B", "E"]);
    s.moveItemBy(0, -1); // top: no-op
    expect(order(s)[0]).toBe("A");
    s.moveItemBy(1, 1); // section X down over block Y
    expect(order(s)).toEqual(["A", "[Y]", "B", "E", "[X]", "C", "D"]);
    s.moveItemBy(4, -1);
    expect(order(s)).toEqual(["A", "[X]", "C", "D", "[Y]", "B", "E"]);
  });

  it("flags are shared across lists, feed the Flagged view and are read-only there", async () => {
    stubStorage();
    const s = await load();
    const first = s.activeListId();
    s.setFlag("AAPL", "red");
    s.setFlag("BTCUSDT", "blue");
    s.setActiveList(s.createList("Two")!);
    s.addToWatchlist("AAPL");
    s.addToWatchlist("ZZZ");
    s.setActiveList(s.FLAGGED_ID);
    expect(s.isFlaggedView()).toBe(true);
    expect(s.watchlist()).toEqual(["BTCUSDT", "AAPL"]);
    s.addToWatchlist("NEW");
    s.removeFromWatchlist("AAPL");
    expect(s.addSection("x")).toBeNull();
    s.moveItem(0, 2);
    expect(s.watchlist()).toEqual(["BTCUSDT", "AAPL"]);
    expect(s.deleteList(s.FLAGGED_ID)).toBe(false);

    s.setFlag("AAPL", null);
    expect(s.watchlist()).toEqual(["BTCUSDT"]);
    s.setActiveList(first);
    expect(s.flags()).toEqual({ BTCUSDT: "blue" });
  });

  it("drops the flag once no list holds the symbol", async () => {
    stubStorage();
    const s = await load();
    s.setFlag("AAPL", "red");
    s.removeFromWatchlist("AAPL");
    expect(s.flags()).toEqual({});
    s.setFlag("GHOST", "red");
    expect(s.flags()).toEqual({});
  });

  it("a symbol never lands hidden: moves and section removal expand the section", async () => {
    // Alt+Up: E is the first row under Y; it goes above Y, into collapsed X.
    let s = await sectioned();
    s.toggleSection(sectionId(s, 2));
    s.moveItemBy(6, -1);
    expect(order(s)).toEqual(["A", "B", "[X]", "C", "D", "E", "[Y]"]);
    expect(s.activeItems()[2]).toMatchObject({ collapsed: false });

    // Dropping on a collapsed header (end of its block) opens it too.
    s = await sectioned();
    s.toggleSection(sectionId(s, 2));
    s.moveItem(0, 5);
    expect(order(s)).toEqual(["B", "[X]", "C", "D", "A", "[Y]", "E"]);
    expect(s.activeItems()[1]).toMatchObject({ collapsed: false });

    // Removing a section folds its symbols into a collapsed one above.
    s = await sectioned();
    s.toggleSection(sectionId(s, 2));
    s.removeSection(sectionId(s, 5));
    expect(order(s)).toEqual(["A", "B", "[X]", "C", "D", "E"]);
    expect(s.activeItems()[2]).toMatchObject({ collapsed: false });
  });

  it("Alt+Down with two collapsed sections in a row ends up visible", async () => {
    const s = await sectioned();
    s.toggleSection(sectionId(s, 2));
    s.toggleSection(sectionId(s, 5));
    s.moveItemBy(1, 1); // B over collapsed X, into collapsed Y
    expect(order(s)).toEqual(["A", "[X]", "C", "D", "[Y]", "B", "E"]);
    expect(s.activeItems()[4]).toMatchObject({ collapsed: false });
  });

  it("a section never goes above the first one or swallows the ungrouped head", async () => {
    const s = await sectioned();
    s.moveItem(5, 0); // Y to the very top: clamped below the head
    expect(order(s)).toEqual(["A", "B", "[Y]", "E", "[X]", "C", "D"]);
    s.moveItem(2, 0); // first section to the top: no-op
    expect(order(s)).toEqual(["A", "B", "[Y]", "E", "[X]", "C", "D"]);
    s.moveItemBy(2, -1);
    expect(order(s)).toEqual(["A", "B", "[Y]", "E", "[X]", "C", "D"]);
  });

  it("a section can lead a list that has no ungrouped head", async () => {
    stubStorage();
    const s = await load();
    s.setActiveList(s.createList("L")!);
    s.addSection("X");
    s.addToWatchlist("A");
    s.addSection("Y");
    s.moveItem(2, 0);
    expect(order(s)).toEqual(["[Y]", "[X]", "A"]);
  });

  it("picks up another tab's write and keeps this tab's active list", async () => {
    type Handler = (e: { key: string | null }) => void;
    const handlers: Handler[] = [];
    vi.stubGlobal("window", {
      addEventListener: (_: string, fn: Handler) => void handlers.push(fn),
    });
    const data = stubStorage();
    const s = await load();
    const mine = s.createList("Mine")!;
    const other = JSON.parse(data.get("chartgen.watchlist.v2")!);
    other.lists.push({ id: "t", name: "Theirs", items: [sym("ZZZ")] });
    other.active = "t";
    data.set("chartgen.watchlist.v2", JSON.stringify(other));
    handlers.forEach((h) => h({ key: "chartgen.watchlist.v2" }));
    expect(s.watchlists().map((l) => l.name)).toEqual([
      "Watchlist",
      "Mine",
      "Theirs",
    ]);
    expect(s.activeListId()).toBe(mine);
    // Unrelated keys and corrupt documents are ignored.
    data.set("chartgen.watchlist.v2", "{bad");
    handlers.forEach((h) => h({ key: "chartgen.watchlist.v2" }));
    handlers.forEach((h) => h({ key: "other" }));
    expect(s.watchlists()).toHaveLength(3);
  });

  it("never overwrites a document from a newer version", async () => {
    const future = JSON.stringify({ version: 3, lists: [], extra: true });
    const data = stubStorage({ "chartgen.watchlist.v2": future });
    const s = await load();
    expect(s.watchlist()).toEqual([...s.DEFAULT_WATCHLIST]);
    s.addToWatchlist("AAPL");
    s.createList("X");
    expect(data.get("chartgen.watchlist.v2")).toBe(future);
  });
});
