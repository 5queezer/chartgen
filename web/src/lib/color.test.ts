import { describe, expect, it } from "vitest";

import { contrastRatio, formatColor, parseColor, readableOn } from "./color";

describe("parseColor", () => {
  it("parses hex forms", () => {
    expect(parseColor("#f00")).toEqual({ hex: "#ff0000", alpha: 1 });
    expect(parseColor("#2962FF")).toEqual({ hex: "#2962ff", alpha: 1 });
    expect(parseColor("#ff000080").alpha).toBeCloseTo(0.5, 2);
  });

  it("parses rgba", () => {
    expect(parseColor("rgba(255, 0, 0, 0.25)")).toEqual({ hex: "#ff0000", alpha: 0.25 });
  });

  it("falls back to opaque black", () => {
    expect(parseColor("nope")).toEqual({ hex: "#000000", alpha: 1 });
  });
});

describe("formatColor", () => {
  it("is hex when opaque and rgba otherwise", () => {
    expect(formatColor("#ff0000", 1)).toBe("#ff0000");
    expect(formatColor("#ff0000", 0.5)).toBe("rgba(255, 0, 0, 0.5)");
  });
});

describe("readableOn", () => {
  it("darkens light colors on a light background to 3:1", () => {
    const out = readableOn("#ffeb3b", "#ffffff");
    expect(out).not.toBe("#ffeb3b");
    expect(contrastRatio(out, "#ffffff")).toBeGreaterThanOrEqual(3);
  });
  it("leaves readable colors and dark backgrounds alone", () => {
    expect(readableOn("#2962ff", "#ffffff")).toBe("#2962ff");
    expect(readableOn("#ffeb3b", "#131722")).toBe("#ffeb3b");
  });
  it("drops the alpha so the contrast holds", () => {
    const out = readableOn("rgba(255, 235, 59, 0.4)", "#ffffff");
    expect(out).toMatch(/^#[0-9a-f]{6}$/);
    expect(contrastRatio(out, "#ffffff")).toBeGreaterThanOrEqual(3);
    expect(readableOn("rgba(41, 98, 255, 0.5)", "#131722")).toBe("#2962ff");
  });
  it("leaves unparsable colors alone", () => {
    expect(readableOn("red", "#ffffff")).toBe("red");
  });
});
