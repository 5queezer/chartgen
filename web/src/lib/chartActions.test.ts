import { describe, expect, it } from "vitest";

import { chartKeyAction, screenshotFilename } from "./chartActions";

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
