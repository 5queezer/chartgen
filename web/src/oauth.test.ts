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
    if (input === DISCOVERY.registration_endpoint) {
      return new Response(JSON.stringify({ client_id: "new" }), { status: 201 });
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

describe("the client of a code", () => {
  const exchangedClient = () => {
    const call = fetchMock.mock.calls.find(([u]) => u === DISCOVERY.token_endpoint)!;
    return new URLSearchParams(String(call[1].body)).get("client_id");
  };

  beforeEach(() => {
    vi.resetModules();
    tokenStatus = 200;
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("is the one that asked, even if another tab replaced the stored one", async () => {
    boot("http://app.test/?code=c1&state=s1");
    sessionStorage.setItem(
      "chartgen.oauth.pkce",
      JSON.stringify({ code_verifier: "v", state: "s1", client_id: "asked" }),
    );
    localStorage.setItem("chartgen.oauth.client_id", "from-other-tab");
    const { tryHandlePendingRedirect } = await import("./oauth");

    expect(await tryHandlePendingRedirect()).toBeNull();

    expect(exchangedClient()).toBe("asked");
    expect(localStorage.getItem("chartgen.oauth.client_id")).toBe("from-other-tab");
  });

  it("is the stored one for a pending entry saved without client_id", async () => {
    boot("http://app.test/?code=c1&state=s1");
    const { tryHandlePendingRedirect } = await import("./oauth");

    expect(await tryHandlePendingRedirect()).toBeNull();

    expect(exchangedClient()).toBe("client");
  });

  it("is saved with the pending entry when a sign-in starts", async () => {
    boot("http://app.test/");
    sessionStorage.removeItem("chartgen.oauth.pkce");
    const { startSignIn } = await import("./oauth");

    void startSignIn();
    await vi.waitFor(() => expect(assign).toHaveBeenCalledOnce());

    expect(
      JSON.parse(sessionStorage.getItem("chartgen.oauth.pkce")!) as { client_id: string },
    ).toMatchObject({ client_id: "client" });
  });
});

describe("stale client (?error=invalid_client)", () => {
  const authorizedClient = () =>
    new URL(String(assign.mock.calls[0]![0])).searchParams.get("client_id");
  const pendingState = () =>
    (JSON.parse(sessionStorage.getItem("chartgen.oauth.pkce")!) as { state: string })
      .state;

  beforeEach(() => {
    vi.resetModules();
    vi.spyOn(console, "error").mockImplementation(() => {});
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("with the pending state, forgets the client and restarts with a new one", async () => {
    boot("http://app.test/?error=invalid_client&state=s1#pane");
    localStorage.setItem("chartgen.oauth.client_secret", "old-secret");
    const { tryHandlePendingRedirect } = await import("./oauth");

    void tryHandlePendingRedirect();
    await vi.waitFor(() => expect(assign).toHaveBeenCalledOnce());

    expect(href).toBe("http://app.test/#pane");
    expect(authorizedClient()).toBe("new");
    expect(localStorage.getItem("chartgen.oauth.client_id")).toBe("new");
    expect(localStorage.getItem("chartgen.oauth.client_secret")).toBe("");
    expect(pendingState()).not.toBe("s1");
    expect(sessionStorage.getItem(RETRIED)).toBe("1");
  });

  it.each([
    ["a forged state", "http://app.test/?error=invalid_client&state=forged"],
    ["no state", "http://app.test/?error=invalid_client"],
  ])("with %s, stops without restarting or touching the client", async (_name, url) => {
    boot(url);
    const { tryHandlePendingRedirect } = await import("./oauth");

    const err = await tryHandlePendingRedirect();

    expect(err?.message).toBe("this address is not a sign-in in progress");
    expect(href).toBe("http://app.test/");
    expect(localStorage.getItem("chartgen.oauth.client_id")).toBe("client");
    expect(pendingState()).toBe("s1");
    expect(sessionStorage.getItem(RETRIED)).toBeNull();
    expect(assign).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("with another error, changes nothing", async () => {
    boot("http://app.test/?error=access_denied&state=s1");
    const { tryHandlePendingRedirect } = await import("./oauth");

    expect(await tryHandlePendingRedirect()).toBeNull();

    expect(localStorage.getItem("chartgen.oauth.client_id")).toBe("client");
    expect(pendingState()).toBe("s1");
    expect(sessionStorage.getItem(RETRIED)).toBeNull();
    expect(assign).not.toHaveBeenCalled();
    expect(replaceState).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("without any pending sign-in, stops and keeps the client", async () => {
    boot("http://app.test/?error=invalid_client&state=s1");
    sessionStorage.removeItem("chartgen.oauth.pkce");
    const { tryHandlePendingRedirect, startSignIn } = await import("./oauth");

    expect((await tryHandlePendingRedirect())?.message).toBe(
      "this address is not a sign-in in progress",
    );
    expect(localStorage.getItem("chartgen.oauth.client_id")).toBe("client");
    expect(assign).not.toHaveBeenCalled();

    void startSignIn();
    await vi.waitFor(() => expect(assign).toHaveBeenCalledOnce());
    expect(authorizedClient()).toBe("client");
  });

  it("with sessionStorage throwing, resolves with an error instead of hanging", async () => {
    boot("http://app.test/?error=invalid_client&state=s1");
    const blocked = () => {
      throw new Error("storage blocked");
    };
    vi.stubGlobal("sessionStorage", {
      getItem: blocked,
      setItem: blocked,
      removeItem: blocked,
    });
    const { tryHandlePendingRedirect } = await import("./oauth");

    expect((await tryHandlePendingRedirect())?.message).toBe("storage blocked");
    expect(assign).not.toHaveBeenCalled();
    expect(localStorage.getItem("chartgen.oauth.client_id")).toBe("client");
  });

  it("keeps a client another tab has already put in place of the rejected one", async () => {
    boot("http://app.test/?error=invalid_client&state=s1");
    sessionStorage.setItem(
      "chartgen.oauth.pkce",
      JSON.stringify({ code_verifier: "v", state: "s1", client_id: "rejected" }),
    );
    localStorage.setItem("chartgen.oauth.client_id", "from-other-tab");
    const { tryHandlePendingRedirect } = await import("./oauth");

    void tryHandlePendingRedirect();
    await vi.waitFor(() => expect(assign).toHaveBeenCalledOnce());

    expect(authorizedClient()).toBe("from-other-tab");
    expect(localStorage.getItem("chartgen.oauth.client_id")).toBe("from-other-tab");
    expect(fetchMock).not.toHaveBeenCalledWith(
      DISCOVERY.registration_endpoint,
      expect.anything(),
    );
  });

  it("forgets the client when the rejected id is the stored one", async () => {
    boot("http://app.test/?error=invalid_client&state=s1");
    sessionStorage.setItem(
      "chartgen.oauth.pkce",
      JSON.stringify({ code_verifier: "v", state: "s1", client_id: "client" }),
    );
    const { tryHandlePendingRedirect } = await import("./oauth");

    void tryHandlePendingRedirect();
    await vi.waitFor(() => expect(assign).toHaveBeenCalledOnce());

    expect(authorizedClient()).toBe("new");
  });

  it("a second failure stops with the error, and Sign in again registers", async () => {
    boot("http://app.test/?error=invalid_client&state=s1");
    sessionStorage.setItem(RETRIED, "1");
    const { tryHandlePendingRedirect, startSignIn } = await import("./oauth");

    const err = await tryHandlePendingRedirect();

    expect(err?.message).toBe("the server no longer knows this client");
    expect(assign).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
    expect(href).toBe("http://app.test/");
    expect(localStorage.getItem("chartgen.oauth.client_id")).toBeNull();

    void startSignIn();
    await vi.waitFor(() => expect(assign).toHaveBeenCalledOnce());
    expect(authorizedClient()).toBe("new");
  });
});

describe("registration rate limit", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("backs off with Retry-After, else 2s, 4s, 8s", async () => {
    const { retryDelayMs } = await import("./oauth");
    expect(retryDelayMs(0, "7")).toBe(7000);
    expect(retryDelayMs(0, "9999")).toBe(60000);
    expect(retryDelayMs(0, null)).toBe(2000);
    expect(retryDelayMs(1, "soon")).toBe(4000);
    expect(retryDelayMs(2, null)).toBe(8000);
  });

  it("parses an HTTP-date Retry-After and clamps to [0, 60 s]", async () => {
    const { retryDelayMs } = await import("./oauth");
    const at = (ms: number) => new Date(Date.now() + ms).toUTCString();
    expect(retryDelayMs(0, at(20_000))).toBeGreaterThan(18_000);
    expect(retryDelayMs(0, at(20_000))).toBeLessThanOrEqual(20_000);
    expect(retryDelayMs(0, at(600_000))).toBe(60000);
    expect(retryDelayMs(0, at(-5_000))).toBe(0);
    expect(retryDelayMs(0, "-3")).toBe(0);
  });

  it("clears the notice when a retry fetch rejects", async () => {
    boot("http://app.test/");
    localStorage.removeItem("chartgen.oauth.client_id");
    localStorage.removeItem("chartgen.oauth.client_secret");
    let registrations = 0;
    const base = fetchMock.getMockImplementation()!;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: string, init?: RequestInit) => {
        if (input !== DISCOVERY.registration_endpoint) return base(input, init);
        if (++registrations > 1) throw new TypeError("network down");
        return new Response("{}", { status: 429, headers: { "Retry-After": "2" } });
      }),
    );
    const { startSignIn, subscribeSignInNotice } = await import("./oauth");
    const notices: (string | null)[] = [];
    subscribeSignInNotice((n) => notices.push(n));
    const result = startSignIn().catch((e: Error) => e.message);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(await result).toMatch(/network down/);
    expect(notices.some((n) => n?.includes("retrying"))).toBe(true);
    expect(notices.at(-1)).toBeNull();
  });

  it("retries a 429 registration, shows a countdown and stores the client", async () => {
    boot("http://app.test/");
    localStorage.removeItem("chartgen.oauth.client_id");
    localStorage.removeItem("chartgen.oauth.client_secret");
    let registrations = 0;
    const base = fetchMock.getMockImplementation()!;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: string, init?: RequestInit) => {
        if (input !== DISCOVERY.registration_endpoint) return base(input, init);
        registrations++;
        return registrations < 3
          ? new Response("{}", { status: 429, headers: { "Retry-After": "3" } })
          : new Response(JSON.stringify({ client_id: "new" }), { status: 201 });
      }),
    );
    const { startSignIn, subscribeSignInNotice } = await import("./oauth");
    const notices: (string | null)[] = [];
    subscribeSignInNotice((n) => notices.push(n));

    void startSignIn();
    await vi.advanceTimersByTimeAsync(10_000);

    expect(registrations).toBe(3);
    expect(notices.some((n) => n?.includes("retrying in 3 s"))).toBe(true);
    expect(notices.at(-1)).toBeNull();
    expect(localStorage.getItem("chartgen.oauth.client_id")).toBe("new");
    await vi.waitFor(() => expect(assign).toHaveBeenCalled());
  });

  it("gives up with a clear message after the retries", async () => {
    boot("http://app.test/");
    localStorage.removeItem("chartgen.oauth.client_id");
    localStorage.removeItem("chartgen.oauth.client_secret");
    const base = fetchMock.getMockImplementation()!;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: string, init?: RequestInit) =>
        input === DISCOVERY.registration_endpoint
          ? new Response("{}", { status: 429, headers: { "Retry-After": "1" } })
          : base(input, init),
      ),
    );
    const { startSignIn } = await import("./oauth");
    const result = startSignIn().catch((e: Error) => e.message);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(await result).toMatch(/Too many sign-ins from this network/);
  });
});
