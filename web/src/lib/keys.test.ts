import { afterEach, describe, expect, it, vi } from "vitest";

import { isTypingContext, keepKeyboardFocus } from "./keys";

class FakeEl {
  constructor(
    public tagName = "DIV",
    public isContentEditable = false,
  ) {}
  focus = vi.fn();
}

function stubDom(open: boolean, active?: unknown, host?: FakeEl) {
  const body = new FakeEl("BODY");
  let cb: (r: { removedNodes: unknown[] }[]) => void = () => {};
  vi.stubGlobal("HTMLElement", FakeEl);
  vi.stubGlobal(
    "MutationObserver",
    class {
      constructor(f: typeof cb) {
        cb = f;
      }
      observe = vi.fn();
      disconnect = vi.fn();
    },
  );
  vi.stubGlobal("document", {
    body,
    activeElement: active === undefined ? body : active,
    querySelector: (sel: string) =>
      sel === ".chart-host" ? (host ?? null) : open ? new FakeEl() : null,
  });
  return { body, fire: (n: number) => cb([{ removedNodes: new Array(n) }]) };
}

const key = (target: unknown, defaultPrevented = false) =>
  ({ target, defaultPrevented }) as unknown as KeyboardEvent;

afterEach(() => vi.unstubAllGlobals());

describe("isTypingContext", () => {
  it("is false on the page with nothing open", () => {
    stubDom(false);
    expect(isTypingContext(key(new FakeEl()))).toBe(false);
  });
  it("is true in inputs and contenteditable", () => {
    stubDom(false);
    expect(isTypingContext(key(new FakeEl("INPUT")))).toBe(true);
    expect(isTypingContext(key(new FakeEl("TEXTAREA")))).toBe(true);
    expect(isTypingContext(key(new FakeEl("DIV", true)))).toBe(true);
  });
  it("is true while a dialog or menu is in the DOM", () => {
    stubDom(true);
    expect(isTypingContext(key(new FakeEl()))).toBe(true);
  });
  it("is true for already handled keys", () => {
    stubDom(false);
    expect(isTypingContext(key(new FakeEl(), true))).toBe(true);
  });
});

describe("keepKeyboardFocus", () => {
  it("focuses the chart host when removal leaves focus on body", () => {
    const host = new FakeEl();
    const { fire } = stubDom(false, undefined, host);
    keepKeyboardFocus();
    fire(1);
    expect(host.focus).toHaveBeenCalledWith({ preventScroll: true });
  });
  it("leaves focus alone when an element still has it", () => {
    const host = new FakeEl();
    const { fire } = stubDom(false, new FakeEl("BUTTON"), host);
    keepKeyboardFocus();
    fire(1);
    expect(host.focus).not.toHaveBeenCalled();
  });
  it("ignores mutations that remove nothing", () => {
    const host = new FakeEl();
    const { fire } = stubDom(false, undefined, host);
    keepKeyboardFocus();
    fire(0);
    expect(host.focus).not.toHaveBeenCalled();
  });
});
