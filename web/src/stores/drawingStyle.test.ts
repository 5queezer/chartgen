import { beforeEach, describe, expect, it } from "vitest";

import { createHistory } from "./drawingHistory";
import {
  loadDrawings,
  overlayStyles,
  parseRecord,
  sanitizeStyle,
  saveDrawings,
} from "./drawings";

const base = {
  name: "segment",
  points: [
    { timestamp: 1000, value: 10 },
    { timestamp: 2000, value: 20 },
  ],
  lock: false,
  visible: true,
};

describe("createHistory", () => {
  it("undoes and redoes in order", () => {
    const h = createHistory<number>();
    h.record(1);
    h.record(2);
    expect(h.undo(3)).toBe(2);
    expect(h.undo(2)).toBe(1);
    expect(h.undo(1)).toBeUndefined();
    expect(h.redo(1)).toBe(2);
    expect(h.redo(2)).toBe(3);
    expect(h.redo(3)).toBeUndefined();
  });

  it("a new change drops the redo side", () => {
    const h = createHistory<number>();
    h.record(1);
    expect(h.undo(2)).toBe(1);
    h.record(1);
    expect(h.redo(5)).toBeUndefined();
  });

  it("is bounded", () => {
    const h = createHistory<number>(3);
    for (let i = 0; i < 10; i++) h.record(i);
    const seen: number[] = [];
    for (let v = h.undo(99); v !== undefined; v = h.undo(v)) seen.push(v);
    expect(seen).toEqual([9, 8, 7]);
  });

  it("clear forgets both sides", () => {
    const h = createHistory<number>();
    h.record(1);
    h.undo(2);
    h.clear();
    expect(h.undo(0)).toBeUndefined();
    expect(h.redo(0)).toBeUndefined();
  });
});

describe("drawing style", () => {
  beforeEach(() => {
    const store = new Map<string, string>();
    globalThis.localStorage = {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
      removeItem: (k: string) => void store.delete(k),
    } as Storage;
  });

  it("old records without a style load with defaults", () => {
    const rec = parseRecord(base);
    expect(rec).not.toBeNull();
    expect(rec).not.toHaveProperty("style");
  });

  it("keeps valid style fields and drops invalid ones", () => {
    expect(
      sanitizeStyle({
        color: "#ff0000",
        width: 3,
        dash: "dotted",
        fill: "rgba(1, 2, 3, 0.5)",
        fontSize: 18,
      }),
    ).toEqual({
      color: "#ff0000",
      width: 3,
      dash: "dotted",
      fill: "rgba(1, 2, 3, 0.5)",
      fontSize: 18,
    });
    expect(
      sanitizeStyle({ color: "red; x", width: 9, dash: "wavy", fontSize: 1 }),
    ).toBeUndefined();
    expect(sanitizeStyle("nope")).toBeUndefined();
  });

  it("round-trips through storage", () => {
    const rec = parseRecord({ ...base, style: { color: "#00ff00", width: 2, dash: "dashed" } })!;
    expect(saveDrawings("X", [rec])).toBe(true);
    expect(loadDrawings("X")[0]?.style).toEqual({
      color: "#00ff00",
      width: 2,
      dash: "dashed",
    });
  });

  it("maps to klinecharts styles and thickens when selected", () => {
    expect(overlayStyles("segment", undefined, false).line).toEqual({ size: 1 });
    const s = overlayStyles("segment", { color: "#f00", width: 2, dash: "dotted" }, true);
    expect(s.line).toMatchObject({
      size: 4,
      color: "#f00",
      style: "dashed",
      dashedValue: [1, 3],
    });
    expect(s.polygon).toMatchObject({ borderSize: 4, borderColor: "#f00" });
  });

  it("applies fill to polygons and size/color to text", () => {
    expect(overlayStyles("cgRect", { fill: "#123456" }, false).polygon).toMatchObject({
      color: "#123456",
    });
    expect(
      overlayStyles("cgText", { color: "#abcdef", fontSize: 18 }, false).text,
    ).toEqual({ color: "#abcdef", size: 18 });
  });
});
