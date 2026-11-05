import { describe, expect, it } from "vitest";

import { undoRedoKey } from "./keys";

const ev = (key: string, code: string, extra: Record<string, boolean> = {}) => ({
  key,
  code,
  ctrlKey: true,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...extra,
});

describe("undoRedoKey", () => {
  it("QWERTY", () => {
    expect(undoRedoKey(ev("z", "KeyZ"))).toBe("undo");
    expect(undoRedoKey(ev("Z", "KeyZ", { shiftKey: true }))).toBe("redo");
    expect(undoRedoKey(ev("y", "KeyY"))).toBe("redo");
  });

  it("QWERTZ: the typed letter wins over the physical position", () => {
    expect(undoRedoKey(ev("z", "KeyY"))).toBe("undo");
    expect(undoRedoKey(ev("y", "KeyZ"))).toBe("redo");
  });

  it("falls back to the code on non-Latin layouts", () => {
    expect(undoRedoKey(ev("я", "KeyZ"))).toBe("undo");
  });

  it("ignores other keys, Alt and missing modifier", () => {
    expect(undoRedoKey(ev("x", "KeyX"))).toBeNull();
    expect(undoRedoKey(ev("z", "KeyZ", { altKey: true }))).toBeNull();
    expect(undoRedoKey(ev("z", "KeyZ", { ctrlKey: false }))).toBeNull();
    expect(undoRedoKey(ev("Y", "KeyY", { shiftKey: true }))).toBeNull();
  });

  it("accepts Cmd", () => {
    expect(undoRedoKey(ev("z", "KeyZ", { ctrlKey: false, metaKey: true }))).toBe("undo");
  });
});
