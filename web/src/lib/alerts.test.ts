import { describe, expect, it } from "vitest";

import {
  alertStatus,
  barIndexOf,
  describeAlert,
  drawingLevels,
  fires,
  indicatorReading,
  isDrawingAlertTool,
  isVerticalLine,
  pointBarIndex,
  rearmedReading,
  type Alert,
  type AlertSource,
  type ChartValues,
} from "./alerts";
import { timeAxisOf } from "../stores/drawings";

const at = (condition: Alert["condition"], price = 100) => ({
  condition,
  price,
});
type Level = ReturnType<typeof at>;

// A price alert's reading is the price minus its level: these run the
// price-alert rules through `fires` / `rearmedReading`.
const evaluate = (a: Level, prev: number | null, price: number) =>
  fires(a.condition, prev === null ? null : [prev - a.price], [price - a.price]);
const rearmed = (a: Level, price: number) => rearmedReading(a.condition, [price - a.price]);
const RSI_SIG = '[{"length":14,"name":"rsi"}]';

describe("fires on a price alert: crossing", () => {
  const a = at("crossing");
  it("fires from below and from above", () => {
    expect(evaluate(a, 99, 101)).toBe(true);
    expect(evaluate(a, 101, 99)).toBe(true);
  });
  it("fires when the price lands on the level", () => {
    expect(evaluate(a, 99, 100)).toBe(true);
    expect(evaluate(a, 101, 100)).toBe(true);
  });
  it("does not fire while staying on one side", () => {
    expect(evaluate(a, 99, 99.5)).toBe(false);
    expect(evaluate(a, 101, 105)).toBe(false);
  });
  it("does not fire when leaving the level or standing on it", () => {
    expect(evaluate(a, 100, 101)).toBe(false);
    expect(evaluate(a, 100, 100)).toBe(false);
  });
  it("needs a previous price", () => {
    expect(evaluate(a, null, 101)).toBe(false);
  });
});

describe("evaluate: crossing_up / crossing_down", () => {
  it("crossing_up only fires upward", () => {
    const a = at("crossing_up");
    expect(evaluate(a, 99, 101)).toBe(true);
    expect(evaluate(a, 99, 100)).toBe(true);
    expect(evaluate(a, 101, 99)).toBe(false);
    expect(evaluate(a, 100, 101)).toBe(false);
    expect(evaluate(a, null, 101)).toBe(false);
  });
  it("crossing_down only fires downward", () => {
    const a = at("crossing_down");
    expect(evaluate(a, 101, 99)).toBe(true);
    expect(evaluate(a, 101, 100)).toBe(true);
    expect(evaluate(a, 99, 101)).toBe(false);
    expect(evaluate(a, 100, 99)).toBe(false);
    expect(evaluate(a, null, 99)).toBe(false);
  });
});

describe("evaluate: greater / less", () => {
  it("greater is strict and needs no previous price", () => {
    const a = at("greater");
    expect(evaluate(a, null, 100.01)).toBe(true);
    expect(evaluate(a, 90, 100)).toBe(false);
    expect(evaluate(a, 101, 102)).toBe(true);
  });
  it("less is strict and needs no previous price", () => {
    const a = at("less");
    expect(evaluate(a, null, 99.99)).toBe(true);
    expect(evaluate(a, 110, 100)).toBe(false);
  });
});

describe("rearmed", () => {
  it("greater re-arms at or below the level, less at or above", () => {
    expect(rearmed(at("greater"), 101)).toBe(false);
    expect(rearmed(at("greater"), 100)).toBe(true);
    expect(rearmed(at("less"), 99)).toBe(false);
    expect(rearmed(at("less"), 100)).toBe(true);
  });
  it("crossing conditions are always armed", () => {
    expect(rearmed(at("crossing"), 150)).toBe(true);
    expect(rearmed(at("crossing_up"), 150)).toBe(true);
    expect(rearmed(at("crossing_down"), 50)).toBe(true);
  });
});

