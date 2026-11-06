import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createFlowFeed, MAX_ITEMS, type FlowPage, type FlowRequest, type FlowState } from "./orderFlowFeed";

interface Item {
  t: number;
  v: number;
}
interface Meta {
  row_size: number;
}

function setup(hidden = false) {
  const requests: FlowRequest[] = [];
  const states: FlowState<Item, Meta>[] = [];
  let next: (r: FlowRequest) => FlowPage<Item, Meta> = () => ({
    items: [{ t: 100, v: 1 }, { t: 200, v: 2 }],
    meta: { row_size: 5 },
    hasMore: true,
  });
  const doc = { visibilityState: hidden ? "hidden" : "visible" } as unknown as Document;
  const feed = createFlowFeed<Item, Meta>({
    pollMs: 10_000,
    doc: doc as never,
    onChange: (s) => states.push(s),
    fetch: async (r) => {
      requests.push(r);
      return next(r);
    },
  });
  return {
    feed,
    requests,
    states,
    doc: doc as { visibilityState: string },
    last: () => states.at(-1)!,
    setNext: (f: typeof next) => (next = f),
  };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("order-flow feed", () => {
  it("loads, then polls only the tail with the row size of the load", async () => {
    const t = setup();
    t.feed.restart();
    await vi.advanceTimersByTimeAsync(0);
    expect(t.requests).toEqual([{}]);
    expect(t.last().status).toBe("live");
    await vi.advanceTimersByTimeAsync(10_000);
    expect(t.requests[1]).toEqual({ tail: true, rowSize: 5 });
    t.feed.dispose();
  });

  it("repaints the forming bar: a poll replaces items by time", async () => {
    const t = setup();
    t.feed.restart();
    await vi.advanceTimersByTimeAsync(0);
    t.setNext(() => ({ items: [{ t: 200, v: 9 }, { t: 300, v: 3 }], meta: { row_size: 5 }, hasMore: true }));
    await vi.advanceTimersByTimeAsync(10_000);
    const items = t.last().items;
    expect([...items.keys()].sort()).toEqual([100, 200, 300]);
    expect(items.get(200)!.v).toBe(9);
    t.feed.dispose();
  });

  it("gives each change a new map, so a chart sees it", async () => {
    const t = setup();
    t.feed.restart();
    await vi.advanceTimersByTimeAsync(10_000);
    const [a, b] = t.states.slice(-2);
    expect(a!.items).not.toBe(b!.items);
    t.feed.dispose();
  });

  it("does not ask while the tab is hidden or the chart replays", async () => {
    const t = setup();
    t.feed.restart();
    await vi.advanceTimersByTimeAsync(0);
    t.doc.visibilityState = "hidden";
    await vi.advanceTimersByTimeAsync(60_000);
    expect(t.requests).toHaveLength(1);
    t.doc.visibilityState = "visible";
    await vi.advanceTimersByTimeAsync(10_000);
    expect(t.requests).toHaveLength(2);
    t.feed.dispose();
  });

  it("pages older items once when the chart reaches them", async () => {
    const t = setup();
    t.feed.restart();
    await vi.advanceTimersByTimeAsync(0);
    t.setNext(() => ({ items: [{ t: 50, v: 0 }], meta: { row_size: 5 }, hasMore: false }));
    t.feed.ensure(150);
    expect(t.requests).toHaveLength(1);
    t.feed.ensure(60);
    t.feed.ensure(60);
    await vi.advanceTimersByTimeAsync(0);
    expect(t.requests.slice(1)).toEqual([{ endTime: 100_000, rowSize: 5 }]);
    expect(t.last().items.has(50)).toBe(true);
    // Nothing older exists: it stops asking.
    t.feed.ensure(10);
    await vi.advanceTimersByTimeAsync(0);
    expect(t.requests).toHaveLength(2);
    t.feed.dispose();
  });

  it("stays bounded, dropping the oldest", async () => {
    const t = setup();
    t.setNext(() => ({
      items: Array.from({ length: MAX_ITEMS + 50 }, (_, k) => ({ t: k, v: k })),
      meta: { row_size: 1 },
      hasMore: true,
    }));
    t.feed.restart();
    await vi.advanceTimersByTimeAsync(0);
    const items = t.last().items;
    expect(items.size).toBe(MAX_ITEMS);
    expect(items.has(0)).toBe(false);
    expect(items.has(MAX_ITEMS + 49)).toBe(true);
    t.feed.dispose();
  });

  it("says what failed, keeps what it has, and backs off", async () => {
    const t = setup();
    t.feed.restart();
    await vi.advanceTimersByTimeAsync(0);
    t.setNext(() => {
      throw new Error("gate closed");
    });
    await vi.advanceTimersByTimeAsync(10_000);
    expect(t.last().status).toBe("error");
    expect(t.last().message).toBe("gate closed");
    expect(t.last().items.size).toBe(2);
    expect(t.requests).toHaveLength(2);
    // Twice the wait after one failure.
    await vi.advanceTimersByTimeAsync(10_000);
    expect(t.requests).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(t.requests).toHaveLength(3);
    t.feed.dispose();
  });

  it("a restart drops the old chart and its answer in flight", async () => {
    const t = setup();
    const answers: ((p: FlowPage<Item, Meta>) => void)[] = [];
    const feed = createFlowFeed<Item, Meta>({
      pollMs: 10_000,
      doc: t.doc as never,
      onChange: (s) => t.states.push(s),
      fetch: () => new Promise((r) => answers.push(r)),
    });
    feed.restart();
    feed.restart();
    answers[1]!({ items: [{ t: 2, v: 2 }], meta: { row_size: 2 }, hasMore: false });
    await vi.advanceTimersByTimeAsync(0);
    answers[0]!({ items: [{ t: 1, v: 1 }], meta: { row_size: 1 }, hasMore: false });
    await vi.advanceTimersByTimeAsync(0);
    expect([...t.last().items.keys()]).toEqual([2]);
    expect(t.last().meta?.row_size).toBe(2);
    feed.dispose();
  });

  it("stop clears it and stops asking", async () => {
    const t = setup();
    t.feed.restart();
    await vi.advanceTimersByTimeAsync(0);
    t.feed.stop();
    expect(t.last().items.size).toBe(0);
    expect(t.last().meta).toBeNull();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(t.requests).toHaveLength(1);
  });

  it("dispose leaves no timer behind", async () => {
    const t = setup();
    t.feed.restart();
    await vi.advanceTimersByTimeAsync(0);
    t.feed.dispose();
    expect(vi.getTimerCount()).toBe(0);
  });
});
