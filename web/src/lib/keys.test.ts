import { afterEach, describe, expect, it, vi } from "vitest";

import { isRelease, isTypingContext, keepKeyboardFocus, swallowMenuGhostClicks } from "./keys";

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

describe("isRelease", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("is the left button, or the last finger", () => {
    class FakeMouse {
      constructor(public button: number) {}
    }
    vi.stubGlobal("MouseEvent", FakeMouse);
    const ev = (o: object) => o as unknown as Event;
    expect(isRelease(ev(new FakeMouse(0)))).toBe(true);
    expect(isRelease(ev(new FakeMouse(2)))).toBe(false);
    expect(isRelease(ev({ touches: [] }))).toBe(true);
    expect(isRelease(ev({ touches: [{}] }))).toBe(false);
    expect(isRelease(ev({}))).toBe(true);
  });
});

describe("swallowMenuGhostClicks", () => {
  class FakeNode {
    contains(other: unknown): boolean {
      return other === this;
    }
  }
  class FakeElement extends FakeNode {
    closest(): FakeElement | null {
      return this;
    }
  }

  function install() {
    const listeners = new Map<string, (e: unknown) => void>();
    vi.stubGlobal("Node", FakeNode);
    vi.stubGlobal("Element", FakeElement);
    vi.stubGlobal("document", {
      addEventListener: (t: string, f: (e: unknown) => void) => listeners.set(t, f),
      removeEventListener: (t: string) => listeners.delete(t),
    });
    const stop = swallowMenuGhostClicks();
    const fire = (type: string, target: unknown, init: Record<string, unknown> = {}) => {
      const e = {
        type,
        target,
        isTrusted: true,
        detail: 1,
        pointerType: "touch",
        preventDefault: vi.fn(),
        stopPropagation: vi.fn(),
        ...init,
      };
      listeners.get(type)?.(e);
      return e;
    };
    return { listeners, stop, fire, item: new FakeElement(), elsewhere: new FakeNode() };
  }

  afterEach(() => vi.unstubAllGlobals());

  it("drops the tap's click when it lands outside the pressed item", () => {
    const { fire, item, elsewhere } = install();
    fire("pointerdown", item);
    expect(fire("mousedown", elsewhere).preventDefault).toHaveBeenCalled();
    expect(fire("click", elsewhere).preventDefault).toHaveBeenCalled();
    // Only that one.
    expect(fire("click", elsewhere).preventDefault).not.toHaveBeenCalled();
  });

  it("lets a click on the pressed item through", () => {
    const { fire, item } = install();
    fire("pointerdown", item);
    expect(fire("click", item).preventDefault).not.toHaveBeenCalled();
  });

  it("leaves a mouse press alone", () => {
    const { fire, item, elsewhere } = install();
    fire("pointerdown", item, { pointerType: "mouse" });
    expect(fire("click", elsewhere).preventDefault).not.toHaveBeenCalled();
  });

  it("lets a script's click() through, and still catches the real one after it", () => {
    const { fire, item, elsewhere } = install();
    fire("pointerdown", item);
    expect(fire("click", elsewhere, { isTrusted: false }).preventDefault).not.toHaveBeenCalled();
    expect(fire("click", elsewhere).preventDefault).toHaveBeenCalled();
  });

  it("lets a keyboard click through", () => {
    const { fire, item, elsewhere } = install();
    fire("pointerdown", item);
    expect(fire("click", elsewhere, { detail: 0 }).preventDefault).not.toHaveBeenCalled();
  });

  it("forgets the press when the touch is cancelled", () => {
    const { fire, item, elsewhere } = install();
    fire("pointerdown", item);
    fire("pointercancel", item);
    expect(fire("click", elsewhere).preventDefault).not.toHaveBeenCalled();
  });

  it("removes every listener on cleanup", () => {
    const { listeners, stop } = install();
    expect([...listeners.keys()].sort()).toEqual(["click", "mousedown", "pointercancel", "pointerdown"]);
    stop();
    expect(listeners.size).toBe(0);
  });
});
