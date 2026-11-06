import { describe, expect, it } from "vitest";

import {
  barCloseMs,
  barCountdown,
  formatCountdown,
  isContinuousMarket,
} from "./countdown";
import { seriesCountdown } from "./countdown";

const T0 = Date.UTC(2026, 0, 5, 12, 0, 0) / 1000; // Monday 12:00 UTC

describe("barCloseMs", () => {
  it.each([
    ["1m", 60],
    ["3m", 180],
    ["2h", 7_200],
    ["6h", 21_600],
    ["12h", 43_200],
    ["3d", 259_200],
    ["15m", 900],
    ["1h", 3600],
    ["4h", 14_400],
    ["1d", 86_400],
    ["1wk", 604_800],
    ["7m", 420],
    ["45m", 2_700],
    ["90m", 5_400],
    ["8h", 28_800],
    ["2d", 172_800],
    ["2wk", 1_209_600],
  ])("%s adds a fixed length", (interval, secs) => {
    expect(barCloseMs(T0, interval)).toBe((T0 + secs) * 1000);
  });
  it("follows the calendar for months", () => {
    const jan = Date.UTC(2026, 0, 1) / 1000;
    expect(barCloseMs(jan, "1mo")).toBe(Date.UTC(2026, 1, 1));
    const dec = Date.UTC(2026, 11, 1) / 1000;
    expect(barCloseMs(dec, "1mo")).toBe(Date.UTC(2027, 0, 1));
  });
  it("is null for unknown intervals", () => {
    expect(barCloseMs(T0, "2x")).toBeNull();
    expect(barCloseMs(T0, "")).toBeNull();
  });
});

describe("formatCountdown", () => {
  const close = 1_000_000_000_000;
  it("shows MM:SS under an hour", () => {
    expect(formatCountdown(close, close - (12 * 60 + 34) * 1000)).toBe("12:34");
    expect(formatCountdown(close, close - 5000)).toBe("00:05");
  });
  it("shows HH:MM:SS under a day", () => {
    expect(formatCountdown(close, close - (3600 + 62) * 1000)).toBe("01:01:02");
  });
  it("shows days with HH:MM from a day up", () => {
    expect(
      formatCountdown(close, close - (86_400 + 3 * 3600 + 12 * 60) * 1000),
    ).toBe("1d 03:12");
    expect(formatCountdown(close, close - 6 * 86_400 * 1000)).toBe("6d 00:00");
  });
  it("rounds partial seconds up so it never shows 00:00 early", () => {
    expect(formatCountdown(close, close - 400)).toBe("00:01");
  });
  it("rolls over exactly at the close and stays at 00:00 after", () => {
    expect(formatCountdown(close, close)).toBe("00:00");
    expect(formatCountdown(close, close + 3_600_000)).toBe("00:00");
  });
});

describe("barCountdown", () => {
  it("combines open time, interval and now", () => {
    const now = (T0 + 3600 - 90) * 1000;
    expect(barCountdown(T0, "1h", now)).toBe("01:30");
  });
  it("handles daily and weekly bars", () => {
    expect(barCountdown(T0, "1d", T0 * 1000)).toBe("1d 00:00");
    expect(barCountdown(T0, "1wk", T0 * 1000)).toBe("7d 00:00");
  });
  it("is null for an unknown interval", () => {
    expect(barCountdown(T0, "nope", 0)).toBeNull();
  });
  it("is null once the bar has closed", () => {
    expect(barCountdown(T0, "1h", (T0 + 3600) * 1000)).toBeNull();
    expect(barCountdown(T0, "1h", (T0 + 7200) * 1000)).toBeNull();
    expect(barCountdown(T0, "1h", (T0 + 3599) * 1000)).toBe("00:01");
  });
});

describe("isContinuousMarket", () => {
  it.each(["BTCUSDT", "ethbtc", "SOLFDUSD", "BNBUSDC", "SOLUSDT"])(
    "%s is 24/7",
    (s) => expect(isContinuousMarket(s)).toBe(true),
  );
  it.each(["AAPL", "^GSPC", "EURUSD=X", "BTC", "USDT", "", null, undefined])(
    "%s is not",
    (s) => expect(isContinuousMarket(s)).toBe(false),
  );
});

describe("seriesCountdown", () => {
  const series = { symbol: "BTCUSDT", interval: "1h", bars: [{ t: 3600 }] };
  const now = 3600 * 1000 + 60_000;

  it("counts down the last bar of a time-based chart", () => {
    expect(seriesCountdown(series, now)).toBe("59:00");
  });

  it("shows none on a price-based chart, whose bars close on price", () => {
    expect(seriesCountdown({ ...series, chart_params: { box_size: 5 } }, now)).toBeNull();
  });

  it("shows none for a session market or without a timed bar", () => {
    expect(seriesCountdown({ ...series, symbol: "AAPL" }, now)).toBeNull();
    expect(seriesCountdown({ ...series, bars: [{ t: null }] }, now)).toBeNull();
    expect(seriesCountdown(null, now)).toBeNull();
  });
});