describe("alertStatus", () => {
  const base: Alert = {
    id: "a",
    symbol: "X",
    condition: "crossing",
    price: 1,
    trigger: "once",
    active: true,
    createdAt: 0,
  };
  it("derives active, triggered and paused", () => {
    expect(alertStatus(base)).toBe("active");
    expect(alertStatus({ ...base, active: false })).toBe("paused");
    expect(alertStatus({ ...base, active: false, lastTriggeredAt: 5 })).toBe(
      "triggered",
    );
  });
});

describe("drawingLevels (ADR-0034)", () => {
  const H = 3_600_000;
  const p = (timestamp: number, value: number) => ({ timestamp, value });

  it("a horizontal line is constant", () => {
    const rec = { name: "horizontalStraightLine" as const, points: [p(5 * H, 42)] };
    expect(drawingLevels(rec, 0)).toEqual([42]);
    expect(drawingLevels(rec, 100 * H)).toEqual([42]);
  });

  it("a horizontal ray only has a value from its start on", () => {
    const rec = { name: "horizontalRayLine" as const, points: [p(5 * H, 42)] };
    expect(drawingLevels(rec, 4 * H)).toEqual([]);
    expect(drawingLevels(rec, 5 * H)).toEqual([42]);
    expect(drawingLevels(rec, 9 * H)).toEqual([42]);
  });

  // Hourly bars 0 h … 20 h without gaps, on a linear axis.
  const hours = (...hs: number[]) => timeAxisOf(hs.map((h) => h * H))!;
  const flat = { axis: hours(...Array.from({ length: 21 }, (_, i) => i)), log: false };

  it("sloped lines need the chart's bars", () => {
    const rec = { name: "segment" as const, points: [p(0, 100), p(10 * H, 200)] };
    expect(drawingLevels(rec, 5 * H)).toEqual([]);
  });

  it("a trend line interpolates and extrapolates by bar", () => {
    const rec = { name: "segment" as const, points: [p(0, 100), p(10 * H, 200)] };
    expect(drawingLevels(rec, 5 * H, flat)).toEqual([150]);
    expect(drawingLevels(rec, 20 * H, flat)).toEqual([300]);
    // Past the last bar and before the first one, in whole bar intervals.
    expect(drawingLevels(rec, 25 * H, flat)).toEqual([350]);
    expect(drawingLevels(rec, -10 * H, flat)).toEqual([0]);
    expect(drawingLevels({ ...rec, name: "straightLine" }, 15 * H, flat)).toEqual([250]);
  });

  it("a gap in the bars takes no room, as on the chart", () => {
    // Bars at 0-4 h, then 20-24 h (a market closed overnight).
    const view = { axis: hours(0, 1, 2, 3, 4, 20, 21, 22, 23, 24), log: false };
    const rec = { name: "segment" as const, points: [p(0, 100), p(4 * H, 140)] };
    // 24 h is bar 9: 100 + 9 × 10, not the 340 time would give.
    expect(drawingLevels(rec, 24 * H, view)).toEqual([190]);
    expect(drawingLevels(rec, 26 * H, view)).toEqual([210]);
    expect(barIndexOf(view.axis, 21.4 * H)).toBe(6);
    expect(barIndexOf(view.axis, 10 * H)).toBe(4);
    expect(barIndexOf(view.axis, 15 * H)).toBe(5);
    expect(barIndexOf(view.axis, -2 * H)).toBe(-2);
  });

  it("on a log axis the line is straight in log(value)", () => {
    const rec = { name: "segment" as const, points: [p(0, 100), p(10 * H, 10_000)] };
    expect(drawingLevels(rec, 5 * H, { ...flat, log: true })[0]).toBeCloseTo(1000);
    expect(drawingLevels(rec, 5 * H, flat)[0]).toBeCloseTo(5050);
    const negative = { name: "segment" as const, points: [p(0, -1), p(10 * H, 5)] };
    expect(drawingLevels(negative, 5 * H, { ...flat, log: true })).toEqual([]);
  });

  it("a vertical line has no value", () => {
    const rec = { name: "straightLine" as const, points: [p(5 * H, 100), p(5 * H, 200)] };
    expect(drawingLevels(rec, 5 * H, flat)).toEqual([]);
  });

  it.each(["segment", "rayLine", "straightLine"] as const)(
    "a %s with both points on one bar reads nothing and never fires",
    (name) => {
      for (const points of [
        [p(5 * H, 100), p(5 * H, 200)],
        [p(5 * H, 100), p(5 * H, 100)],
      ]) {
        for (const view of [flat, { ...flat, log: true }]) {
          const cur = drawingLevels({ name, points }, 5 * H, view);
          expect(cur).toEqual([]);
          for (const condition of ["crossing", "greater", "less"] as const) {
            expect(fires(condition, null, cur)).toBe(false);
            expect(fires(condition, [1], cur)).toBe(false);
          }
        }
      }
    },
  );

  it("pointBarIndex: exact from the point, a time over an index, nothing without either", () => {
    expect(pointBarIndex(flat.axis, { timestamp: 10 * H })).toBe(10);
    expect(pointBarIndex(flat.axis, { timestamp: 10 * H, dataIndex: 1 })).toBe(10);
    expect(pointBarIndex(flat.axis, { dataIndex: 3 })).toBe(3);
    expect(pointBarIndex(flat.axis, { dataIndex: 900 })).toBe(900);
    expect(pointBarIndex(flat.axis, {})).toBeUndefined();
  });

  it("isVerticalLine: both anchors on one bar, on a future bar too", () => {
    expect(isVerticalLine({ points: [p(5 * H, 100), p(5 * H, 200)] })).toBe(true);
    expect(isVerticalLine({ points: [p(5 * H, 100), p(6 * H, 200)] })).toBe(false);
    expect(isVerticalLine({ points: [p(5 * H, 100)] })).toBe(false);
    expect(isVerticalLine({ points: [] })).toBe(false);
    expect(isVerticalLine({ points: [{ dataIndex: 900 }, { dataIndex: 900 }] })).toBe(true);
    expect(isVerticalLine({ points: [{ dataIndex: 900 }, { dataIndex: 901 }] })).toBe(false);
    expect(isVerticalLine({ points: [{ timestamp: 5 * H, dataIndex: 900 }, { dataIndex: 900 }] })).toBe(true);
    expect(isVerticalLine({ points: [{}, {}] })).toBe(false);
  });

  it("a ray only runs from its start towards its second point", () => {
    const right = { name: "rayLine" as const, points: [p(10 * H, 100), p(20 * H, 200)] };
    expect(drawingLevels(right, 15 * H, flat)).toEqual([150]);
    expect(drawingLevels(right, 5 * H, flat)).toEqual([]);
    const left = { name: "rayLine" as const, points: [p(10 * H, 100), p(0, 200)] };
    expect(drawingLevels(left, 5 * H, flat)).toEqual([150]);
    expect(drawingLevels(left, 15 * H, flat)).toEqual([]);
  });

  it("a parallel channel has two lines", () => {
    const rec = {
      name: "parallelStraightLine" as const,
      points: [p(0, 100), p(10 * H, 200), p(5 * H, 120)],
    };
    // The main line is 150 at 5 h; the parallel one runs 30 below it.
    expect(drawingLevels(rec, 5 * H, flat)).toEqual([150, 120]);
    expect(drawingLevels(rec, 10 * H, flat)).toEqual([200, 170]);
    // The third point's offset is measured in bars too: across a gap.
    const gapped = { axis: hours(0, 1, 2, 3, 4, 20, 21, 22, 23, 24), log: false };
    const across = {
      name: "parallelStraightLine" as const,
      points: [p(0, 100), p(4 * H, 140), p(20 * H, 130)],
    };
    // At 20 h (bar 5) the main line is 150: the parallel one runs 20 below.
    expect(drawingLevels(across, 24 * H, gapped)).toEqual([190, 170]);
  });

  it("other tools have no level", () => {
    expect(drawingLevels({ name: "cgRect", points: [p(0, 1), p(1, 2)] }, 0, flat)).toEqual([]);
    expect(isDrawingAlertTool("cgRect")).toBe(false);
    expect(isDrawingAlertTool("segment")).toBe(true);
  });
});

