import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  MAX_LAYOUTS,
  MAX_IMPORT_BYTES,
  MAX_INDICATORS,
  isDirty,
  normalizeIndicators,
  parseExport,
  parseLayouts,
  parseState,
  serializeExport,
  uniqueName,
  withAdded,
  withRenamed,
  withSaved,
  withoutLayout,
  type Layout,
  type LayoutState,
} from "./layouts";

const base: LayoutState = {
  symbol: "BTCUSDT",
  timeframe: "1h",
  chartType: "candles",
  scaleMode: "normal",
  timeZone: "local",
  compare: [],
  indicators: ["ema_stack", { name: "rsi", length: 14 }],
};

const file = () => parseLayouts(undefined, base, "a", 1);

describe("parseLayouts", () => {
  it("migrates a fresh install into an Unnamed layout of the current state", () => {
    const f = file();
    expect(f.active).toBe("a");
    expect(f.items).toEqual([
      { id: "a", name: "Unnamed", updatedAt: 1, state: base },
    ]);
  });

  it("round-trips through JSON", () => {
    const f = withAdded(file(), "B", { ...base, timeframe: "4h" }, "b", 2)!;
    expect(parseLayouts(JSON.parse(JSON.stringify(f)), base, "x", 9)).toEqual(
      f,
    );
  });

  it("falls back to the default layout on corruption", () => {
    for (const raw of [
      null,
      42,
      "x",
      [],
      { version: 2, items: [] },
      { version: 1, items: "no" },
    ]) {
      expect(
        parseLayouts(raw, base, "a", 1).items.map((l) => l.name),
      ).toEqual(["Unnamed"]);
    }
  });

  it("drops malformed items, duplicate ids and bad fields", () => {
    const good = file().items[0]!;
    const raw = {
      version: 1,
      active: "gone",
      items: [
        null,
        { ...good, id: "" },
        { ...good, name: "  " },
        { ...good, id: "b", state: { ...good.state, timeframe: "7m" } },
        { ...good, id: "c", state: { ...good.state, indicators: "rsi" } },
        {
          ...good,
          id: "d",
          name: "Keep",
          state: {
            ...good.state,
            indicators: ["rsi", "rsi", 3, {}],
            drawingsBySymbol: { BTCUSDT: [] },
          },
        },
        { ...good, id: "d", name: "Dupe id" },
      ],
    };
    const f = parseLayouts(raw, base, "x", 1);
    expect(f.items.map((l) => l.id)).toEqual(["d"]);
    expect(f.active).toBe("d");
    // A repeated name is a second instance (ADR-0028).
    expect(f.items[0]!.state.indicators).toEqual(["rsi", { name: "rsi", id: "rsi#2" }]);
    expect(f.items[0]!.state).not.toHaveProperty("drawingsBySymbol");
  });

  it("keeps indicator instances and their ids (ADR-0028)", () => {
    const good = file().items[0]!;
    const indicators = [
      { name: "rsi", length: 14 },
      { name: "rsi", id: "rsi#2", length: 21, hidden: true },
      { name: "rsi", id: "rsi#2" },
    ];
    const state = parseState({ ...good.state, indicators })!;
    expect(state.indicators).toEqual([
      { name: "rsi", length: 14 },
      { name: "rsi", id: "rsi#2", length: 21, hidden: true },
      { name: "rsi", id: "rsi#3" },
    ]);
    // Instances are part of the layout: a new or edited one is a change.
    expect(isDirty(state, { ...state, indicators: state.indicators.slice(0, 2) })).toBe(true);
    const renumbered = state.indicators.map((e, i) =>
      i === 2 ? { name: "rsi", id: "rsi#4" } : e,
    );
    expect(isDirty(state, { ...state, indicators: renumbered })).toBe(true);
    expect(isDirty(state, { ...state, indicators: [...state.indicators] })).toBe(false);
  });

  it("caps at MAX_LAYOUTS", () => {
    const good = file().items[0]!;
    const items = Array.from({ length: MAX_LAYOUTS + 10 }, (_, i) => ({
      ...good,
      id: `i${i}`,
      name: `L${i}`,
    }));
    expect(
      parseLayouts({ version: 1, active: "i0", items }, base, "x", 1).items,
    ).toHaveLength(MAX_LAYOUTS);
  });

  it("de-duplicates names loaded from storage", () => {
    const good = file().items[0]!;
    const items = [good, { ...good, id: "b" }];
    const f = parseLayouts({ version: 1, active: "a", items }, base, "x", 1);
    expect(f.items.map((l) => l.name)).toEqual(["Unnamed", "Unnamed (2)"]);
  });
});

