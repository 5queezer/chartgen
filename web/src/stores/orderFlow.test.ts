import { describe, expect, it } from "vitest";

import { defaultOrderFlow, sanitizeOrderFlow, tpoSessions } from "./orderFlow";

describe("order-flow settings", () => {
  it("defaults to what the research list says", () => {
    const d = defaultOrderFlow();
    expect(d.footprint.imbalance).toBe(300);
    expect(d.footprint.valueArea).toBe(70);
    expect(d.footprint.atrLength).toBe(14);
    expect(d.footprint.type).toBe("buy_sell");
    expect(d.tpo.period).toBe("1D");
    expect(d.tpo.block).toBe("30m");
    expect(d.tpo.valueArea).toBe(70);
  });

  it("falls back on anything stored that is not valid", () => {
    expect(sanitizeOrderFlow(null)).toEqual(defaultOrderFlow());
    expect(sanitizeOrderFlow("nope")).toEqual(defaultOrderFlow());
    const s = sanitizeOrderFlow({
      footprint: { imbalance: 50, type: "bogus", gradient: "yes", gradientColors: ["red"] },
      tpo: { period: "9Y", outsideOpacity: 5, sessions: 7 },
    });
    expect(s).toEqual(defaultOrderFlow());
  });

  it("keeps what is valid", () => {
    const s = sanitizeOrderFlow({
      footprint: { imbalance: 150, type: "delta", gradient: true },
      tpo: { period: "1W", splitByBlocks: true, sessions: "09:30-16:00" },
    });
    expect(s.footprint.imbalance).toBe(150);
    expect(s.footprint.type).toBe("delta");
    expect(s.footprint.gradient).toBe(true);
    expect(s.tpo.period).toBe("1W");
    expect(s.tpo.splitByBlocks).toBe(true);
    expect(tpoSessions(s.tpo.sessions)).toEqual(["09:30-16:00"]);
  });

  it("reads sessions as a list, at most 5", () => {
    expect(tpoSessions("")).toEqual([]);
    expect(tpoSessions(" a , b,, c ")).toEqual(["a", "b", "c"]);
    expect(tpoSessions("1,2,3,4,5,6")).toHaveLength(5);
  });
});
