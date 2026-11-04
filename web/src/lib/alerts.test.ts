import { describe, expect, it } from "vitest";

import { alertStatus, evaluate, rearmed, type Alert } from "./alerts";

const at = (condition: Alert["condition"], price = 100) => ({
  condition,
  price,
});

describe("evaluate: crossing", () => {
  const a = at("crossing");
  it("fires from below and from above", () => {
    expect(evaluate(a, 99, 101)).toBe(true);
    expect(evaluate(a, 101, 99)).toBe(true);
  });
  it("fires when the price lands on the level", () => {
    expect(evaluate(a, 99, 100)).toBe(true);
    expect(evaluate(a, 101, 100)).toBe(true);
  });
  it("does not fire while staying on one side", () => {
    expect(evaluate(a, 99, 99.5)).toBe(false);
    expect(evaluate(a, 101, 105)).toBe(false);
  });
  it("does not fire when leaving the level or standing on it", () => {
    expect(evaluate(a, 100, 101)).toBe(false);
    expect(evaluate(a, 100, 100)).toBe(false);
  });
  it("needs a previous price", () => {
    expect(evaluate(a, null, 101)).toBe(false);
  });
});

describe("evaluate: crossing_up / crossing_down", () => {
  it("crossing_up only fires upward", () => {
    const a = at("crossing_up");
    expect(evaluate(a, 99, 101)).toBe(true);
    expect(evaluate(a, 99, 100)).toBe(true);
    expect(evaluate(a, 101, 99)).toBe(false);
    expect(evaluate(a, 100, 101)).toBe(false);
    expect(evaluate(a, null, 101)).toBe(false);
  });
  it("crossing_down only fires downward", () => {
    const a = at("crossing_down");
    expect(evaluate(a, 101, 99)).toBe(true);
    expect(evaluate(a, 101, 100)).toBe(true);
    expect(evaluate(a, 99, 101)).toBe(false);
    expect(evaluate(a, 100, 99)).toBe(false);
    expect(evaluate(a, null, 99)).toBe(false);
  });
});

describe("evaluate: greater / less", () => {
  it("greater is strict and needs no previous price", () => {
    const a = at("greater");
    expect(evaluate(a, null, 100.01)).toBe(true);
    expect(evaluate(a, 90, 100)).toBe(false);
    expect(evaluate(a, 101, 102)).toBe(true);
  });
  it("less is strict and needs no previous price", () => {
    const a = at("less");
    expect(evaluate(a, null, 99.99)).toBe(true);
    expect(evaluate(a, 110, 100)).toBe(false);
  });
});

describe("rearmed", () => {
  it("greater re-arms at or below the level, less at or above", () => {
    expect(rearmed(at("greater"), 101)).toBe(false);
    expect(rearmed(at("greater"), 100)).toBe(true);
    expect(rearmed(at("less"), 99)).toBe(false);
    expect(rearmed(at("less"), 100)).toBe(true);
  });
  it("crossing conditions are always armed", () => {
    expect(rearmed(at("crossing"), 150)).toBe(true);
    expect(rearmed(at("crossing_up"), 150)).toBe(true);
    expect(rearmed(at("crossing_down"), 50)).toBe(true);
  });
});

describe("alertStatus", () => {
  const base: Alert = {
    id: "a",
    symbol: "X",
    condition: "crossing",
    price: 1,
    trigger: "once",
    active: true,
    createdAt: 0,
  };
  it("derives active, triggered and paused", () => {
    expect(alertStatus(base)).toBe("active");
    expect(alertStatus({ ...base, active: false })).toBe("paused");
    expect(alertStatus({ ...base, active: false, lastTriggeredAt: 5 })).toBe(
      "triggered",
    );
  });
});