describe("reducers", () => {
  it("refuses to add past the cap or with an empty name", () => {
    let f = file();
    for (let i = 1; i < MAX_LAYOUTS; i++) {
      f = withAdded(f, `L${i}`, base, `i${i}`, 1)!;
    }
    expect(f.items).toHaveLength(MAX_LAYOUTS);
    expect(withAdded(f, "one more", base, "z", 1)).toBeNull();
    expect(withAdded(file(), "   ", base, "z", 1)).toBeNull();
  });

  it("adding makes the new layout active and suffixes a taken name", () => {
    const f = withAdded(file(), "unnamed", base, "b", 2)!;
    expect(f.active).toBe("b");
    expect(f.items[1]!.name).toBe("unnamed (2)");
  });

  it("rename handles collisions, keeps own name and ignores empty", () => {
    const f = withAdded(file(), "B", base, "b", 2)!;
    expect(withRenamed(f, "b", "Unnamed").items[1]!.name).toBe("Unnamed (2)");
    expect(withRenamed(f, "b", "b").items[1]!.name).toBe("b");
    expect(withRenamed(f, "b", "  ")).toBe(f);
    expect(uniqueName(f.items, "B", "b")).toBe("B");
  });

  it("delete removes other layouts but never the active one", () => {
    const f = withAdded(file(), "B", base, "b", 2)!;
    expect(f.active).toBe("b");
    expect(withoutLayout(f, "a").items.map((l) => l.id)).toEqual(["b"]);
    expect(withoutLayout(f, "b")).toBe(f);
  });
});

describe("snapshot and dirty", () => {
  it("detects changes in every field and ignores key order", () => {
    expect(isDirty(base, { ...base })).toBe(false);
    expect(
      isDirty(base, {
        ...base,
        indicators: ["ema_stack", { length: 14, name: "rsi" }],
      }),
    ).toBe(false);
    expect(
      isDirty(base, {
        ...base,
        indicators: [{ length: 14, name: "rsi" }, "ema_stack"],
      }),
    ).toBe(true);
    expect(isDirty(base, { ...base, timeframe: "4h" })).toBe(true);
    expect(isDirty(base, { ...base, chartType: "line" })).toBe(true);
    expect(isDirty(base, { ...base, scaleMode: "log" })).toBe(true);
    expect(isDirty(base, { ...base, timeZone: "utc" })).toBe(true);
    expect(isDirty(base, { ...base, symbol: "ETHUSDT" })).toBe(true);
    expect(isDirty(base, { ...base, compare: ["ETHUSDT"] })).toBe(true);
  });

  it("keeps validated compare symbols and defaults to none for older layouts", () => {
    const withCompare = parseState({ ...base, compare: ["ethusdt", "x!", "SOLUSDT"] });
    expect(withCompare?.compare).toEqual(["ETHUSDT", "SOLUSDT"]);
    const { compare: _omit, ...older } = base;
    expect(parseState(older)?.compare).toEqual([]);
  });

  it("keeps invert and lock only when true and counts them as changes", () => {
    const on = parseState({ ...base, invertScale: true, lockRatio: true });
    expect(on?.invertScale).toBe(true);
    expect(on?.lockRatio).toBe(true);
    expect(parseState({ ...base, invertScale: "yes" })).not.toHaveProperty("invertScale");
    expect(isDirty(base, { ...base, invertScale: true })).toBe(true);
    expect(isDirty(base, { ...base, lockRatio: true })).toBe(true);
  });

  it("parses a state saved before time zones as the live zone", () => {
    const { timeZone: _omit, ...old } = base;
    expect(parseState(old)?.timeZone).toBe("local");
    expect(parseState({ ...old, timeZone: "Mars/Base" })?.timeZone).toBe("local");
    expect(parseState({ ...old, timeZone: "utc" })?.timeZone).toBe("utc");
  });

  it("ignores params equal to the registry default, but not hidden", () => {
    const defaults = { rsi: { length: 14 } };
    const bare = { ...base, indicators: ["ema_stack", "rsi"] };
    expect(isDirty(base, bare)).toBe(true);
    expect(isDirty(base, bare, defaults)).toBe(false);
    expect(
      isDirty(base, { ...base, indicators: ["ema_stack", { name: "rsi", length: 21 }] }, defaults),
    ).toBe(true);
    expect(
      isDirty(base, { ...base, indicators: ["ema_stack", { name: "rsi", length: 14, hidden: true }] }, defaults),
    ).toBe(true);
    expect(normalizeIndicators(base.indicators, defaults)).toEqual(["ema_stack", "rsi"]);
  });

  it("withSaved clears the dirty state", () => {
    const live = { ...base, timeframe: "4h" as const };
    const f = withSaved(file(), "a", live, 5);
    expect(isDirty(f.items[0]!.state, live)).toBe(false);
    expect(f.items[0]!.updatedAt).toBe(5);
  });
});

