import { describe, expect, it } from "vitest";

import { loadErrorReason } from "./chartLoadError";

describe("loadErrorReason", () => {
  it("names unknown indicators, one or several", () => {
    const one = "MCP tool error: Unknown indicator(s): gone_x. Call list_indicators to see available indicators.";
    expect(loadErrorReason(one)).toEqual({ text: "unknown indicator gone_x", indicators: ["gone_x"] });
    const two = "MCP tool error: Unknown indicator(s): a, b. Call list_indicators to see available indicators.";
    expect(loadErrorReason(two).indicators).toEqual(["a", "b"]);
  });

  it("names the indicator with an invalid parameter", () => {
    const r = loadErrorReason("MCP tool error: Invalid parameter for rsi: length must be >= 1");
    expect(r).toEqual({ text: "Invalid parameter for rsi: length must be >= 1", indicators: ["rsi"] });
  });

  it("offers nothing to remove for other failures", () => {
    expect(loadErrorReason("MCP tool error: no data for ZZZ")).toEqual({ text: "symbol not found", indicators: [] });
    expect(loadErrorReason("HTTP 502")).toEqual({ text: "request failed", indicators: [] });
  });
});

describe("errors that say what to change", () => {
  it("keeps the server's own words", () => {
    expect(
      loadErrorReason(
        "MCP tool error: The chart would have too many bars. Box size 1e-7 is too small; choose a larger one.",
      ).text,
    ).toBe("Box size 1e-7 is too small; choose a larger one.");
    expect(
      loadErrorReason(
        "MCP tool error: Data fetch failed: Yahoo has no seconds data: AAPL cannot be charted at 5s. Seconds intervals need a Binance symbol.",
      ).text,
    ).toBe("Yahoo has no seconds data: AAPL cannot be charted at 5s. Seconds intervals need a Binance symbol.");
    expect(
      loadErrorReason("MCP tool error: The source bars give no box size (no price movement); use a fixed box.").text,
    ).toBe("no box size (no price movement); use a fixed box.");
  });
});
