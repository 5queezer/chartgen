import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

class MemoryStorage {
  private m = new Map<string, string>();
  getItem(k: string): string | null {
    return this.m.get(k) ?? null;
  }
  setItem(k: string, v: string): void {
    this.m.set(k, v);
  }
  removeItem(k: string): void {
    this.m.delete(k);
  }
}

const RETRIED = "chartgen.oauth.retried";
const DISCOVERY = {
  authorization_endpoint: "http://auth.test/authorize",
  token_endpoint: "http://auth.test/token",
  registration_endpoint: "http://auth.test/register",
};

let href: string;
let assign: ReturnType<typeof vi.fn>;
let replaceState: ReturnType<typeof vi.fn>;
let fetchMock: ReturnType<typeof vi.fn>;
let tokenStatus: number;

/** Browser globals for a page at `url` that was redirected back with a code. */
function boot(url: string): void {
  href = url;
  assign = vi.fn();
  replaceState = vi.fn((_s: unknown, _t: string, next: string) => {
    href = next;
  });
  const location = {
    get origin() {
      return new URL(href).origin;
    },
    get pathname() {
      return new URL(href).pathname;
    },
    get search() {
      return new URL(href).search;
    },
    get hash() {
      return new URL(href).hash;
    },
    assign,
  };
  vi.stubGlobal("window", { location, history: { replaceState } });
  const session = new MemoryStorage();
  session.setItem(
    "chartgen.oauth.pkce",
    JSON.stringify({ code_verifier: "verifier", state: "s1" }),
  );
  vi.stubGlobal("sessionStorage", session);
  const local = new MemoryStorage();
  local.setItem("chartgen.oauth.client_id", "client");
  local.setItem("chartgen.oauth.client_secret", "");
  vi.stubGlobal("localStorage", local);

  fetchMock = vi.fn(async (input: string) => {
    if (input.endsWith("/.well-known/oauth-authorization-server")) {
      return new Response(JSON.stringify(DISCOVERY), { status: 200 });
    }
    if (input === DISCOVERY.token_endpoint) {
      return tokenStatus === 200
        ? new Response(JSON.stringify({ access_token: "tok" }), { status: 200 })
        : new Response("{}", { status: tokenStatus });
    }
    throw new Error(`unexpected fetch ${input}`);
  });
  vi.stubGlobal("fetch", fetchMock);
}

const tokenCallOrder = () => {
  const i = fetchMock.mock.calls.findIndex(
    ([u]) => u === DISCOVERY.token_endpoint,
  );
  return fetchMock.mock.invocationCallOrder[i]!;
};

describe("tryHandlePendingRedirect", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.spyOn(console, "error").mockImplementation(() => {});
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("scrubs code/state before the exchange and retries a failure once", async () => {
    boot("http://app.test/?code=c1&state=s1#pane");
    tokenStatus = 400;
    const { tryHandlePendingRedirect } = await import("./oauth");

    void tryHandlePendingRedirect();
    await vi.waitFor(() => expect(assign).toHaveBeenCalledOnce());

    expect(href).toBe("http://app.test/#pane");
    expect(replaceState.mock.invocationCallOrder[0]).toBeLessThan(
      tokenCallOrder(),
    );
    expect(sessionStorage.getItem(RETRIED)).toBe("1");
    expect(String(assign.mock.calls[0]![0])).toMatch(
      /^http:\/\/auth\.test\/authorize\?/,
    );
  });

  it("after the retry, a second failure resolves with the error and does not redirect", async () => {
    boot("http://app.test/?code=c2&state=s1");
    tokenStatus = 400;
    sessionStorage.setItem(RETRIED, "1");
    const { tryHandlePendingRedirect } = await import("./oauth");

    const err = await tryHandlePendingRedirect();

    expect(err?.message).toBe("token exchange failed: 400");
    expect(assign).not.toHaveBeenCalled();
    expect(href).toBe("http://app.test/");
  });

  it("a state mismatch is scrubbed and reported too", async () => {
    boot("http://app.test/?code=c3&state=forged");
    sessionStorage.setItem(RETRIED, "1");
    const { tryHandlePendingRedirect } = await import("./oauth");

    const err = await tryHandlePendingRedirect();

    expect(err?.message).toBe("oauth: state mismatch");
    expect(href).toBe("http://app.test/");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("a successful exchange clears the retry marker and caches the token", async () => {
    boot("http://app.test/?code=c4&state=s1");
    tokenStatus = 200;
    sessionStorage.setItem(RETRIED, "1");
    const { tryHandlePendingRedirect, getAccessToken } = await import("./oauth");

    expect(await tryHandlePendingRedirect()).toBeNull();
    expect(sessionStorage.getItem(RETRIED)).toBeNull();
    expect(href).toBe("http://app.test/");
    expect(await getAccessToken()).toBe("tok");
  });

  it("does nothing without a code in the URL", async () => {
    boot("http://app.test/");
    const { tryHandlePendingRedirect } = await import("./oauth");

    expect(await tryHandlePendingRedirect()).toBeNull();
    expect(replaceState).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