describe("export", () => {
  it("round-trips and rejects garbage", () => {
    const layout: Layout = file().items[0]!;
    expect(parseExport(serializeExport(layout))).toEqual({
      name: "Unnamed",
      state: base,
    });
    expect(parseExport("{nope")).toBeNull();
    expect(parseExport('{"name":"x","state":{}}')).toBeNull();
    expect(parseState({ ...base, symbol: "" })).toBeNull();
  });

  const exportOf = (state: unknown, version: unknown = 1) =>
    JSON.stringify({ version, name: "X", state });

  it("requires version 1 and rejects oversized files", () => {
    expect(parseExport(exportOf(base))).not.toBeNull();
    expect(parseExport(exportOf(base, 2))).toBeNull();
    expect(parseExport(exportOf(base, null))).toBeNull();
    const big = { ...base, indicators: ["a".repeat(MAX_IMPORT_BYTES)] };
    expect(parseExport(exportOf(big))).toBeNull();
  });

  it("caps indicators", () => {
    const many = Array.from({ length: MAX_INDICATORS + 20 }, (_, i) => `i${i}`);
    expect(parseExport(exportOf({ ...base, indicators: many }))!.state.indicators).toHaveLength(MAX_INDICATORS);
  });

  it("keeps only primitive params and the hidden flag", () => {
    const state = {
      ...base,
      indicators: [
        { name: "rsi", length: 14, mode: "x", on: true, nested: { a: 1 }, list: [1], nil: null, inf: 1e999, hidden: true },
      ],
    };
    expect(parseExport(exportOf(state))!.state.indicators).toEqual([
      { name: "rsi", length: 14, mode: "x", on: true, hidden: true },
    ]);
  });

  it("keeps a valid plot style and drops an invalid one on import", () => {
    const good = { plots: { "l:RSI": { color: "#f00", width: 2 } } };
    const state = {
      ...base,
      indicators: [
        { name: "rsi", style: good },
        { name: "ema_stack", style: { plots: { line0: { color: "red" } } } },
      ],
    };
    expect(parseExport(exportOf(state))!.state.indicators).toEqual([
      { name: "rsi", style: good },
      { name: "ema_stack" },
    ]);
  });

  it("a plot style change marks the layout dirty, regardless of key order", () => {
    const a = { plots: { "l:RSI": { color: "#f00", width: 2 }, hist0: { visible: false } } };
    const b = { plots: { hist0: { visible: false }, "l:RSI": { width: 2, color: "#f00" } } };
    const withStyle = (style: unknown) => ({
      ...base,
      indicators: ["ema_stack", { name: "rsi", style }],
    });
    expect(isDirty(base, withStyle(a))).toBe(true);
    expect(isDirty(withStyle(a), withStyle(b))).toBe(false);
    expect(isDirty(withStyle(a), withStyle({ plots: { "l:RSI": { width: 3 } } }))).toBe(true);
  });

  it("drops prototype-polluting keys", () => {
    const text = exportOf(base).replace(
      '"indicators":[',
      '"indicators":[{"name":"evil","__proto__":{"polluted":1},"constructor":5,"prototype":6,"ok":1},',
    );
    const [evil] = parseExport(text)!.state.indicators;
    expect(evil).toEqual({ name: "evil", ok: 1 });
    expect(Object.keys(evil as object)).toEqual(["name", "ok"]);
    expect(({} as Record<string, unknown>).polluted).toBeUndefined();
  });
});