describe("fires with several levels (channel)", () => {
  it("a crossing of either line fires", () => {
    // Lines at 170 and 130.
    expect(fires("crossing_up", [160 - 170, 160 - 130], [175 - 170, 175 - 130])).toBe(true);
    expect(fires("crossing_up", [140 - 170, 140 - 130], [150 - 170, 150 - 130])).toBe(false);
    expect(fires("crossing_down", [140 - 170, 140 - 130], [120 - 170, 120 - 130])).toBe(true);
  });
  it("greater means above both lines, less below both", () => {
    expect(fires("greater", null, [5, 45])).toBe(true);
    expect(fires("greater", null, [-5, 35])).toBe(false);
    expect(fires("less", null, [-45, -5])).toBe(true);
    expect(rearmedReading("greater", [-5, 35])).toBe(true);
    expect(rearmedReading("greater", [5, 45])).toBe(false);
  });
  it("an event needs readings that line up", () => {
    expect(fires("crossing", [-1], [1, 2])).toBe(false);
    expect(fires("crossing", [-1], [])).toBe(false);
  });
});

describe("indicatorReading", () => {
  const chart: ChartValues = {
    interval: "1h",
    indicators: {
      rsi: { lines: [{ values: [50, 65, 72] }] },
      macd: { lines: [{ values: [1, 2] }, { values: [1.5, 1.8] }] },
      ema_stack: { lines: [{ values: [null, 99] }, { values: [90, null] }] },
    },
  };
  type Src = Extract<AlertSource, { kind: "indicator" }>;
  const src = (over: Partial<Src>): Src => ({
    kind: "indicator",
    interval: "1h",
    instanceKey: "rsi",
    signature: RSI_SIG,
    line: 0,
    name: "RSI (14)",
    against: { kind: "value", value: 70 },
    ...over,
  });

  it("reads the forming bar against a value, a plot or the price", () => {
    expect(indicatorReading(src({}), 0, chart)).toEqual([2]);
    const macd = src({ instanceKey: "macd", against: { kind: "plot", line: 1, name: "Signal" } });
    expect(indicatorReading(macd, 0, chart)![0]).toBeCloseTo(0.2);
    const ema = src({ instanceKey: "ema_stack", against: { kind: "price" } });
    expect(indicatorReading(ema, 101, chart)).toEqual([2]);
  });

  it("is null off the interval, for a missing plot or a bar without a value", () => {
    expect(indicatorReading(src({}), 0, { ...chart, interval: "4h" })).toBeNull();
    expect(indicatorReading(src({}), 0, undefined)).toBeNull();
    expect(indicatorReading(src({ instanceKey: "nope" }), 0, chart)).toBeNull();
    expect(indicatorReading(src({ line: 3 }), 0, chart)).toBeNull();
    const vs21 = src({
      instanceKey: "ema_stack",
      against: { kind: "plot", line: 1, name: "EMA 21" },
    });
    expect(indicatorReading(vs21, 0, chart)).toBeNull();
  });
});

describe("describeAlert", () => {
  const ind = {
    kind: "indicator" as const,
    interval: "1h",
    instanceKey: "rsi",
    signature: RSI_SIG,
    line: 0,
    name: "RSI (14)",
  };
  it("names price, drawing and indicator sources", () => {
    expect(describeAlert({ condition: "crossing_up", price: 65000 })).toBe("Crossing up 65000");
    expect(
      describeAlert({
        condition: "crossing_up",
        price: 0,
        source: { kind: "drawing", drawingId: "d", label: "Trend line" },
      }),
    ).toBe("Close crossing up Trend line");
    expect(
      describeAlert({
        condition: "crossing_up",
        price: 0,
        source: { ...ind, against: { kind: "value", value: 70 } },
      }),
    ).toBe("RSI (14) crossing up 70");
    expect(
      describeAlert({
        condition: "crossing",
        price: 0,
        source: { ...ind, name: "EMA 200", against: { kind: "price" } },
      }),
    ).toBe("Close crossing EMA 200");
    expect(
      describeAlert({
        condition: "crossing",
        price: 0,
        source: { ...ind, name: "MACD", against: { kind: "plot", line: 1, name: "Signal" } },
      }),
    ).toBe("MACD crossing Signal");
  });
});
