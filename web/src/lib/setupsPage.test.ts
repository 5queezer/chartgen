import { describe, expect, it } from "vitest";

import answers from "../../../tests/fixtures/setup_responses.json";
import { SetupsPayloadSchema, type SetupRow } from "../types/responses";
import {
  NO_FILTERS,
  age,
  featureNames,
  filterRows,
  nextSort,
  recordArgs,
  resultsByDay,
  sortRows,
  tagGroupings,
  withOpen,
} from "./setupsPage";

// The server's own answers: a cancelled take, a skip that won and a take that was stopped.
const base = SetupsPayloadSchema.parse(answers.list_setups).setups[0]!;
const day = (d: string, h = 12) => new Date(`${d}T${String(h).padStart(2, "0")}:00`).getTime();
/** A made-up setup on the fixture's shape. */
const row = (id: string, over: Partial<SetupRow>): SetupRow =>
  ({ ...base, direction: "long", decision: "take", features: {}, id, ...over });

const rows: SetupRow[] = [
  row("a", { symbol: "ETHUSDT", status: "won", result_r: 1.5, created_at: day("2026-03-01"), exit: { price: 1, bar_time: day("2026-03-02") }, tags: ["strategy:breakout", "trigger:alert"] }),
  row("b", { symbol: "BTCUSDT", status: "invalidated", result_r: -1.02, created_at: day("2026-03-01", 13), exit: { price: 1, bar_time: day("2026-03-02", 15) }, tags: ["strategy:range"] }),
  row("c", { symbol: "BTCUSDT", direction: "short", decision: "skip", status: "watching", result_r: null, exit: null, created_at: day("2026-03-03"), tags: ["strategy:breakout"], features: { ctx_trend_4h: "up", example_flag: true } }),
  row("d", { symbol: "SOLUSDT", status: "expired", result_r: null, created_at: day("2026-03-04"), exit: null, scored_through: day("2026-03-05"), tags: [] }),
];
const ids = (list: SetupRow[]) => list.map((s) => s.id);

describe("the page's filters (ADR-0091)", () => {
  it("lets through what every filter allows", () => {
    expect(ids(filterRows(rows, NO_FILTERS))).toEqual(["a", "b", "c", "d"]);
    expect(ids(filterRows(rows, { ...NO_FILTERS, symbol: "BTCUSDT" }))).toEqual(["b", "c"]);
    expect(ids(filterRows(rows, { ...NO_FILTERS, decision: "skip" }))).toEqual(["c"]);
    expect(ids(filterRows(rows, { ...NO_FILTERS, direction: "short" }))).toEqual(["c"]);
    expect(ids(filterRows(rows, { ...NO_FILTERS, status: "won" }))).toEqual(["a"]);
    expect(ids(filterRows(rows, { ...NO_FILTERS, tag: "strategy:breakout" }))).toEqual(["a", "c"]);
  });

  it("takes a day range by when a setup was made, both days in", () => {
    const range = { ...NO_FILTERS, from: "2026-03-01", to: "2026-03-03" };
    expect(ids(filterRows(rows, range))).toEqual(["a", "b", "c"]);
    expect(ids(filterRows(rows, { ...NO_FILTERS, from: "2026-03-04" }))).toEqual(["d"]);
  });

  it("gives the track record the filters it has, the end as the next day's start", () => {
    expect(recordArgs(NO_FILTERS)).toEqual({});
    expect(
      recordArgs({ ...NO_FILTERS, symbol: "BTCUSDT", direction: "long", decision: "take", status: "won", tag: "x", from: "2026-03-01", to: "2026-03-03" }),
    ).toEqual({ symbol: "BTCUSDT", direction: "long", decision: "take", from: day("2026-03-01", 0), to: day("2026-03-04", 0) });
  });
});

describe("the setups listed", () => {
  it("adds the open setups older than the newest, each once", () => {
    const old = row("old", { status: "triggered", created_at: day("2026-01-01") });
    expect(ids(withOpen(rows, [rows[2]!, old]))).toEqual(["a", "b", "c", "d", "old"]);
  });
});

describe("sorting a table", () => {
  it("sorts by a column either way, with setups that have no value last", () => {
    expect(ids(sortRows(rows, { column: "result", descending: true }))).toEqual(["a", "b", "c", "d"]);
    expect(ids(sortRows(rows, { column: "result", descending: false }))).toEqual(["b", "a", "c", "d"]);
    expect(ids(sortRows(rows, { column: "symbol", descending: false }))).toEqual(["b", "c", "a", "d"]);
    expect(ids(sortRows(rows, { column: "ended", descending: true }))).toEqual(["d", "b", "a", "c"]);
  });

  it("flips the same column, and starts a time or a result newest or best first", () => {
    expect(nextSort({ column: "made", descending: true }, "made")).toEqual({ column: "made", descending: false });
    expect(nextSort({ column: "made", descending: true }, "symbol")).toEqual({ column: "symbol", descending: false });
    expect(nextSort({ column: "symbol", descending: false }, "result")).toEqual({ column: "result", descending: true });
  });
});

describe("the chart and the groupings", () => {
  it("plots each closed setup's own result on the day it ended, and the running sum", () => {
    const days = resultsByDay(rows);
    expect(days.map((d) => d.day)).toEqual(["2026-03-02"]);
    expect(days[0]!.count).toBe(2);
    expect(days[0]!.r).toBeCloseTo(0.48);
    const more = resultsByDay([...rows, row("e", { status: "won", result_r: 2, exit: { price: 1, bar_time: day("2026-03-06") } })]);
    expect(more.map((d) => d.cumulative)).toEqual([expect.closeTo(0.48), expect.closeTo(2.48)]);
  });

  it("offers tag families, each family's tags, and the tags most used first", () => {
    expect(tagGroupings(rows)).toEqual([
      { label: "tag families", prefixes: ["strategy:", "trigger:"] },
      { label: "strategy:…", prefixes: ["strategy:breakout", "strategy:range"] },
      { label: "trigger:…", prefixes: ["trigger:alert"] },
      { label: "tags", prefixes: ["strategy:breakout", "strategy:range", "trigger:alert"] },
    ]);
    const many = Array.from({ length: 12 }, (_, i) => row(`t${i}`, { tags: [`k:${String(i).padStart(2, "0")}`] }));
    expect(tagGroupings(many).find((g) => g.label === "k:…")!.prefixes).toHaveLength(10);
  });

  it("names the features to group by", () => {
    expect(featureNames(rows)).toEqual(["ctx_trend_4h", "example_flag"]);
  });

  it("says an age coarsely", () => {
    const now = day("2026-03-10");
    expect(age(now - 45 * 60_000, now)).toBe("45m");
    expect(age(now - 5 * 3_600_000, now)).toBe("5h");
    expect(age(now - 76 * 3_600_000, now)).toBe("3d 4h");
  });
});
