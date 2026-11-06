// `public/sw.js` is served as it is, so this runs that file against a stand-in for its
// global scope. Playwright covers it in a real browser (alerts-push.spec.ts).

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { beforeEach, describe, expect, it, vi } from "vitest";

type Handler = (event: Record<string, unknown>) => void;

function loadWorker(windows: unknown[] = []) {
  const handlers: Record<string, Handler> = {};
  const self = {
    addEventListener: (name: string, fn: Handler) => {
      handlers[name] = fn;
    },
    registration: { showNotification: vi.fn(async () => {}) },
    clients: {
      matchAll: vi.fn(async () => windows),
      openWindow: vi.fn(async () => null),
    },
  };
  const source = readFileSync(resolve(__dirname, "../../public/sw.js"), "utf8");
  new Function("self", source)(self);
  return { self, handlers };
}

const FIRE = {
  id: "e1",
  alert_id: "a1",
  symbol: "BTCUSDT",
  condition: "crossing_up",
  level: 70000,
  price: 70012.5,
  message: "take profit",
  fire_count: 1,
};

async function push(handlers: Record<string, Handler>, data: unknown): Promise<void> {
  let waited: Promise<unknown> = Promise.resolve();
  handlers.push?.({ data, waitUntil: (p: Promise<unknown>) => (waited = p) });
  await waited;
}

describe("the service worker", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("does not intercept requests", () => {
    expect(loadWorker().handlers.fetch).toBeUndefined();
  });

  it("shows symbol, condition and price, tagged by the event", async () => {
    const { self, handlers } = loadWorker();
    await push(handlers, { json: () => FIRE });
    expect(self.registration.showNotification).toHaveBeenCalledTimes(1);
    expect(self.registration.showNotification).toHaveBeenCalledWith(
      "BTCUSDT crossing up 70000",
      expect.objectContaining({
        body: "Price 70012.5 · take profit",
        tag: "alert-fired-e1",
        data: { symbol: "BTCUSDT" },
      }),
    );
  });

  it("still shows something for a push it cannot read", async () => {
    const { self, handlers } = loadWorker();
    await push(handlers, null);
    await push(handlers, {
      json: () => {
        throw new Error("not JSON");
      },
    });
    expect(self.registration.showNotification).toHaveBeenCalledTimes(2);
    expect(self.registration.showNotification).toHaveBeenCalledWith(
      "chartgen alert",
      expect.anything(),
    );
  });

  async function click(windows: unknown[]) {
    const worker = loadWorker(windows);
    const close = vi.fn();
    let waited: Promise<unknown> = Promise.resolve();
    worker.handlers.notificationclick?.({
      notification: { close, data: { symbol: "ETH/USDT" } },
      waitUntil: (p: Promise<unknown>) => (waited = p),
    });
    await waited;
    expect(close).toHaveBeenCalled();
    return worker;
  }

  it("opens the chart on that symbol when no window is open", async () => {
    const { self } = await click([]);
    expect(self.clients.openWindow).toHaveBeenCalledWith("/?symbol=ETH%2FUSDT");
  });

  it("uses a window that is open, and tells it the symbol", async () => {
    const open = { focus: vi.fn(async () => {}), postMessage: vi.fn() };
    const { self } = await click([open]);
    expect(open.postMessage).toHaveBeenCalledWith({
      type: "chartgen.open-symbol",
      symbol: "ETH/USDT",
    });
    expect(open.focus).toHaveBeenCalled();
    expect(self.clients.openWindow).not.toHaveBeenCalled();
  });
});
