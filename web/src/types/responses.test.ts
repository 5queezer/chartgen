import { describe, expect, it } from "vitest";

import answers from "../../../tests/fixtures/setup_responses.json";
import {
  ListIndicatorsPayloadSchema,
  SeriesPayloadSchema,
  SetupPayloadSchema,
  SetupsPayloadSchema,
  SummaryPayloadSchema,
} from "./responses";

const series = {
  symbol: null,
  interval: "4h",
  bars: [{ t: null, o: 1, h: 2, l: 0.5, c: 1.5, v: 10 }],
  indicators: { rsi: { label: "RSI", is_overlay: false } },
  has_more: false,
};

describe("indicator_errors (ADR-0025)", () => {
  it("is optional, so older payloads still parse", () => {
    expect(SeriesPayloadSchema.parse(series).indicator_errors).toBeUndefined();
  });

  it("carries one structured error per failed indicator", () => {
    const p = SeriesPayloadSchema.parse({
      ...series,
      indicator_errors: {
        mine: { step: "runtime", line: 7, message: "too many bars" },
        other: { step: "runtime", line: null, message: "time limit exceeded" },
      },
    });
    expect(p.indicator_errors?.mine?.line).toBe(7);
    expect(p.indicator_errors?.other?.line).toBeNull();
  });

  it("is accepted on the summary payload too", () => {
    const p = SummaryPayloadSchema.parse({
      symbol: null,
      interval: "4h",
      price: { bars: 0 },
      indicators: {},
      indicator_errors: { mine: { step: "runtime", line: 1, message: "x" } },
    });
    expect(Object.keys(p.indicator_errors ?? {})).toEqual(["mine"]);
  });

  it("rejects a malformed error", () => {
    expect(() =>
      SeriesPayloadSchema.parse({
        ...series,
        indicator_errors: { mine: { step: "runtime" } },
      }),
    ).toThrow();
  });
});

describe("future (ADR-0033)", () => {
  const future = {
    bars: 2,
    t: [100, 200],
    lines: [{ index: 2, values: [1.5, null] }],
    fills: [{ index: 0, y1: [1.5, null], y2: [1, 1] }],
  };
  const withFuture = (f: unknown) => ({
    ...series,
    indicators: { ichimoku: { label: "Ichimoku", is_overlay: true, future: f } },
  });

  it("parses the projection past the last bar, with or without times", () => {
    const p = SeriesPayloadSchema.parse(withFuture(future));
    expect(p.indicators.ichimoku?.future?.lines[0]?.values).toEqual([1.5, null]);
    const { t: _t, ...untimed } = future;
    expect(SeriesPayloadSchema.parse(withFuture(untimed)).indicators.ichimoku?.future?.t).toBeUndefined();
  });

  it("rejects a malformed projection", () => {
    expect(() => SeriesPayloadSchema.parse(withFuture({ bars: 2, lines: [] }))).toThrow();
    expect(() => SeriesPayloadSchema.parse(withFuture({ ...future, bars: -1 }))).toThrow();
  });
});

