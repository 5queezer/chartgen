import { describe, expect, it } from "vitest";

import {
  figureOverrides,
  figureStyle,
  plotsOf,
  rekeyPlots,
  resolvePlot,
} from "./plotStyle";

const colorAt = (n: number) => `#00000${n}`;

describe("plotsOf", () => {
  it("keys unique line labels by label and the rest by index", () => {
    const plots = plotsOf(
      {
        lines: [{ label: "MACD" }, { label: null }, { label: "Dup" }, { label: "Dup" }],
        histogram: [{}],
      },
      colorAt,
    );
    expect(plots.map((p) => [p.key, p.indexKeyed, p.label])).toEqual([
      ["l:MACD", false, "MACD"],
      ["line1", true, "line1"],
      ["line2", true, "Dup"],
      ["line3", true, "Dup"],
      ["hist0", true, "hist0"],
    ]);
    expect(plots[4]!.defaultColor).toBe("#000004");
  });

  it("is empty for an indicator without plots", () => {
    expect(plotsOf({}, colorAt)).toEqual([]);
  });

  it("keeps a plot's key when another line is removed before it", () => {
    const before = plotsOf({ lines: [{ label: "WT1" }, { label: "RSI" }] }, colorAt);
    const after = plotsOf({ lines: [{ label: "RSI" }] }, colorAt);
    expect(before[1]!.key).toBe(after[0]!.key);
  });
});

describe("overrides", () => {
  const plots = plotsOf({ lines: [{ label: "WT1" }, { label: "RSI" }] }, colorAt);

  it("a hidden WT1 does not hide RSI after WT1 disappears", () => {
    const style = { plots: { "l:WT1": { visible: false } } };
    const shifted = plotsOf({ lines: [{ label: "RSI" }] }, colorAt);
    expect(resolvePlot(plots[0]!, style).visible).toBe(false);
    expect(resolvePlot(shifted[0]!, style).visible).toBe(true);
    expect(figureOverrides(shifted, style)).toEqual({ plots: {} });
  });

  it("ignores an index-keyed entry whose label no longer matches", () => {
    const dup = plotsOf({ lines: [{ label: "A" }, { label: "A" }] }, colorAt);
    const style = { plots: { line0: { width: 3, label: "A" }, line1: { width: 4, label: "B" } } };
    expect(resolvePlot(dup[0]!, style).width).toBe(3);
    expect(resolvePlot(dup[1]!, style).width).toBe(1);
  });

  it("maps overrides to figure keys", () => {
    const style = { plots: { "l:RSI": { color: "#fff" } } };
    expect(figureOverrides(plots, style)).toEqual({ plots: { line1: { color: "#fff" } } });
  });

  it("re-keys legacy line<i> entries and records labels on index keys", () => {
    const legacy = { plots: { line1: { color: "#fff" }, hist0: { visible: false } } };
    const mixed = plotsOf(
      { lines: [{ label: "WT1" }, { label: "RSI" }, { label: "RSI" }], histogram: [{}] },
      colorAt,
    );
    expect(rekeyPlots(plots, legacy)).toEqual({ "l:RSI": { color: "#fff" } });
    expect(rekeyPlots(mixed, legacy)).toEqual({
      line1: { color: "#fff", label: "RSI" },
      hist0: { visible: false },
    });
  });
});

describe("resolvePlot / figureStyle", () => {
  const [plot] = plotsOf({ lines: [{ label: "A" }] }, () => "#123456");

  it("falls back to defaults", () => {
    expect(resolvePlot(plot!, undefined)).toEqual({
      color: "#123456",
      width: 1,
      lineStyle: "solid",
      visible: true,
    });
  });

  it("applies overrides per plot key only", () => {
    const style = { plots: { "l:A": { color: "#fff", visible: false }, "l:B": { width: 3 } } };
    expect(resolvePlot(plot!, style)).toMatchObject({ color: "#fff", visible: false, width: 1 });
  });

  it("maps line styles to klinecharts figure styles", () => {
    const r = { color: "#fff", width: 2, visible: true };
    expect(figureStyle("line", { ...r, lineStyle: "solid" })).toEqual({
      color: "#fff",
      size: 2,
      style: "solid",
    });
    expect(figureStyle("line", { ...r, lineStyle: "dashed" })).toMatchObject({
      style: "dashed",
      dashedValue: [6, 4],
    });
    expect(figureStyle("line", { ...r, lineStyle: "dotted" })).toMatchObject({
      style: "dashed",
      dashedValue: [2, 3],
    });
    expect(figureStyle("hist", { ...r, lineStyle: "dotted" })).toEqual({ color: "#fff" });
  });
});
