import { describe, expect, it } from "vitest";

import { RANGES, RANGE_TIMEFRAME, barAlignedStart, rangeStart } from "./range";
import { isTimeframe } from "./timeframe";

const at = (y: number, mo: number, d: number, h = 0, mi = 0) =>
  Date.UTC(y, mo - 1, d, h, mi);

describe("RANGE_TIMEFRAME", () => {
  it("uses TradingView's intervals (6M is 4h, no 2h)", () => {
    expect(RANGE_TIMEFRAME).toEqual({
      "1D": "1m",
      "5D": "5m",
      "1M": "30m",
      "3M": "1h",
      "6M": "4h",
      YTD: "1d",
      "1Y": "1d",
      "5Y": "1wk",
      All: "1mo",
    });
    for (const r of RANGES) expect(isTimeframe(RANGE_TIMEFRAME[r])).toBe(true);
  });
});

describe("rangeStart", () => {
  const end = at(2026, 11, 5, 14, 30);

  it("1D and 5D are 24 h multiples", () => {
    expect(rangeStart("1D", end, "UTC")).toBe(at(2026, 11, 4, 14, 30));
    expect(rangeStart("5D", end, "UTC")).toBe(at(2026, 10, 31, 14, 30));
  });

  it("months are calendar months at the same wall time", () => {
    expect(rangeStart("1M", end, "UTC")).toBe(at(2026, 10, 5, 14, 30));
    expect(rangeStart("3M", end, "UTC")).toBe(at(2026, 8, 5, 14, 30));
    expect(rangeStart("6M", end, "UTC")).toBe(at(2026, 5, 5, 14, 30));
    expect(rangeStart("1Y", end, "UTC")).toBe(at(2025, 11, 5, 14, 30));
    expect(rangeStart("5Y", end, "UTC")).toBe(at(2021, 11, 5, 14, 30));
  });

  it("crosses a year boundary", () => {
    expect(rangeStart("3M", at(2026, 1, 15), "UTC")).toBe(at(2025, 10, 15));
  });

  it("clamps the day to the shorter target month", () => {
    expect(rangeStart("1M", at(2026, 3, 31), "UTC")).toBe(at(2026, 2, 28));
    expect(rangeStart("1M", at(2024, 3, 31), "UTC")).toBe(at(2024, 2, 29));
    expect(rangeStart("1Y", at(2024, 2, 29), "UTC")).toBe(at(2023, 2, 28));
  });

  it("keeps the wall clock across DST in the selected zone", () => {
    // 2026-11-05 09:30 New York (EST); 3M back is 2026-08-05 09:30 (EDT).
    const ny = "America/New_York";
    expect(rangeStart("3M", at(2026, 11, 5, 14, 30), ny)).toBe(at(2026, 8, 5, 13, 30));
  });

  it("YTD is 1 Jan 00:00 in the selected zone", () => {
    expect(rangeStart("YTD", end, "UTC")).toBe(at(2026, 1, 1));
    expect(rangeStart("YTD", end, "America/New_York")).toBe(at(2026, 1, 1, 5));
    expect(rangeStart("YTD", end, "Asia/Tokyo")).toBe(at(2025, 12, 31, 15));
  });

  it("YTD follows the year in the zone, not in UTC", () => {
    // 2026-01-01 02:00 UTC is still 2025 in New York.
    expect(rangeStart("YTD", at(2026, 1, 1, 2), "America/New_York")).toBe(at(2025, 1, 1, 5));
  });

  it("All has no start", () => {
    expect(rangeStart("All", end, "UTC")).toBeNull();
  });
});

describe("barAlignedStart", () => {
  it("keeps the daily bar of the start date west of UTC", () => {
    // YTD in New York starts 2026-01-01 05:00 UTC; the 1 Jan bar opens 00:00 UTC.
    const start = rangeStart("YTD", at(2026, 11, 5), "America/New_York")!;
    expect(barAlignedStart(start, "America/New_York")).toBe(at(2026, 1, 1));
  });
  it("is the same date east of UTC", () => {
    const start = rangeStart("YTD", at(2026, 11, 5), "Asia/Tokyo")!;
    expect(barAlignedStart(start, "Asia/Tokyo")).toBe(at(2026, 1, 1));
  });
});
