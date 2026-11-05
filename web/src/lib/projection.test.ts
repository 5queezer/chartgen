import { describe, expect, it } from "vitest";

import { valueExtents } from "../components/geometry";
import type { SeriesPayload } from "../types/responses";
import { dataWindowRows } from "./dataWindow";
import {
  DEFAULT_RIGHT_OFFSET,
  fitProjection,
  futureFills,
  futureLines,
  futureTime,
  futureValue,
  inProjection,
  projectionBars,
  projectionBarSpace,
  rightOffset,
  setShownProjection,
  sliceFuture,
} from "./projection";
import { sliceSeries } from "./replay";

// Five bars of an Ichimoku-like indicator whose span (line 1, fill 0)
// projects three bars ahead: future bar k shows bar k + 2's span, and the
// span plotted at bar i is bar i - 3's, so the plotted values and the
// projection are one shifted array.
const T0 = 1_700_000_000;
const STEP = 3600;
const span = (j: number) => 10 + j; // span computed at bar j
const shifted = (i: number) => (i >= 3 ? span(i - 3) : null);

function series(): SeriesPayload {
  const n = 5;
  const idx = [...Array(n).keys()];
  return {
    symbol: "TEST",
    interval: "1h",
    has_more: false,
    bars: idx.map((i) => ({ t: T0 + i * STEP, o: 1, h: 2, l: 0, c: 1, v: 1 })),
    indicators: {
      ichimoku: {
        label: "Ichimoku",
        is_overlay: true,
        lines: [
          { index: 0, label: "Tenkan", values: idx.map((i) => i) },
          { index: 1, label: "Senkou A", values: idx.map(shifted) },
        ],
        fills: [{ y1: idx.map(shifted), y2: idx.map(() => 11) }],
        future: {
          bars: 3,
          t: [1, 2, 3].map((k) => T0 + (n - 1 + k) * STEP),
          lines: [{ index: 1, values: [0, 1, 2].map((k) => span(n + k - 3)) }],
          fills: [{ index: 0, y1: [0, 1, 2].map((k) => span(n + k - 3)), y2: [11, 11, 11] }],
        },
      },
      rsi: {
        label: "RSI",
        is_overlay: false,
        lines: [{ index: 0, label: "RSI", values: idx.map(() => 50) }],
      },
    },
  };
}

