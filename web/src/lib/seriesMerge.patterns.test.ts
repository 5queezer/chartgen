// The pattern channels (ADR-0046) through the merges: labels belong to their bar,
// patterns, levels and events are re-detected as a whole on the newest bars.

import { describe, expect, it } from "vitest";

import type { SeriesPayload } from "../types/responses";
import { mergeOlder, mergeTail, sortChannels, writeLevels } from "./seriesMerge";

const BASE = 1_700_000_000;
const STEP = 60;
const tOf = (i: number) => BASE + i * STEP;

type Ind = SeriesPayload["indicators"][string];
type Pattern = NonNullable<Ind["patterns"]>[number];

const label = (bar: number, first: number, text = "H") => ({
  x: bar - first,
  t: tOf(bar),
  y: 100,
  above: false,
  text,
  key: "hammer",
  name: "Hammer",
  bias: "bullish" as const,
  tooltip: "Hammer",
});

/** A pattern over bars `a`..`b` of the chart, in a window starting at `first`. */
const pattern = (a: number, b: number, first: number, status: Pattern["status"]): Pattern => ({
  key: "double_top",
  name: "Double Top",
  bias: "bearish",
  status,
  strokes: [{ x1: a - first, t1: tOf(a), y1: 1, x2: b - first, t2: tOf(b), y2: 2, dashed: false }],
  label: { x: a - first, t: tOf(a), y: 2, above: true },
  tooltip: status,
});

/** A response for bars `first`..`last` of one indicator with `ind` on them. */
function res(first: number, last: number, ind: (first: number) => Partial<Ind>): SeriesPayload {
  const idx = Array.from({ length: last - first + 1 }, (_, k) => first + k);
  return {
    symbol: "X",
    interval: "1m",
    bars: idx.map((i) => ({ t: tOf(i), o: 1, h: 2, l: 0, c: 1, v: 1 })),
    has_more: true,
    indicators: { p: { label: "P", is_overlay: true, ...ind(first) } },
  };
}

describe("labels", () => {
  it("a tail replaces those in its window and keeps older ones", () => {
    const cur = sortChannels(
      res(0, 9, (f) => ({ labels: [label(2, f), label(8, f), label(9, f, "tentative")] })),
    );
    const tail = res(8, 10, (f) => ({ labels: [label(8, f), label(10, f, "new")] }));
    const r = mergeTail(cur, tail);
    if (!r.ok) throw new Error(r.reason);
    const got = r.series.indicators.p!.labels!;
    expect(got.map((l) => [l.x, l.text])).toEqual([
      [2, "H"],
      [8, "H"],
      [10, "new"],
    ]);
  });

  it("a label that vanished from the forming bar is gone", () => {
    const cur = res(0, 9, (f) => ({ labels: [label(9, f)] }));
    const r = mergeTail(cur, res(8, 9, () => ({})));
    if (!r.ok) throw new Error(r.reason);
    expect(r.series.indicators.p!.labels).toEqual([]);
  });

  it("an older page's labels go in front, on their bars", () => {
    const cur = res(10, 19, (f) => ({ labels: [label(12, f)] }));
    const page = res(0, 9, (f) => ({ labels: [label(3, f)] }));
    const r = mergeOlder(cur, page);
    if (!r.ok) throw new Error(r.reason);
    expect(r.series.indicators.p!.labels!.map((l) => l.x)).toEqual([3, 12]);
  });
});

describe("patterns", () => {
  it("a tail replaces them, on our bars though its indices run from its own first bar", () => {
    const cur = res(0, 99, (f) => ({ patterns: [pattern(40, 60, f, "forming")] }));
    // Bars 98 and 99 only: the pattern's points are before them.
    const tail = res(98, 99, (f) => ({ patterns: [pattern(40, 60, f, "confirmed")] }));
    expect(tail.indicators.p!.patterns![0]!.strokes[0]!.x1).toBe(-58);
    const r = mergeTail(cur, tail);
    if (!r.ok) throw new Error(r.reason);
    const [p] = r.series.indicators.p!.patterns!;
    expect(p!.status).toBe("confirmed");
    expect(p!.strokes[0]).toMatchObject({ x1: 40, x2: 60 });
    expect(p!.label.x).toBe(40);
  });

  it("one the tail no longer has is gone, even far before its window", () => {
    const cur = res(0, 99, (f) => ({ patterns: [pattern(40, 60, f, "forming")] }));
    const r = mergeTail(cur, res(98, 99, () => ({})));
    if (!r.ok) throw new Error(r.reason);
    expect(r.series.indicators.p!.patterns).toEqual([]);
  });

  it("an older page's are ignored, and ours move with the bars", () => {
    const cur = res(10, 19, (f) => ({ patterns: [pattern(12, 15, f, "forming")] }));
    const page = res(0, 9, (f) => ({ patterns: [pattern(1, 3, f, "confirmed")] }));
    const r = mergeOlder(cur, page);
    if (!r.ok) throw new Error(r.reason);
    const got = r.series.indicators.p!.patterns!;
    expect(got).toHaveLength(1);
    expect(got[0]!.strokes[0]).toMatchObject({ x1: 12, x2: 15 });
  });

  it("the first one to appear on a tail needs no reload", () => {
    const cur = res(0, 9, () => ({ events: [] }));
    const r = mergeTail(cur, res(8, 9, (f) => ({ patterns: [pattern(2, 5, f, "forming")] })));
    if (!r.ok) throw new Error(r.reason);
    expect(r.series.indicators.p!.patterns).toHaveLength(1);
  });
});

