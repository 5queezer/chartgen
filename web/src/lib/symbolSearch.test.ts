import { describe, expect, it } from "vitest";

import type { SearchResult } from "../types/responses";
import { CATEGORIES, buildRows, type Category } from "./symbolSearch";

const cat = (id: string): Category => {
  const c = CATEGORIES.find((c) => c.id === id);
  if (!c) throw new Error(id);
  return c;
};
const ALL = cat("all");

const remote = (symbol: string, type: SearchResult["type"]): SearchResult => ({
  symbol,
  name: `${symbol} name`,
  exchange: "X",
  type,
  source: "yahoo",
});

describe("buildRows", () => {
  it("lists recents before the curated list for an empty query", () => {
    const rows = buildRows({
      query: "  ",
      category: ALL,
      recents: ["AAPL", "ZZZ"],
      remote: null,
    });
    expect(rows.slice(0, 2).map((r) => [r.section, r.ticker])).toEqual([
      ["Recent", "AAPL"],
      ["Recent", "ZZZ"],
    ]);
    expect(rows[2]?.section).toBe("Popular");
  });

  it("filters the empty-query view by category and drops unknown recents", () => {
    const rows = buildRows({
      query: "",
      category: cat("crypto"),
      recents: ["AAPL", "ETHUSDT", "ZZZ"],
      remote: null,
    });
    expect(rows.every((r) => r.badge === "crypto")).toBe(true);
    expect(rows.filter((r) => r.section === "Recent")).toHaveLength(1);
  });

  it("uses server results in their order, with no as-typed row when listed", () => {
    const rows = buildRows({
      query: "spy",
      category: ALL,
      recents: [],
      remote: [remote("SPYM", "etf"), remote("SPY", "etf")],
    });
    expect(rows.map((r) => r.ticker)).toEqual(["SPYM", "SPY"]);
  });

  it("appends the as-typed row last when the ticker is not listed", () => {
    const rows = buildRows({
      query: "zzzz",
      category: ALL,
      recents: [],
      remote: [],
    });
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({ ticker: "ZZZZ", badge: "custom" });
  });

  it("falls back to the curated matches while server results are missing", () => {
    const rows = buildRows({
      query: "eth",
      category: ALL,
      recents: [],
      remote: null,
    });
    expect(rows[0]?.ticker).toBe("ETHUSDT");
    const stocks = buildRows({
      query: "eth",
      category: cat("stocks"),
      recents: [],
      remote: null,
    });
    expect(stocks.some((r) => r.ticker === "ETHUSDT")).toBe(false);
  });

  it("Funds covers etf and fund", () => {
    expect(cat("funds").types).toEqual(["etf", "fund"]);
  });
});
