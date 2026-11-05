import { describe, expect, it } from "vitest";

import { effectiveMagnet } from "./magnet";

describe("effectiveMagnet", () => {
  it("keeps the chosen magnet without the modifier", () => {
    expect(effectiveMagnet("normal", false)).toBe("normal");
    expect(effectiveMagnet("weak_magnet", false)).toBe("weak_magnet");
    expect(effectiveMagnet("strong_magnet", false)).toBe("strong_magnet");
  });

  it("inverts it while the modifier is held", () => {
    expect(effectiveMagnet("normal", true)).toBe("strong_magnet");
    expect(effectiveMagnet("weak_magnet", true)).toBe("normal");
    expect(effectiveMagnet("strong_magnet", true)).toBe("normal");
  });
});