describe("events", () => {
  it("a tail's replace ours", () => {
    const ev = (t: number | null) => [{ key: "bullish", name: "Bullish", t }];
    const cur = res(0, 9, () => ({ events: ev(null) }));
    const r = mergeTail(cur, res(8, 9, () => ({ events: ev(tOf(8)) })));
    if (!r.ok) throw new Error(r.reason);
    expect(r.series.indicators.p!.events).toEqual(ev(tOf(8)));
  });
});

describe("levels", () => {
  const lines = (values: Array<number | null>) => [{ index: 0, label: "0.5", values }];
  const level = (bar: number, first: number, y: number) => ({
    line: 0,
    x: bar - first,
    t: tOf(bar),
    y,
  });
  const flat = (n: number, from: number, y: number) =>
    Array.from({ length: n }, (_, i) => (i >= from ? y : null));

  it("writeLevels makes a plot flat from its bar on, and null without a level", () => {
    const two = [
      { index: 0, label: "a", values: [9, 9, 9, 9] as Array<number | null> },
      { index: 1, label: "b", values: [9, 9, 9, 9] as Array<number | null> },
    ];
    writeLevels(two, [{ line: 1, x: 2, t: 0, y: 5 }], 4);
    expect(two[0]!.values).toEqual([null, null, null, null]);
    expect(two[1]!.values).toEqual([null, null, 5, 5]);
    // A level from before the first bar covers every bar.
    writeLevels(two, [{ line: 0, x: -3, t: 0, y: 7 }], 4);
    expect(two[0]!.values).toEqual([7, 7, 7, 7]);
  });

  it("an unchanged swing leaves the older bars alone", () => {
    const cur = res(0, 9, (f) => ({ lines: lines(flat(10, 3, 50)), levels: [level(3, f, 50)] }));
    const r = mergeTail(
      cur,
      res(8, 10, (f) => ({ lines: lines([50, 50, 50]), levels: [level(3, f, 50)] })),
    );
    if (!r.ok) throw new Error(r.reason);
    expect(r.from).toBe(8);
    expect(r.series.indicators.p!.lines![0]!.values).toEqual(flat(11, 3, 50));
  });

  it("a new swing rewrites the plot on every bar, and says from which", () => {
    const cur = res(0, 9, (f) => ({ lines: lines(flat(10, 3, 50)), levels: [level(3, f, 50)] }));
    const r = mergeTail(
      cur,
      res(8, 9, (f) => ({ lines: lines([70, 70]), levels: [level(6, f, 70)] })),
    );
    if (!r.ok) throw new Error(r.reason);
    // Bars 3..5 lost their old level, so the chart must update from bar 3.
    expect(r.from).toBe(3);
    expect(r.series.indicators.p!.lines![0]!.values).toEqual(flat(10, 6, 70));
    expect(r.series.indicators.p!.levels![0]!.x).toBe(6);
  });

  it("levels that went away clear the plot", () => {
    const cur = res(0, 9, (f) => ({ lines: lines(flat(10, 3, 50)), levels: [level(3, f, 50)] }));
    const r = mergeTail(cur, res(8, 9, () => ({ lines: lines([null, null]) })));
    if (!r.ok) throw new Error(r.reason);
    expect(r.from).toBe(3);
    expect(r.series.indicators.p!.lines![0]!.values).toEqual(flat(10, 99, 0));
  });

  it("an older page does not bring its own swing", () => {
    const cur = res(10, 19, (f) => ({ lines: lines(flat(10, 2, 50)), levels: [level(12, f, 50)] }));
    const page = res(0, 9, (f) => ({ lines: lines(flat(10, 1, 33)), levels: [level(1, f, 33)] }));
    const r = mergeOlder(cur, page);
    if (!r.ok) throw new Error(r.reason);
    expect(r.series.indicators.p!.lines![0]!.values).toEqual(flat(20, 12, 50));
    expect(r.series.indicators.p!.levels).toEqual([level(12, 0, 50)]);
  });
});
