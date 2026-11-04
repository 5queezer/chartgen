import { describe, expect, it } from "vitest";

import {
  formatChange,
  formatChangePct,
  formatPrice,
  priceDecimals,
  trend,
} from "./quote";

describe("quote formatting", () => {
  it("picks decimals by price magnitude", () => {
    expect(priceDecimals(80776.01)).toBe(2);
    expect(priceDecimals(10)).toBe(2);
    expect(priceDecimals(1.1235)).toBe(4);
    expect(priceDecimals(0.25)).toBe(6);
    expect(priceDecimals(0.0000123)).toBe(8);
  });

  it("formats price, change and percent with signs", () => {
    expect(formatPrice(80776.014)).toBe("80776.01");
    expect(formatPrice(1.12351)).toBe("1.1235");
    expect(formatChange(2994.0099, 80776.01)).toBe("+2994.01");
    expect(formatChange(-0.00431, 1.1235)).toBe("−0.0043");
    expect(formatChangePct(3.8492)).toBe("+3.85%");
    expect(formatChangePct(-1.5)).toBe("−1.50%");
  });

  it("drops the sign when the value rounds to zero", () => {
    expect(formatChangePct(0.001)).toBe("0.00%");
    expect(formatChangePct(-0.004)).toBe("0.00%");
    expect(formatChange(0, 100)).toBe("0.00");
  });

  it("renders missing values as an em dash", () => {
    expect(formatPrice(null)).toBe("—");
    expect(formatChange(null, 1)).toBe("—");
    expect(formatChangePct(null)).toBe("—");
  });

  it("classifies the change direction", () => {
    expect(trend(1)).toBe("up");
    expect(trend(-1)).toBe("down");
    expect(trend(0)).toBe("flat");
    expect(trend(null)).toBe("flat");
  });
});
