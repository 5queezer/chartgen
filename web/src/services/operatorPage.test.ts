import { describe, expect, it } from "vitest";

import { OperatorOverviewSchema } from "../types/responses";
import { fetchOverview } from "./operatorPage";

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
    expect(await fetchOverview()).toBeNull();
  });
});
