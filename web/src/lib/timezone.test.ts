import { describe, expect, it } from "vitest";

import {
  exchangeTimeZone,
  formatClock,
  formatDateTime,
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
  it("puts a time inside a DST gap an hour early", () => {
    // 2026-03-08 02:30 does not exist in New York (clocks jump 02:00 -> 03:00).
    const w = { year: 2026, month: 3, day: 8, hour: 2, minute: 30, second: 0 };
    expect(wallClock(zonedToUtc(w, "America/New_York"), "America/New_York").hour).toBe(1);
  });
});
