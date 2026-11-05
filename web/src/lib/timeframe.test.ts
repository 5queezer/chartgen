import { describe, expect, it } from "vitest";

import { parseTimeframe } from "./timeframe";

describe("parseTimeframe", () => {
  it.each([
    ["1", "1m"],
    ["5", "5m"],
    ["15", "15m"],
    ["30", "30m"],
    ["60", "1h"],
    ["240", "4h"],
    ["1h", "1h"],
    ["4H", "4h"],
    ["60m", "1h"],
    ["1D", "1d"],
    ["D", "1d"],
    ["d", "1d"],
    ["W", "1wk"],
    ["1w", "1wk"],
    ["24h", "1d"],
    ["M", "1mo"],
    ["1M", "1mo"],
    ["1mo", "1mo"],
    ["1MO", "1mo"],
    ["1m", "1m"],
    [" 15 ", "15m"],
  ])("%s -> %s", (input, expected) => {
    expect(parseTimeframe(input)).toBe(expected);
  });

  it.each(["", "h", "m", "7", "2M", "2h", "abc", "1x", "-5", "1.5"])(
    "rejects %j",
    (input) => {
      expect(parseTimeframe(input)).toBeNull();
    },
  );
});