describe("projection (ADR-0033)", () => {
  it("sizes the right margin for the longest projection", () => {
    const s = series();
    expect(projectionBars(s, ["ichimoku", "rsi"])).toBe(3);
    expect(projectionBars(s, ["rsi"])).toBe(0);
    expect(rightOffset(0, 8, 1000)).toBe(DEFAULT_RIGHT_OFFSET);
    expect(rightOffset(3, 8, 1000)).toBe(DEFAULT_RIGHT_OFFSET);
    expect(rightOffset(26, 8, 1000)).toBe((26 + 3) * 8);
    // At most half the pane: 500 bars at 8 px would be 4024 px.
    expect(rightOffset(500, 8, 1000)).toBe(500);
    expect(rightOffset(25, 20, 0)).toBe(28 * 20);
  });

  it("refits the margin at the chart's current bar width", () => {
    const set: number[] = [];
    let space = 8;
    const chart = {
      getBarSpace: () => space,
      getSize: () => ({ width: 1000 }),
      setOffsetRightDistance: (d: number) => set.push(d),
    };
    setShownProjection(25);
    fitProjection(chart);
    space = 20; // zoomed in: 25 bars now need 560 px, capped at 500
    fitProjection(chart);
    setShownProjection(0);
    fitProjection(chart);
    expect(set).toEqual([28 * 8, 500, DEFAULT_RIGHT_OFFSET]);
  });

  it("fits a date range together with the projection's margin", () => {
    setShownProjection(25);
    // 100 bars + 28 margin bars in 1280 px: 10 px each.
    expect(projectionBarSpace(100, 1280)).toBe(10);
    // 10 bars would leave a 737 px margin: the cap keeps it at half the pane.
    expect(projectionBarSpace(10, 1000)).toBe(50);
    setShownProjection(0);
    expect(projectionBarSpace(100, 1280)).toBeNull();
  });

  it("reads projected values and times past the last bar", () => {
    const s = series();
    const ind = s.indicators.ichimoku!;
    expect(inProjection(s, ["ichimoku"], 4)).toBe(false);
    expect(inProjection(s, ["ichimoku"], 5)).toBe(true);
    expect(inProjection(s, ["ichimoku"], 8)).toBe(false);
    expect(inProjection(s, ["rsi"], 5)).toBe(false);
    expect(futureValue(ind, 1, 5, 5)).toBe(span(2));
    expect(futureValue(ind, 1, 5, 7)).toBe(span(4));
    expect(futureValue(ind, 0, 5, 5)).toBeNull();
    expect(futureValue(ind, 1, 5, 8)).toBeNull();
    expect(futureTime(s, ["rsi", "ichimoku"], 6)).toBe(T0 + 6 * STEP);
    expect(futureTime(s, ["rsi"], 6)).toBeNull();
  });

  it("joins the projected runs to the last bar", () => {
    const ind = series().indicators.ichimoku!;
    expect(futureLines(ind, 5)).toEqual([
      { index: 1, from: 4, values: [span(1), span(2), span(3), span(4)] },
    ]);
    const [fill] = futureFills(ind, 5);
    expect(fill!.from).toBe(4);
    expect(fill!.values.y1).toEqual([span(1), span(2), span(3), span(4)]);
    expect(fill!.values.y2).toEqual([11, 11, 11, 11]);
    expect(futureLines(series().indicators.rsi!, 5)).toEqual([]);
  });

  it("replay sees the projection from its cursor", () => {
    const s = series();
    // Cut after bar 2: bars 3 and 4 are plotted values, then the projection.
    const f = sliceFuture(s.indicators.ichimoku!, 3, s.bars)!;
    expect(f.lines[0]!.values).toEqual([span(0), span(1), span(2)]);
    expect(f.fills[0]!.y1).toEqual([span(0), span(1), span(2)]);
    expect(f.t).toEqual([3, 4, 5].map((i) => T0 + i * STEP));
    const cut = sliceSeries(s, 2);
    expect(cut.indicators.ichimoku!.future).toEqual(f);
    expect(cut.indicators.rsi!.future).toBeUndefined();
    // At the last bar the loaded projection is unchanged.
    expect(sliceFuture(s.indicators.ichimoku!, 5, s.bars)).toEqual(s.indicators.ichimoku!.future);
  });

  it("the y-axis range of the last bar covers the projected cloud", () => {
    const ind = series().indicators.ichimoku!;
    const ext = valueExtents(ind, 5);
    expect(ext[4]).toEqual({ lo: 11, hi: span(4) });
    expect(ext[3]).toEqual({ lo: 10, hi: 11 });
  });

  it("the Data Window shows the projection at the future bar's time", () => {
    const s = series();
    const opts = { timeZone: "UTC", colorAt: () => "#000" };
    const d = dataWindowRows(s, 6, {}, [], opts);
    expect(d.bar?.c).toBe("—");
    expect(d.time).toContain("2023");
    const ichimoku = d.indicators.find((x) => x.key === "ichimoku")!;
    expect(ichimoku.rows.map((r) => r.value)).toEqual(["—", span(3).toFixed(2)]);
    expect(d.indicators.find((x) => x.key === "rsi")!.rows[0]!.value).toBe("—");
    // A bar past the projection reads the last bar, as before.
    const rest = dataWindowRows(s, 9, {}, [], opts);
    expect(rest.bar?.c).not.toBe("—");
    expect(rest.indicators.find((x) => x.key === "rsi")!.rows[0]!.value).toBe("50.00");
  });
});
