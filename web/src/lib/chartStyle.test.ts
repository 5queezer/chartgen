import { describe, expect, it } from "vitest";

import { composeStyles, resolveColors, settingsStyles } from "./chartStyle";
import { dark, light } from "./theme";
import { DEFAULT_OPTIONS } from "../stores/chartSettings";

const build = (
  o = {},
  opts: Partial<typeof DEFAULT_OPTIONS> = {},
  type: "candles" | "area" = "candles",
) => settingsStyles(dark, o, { ...DEFAULT_OPTIONS, ...opts }, type);

describe("resolveColors", () => {
  it("falls back to the palette", () => {
    const c = resolveColors(light, {});
    expect(c.bodyUp).toBe(light.bull);
    expect(c.bodyDown).toBe(light.bear);
    expect(c.background).toBe(light.bg);
    expect(c.grid).toBe(light.grid);
    expect(c.crosshair).toBe(light.crosshair);
  });

  it("lets border and wick follow the body until overridden", () => {
    const c = resolveColors(dark, { bodyUp: "#111111", wickUp: "#222222" });
    expect(c.borderUp).toBe("#111111");
    expect(c.wickUp).toBe("#222222");
    expect(c.borderDown).toBe(dark.bear);
  });
});

describe("settingsStyles", () => {
  it("matches the palette for empty overrides", () => {
    const s = build();
    expect(s.candle.bar.upColor).toBe(dark.bull);
    expect(s.candle.bar.downBorderColor).toBe(dark.bear);
    expect(s.grid.vertical).toEqual({ show: true, color: dark.grid });
    expect(s.yAxis.position).toBe("right");
    expect(s.candle.priceMark.high.show).toBe(false);
  });

  it("applies color overrides and part toggles", () => {
    const s = build({ bodyUp: "#ff0000", borderOn: false, wickOn: false });
    expect(s.candle.bar.upColor).toBe("#ff0000");
    expect(s.candle.bar.upBorderColor).toBe("rgba(0, 0, 0, 0)");
    expect(s.candle.bar.downWickColor).toBe("rgba(0, 0, 0, 0)");
    expect(s.candle.bar.downColor).toBe(dark.bear);
  });

  it("maps grid, price mark and scale options", () => {
    const s = build(
      {},
      {
        gridHorizontal: false,
        lastPriceLine: false,
        lastPriceLabel: false,
        highLowMarks: true,
        scalePlacement: "left",
      },
    );
    expect(s.grid.horizontal.show).toBe(false);
    expect(s.candle.priceMark.last.line.show).toBe(false);
    expect(s.candle.priceMark.last.text.show).toBe(false);
    expect(s.candle.priceMark.low.show).toBe(true);
    expect(s.yAxis.position).toBe("left");
  });

  it("maps crosshair style and color", () => {
    const s = build({ crosshair: "#00ff00" }, { crosshairStyle: "dotted" });
    expect(s.crosshair.vertical.line).toEqual({
      color: "#00ff00",
      style: "dashed",
      dashedValue: [1, 3],
    });
    expect(
      build({}, { crosshairStyle: "solid" }).crosshair.horizontal.line.style,
    ).toBe("solid");
  });

  it("ignores the body toggle for bars, which have no other fill", () => {
    const bars = settingsStyles(dark, { bodyOn: false }, DEFAULT_OPTIONS, "bars");
    expect(bars.candle.bar.upColor).toBe(dark.bull);
    expect(build({ bodyOn: false }).candle.bar.upColor).toBe("rgba(0, 0, 0, 0)");
  });

  it("recolors the area fill only for a picked line color", () => {
    expect(build({}, {}, "area").candle.area).not.toHaveProperty(
      "backgroundColor",
    );
    const area = build({ line: "#ff0000" }, {}, "area").candle.area;
    expect(area.lineColor).toBe("#ff0000");
    expect(area).toMatchObject({
      backgroundColor: [
        { offset: 0, color: "rgba(255, 0, 0, 0.28)" },
        { offset: 1, color: "rgba(255, 0, 0, 0.02)" },
      ],
    });
    expect(
      build({ line: "#ff0000" }, {}, "candles").candle.area,
    ).not.toHaveProperty("backgroundColor");
  });
});

describe("composeStyles", () => {
  it("deep-merges layers, later winning, replacing arrays whole", () => {
    expect(
      composeStyles(
        { a: { x: 1, y: [1, 2] }, b: 1 },
        { a: { y: [3] }, c: 2 },
        { a: { x: 4 } },
      ),
    ).toEqual({ a: { x: 4, y: [3] }, b: 1, c: 2 });
  });
});
