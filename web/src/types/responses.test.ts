import { describe, expect, it } from "vitest";

import {
  ListIndicatorsPayloadSchema,
  SeriesPayloadSchema,
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
