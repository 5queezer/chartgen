import { describe, expect, it } from "vitest";

import { keyName } from "./instances";

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
