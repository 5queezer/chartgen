import { describe, expect, it } from "vitest";

import {
  alertStatus,
  describeAlert,
  drawingLevels,
  evaluate,
  fires,
  indicatorReading,
  isDrawingAlertTool,
  rearmed,
  rearmedReading,
  type Alert,
  type AlertSource,
  type ChartValues,
} from "./alerts";

const at = (condition: Alert["condition"], price = 100) => ({
  condition,
  price,
});

describe("evaluate: crossing", () => {
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

  it("a trend line interpolates and extrapolates in time/value space", () => {
    const rec = { name: "segment" as const, points: [p(0, 100), p(10 * H, 200)] };
    expect(drawingLevels(rec, 5 * H)).toEqual([150]);
    expect(drawingLevels(rec, 20 * H)).toEqual([300]);
    expect(drawingLevels(rec, -10 * H)).toEqual([0]);
    expect(drawingLevels({ ...rec, name: "straightLine" }, 15 * H)).toEqual([250]);
  });

  it("a ray only runs from its start towards its second point", () => {
    const right = { name: "rayLine" as const, points: [p(10 * H, 100), p(20 * H, 200)] };
    expect(drawingLevels(right, 30 * H)).toEqual([300]);
    expect(drawingLevels(right, 5 * H)).toEqual([]);
    const left = { name: "rayLine" as const, points: [p(10 * H, 100), p(0, 200)] };
    expect(drawingLevels(left, 5 * H)).toEqual([150]);
    expect(drawingLevels(left, 15 * H)).toEqual([]);
  });

  it("a parallel channel has two lines", () => {
    const rec = {
      name: "parallelStraightLine" as const,
      points: [p(0, 100), p(10 * H, 200), p(5 * H, 120)],
    };
    // The main line is 150 at 5 h; the parallel one runs 30 below it.
    expect(drawingLevels(rec, 5 * H)).toEqual([150, 120]);
    expect(drawingLevels(rec, 10 * H)).toEqual([200, 170]);
  });

  it("other tools have no level", () => {
    expect(drawingLevels({ name: "cgRect", points: [p(0, 1), p(1, 2)] }, 0)).toEqual([]);
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