describe("live bindings", () => {
  beforeEach(() => vi.unstubAllGlobals());

  function stub(initial: Record<string, string> = {}) {
    const data = new Map(Object.entries(initial));
    vi.stubGlobal("localStorage", {
      getItem: (k: string) => data.get(k) ?? null,
      setItem: (k: string, v: string) => void data.set(k, v),
      removeItem: (k: string) => void data.delete(k),
    });
    return data;
  }

  const KEY = "chartgen.layouts";

  /** Add a layout to the stored file, as another tab would. */
  function otherTabAdds(data: Map<string, string>) {
    const f = JSON.parse(data.get(KEY)!);
    f.items.push({ ...f.items[0], id: "other", name: "Other" });
    data.set(KEY, JSON.stringify(f));
  }

  it("keeps layouts another tab wrote between load and update", async () => {
    const data = stub();
    vi.resetModules();
    const l = await import("./layouts");
    otherTabAdds(data);
    l.renameLayout(l.activeLayout().id, "Mine");
    const names = JSON.parse(data.get(KEY)!).items.map((x: Layout) => x.name);
    expect(names).toEqual(["Mine", "Other"]);
  });

  it("refreshes from the storage event and keeps this tab's active layout", async () => {
    const data = stub();
    const handlers: ((e: { key: string | null }) => void)[] = [];
    vi.stubGlobal("window", { addEventListener: (_: string, h: never) => handlers.push(h) });
    vi.resetModules();
    const l = await import("./layouts");
    const active = l.activeLayout().id;
    otherTabAdds(data);
    handlers.forEach((h) => h({ key: KEY }));
    expect(l.layouts().map((x) => x.name)).toEqual(["Unnamed", "Other"]);
    expect(l.activeLayout().id).toBe(active);
  });

  it("never overwrites an unusable stored value with the fallback", async () => {
    for (const bad of ["{not json", '{"version":2,"items":[]}', '{"version":1,"active":"a","items":[null,{}]}']) {
      const data = stub({ [KEY]: bad });
      vi.resetModules();
      const l = await import("./layouts");
      expect(l.activeLayout().name).toBe("Unnamed");
      expect(data.get(KEY)).toBe(bad);
      expect(data.get(KEY + ".bak")).toBe(bad);

      l.renameLayout(l.activeLayout().id, "Fresh");
      expect(JSON.parse(data.get(KEY)!).items[0].name).toBe("Fresh");
      expect(data.get(KEY + ".bak")).toBe(bad);
    }
  });

  it("keeps an existing backup instead of replacing it", async () => {
    const data = stub({ [KEY]: "{bad", [KEY + ".bak"]: "first" });
    vi.resetModules();
    await import("./layouts");
    expect(data.get(KEY + ".bak")).toBe("first");
  });

  it("seeds Unnamed from the current stores and tracks dirty/save/switch", async () => {
    const data = stub({ "chartgen.layout.timeframe": '"4h"' });
    vi.resetModules();
    const l = await import("./layouts");
    const lay = await import("./layout");
    expect(l.activeLayout().name).toBe("Unnamed");
    expect(l.activeLayout().state.timeframe).toBe("4h");
    expect(JSON.parse(data.get("chartgen.layouts")!).items).toHaveLength(1);
    expect(l.layoutDirty()).toBe(false);

    lay.setChartType("line");
    expect(l.layoutDirty()).toBe(true);
    l.saveLayout();
    expect(l.layoutDirty()).toBe(false);

    expect(l.createLayout("B")).toBe(true);
    expect(l.activeLayout().name).toBe("B");
    expect(l.activeLayout().state).toMatchObject({
      symbol: "BTCUSDT",
      timeframe: "4h",
      chartType: "candles",
      indicators: ["ema_stack", "volume", { name: "rsi", length: 14 }],
    });

    lay.setTimeframe("15m");
    const a = l.layouts()[0]!;
    l.switchLayout(a.id);
    expect(lay.timeframe()).toBe("4h");
    expect(lay.chartType()).toBe("line");
    expect(l.layoutDirty()).toBe(false);
  });
});