describe("chart pattern times (ADR-0046)", () => {
  // As the server's tests get them: a double top on hourly bars from unix second 1e6.
  const ms = (bar: number) => (1_000_000 + bar * 3600) * 1000;
  const common = {
    key: "double_top",
    name: "Double Top",
    bias: "bearish",
    tooltip: "Double Top: ...",
    invalidation: 111.63,
    start_time: ms(20),
    end_time: ms(40),
    known_time: ms(43),
    height: 10.2,
  };
  const confirmed = { ...common, status: "confirmed", target: 89.7, confirmed_time: ms(47), failed_time: null };
  const failed = { ...common, status: "failed", confirmed_time: null, failed_time: ms(45) };
  const summaryOf = (patterns: unknown[]) => ({
    symbol: null,
    interval: "1h",
    price: { bars: 0 },
    indicators: { chart_patterns: { label: "Chart Patterns", is_overlay: true, patterns } },
  });

  it("parses them on series and summary patterns", () => {
    const stroke = { x1: 20, t1: 1072000, y1: 110.1, x2: 30, t2: 1108000, y2: 99.9, dashed: false };
    const label = { x: 30, t: 1108000, y: 110.1, above: true };
    const s = SeriesPayloadSchema.parse({
      ...series,
      indicators: {
        chart_patterns: {
          label: "Chart Patterns",
          is_overlay: true,
          patterns: [
            { ...confirmed, strokes: [stroke], label, confirmed_t: 1169200 },
            { ...failed, strokes: [stroke], label },
          ],
        },
      },
    });
    const [c, f] = s.indicators.chart_patterns?.patterns ?? [];
    expect([c?.confirmed_time, c?.failed_time]).toEqual([ms(47), null]);
    expect([f?.confirmed_time, f?.failed_time, f?.start_time]).toEqual([null, ms(45), ms(20)]);
    const summary = SummaryPayloadSchema.parse(summaryOf([{ ...confirmed, confirmed_x: 47 }, failed]));
    expect(summary.indicators.chart_patterns?.patterns?.[1]?.failed_time).toBe(ms(45));
  });

  it("leaves them out where a pattern has none, and refuses a time that is not one", () => {
    const { start_time: _s, end_time: _e, known_time: _k, confirmed_time: _c, failed_time: _f, height: _h, ...old } =
      failed;
    const p = SummaryPayloadSchema.parse(summaryOf([old]));
    expect(p.indicators.chart_patterns?.patterns?.[0]?.start_time).toBeUndefined();
    expect(() => SummaryPayloadSchema.parse(summaryOf([{ ...failed, start_time: "2026-11-12" }]))).toThrow();
  });
});

describe("list_indicators source tag", () => {
  it("tags user scripts and leaves built-ins untagged", () => {
    const p = ListIndicatorsPayloadSchema.parse({
      total: 2,
      indicators: [
        { name: "rsi", description: "RSI", type: "panel", overlay: false },
        {
          name: "mine",
          description: "Mine",
          type: "panel",
          overlay: false,
          source: "user",
        },
      ],
    });
    expect(p.indicators.map((i) => i.source)).toEqual([undefined, "user"]);
  });
});

it("accepts additive drawing metadata while preserving older series payloads", () => {
  const ind = {label: "SAR", is_overlay: true, point_lines: true, steps: true,
    line_breaks: [0, 2], hlines: [{ y: 12, label: "POC" }],
    lines: [{ index: 0, label: "SAR Up", values: [1, null, 3] }],
  };
  const p = SeriesPayloadSchema.parse({...series, indicators: { sar: ind }});
  expect(p.indicators.sar).toEqual(ind);
  expect(SeriesPayloadSchema.parse(series).indicators.rsi?.steps).toBeUndefined();
});

describe("setups (ADR-0059), against the server's own answers", () => {
  it("parses a setup with its log from each tool that returns one", () => {
    for (const tool of [
      "create_setup",
      "create_setup_skip",
      "revise_setup",
      "get_setup",
      "cancel_setup",
    ] as const) {
      const setup = SetupPayloadSchema.strict().parse(answers[tool]);
      expect(setup.log[0]?.kind, tool).toBe("created");
    }
    expect(SetupPayloadSchema.parse(answers.create_setup).bracket).toBe("agent");
    expect(SetupPayloadSchema.parse(answers.create_setup_skip).bracket).not.toBe("agent");
    expect(SetupPayloadSchema.parse(answers.revise_setup).pending[0]?.kind).toBe("levels");
    expect(SetupPayloadSchema.parse(answers.cancel_setup).pending[0]?.kind).toBe("cancel");
  });

  it("parses the list, with closed setups scored", () => {
    const list = SetupsPayloadSchema.strict().parse(answers.list_setups);
    expect(list.total).toBe(3);
    for (const setup of list.setups) {
      expect(setup.status === "watching" || setup.status === "triggered").toBe(false);
      expect(setup.result_r).not.toBeNull();
    }
    expect(list.setups.some((s) => s.fill !== null && s.exit !== null)).toBe(true);
  });

  it("parses a server with setups off", () => {
    expect(SetupsPayloadSchema.parse({ enabled: false, setups: [] }).total).toBeUndefined();
  });

  it("refuses a status the server never sends", () => {
    const bad = { ...answers.get_setup, status: "lost" };
    expect(SetupPayloadSchema.safeParse(bad).success).toBe(false);
  });
});
