import { describe, expect, it } from "vitest";

import { describeDrawing } from "./objectTree";

const ms = (y: number, m: number, d: number) => Date.UTC(y, m - 1, d, 12);

describe("describeDrawing", () => {
  it("shows the date span of a two-point drawing", () => {
    const rec = {
      points: [
        { timestamp: ms(2026, 11, 3), value: 1 },
        { timestamp: ms(2026, 11, 4), value: 2 },
      ],
    };
    expect(describeDrawing(rec, "Trend line", "UTC", 2)).toBe("Trend line 2026-11-03 → 11-04");
  });

  it("keeps the year when the span crosses it, and collapses a single day", () => {
    const p = (t: number) => ({ timestamp: t, value: 1 });
    expect(describeDrawing({ points: [p(ms(2025, 12, 31)), p(ms(2026, 1, 2))] }, "Ray", "UTC", 2)).toBe(
      "Ray 2025-12-31 → 2026-01-02",
    );
    expect(describeDrawing({ points: [p(ms(2026, 11, 3)), p(ms(2026, 11, 3) + 3600_000)] }, "Ray", "UTC", 2)).toBe(
      "Ray 2026-11-03",
    );
  });

  it("uses the selected zone for the date", () => {
    const rec = { points: [{ timestamp: Date.UTC(2026, 10, 3, 20), value: 1 }] };
    expect(describeDrawing(rec, "Horizontal line", "Asia/Tokyo", 2)).toBe(
      "Horizontal line 2026-11-04 @ 1.00",
    );
  });

  it("quotes annotation text and survives no points", () => {
    expect(
      describeDrawing({ points: [{ timestamp: ms(2026, 11, 3), value: 5 }], extendData: "hi" }, "Text", "UTC", 2),
    ).toBe("Text “hi”");
    expect(describeDrawing({ points: [] }, "Text", "UTC", 2)).toBe("Text");
  });
});
