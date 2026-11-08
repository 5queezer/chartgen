import { describe, expect, it } from "vitest";

import {
  chartKeyAction,
  screenshotCaption,
  screenshotFilename,
  wrapJoined,
} from "./chartActions";

// Vitest runs in node, which has no KeyboardEvent; the mapper only reads these fields.
const key = (
  init: { key: string } & Partial<
    Pick<KeyboardEvent, "code" | "altKey" | "ctrlKey" | "metaKey">
  >,
) => ({ altKey: false, ctrlKey: false, metaKey: false, code: "", ...init }) as KeyboardEvent;

describe("chartKeyAction", () => {
  it.each([
    ["ArrowLeft", "scrollLeft"],
    ["ArrowRight", "scrollRight"],
    ["ArrowUp", "zoomIn"],
    ["+", "zoomIn"],
    ["=", "zoomIn"],
    ["ArrowDown", "zoomOut"],
    ["-", "zoomOut"],
    ["Home", "home"],
    ["End", "end"],
  ])("%s -> %s", (k, action) => {
    expect(chartKeyAction(key({ key: k }))).toBe(action);
  });
  it("maps Alt+R to reset and ignores plain r", () => {
    expect(chartKeyAction(key({ key: "r", code: "KeyR", altKey: true }))).toBe(
      "reset",
    );
    expect(chartKeyAction(key({ key: "r", code: "KeyR" }))).toBeNull();
  });
  it("maps Alt+I to invertScale, not when held or without Alt", () => {
    const i = { key: "i", code: "KeyI" } as const;
    expect(chartKeyAction(key({ ...i, altKey: true }))).toBe("invertScale");
    expect(
      chartKeyAction({ ...key({ ...i, altKey: true }), repeat: true } as KeyboardEvent),
    ).toBeNull();
    expect(chartKeyAction(key(i))).toBeNull();
    expect(chartKeyAction(key({ ...i, altKey: true, ctrlKey: true }))).toBeNull();
  });
  it("ignores other modified keys and unrelated keys", () => {
    expect(chartKeyAction(key({ key: "ArrowLeft", ctrlKey: true }))).toBeNull();
    expect(chartKeyAction(key({ key: "Home", altKey: true }))).toBeNull();
    expect(chartKeyAction(key({ key: "a" }))).toBeNull();
  });
});

describe("screenshotFilename", () => {
  it("formats symbol, timeframe and local timestamp", () => {
    expect(screenshotFilename("BTCUSDT", "1h", new Date(2026, 10, 4, 9, 5))).toBe(
      "BTCUSDT_1h_2026-11-04_0905.png",
    );
  });
  it("replaces filesystem-hostile characters", () => {
    expect(screenshotFilename("^GSPC/X", "1d", new Date(2026, 0, 2, 23, 59))).toBe(
      "-GSPC-X_1d_2026-01-02_2359.png",
    );
  });
});

describe("screenshotCaption", () => {
  it("titles symbol and timeframe as the toolbar writes them", () => {
    expect(screenshotCaption("BTCUSDT", "1h", [], {})).toEqual({
      title: "BTCUSDT · 1h",
      indicators: [],
    });
    expect(screenshotCaption("AAPL", "1wk", [], {}).title).toBe("AAPL · 1W");
  });
  it("names the visible indicators by their short names", () => {
    const entries = [
      "rsi",
      { name: "rsi", id: "rsi#2", length: 21 },
      { name: "macd", hidden: true },
      "bbands",
    ];
    expect(
      screenshotCaption("BTCUSDT", "1h", entries, { rsi: { length: 14 } }).indicators,
    ).toEqual(["RSI (14)", "RSI (21)", "Bollinger Bands"]);
  });
});

describe("wrapJoined", () => {
  const measure = (t: string) => t.length;
  it("joins items with a separator while they fit", () => {
    expect(wrapJoined(["A", "B", "C"], 100, measure)).toEqual(["A · B · C"]);
  });
  it("starts a new line instead of splitting an item", () => {
    expect(wrapJoined(["AAAA", "BBBB", "CC", "DDDDDDDDDD"], 9, measure)).toEqual([
      "AAAA",
      "BBBB · CC",
      "DDDDDDDDDD",
    ]);
  });
  it("is empty without items", () => {
    expect(wrapJoined([], 100, measure)).toEqual([]);
  });
});
