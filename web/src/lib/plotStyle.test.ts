import { describe, expect, it } from "vitest";

import { figureStyle, plotsOf, resolvePlot } from "./plotStyle";

const colorAt = (n: number) => `#00000${n}`;

describe("plotsOf", () => {
  it("maps lines then histograms to stable plot keys", () => {
    const plots = plotsOf(
      { lines: [{ label: "MACD" }, { label: null }], histogram: [{}] },
      colorAt,
    );
    expect(plots).toEqual([
      { key: "line0", kind: "line", index: 0, label: "MACD", defaultColor: "#000000" },
      { key: "line1", kind: "line", index: 1, label: "line1", defaultColor: "#000001" },
      { key: "hist0", kind: "hist", index: 0, label: "hist0", defaultColor: "#000002" },
    ]);
  });

  it("is empty for an indicator without plots", () => {
    expect(plotsOf({}, colorAt)).toEqual([]);
  });
});

describe("resolvePlot / figureStyle", () => {
  const plot = { key: "line0", defaultColor: "#123456" };

  it("falls back to defaults", () => {
    expect(resolvePlot(plot, undefined)).toEqual({
      color: "#123456",
      width: 1,
      lineStyle: "solid",
      visible: true,
    });
  });

  it("applies overrides per plot key only", () => {
    const style = { plots: { line0: { color: "#fff", visible: false }, line1: { width: 3 } } };
    expect(resolvePlot(plot, style)).toMatchObject({ color: "#fff", visible: false, width: 1 });
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
