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

  it("close() closes the shared client and allows reconnecting", async () => {
    const s = createMcpSession();
    await call(s);
    await s.close();
    expect(cl(0).close).toHaveBeenCalledTimes(1);
    await call(s);
    expect(h.clients).toHaveLength(2);
  });
});
