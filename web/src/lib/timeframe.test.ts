import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import {
  TIMEFRAME_OPTIONS,
  isTimeframe,
  parseTimeframe,
  timeframeGroup,
  timeframeLabel,
} from "./timeframe";

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
    ["3", "3m"],
    ["120", "2h"],
    ["2h", "2h"],
    ["2H", "2h"],
    ["360", "6h"],
    ["6h", "6h"],
    ["720", "12h"],
    ["12h", "12h"],
    ["3D", "3d"],
    ["3d", "3d"],
    ["72h", "3d"],
    ["4320", "3d"],
  ])("%s -> %s", (input, expected) => {
    expect(parseTimeframe(input)).toBe(expected);
  });

  it.each([
    ["7", "7m"],
    ["45", "45m"],
    ["90", "90m"],
    ["7m", "7m"],
    ["8h", "8h"],
    ["2D", "2d"],
    ["2d", "2d"],
    ["2W", "2wk"],
    ["2M", "2mo"],
    ["3M", "3mo"],
    ["5h", "5h"],
    ["2 h", "2h"],
  ])("custom: %s -> %s", (input, expected) => {
    expect(parseTimeframe(input)).toBe(expected);
  });

  it.each(["", "h", "m", "0", "61", "abc", "1x", "-5", "1.5", "61m", "53W", "13M"])(
    "rejects %j",
    (input) => {
      expect(parseTimeframe(input)).toBeNull();
    },
  );
});

describe("the server's grammar", () => {
  // src/interval.rs runs the same cases, so the two parsers cannot drift apart.
  const fixture = JSON.parse(
    readFileSync(resolve(__dirname, "../../../tests/fixtures/interval_grammar.json"), "utf8"),
  ) as { valid: [string, string][]; invalid: string[] };

  it.each(fixture.valid)("%s -> %s", (input, canonical) => {
    expect(parseTimeframe(input)).toBe(canonical);
  });

  // The popup's own shortcuts and spacing are tested above; the rest must fail.
  const popupOnly = new Set(["7", "D", "7 m", " 7m", "7m "]);
  it.each(fixture.invalid.filter((c) => !popupOnly.has(c)))("rejects %j", (input) => {
    expect(parseTimeframe(input)).toBeNull();
  });
});

describe("isTimeframe", () => {
  it("takes canonical forms only, so stored values stay comparable", () => {
    for (const tf of [...TIMEFRAME_OPTIONS, "7m", "45m", "90m", "8h", "2d", "2wk", "3mo"]) {
      expect(isTimeframe(tf)).toBe(true);
    }
    for (const bad of ["2W", "120m", "1w", "1M", "61m", "", "banana", 7, null, undefined]) {
      expect(isTimeframe(bad)).toBe(false);
    }
  });
});

describe("timeframeLabel", () => {
  it.each([
    ["1m", "1m"],
    ["7m", "7m"],
    ["8h", "8h"],
    ["1d", "1D"],
    ["2d", "2D"],
    ["1wk", "1W"],
    ["2wk", "2W"],
    ["1mo", "1M"],
    ["3mo", "3M"],
  ])("%s -> %s", (tf, label) => {
    expect(timeframeLabel(tf)).toBe(label);
  });

  it("groups custom intervals as the presets", () => {
    expect(timeframeGroup("7m")).toBe("Minutes");
    expect(timeframeGroup("8h")).toBe("Hours");
    expect(timeframeGroup("2wk")).toBe("Days");
  });
});
