import { describe, expect, it } from "vitest";

import answers from "../../../tests/fixtures/setup_responses.json";
import record from "../../../tests/fixtures/track_record_responses.json";
import {
  EventsPayloadSchema,
  ListIndicatorsPayloadSchema,
  SeriesPayloadSchema,
  SetupPayloadSchema,
  SetupsPayloadSchema,
  SummaryPayloadSchema,
  TrackRecordPayloadSchema,
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

  it("parses shadow results (ADR-0066): null before a fill, kept apart from result_r", () => {
    expect(SetupPayloadSchema.parse(answers.create_setup).shadow_results).toBeNull();
    const list = SetupsPayloadSchema.parse(answers.list_setups);
    for (const setup of list.setups) {
      expect(setup.shadow_results === null).toBe(setup.fill === null);
    }
    // A setup out at T1 whose v1 runner was scored on after it closed.
    const ran = list.setups.find(
      (s) => s.status === "won" && s.shadow_results?.v1_half_t1_be.exits.length === 2,
    );
    expect(ran?.shadow_results?.v1_half_t1_be.result_r).not.toBe(ran?.result_r);
    expect(ran?.shadow_results?.v2_be_at_1r.status).toBe("won");
  });

  it("refuses a shadow status the server never sends", () => {
    const won = SetupsPayloadSchema.parse(answers.list_setups).setups.find(
      (s) => s.shadow_results !== null,
    );
    const shadows = won?.shadow_results;
    const bad = {
      ...won,
      shadow_results: { ...shadows, v2_be_at_1r: { ...shadows?.v2_be_at_1r, status: "lost" } },
    };
    expect(SetupPayloadSchema.omit({ log: true }).safeParse(bad).success).toBe(false);
  });

  it("parses a server with setups off", () => {
    expect(SetupsPayloadSchema.parse({ enabled: false, setups: [] }).total).toBeUndefined();
  });

  it("refuses a status the server never sends", () => {
    const bad = { ...answers.get_setup, status: "lost" };
    expect(SetupPayloadSchema.safeParse(bad).success).toBe(false);
  });
});

describe("list_events (ADR-0061)", () => {
  const fired = {
    seq: 1,
    id: "e-1",
    type: "alert.fired",
    at: 1_700_000_000_000,
    alert_id: "a-1",
    setup_id: "s-1",
    what: {
      symbol: "BTCUSDT",
      interval: "1h",
      evaluate_on: "live",
      trigger: "once",
      price: 101.5,
      level: 100,
      condition: "greater",
      bar_time: 1_700_000_000,
      fire_count: 1,
    },
    delivery: "delivered",
  };
  const status = {
    seq: 2,
    id: "e-2",
    type: "setup.status",
    at: 1_700_000_060_000,
    setup_id: "s-1",
    what: { from: "triggered", to: "won", gap: false, result_r: 2 },
    delivery: "none",
  };

  it("parses an indicator alert's fire with its line and value", () => {
    const indicator = {
      name: "ma",
      params: { type: "ema", length: 50, source: "close", offset: 0 },
      line: "EMA 50",
      compare: "close",
      value: 64_210.5,
    };
    const p = EventsPayloadSchema.parse({
      events: [{ ...fired, what: { ...fired.what, level: 64_210.5, indicator } }],
      cursor: 1,
      more: false,
      truncated: false,
    });
    const what = p.events[0]?.what;
    expect(what && "indicator" in what ? what.indicator?.line : undefined).toBe("EMA 50");
    expect(() =>
      EventsPayloadSchema.parse({
        events: [{ ...fired, what: { ...fired.what, indicator: { ...indicator, compare: "above" } } }],
        cursor: 1,
        more: false,
        truncated: false,
      }),
    ).toThrow();
  });

  it("parses a fire that had no level", () => {
    const p = EventsPayloadSchema.parse({
      events: [{ ...fired, what: { ...fired.what, level: null } }],
      cursor: 1,
      more: false,
      truncated: false,
    });
    const what = p.events[0]?.what;
    expect(what && "level" in what ? what.level : undefined).toBeNull();
  });

  it("parses both kinds of event and the paging fields", () => {
    const p = EventsPayloadSchema.parse({
      events: [fired, status],
      cursor: 2,
      more: false,
      truncated: false,
    });
    expect(p.events.map((e) => e.type)).toEqual([
      "alert.fired",
      "setup.status",
    ]);
    expect(p.events[1]?.alert_id).toBeUndefined();
  });

  it("rejects a kind or a delivery state it does not know", () => {
    const page = { cursor: 1, more: false, truncated: false };
    expect(() =>
      EventsPayloadSchema.parse({
        ...page,
        events: [{ ...fired, type: "other" }],
      }),
    ).toThrow();
    expect(() =>
      EventsPayloadSchema.parse({
        ...page,
        events: [{ ...fired, delivery: "lost" }],
      }),
    ).toThrow();
  });
});

describe("what a setup was made with, and the track record (ADR-0064)", () => {
  it("parses a setup that states the three fields, as the server answers it", () => {
    const setup = SetupPayloadSchema.parse(record.create_setup_stated);
    expect(setup.probability).toBe(0.6);
    expect(setup.playbook_version).toBe("v1");
    expect(setup.features).toEqual({ example_number: 1.5, example_flag: true, example_label: "a" });
  });

  it("leaves them out of a setup made without them", () => {
    const setup = SetupPayloadSchema.parse(answers.create_setup);
    expect(setup.features).toBeUndefined();
    expect(setup.probability).toBeUndefined();
    expect(setup.playbook_version).toBeUndefined();
  });

  it("parses the record, with every field the server sends and no other", () => {
    const all = TrackRecordPayloadSchema.strict().parse(record.get_track_record);
    expect(all.closed).toBe(2);
    expect(all.total.entered).toBeLessThanOrEqual(all.total.count);
    expect(all.groups).toEqual([]);
    const grouped = TrackRecordPayloadSchema.strict().parse(record.get_track_record_grouped);
    expect(grouped.group_by).toBe("feature");
    expect(grouped.groups.map((g) => g.key)).toEqual([null, true]);
    expect(grouped.groups.reduce((n, g) => n + g.count, 0)).toBe(grouped.total.count);
  });

  it("refuses a field the server does not send, at every level of the record", () => {
    const strict = (change: (r: Record<string, any>) => void) => {
      const r = structuredClone(record.get_track_record_grouped) as Record<string, any>;
      change(r);
      return TrackRecordPayloadSchema.safeParse(r).success;
    };
    expect(strict(() => {})).toBe(true);
    expect(strict((r) => (r.extra = 1))).toBe(false);
    expect(strict((r) => (r.total.extra = 1))).toBe(false);
    expect(strict((r) => (r.total.probability.extra = 1))).toBe(false);
    expect(strict((r) => (r.groups[0].extra = 1))).toBe(false);
    expect(strict((r) => (r.shadows.extra = 1))).toBe(false);
    expect(strict((r) => (r.shadows.v1_half_t1_be.extra = 1))).toBe(false);
  });

  it("refuses a group whose key is not a value a setup can state", () => {
    const bad = structuredClone(record.get_track_record_grouped) as { groups: { key: unknown }[] };
    bad.groups[0]!.key = { nested: 1 };
    expect(() => TrackRecordPayloadSchema.parse(bad)).toThrow();
  });
});

