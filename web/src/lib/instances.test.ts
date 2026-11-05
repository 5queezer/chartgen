import { describe, expect, it } from "vitest";

import { instanceLineColor, instanceNumber, keyName } from "./instances";

describe("keyName", () => {
  it("strips the instance number of a repeat", () => {
    expect(keyName("rsi")).toBe("rsi");
    expect(keyName("rsi#2")).toBe("rsi");
    expect(keyName("cipher_b#12")).toBe("cipher_b");
  });

  it("keeps anything that is not <name>#<n>", () => {
    for (const k of ["#2", "rsi#", "rsi#0", "rsi#02", "rsi#x", "a#b"]) {
      expect(keyName(k)).toBe(k);
    }
  });
});

describe("instance colors", () => {
  it("leaves the first instance to the server", () => {
    expect(instanceLineColor("ma", 0)).toBeUndefined();
  });

  it("gives later instances distinct colors", () => {
    expect(instanceNumber("ma#3")).toBe(3);
    const a = instanceLineColor("ma#2", 0);
    const b = instanceLineColor("ma#3", 0);
    expect(a).toBeDefined();
    expect(a).not.toBe(b);
  });
});
