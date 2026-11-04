import { afterEach, describe, expect, it, vi } from "vitest";

const KEY = "chartgen.indicators.active";
const style = { plots: { line0: { color: "#ff0000", width: 2 } } };

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

describe("indicator plot styles (ADR-0017)", () => {
  it("plotKey is kind plus index", async () => {
    const { plotKey } = await loadStore();
    expect(plotKey("line", 0)).toBe("line0");
    expect(plotKey("hist", 2)).toBe("hist2");
  });

  it("sanitizeStyle keeps valid colors and drops the rest", async () => {
    const { sanitizeStyle } = await loadStore();
    const ok = ["#f00", "#ff0000", "#ff000080", "rgba(1, 2, 3, 0.5)", "rgba(0,0,0,1)"];
    for (const color of ok) {
      expect(sanitizeStyle({ plots: { line0: { color } } })).toEqual({
        plots: { line0: { color } },
      });
    }
    for (const color of ["red", "#ff", "#ggg", "rgba(1,2,3)", "url(x)", 5, null]) {
      expect(sanitizeStyle({ plots: { line0: { color } } })).toBeUndefined();
    }
  });

  it("sanitizeStyle validates width, lineStyle, visible and plot keys", async () => {
    const { sanitizeStyle } = await loadStore();
    expect(
      sanitizeStyle({
        plots: {
          line0: { width: 4, lineStyle: "dotted", visible: false },
          line1: { width: 5, lineStyle: "wavy", visible: "no" },
          line2: { width: 1.5 },
          bogus: { width: 2 },
          hist0: { width: 0, visible: true },
        },
      }),
    ).toEqual({
      plots: {
        line0: { width: 4, lineStyle: "dotted", visible: false },
        hist0: { visible: true },
      },
    });
    expect(sanitizeStyle("x")).toBeUndefined();
    expect(sanitizeStyle({ plots: [] })).toBeUndefined();
    expect(sanitizeStyle({})).toBeUndefined();
  });

  it("loads old entries untouched and sanitizes stored styles", async () => {
    stubStorage(
      JSON.stringify([
        "ema_stack",
        { name: "rsi", length: 14, style },
        { name: "macd", style: { plots: { line0: { color: "nope" } } } },
      ]),
    );
    const { activeIndicators } = await loadStore();
    expect(activeIndicators()).toEqual([
      "ema_stack",
      { name: "rsi", length: 14, style },
      "macd",
    ]);
  });

  it("keeps style across param and visibility edits, clears it with undefined", async () => {
    const { withStyle, withParams, withHidden, indicatorStyle, indicatorParams } =
      await loadStore();
    const styled = withStyle(["rsi"], 0, style);
    expect(styled).toEqual([{ name: "rsi", style }]);
    const edited = withHidden(withParams(styled, 0, { length: 9 }), 0, true);
    expect(edited).toEqual([{ name: "rsi", length: 9, hidden: true, style }]);
    expect(indicatorStyle(edited[0]!)).toEqual(style);
    expect(indicatorParams(edited[0]!)).toEqual({ length: 9 });
    expect(withStyle(edited, 0, undefined)).toEqual([
      { name: "rsi", length: 9, hidden: true },
    ]);
    expect(withStyle(styled, 0, undefined)).toEqual(["rsi"]);
  });

  it("strips style from the wire", async () => {
    const { toWire } = await loadStore();
    expect(
      toWire([{ name: "rsi", length: 14, style }, { name: "macd", style }]),
    ).toEqual([{ name: "rsi", length: 14 }, "macd"]);
  });
});
