import { describe, expect, it } from "vitest";

import {
  decodeIndicators,
  decodeShare,
  encodeIndicators,
  encodeShare,
  hasShareParams,
  stripShareParams,
  type ShareState,
} from "./shareLink";

const state: ShareState = {
  symbol: "ETHUSDT",
  timeframe: "4h",
  chartType: "hollow",
  indicators: ["ema_stack", { name: "rsi", length: 21 }],
  compare: ["BTCUSDT", "SOLUSDT"],
  timeZone: "Europe/Berlin",
};

describe("share links", () => {
  it("round-trips a full chart", () => {
    const q = encodeShare(state);
    expect(q).toBe(
      "symbol=ETHUSDT&interval=4h&type=hollow&ind=ema_stack,rsi(length:21)&compare=BTCUSDT,SOLUSDT&tz=Europe/Berlin",
    );
    expect(decodeShare(q)).toEqual(state);
  });

  it("omits default params and hidden entries", () => {
    const out = encodeIndicators(
      [
        { name: "rsi", length: 14 },
        { name: "macd", fast: 8, slow: 26 },
        { name: "ema_stack", hidden: true },
      ],
      { rsi: { length: 14 }, macd: { fast: 12, slow: 26 } },
    );
    expect(out).toBe("rsi,macd(fast:8)");
  });

  it("round-trips multiple params, booleans and words", () => {
    const entries = [{ name: "x", a: 1.5, b: true, c: "close" }];
    expect(decodeIndicators(encodeIndicators(entries))).toEqual(entries);
  });

  it("keeps an empty ind as no indicators, absent as untouched", () => {
    expect(decodeShare("ind=").indicators).toEqual([]);
    expect(decodeShare("symbol=AAPL").indicators).toBeUndefined();
    expect(decodeShare("ind=nope!!").indicators).toBeUndefined();
  });

  it("does not split on commas inside parentheses", () => {
    expect(decodeIndicators("macd(fast:8;slow:20),rsi")).toEqual([
      { name: "macd", fast: 8, slow: 20 },
      "rsi",
    ]);
  });

  it("is tolerant of bad indicator input", () => {
    expect(decodeIndicators("rsi(length:,bogus,x:<b>),,a b,rsi,ok")).toEqual([
      "rsi",
      "ok",
    ]);
    expect(decodeIndicators("rsi(length:21),rsi(length:5)")).toEqual([
      { name: "rsi", length: 21 },
    ]);
  });

  it("keeps a __proto__ param as data", () => {
    const [e] = decodeIndicators("x(__proto__:1)");
    expect(Object.getPrototypeOf(e)).toBe(Object.prototype);
    expect(Object.keys(e as object)).toContain("__proto__");
  });

  it("falls back per field on invalid values", () => {
    expect(
      decodeShare("symbol=&interval=7x&type=pie&compare=,,%20&tz=Mars/Base"),
    ).toEqual({ compare: [] });
    expect(decodeShare("symbol=a%20b!&interval=").symbol).toBeUndefined();
  });

  it("normalises symbol, interval and type", () => {
    expect(decodeShare("symbol=ethusdt&interval=240&type=Candles")).toEqual({
      symbol: "ETHUSDT",
      timeframe: "4h",
      chartType: "candles",
    });
  });

  it("detects and strips only share params", () => {
    expect(hasShareParams("?code=1&symbol=X")).toBe(true);
    expect(hasShareParams("?code=1")).toBe(false);
    expect(hasShareParams("")).toBe(false);
    expect(stripShareParams("?symbol=X&code=1&state=s&ind=rsi")).toBe(
      "?code=1&state=s",
    );
    expect(stripShareParams("?symbol=X")).toBe("");
  });

  it("never throws on garbage", () => {
    expect(() => decodeShare("%E0%A4%A&&===&ind=(((")).not.toThrow();
  });
});
