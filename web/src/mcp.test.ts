import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
  clients: [] as Array<{
    connect: ReturnType<typeof vi.fn>;
    close: ReturnType<typeof vi.fn>;
    callTool: ReturnType<typeof vi.fn>;
  }>,
  connectImpl: undefined as undefined | (() => Promise<void>),
  callImpl: undefined as undefined | ((n: number) => Promise<unknown>),
  getAccessToken: vi.fn(),
  clearAccessToken: vi.fn(),
}));

function okResult() {
  return {
    content: [{ type: "text", text: JSON.stringify({ indicators: [] }) }],
  };
}

vi.mock("@modelcontextprotocol/sdk/client/index.js", () => ({
  Client: class {
    fallbackNotificationHandler: unknown;
    connect = vi.fn(() =>
      h.connectImpl ? h.connectImpl() : Promise.resolve(),
    );
    close = vi.fn(() => Promise.resolve());
    callTool = vi.fn(() => {
      const n = h.clients.indexOf(this as never);
      return h.callImpl ? h.callImpl(n) : Promise.resolve(okResult());
    });
    constructor() {
      h.clients.push(this as never);
    }
  },
}));
vi.mock("@modelcontextprotocol/sdk/client/streamableHttp.js", () => ({
  StreamableHTTPClientTransport: class {},
}));
vi.mock("./oauth", () => ({
  getAccessToken: h.getAccessToken,
  clearAccessToken: h.clearAccessToken,
}));

import { createMcpSession } from "./mcp";

const cl = (i: number) => {
  const c = h.clients[i];
  if (!c) throw new Error(`no client #${i}`);
  return c;
};

beforeEach(() => {
  h.clients.length = 0;
  h.connectImpl = undefined;
  h.callImpl = undefined;
  h.getAccessToken.mockReset().mockResolvedValue("tok");
  h.clearAccessToken.mockReset();
  vi.stubGlobal("window", { location: { origin: "http://localhost" } });
});

// Only the connection lifecycle matters here; swallow response-schema
// parse errors from the minimal mocked payload.
const call = (s: ReturnType<typeof createMcpSession>) =>
  s.callListIndicators().catch(() => undefined);

describe("createMcpSession connection lifecycle", () => {
  it("shares one connection across concurrent calls", async () => {
    const s = createMcpSession();
    await Promise.all([call(s), call(s), call(s)]);
    expect(h.clients).toHaveLength(1);
    expect(cl(0).connect).toHaveBeenCalledTimes(1);
    expect(h.getAccessToken).toHaveBeenCalledTimes(1);
  });

  it("retries after a failed connect", async () => {
    const s = createMcpSession();
    h.connectImpl = () => Promise.reject(new Error("boom"));
    await expect(s.callListIndicators()).rejects.toThrow("boom");
    expect(cl(0).close).toHaveBeenCalled();
    h.connectImpl = undefined;
    await call(s);
    expect(h.clients).toHaveLength(2);
    await call(s);
    expect(h.clients).toHaveLength(2);
  });

  it("closes the old client on 401 re-auth", async () => {
    const s = createMcpSession();
    h.callImpl = (n) =>
      n === 0
        ? Promise.reject(new Error("HTTP 401"))
        : Promise.resolve(okResult());
    await call(s);
    expect(h.clearAccessToken).toHaveBeenCalledTimes(1);
    expect(h.clients).toHaveLength(2);
    expect(cl(0).close).toHaveBeenCalledTimes(1);
    expect(cl(1).close).not.toHaveBeenCalled();
  });

  it("routes validated chart ticks and other notifications to their handlers", async () => {
    const s = createMcpSession();
    const ticks: Array<[string, unknown]> = [];
    const others: string[] = [];
    // Handlers installed before or after connecting both apply.
    s.onChartTick((id, series) => ticks.push([id, series.symbol]));
    await call(s);
    s.onNotification((n) => {
      others.push(n.method);
    });
    const deliver = (cl(0) as unknown as {
      fallbackNotificationHandler: (n: { method: string; params?: unknown }) => Promise<void>;
    }).fallbackNotificationHandler;
    const series = {
      symbol: "BTCUSDT",
      interval: "1m",
      bars: [{ t: 60, o: 1, h: 1, l: 1, c: 1, v: 1 }],
      indicators: {},
      has_more: true,
    };
    await deliver({
      method: "notifications/chart_tick",
      params: { subscription_id: "cs-1", series },
    });
    // Malformed: dropped at the boundary.
    await deliver({ method: "notifications/chart_tick", params: { series } });
    await deliver({ method: "notifications/alert_triggered", params: {} });
    expect(ticks).toEqual([["cs-1", "BTCUSDT"]]);
    expect(others).toEqual(["notifications/alert_triggered"]);
  });

  it("close() closes the shared client and allows reconnecting", async () => {
    const s = createMcpSession();
    await call(s);
    await s.close();
    expect(cl(0).close).toHaveBeenCalledTimes(1);
    await call(s);
    expect(h.clients).toHaveLength(2);
  });
});

describe("tool errors and auth", () => {
  const refused = (text: string) => () =>
    Promise.resolve({ content: [{ type: "text", text }], isError: true });

  it("a tool error mentioning 401 does not sign the user out", async () => {
    const body = { ok: false, errors: [{ step: "dry_run", line: 401, message: "boom 401" }] };
    h.callImpl = refused(JSON.stringify(body));
    const r = await createMcpSession().callRegisterIndicator({ name: "a", source: "x" });
    expect(r).toEqual(body);
    expect(h.clearAccessToken).not.toHaveBeenCalled();
    expect(h.clients).toHaveLength(1);
  });

  it("a plain refusal comes back as an error without a line", async () => {
    h.callImpl = refused("name must be 1-48 lowercase letters");
    const r = await createMcpSession().callRegisterIndicator({ name: "A", source: "x" });
    expect(r).toEqual({
      ok: false,
      errors: [{ step: "register", line: null, message: "name must be 1-48 lowercase letters" }],
    });
  });

  it("a 401 from the transport still re-authenticates", async () => {
    h.callImpl = (n) =>
      n === 0
        ? Promise.reject(new Error("HTTP 401"))
        : Promise.resolve({
            content: [{ type: "text", text: JSON.stringify({ total: 0, indicators: [] }) }],
          });
    const r = await createMcpSession().callListMyIndicators();
    expect(r.total).toBe(0);
    expect(h.clearAccessToken).toHaveBeenCalledTimes(1);
    expect(h.clients).toHaveLength(2);
  });
});
