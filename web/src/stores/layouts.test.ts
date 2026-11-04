import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  MAX_LAYOUTS,
  isDirty,
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
    expect(f.items[0]!.state.indicators).toEqual(["rsi"]);
    expect(f.items[0]!.state).not.toHaveProperty("drawingsBySymbol");
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

  it("delete moves the active layout and refuses to empty the list", () => {
    const f = withAdded(file(), "B", base, "b", 2)!;
    const g = withoutLayout(f, "b")!;
    expect(g.active).toBe("a");
    expect(withoutLayout(f, "a")!.active).toBe("b");
    expect(withoutLayout(g, "a")).toBeNull();
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
    expect(isDirty(base, { ...base, symbol: "ETHUSDT" })).toBe(true);
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
      indicators: ["ema_stack", { name: "rsi", length: 14 }],
    });

    lay.setTimeframe("15m");
    const a = l.layouts()[0]!;
    l.switchLayout(a.id);
    expect(lay.timeframe()).toBe("4h");
    expect(lay.chartType()).toBe("line");
    expect(l.layoutDirty()).toBe(false);
  });
});
