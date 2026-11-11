import { afterEach, describe, expect, it, vi } from "vitest";

import { OperatorOverviewSchema } from "../types/responses";
import { fetchOverview } from "./operatorPage";

const token = vi.hoisted(() => ({ value: null as string | null }));
vi.mock("../oauth", () => ({
  currentAccessToken: () => token.value,
  url: (path: string) => path,
}));

const overview = {
  server: {
    version: "0.1.0",
    started_at: 1_794_000_000,
    schema_version: 9,
    pending_deliveries: 0,
    store_bytes: null,
    setups_total: 3,
    features: { sign_in: true, replay: false },
    feeds: { watched_pairs: 1, watched_pairs_limit: 16, push_subscriptions: 0, push_feeds: 0 },
  },
  users: {
    total: 1,
    shown: [
      {
        id: "github:1000001",
        name: "Ann",
        provider: "github",
        created_at: 1,
        last_login_at: 2,
        last_token_at: null,
        workspace_docs: 0,
        workspace_bytes: 0,
        setups_open: 1,
        setups_closed: 2,
        server_alerts: 0,
        authorisations: 1,
        push_devices: 0,
      },
    ],
  },
  clients: {
    total: 1,
    limit: 1000,
    shown: [
      {
        id: "01234567",
        name: null,
        created_at: 1,
        last_token_at: null,
        has_authorisation: true,
        owns_setups: false,
      },
    ],
  },
};

describe("the operator overview", () => {
  it("is read as the server sends it", () => {
    expect(OperatorOverviewSchema.parse(overview)).toEqual(overview);
  });

  it("is refused when a count is not a number, rather than shown half", () => {
    const bad = structuredClone(overview) as { users: { total: unknown } };
    bad.users.total = "many";
    expect(OperatorOverviewSchema.safeParse(bad).success).toBe(false);
  });

  it("is not asked for without a token", async () => {
    const fetched = vi.fn();
    vi.stubGlobal("fetch", fetched);
    expect(await fetchOverview()).toEqual({ kind: "refused" });
    expect(fetched).not.toHaveBeenCalled();
  });
});

describe("what the server said about the overview", () => {
  afterEach(() => {
    token.value = null;
    vi.unstubAllGlobals();
  });

  const answering = (response: () => Response | Promise<Response>) => {
    token.value = "a-token";
    const fetched = vi.fn(response);
    vi.stubGlobal("fetch", fetched);
    return fetched;
  };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });

  it("is the overview, asked for as JSON with the token", async () => {
    const fetched = answering(() => json(200, overview));
    expect(await fetchOverview()).toEqual({ kind: "overview", overview });
    const [path, init] = fetched.mock.calls[0] as unknown as [string, RequestInit];
    expect(path).toBe("/admin/overview");
    expect(init.headers).toEqual({ Accept: "application/json", Authorization: "Bearer a-token" });
  });

  it("is a refusal when the server answers as for a path that does not exist", async () => {
    answering(() => new Response(null, { status: 404 }));
    expect(await fetchOverview()).toEqual({ kind: "refused" });
  });

  it("is a refusal when the answer is something else than the overview", async () => {
    answering(() => new Response("<!doctype html><title>app</title>", { status: 200 }));
    expect(await fetchOverview()).toEqual({ kind: "refused" });
    answering(() => json(200, { users: "many" }));
    expect(await fetchOverview()).toEqual({ kind: "refused" });
  });

  it("is not a refusal when the server fails or does not answer", async () => {
    for (const status of [500, 502, 503]) {
      answering(() => new Response(null, { status }));
      expect(await fetchOverview()).toEqual({ kind: "unavailable" });
    }
    answering(() => Promise.reject(new TypeError("Failed to fetch")));
    expect(await fetchOverview()).toEqual({ kind: "unavailable" });
  });
});
