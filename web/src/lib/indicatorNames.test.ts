import { describe, expect, it } from "vitest";

import { displayName, prettifyId, rememberLabels } from "./indicatorNames";

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
