import { describe, expect, it } from "vitest";

import {
  chipName,
  displayName,
  prettifyId,
  rememberLabels,
  withoutParamNumber,
} from "./indicatorNames";

describe("indicator display names", () => {
  it("prettifies registry ids", () => {
    expect(prettifyId("volume_profile")).toBe("Volume Profile");
    expect(prettifyId("ema_stack")).toBe("EMA Stack");
    expect(prettifyId("bbands")).toBe("Bollinger Bands");
    expect(prettifyId("rsi")).toBe("RSI");
  });

  it("prefers the legend label of a payload once seen", () => {
    expect(displayName("macd_test")).toBe("MACD Test");
    rememberLabels({ macd_test: { label: "MACD" }, other: { label: null } });
    expect(displayName("macd_test")).toBe("MACD");
    expect(displayName("other")).toBe("Other");
  });
});

describe("withoutParamNumber / chipName", () => {
  it("drops the length the params already show", () => {
    expect(withoutParamNumber("SMA 20", "(20)")).toBe("SMA");
    expect(withoutParamNumber("SMA 50", "(50)")).toBe("SMA");
  });

  it("keeps labels without params or without a matching number", () => {
    expect(withoutParamNumber("SMA 20")).toBe("SMA 20");
    expect(withoutParamNumber("RSI", "(14)")).toBe("RSI");
  });

  it("reads SMA (50), not SMA 20 (50)", () => {
    rememberLabels({ ma: { label: "SMA 20" } });
    expect(chipName("ma", "(50)")).toBe("SMA (50)");
    expect(chipName("ma", "(20)")).toBe("SMA (20)");
  });
});
