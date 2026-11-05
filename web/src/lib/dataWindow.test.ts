import { describe, expect, it } from "vitest";

import type { SeriesPayload } from "../types/responses";
import { dataWindowRows } from "./dataWindow";

const T0 = Date.UTC(2026, 10, 3, 12, 0) / 1000;

const series: SeriesPayload = {
  symbol: "BTCUSDT",
  interval: "1h",
  has_more: false,
  bars: [
    { t: T0, o: 10, h: 12, l: 9, c: 11, v: 1500 },
    { t: T0 + 3600, o: 11, h: 14, l: 10, c: 13, v: 2_500_000 },
    { t: T0 + 7200, o: 13, h: 13.5, l: 12, c: 12.5, v: 40 },
  ],
  indicators: {
    rsi: {
      label: "RSI",
      is_overlay: false,
      lines: [{ index: 0, label: "RSI", values: [null, 55.123, 60] }],
    },
    macd: {
      label: "MACD",
      is_overlay: false,
      lines: [{ index: 0, label: "MACD", values: [0.1, 0.2, 0.3] }],
      histogram: [{ index: 0, values: [1, -2, 3] }],
    },
  },
};

const opts = { timeZone: "UTC", colorAt: (_: string, n: number) => `#00000${n}` };

describe("dataWindowRows", () => {
  it("reads the bar at the crosshair index", () => {
    const d = dataWindowRows(series, 1, {}, [], opts);
    expect(d.symbol).toBe("BTCUSDT");
    expect(d.interval).toBe("1h");
    expect(d.time).toBe("2026-11-03 13:00");
    expect(d.bar).toMatchObject({ o: "11.00", h: "14.00", l: "10.00", c: "13.00", v: "2.5M" });
    expect(d.bar?.change).toMatchObject({ abs: "+2.00", pct: "+18.18%" });
  });

  it("formats overlays with the price precision, not K", () => {
    const fx: SeriesPayload = {
      ...series,
      bars: series.bars.map((b, i) => ({ ...b, o: 1.1236, h: 1.1239, l: 1.1233, c: 1.12385 + i * 0.00001, v: 0 })),
      indicators: {
        ema: { label: "EMA", is_overlay: true, lines: [{ index: 0, label: "EMA 8", values: [1.12, 1.1234, 1.12345] }] },
      },
    };
    const d = dataWindowRows(fx, 2, {}, [], opts);
    expect(d.indicators[0]!.rows[0]!.value).toBe("1.12345");
  });

  it("follows the legend order and skips an empty Volume pane", () => {
    const withVolume: SeriesPayload = {
      ...series,
      bars: series.bars.map((b) => ({ ...b, v: 0 })),
      indicators: {
        ...series.indicators,
        volume: { label: "Volume", is_overlay: false, histogram: [{ index: 0, label: "Volume", values: [0, 0, 0] }] },
      },
    };
    const d = dataWindowRows(withVolume, 1, {}, [], { ...opts, visible: new Set(["macd", "volume", "rsi"]) });
    expect(d.indicators.map((s) => s.key)).toEqual(["macd", "rsi"]);
  });

  it("falls back to the last bar without a crosshair or with a stale index", () => {
    const rest = dataWindowRows(series, null, {}, [], opts);
    expect(rest.bar?.c).toBe("12.50");
    expect(rest.time).toBe("2026-11-03 14:00");
    expect(dataWindowRows(series, 99, {}, [], opts)).toEqual(rest);
  });

  it("formats the time in the selected zone", () => {
    const d = dataWindowRows(series, 0, {}, [], { ...opts, timeZone: "Asia/Tokyo" });
    expect(d.time).toBe("2026-11-03 21:00");
  });

  it("shows the date only for daily bars", () => {
    const daily = { ...series, interval: "1d", bars: [{ ...series.bars[0]!, t: Date.UTC(2026, 10, 3) / 1000 }] };
    expect(dataWindowRows(daily, 0, {}, [], { ...opts, timeZone: "Asia/Tokyo" }).time).toBe("2026-11-03");
  });

  it("lists every plot of each indicator at the index, null as a dash", () => {
    const d = dataWindowRows(series, 0, {}, [], opts);
    expect(d.indicators.map((s) => [s.key, s.title])).toEqual([
      ["rsi", "RSI"],
      ["macd", "MACD"],
    ]);
    expect(d.indicators[0]!.rows).toEqual([{ label: "RSI", value: "—", color: "#000000" }]);
    expect(d.indicators[1]!.rows.map((r) => r.value)).toEqual(["0.10", "1.00"]);
    expect(dataWindowRows(series, 1, {}, [], opts).indicators[0]!.rows[0]!.value).toBe("55.12");
  });

  it("applies style overrides: color, and hidden plots are dropped", () => {
    const styles = { rsi: { plots: { "l:RSI": { color: "#ff0000" } } }, macd: { plots: { "l:MACD": { visible: false } } } };
    const d = dataWindowRows(series, 1, styles, [], opts);
    expect(d.indicators[0]!.rows[0]!.color).toBe("#ff0000");
    expect(d.indicators[1]!.rows).toHaveLength(1);
    expect(d.indicators[1]!.rows[0]!.value).toBe("-2.00");
  });

  it("uses server colors: a line's, and a histogram's per bar unless overridden", () => {
    const colored: SeriesPayload = {
      ...series,
      indicators: {
        volume: {
          label: "Volume",
          is_overlay: false,
          lines: [{ index: 0, label: "Volume MA", values: [1, 2, 3], color: "rgba(41,98,255,1)" }],
          histogram: [
            { index: 0, label: "Volume", values: [1, 2, 3], palette: ["#0f0", "#f00"], color_idx: [0, 1, 0] },
          ],
        },
      },
    };
    const rows = (at: number, styles = {}) =>
      dataWindowRows(colored, at, styles, [], opts).indicators[0]!.rows;
    expect(rows(1).map((r) => [r.label, r.color])).toEqual([
      ["Volume MA", "rgba(41,98,255,1)"],
      ["Volume", "#f00"],
    ]);
    expect(rows(2)[1]!.color).toBe("#0f0");
    expect(rows(1, { volume: { plots: { hist0: { color: "#ffeb3b" } } } })[1]!.color).toBe("#ffeb3b");
  });

  it("skips indicators outside the visible set", () => {
    const d = dataWindowRows(series, 1, {}, [], { ...opts, visible: new Set(["rsi"]) });
    expect(d.indicators.map((s) => s.key)).toEqual(["rsi"]);
  });

  it("maps every instance of an indicator by its key (ADR-0028)", () => {
    const rsi2 = { ...series.indicators.rsi!, label: "", lines: [{ index: 0, label: "RSI", values: [null, 40, 45] }] };
    const two: SeriesPayload = { ...series, indicators: { rsi: series.indicators.rsi!, "rsi#2": rsi2 } };
    const styles = { "rsi#2": { plots: { "l:RSI": { color: "#ff0000" } } } };
    const d = dataWindowRows(two, 1, styles, [], { ...opts, visible: new Set(["rsi", "rsi#2"]) });
    expect(d.indicators.map((s) => [s.key, s.title, s.rows[0]!.value])).toEqual([
      ["rsi", "RSI", "55.12"],
      ["rsi#2", "rsi", "40.00"],
    ]);
    expect(d.indicators[0]!.rows[0]!.color).not.toBe("#ff0000");
    expect(d.indicators[1]!.rows[0]!.color).toBe("#ff0000");
    const one = dataWindowRows(two, 1, {}, [], { ...opts, visible: new Set(["rsi#2"]) });
    expect(one.indicators.map((s) => s.key)).toEqual(["rsi#2"]);
  });

  it("reads compare symbols with their own precision, value and percent", () => {
    const eth = { symbol: "ETHUSDT", color: "#f23645", values: [100, 110, 121] };
    expect(dataWindowRows(series, 2, {}, [eth], opts).compare).toEqual([
      { label: "ETHUSDT", value: "121.00 (+21.00%)", color: "#f23645" },
    ]);
    const doge = { symbol: "DOGE", color: "#fff", values: [0.12345, 0.2, 0.3] };
    expect(dataWindowRows(series, 2, {}, [doge], opts).compare[0]!.value).toBe(
      "0.30000 (+143.01%)",
    );
  });

  it("measures the percent from the given bar", () => {
    const eth = { symbol: "E", color: "#fff", values: [100, 110, 121] };
    const d = dataWindowRows(series, 2, {}, [eth], { ...opts, compareFrom: 1 });
    expect(d.compare[0]!.value).toBe("121.00 (+10.00%)");
  });

  it("shows a dash where the compare has no value at the crosshair bar", () => {
    const line = { symbol: "X", color: "#fff", values: [null, 110, 121] };
    expect(dataWindowRows(series, 0, {}, [line], opts).compare[0]!.value).toBe("—");
    // At rest it reads the last value.
    expect(dataWindowRows(series, null, {}, [line], opts).compare[0]!.value).toBe(
      "121.00 (+10.00%)",
    );
    const none = { symbol: "X", color: "#fff", values: [null, null, null] };
    expect(dataWindowRows(series, 2, {}, [none], opts).compare[0]!.value).toBe("—");
  });

  it("handles a series without bars", () => {
    const d = dataWindowRows({ ...series, bars: [], indicators: {} }, null, {}, [], opts);
    expect(d.bar).toBeNull();
    expect(d.time).toBeNull();
  });
});
