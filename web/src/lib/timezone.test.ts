import { describe, expect, it } from "vitest";

import {
  exchangeTimeZone,
  formatBarTime,
  formatClock,
  formatDateTime,
  isDateBar,
  isTimeZoneChoice,
  offsetLabel,
  resolveTimeZone,
  wallClock,
  zonedToUtc,
} from "./timezone";

// 2026-07-01 12:30:45 UTC (summer) and 2026-01-15 12:30:45 UTC (winter).
const SUMMER = Date.UTC(2026, 6, 1, 12, 30, 45);
const WINTER = Date.UTC(2026, 0, 15, 12, 30, 45);

describe("exchangeTimeZone (heuristic)", () => {
  it("maps crypto to UTC", () => {
    expect(exchangeTimeZone("BTCUSDT")).toBe("UTC");
    expect(exchangeTimeZone("ethusdt")).toBe("UTC");
  });
  it("maps known suffixes", () => {
    expect(exchangeTimeZone("SAP.DE")).toBe("Europe/Berlin");
    expect(exchangeTimeZone("VOD.L")).toBe("Europe/London");
    expect(exchangeTimeZone("SHOP.TO")).toBe("America/Toronto");
    expect(exchangeTimeZone("EURUSD=X")).toBe("UTC");
  });
  it("defaults to New York", () => {
    expect(exchangeTimeZone("AAPL")).toBe("America/New_York");
    expect(exchangeTimeZone("^GSPC")).toBe("America/New_York");
  });
});

describe("resolveTimeZone", () => {
  it("resolves the three symbolic choices and passes IANA names through", () => {
    expect(resolveTimeZone("utc", "AAPL")).toBe("UTC");
    expect(resolveTimeZone("exchange", "SAP.DE")).toBe("Europe/Berlin");
    expect(resolveTimeZone("local", "AAPL")).toBe(
      Intl.DateTimeFormat().resolvedOptions().timeZone,
    );
    expect(resolveTimeZone("Asia/Tokyo", "AAPL")).toBe("Asia/Tokyo");
  });
  it("validates stored choices", () => {
    expect(isTimeZoneChoice("utc")).toBe(true);
    expect(isTimeZoneChoice("Europe/Berlin")).toBe(true);
    expect(isTimeZoneChoice("Mars/Base")).toBe(false);
    expect(isTimeZoneChoice(3)).toBe(false);
  });
});

describe("formatters", () => {
  it("formats the crosshair label in the zone", () => {
    expect(formatDateTime(SUMMER, "UTC")).toBe("2026-07-01 12:30");
    expect(formatDateTime(SUMMER, "America/New_York")).toBe("2026-07-01 08:30");
    expect(formatDateTime(WINTER, "America/New_York")).toBe("2026-01-15 07:30");
    expect(formatDateTime(SUMMER, "Asia/Tokyo")).toBe("2026-07-01 21:30");
  });
  it("rolls the date over across midnight", () => {
    expect(formatDateTime(Date.UTC(2026, 11, 31, 23, 0), "Asia/Tokyo")).toBe(
      "2027-01-01 08:00",
    );
  });
  it("formats midnight as 00, not 24", () => {
    expect(formatClock(Date.UTC(2026, 0, 1, 0, 5, 9), "UTC")).toBe("00:05:09");
  });
  it("formats the clock", () => {
    expect(formatClock(SUMMER, "Europe/Berlin")).toBe("14:30:45");
  });
  it("labels the offset, following DST", () => {
    expect(offsetLabel(SUMMER, "UTC")).toBe("UTC");
    expect(offsetLabel(SUMMER, "America/New_York")).toBe("UTC-4");
    expect(offsetLabel(WINTER, "America/New_York")).toBe("UTC-5");
    expect(offsetLabel(WINTER, "Asia/Kolkata")).toBe("UTC+5:30");
  });
});

describe("zonedToUtc", () => {
  it("inverts wallClock", () => {
    for (const tz of ["UTC", "America/New_York", "Asia/Kolkata", "Australia/Sydney"]) {
      expect(zonedToUtc(wallClock(SUMMER, tz), tz)).toBe(SUMMER);
      expect(zonedToUtc(wallClock(WINTER, tz), tz)).toBe(WINTER);
    }
  });
  it("reads a wall time in the zone", () => {
    const w = { year: 2026, month: 7, day: 1, hour: 9, minute: 0, second: 0 };
    expect(zonedToUtc(w, "America/New_York")).toBe(Date.UTC(2026, 6, 1, 13, 0));
  });
  it("puts a time inside a DST gap after it", () => {
    // 2026-03-08 02:30 does not exist in New York (clocks jump 02:00 -> 03:00).
    const ny = { year: 2026, month: 3, day: 8, hour: 2, minute: 30, second: 0 };
    expect(wallClock(zonedToUtc(ny, "America/New_York"), "America/New_York")).toMatchObject({
      hour: 3,
      minute: 30,
    });
    // Berlin: 2026-03-29 02:00 -> 03:00.
    const de = { year: 2026, month: 3, day: 29, hour: 2, minute: 30, second: 0 };
    expect(wallClock(zonedToUtc(de, "Europe/Berlin"), "Europe/Berlin")).toMatchObject({
      day: 29,
      hour: 3,
      minute: 30,
    });
  });
  it("takes the first occurrence in a DST overlap", () => {
    // 2026-10-25 02:30 happens twice in Berlin; the first is CEST (UTC+2).
    const w = { year: 2026, month: 10, day: 25, hour: 2, minute: 30, second: 0 };
    expect(zonedToUtc(w, "Europe/Berlin")).toBe(Date.UTC(2026, 9, 25, 0, 30));
  });
});

describe("date bars", () => {
  it("recognises daily and longer intervals", () => {
    for (const i of ["1d", "2d", "1wk", "2wk", "1mo", "3mo"]) expect(isDateBar(i)).toBe(true);
    for (const i of ["1m", "7m", "4h", "36h", null]) expect(isDateBar(i)).toBe(false);
  });
  it("shows the date alone, the same in every zone", () => {
    const midnight = Date.UTC(2026, 0, 1);
    expect(formatBarTime(midnight, "America/New_York", "1d")).toBe("2026-01-01");
    expect(formatBarTime(midnight, "Asia/Tokyo", "1wk")).toBe("2026-01-01");
    expect(formatBarTime(midnight, "America/New_York", "1h")).toBe("2025-12-31 19:00");
  });
});
