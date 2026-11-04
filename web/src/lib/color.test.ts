import { describe, expect, it } from "vitest";

import { formatColor, parseColor } from "./color";

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
