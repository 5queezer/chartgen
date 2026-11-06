---
title: Persistence
description: On-disk state written by the trading engine — alerts, audit log, subscriptions.
---

`--trade` and `--serve` write state under `~/.chartgen/` (Windows:
`%USERPROFILE%\.chartgen\`). The directory is created on startup if missing. Override by setting `$HOME`
before launch.

| File | Written by | Format |
|------|------------|--------|
| `alerts.json` | `set_alert`, `cancel_alert`, fired alerts | JSON array, atomic replace |
| `trades.log` | Order lifecycle + alert triggers | Append-only text |
| `subscriptions.json` | `subscribe_notifications`, `unsubscribe_notifications` | JSON map keyed by token |
| `chartgen.db` | `/register` and `/token` on the HTTP server; sign-in, when configured; scripted indicators, with `CHARTGEN_SCRIPTING=1` (any mode); server alerts, with `CHARTGEN_SERVER_ALERTS=1` | SQLite, path from `CHARTGEN_DB` |

## `alerts.json`

Array of alerts awaiting evaluation. Each entry:

```json
[
  {
    "id": "8f3a…",
    "symbol": "BTCUSDT",
    "condition": { "PriceAbove": 50000 },
    "created_at": "2026-04-19T12:00:00Z"
  },
  {
    "id": "b21c…",
    "symbol": "ETHUSDT",
    "condition": {
      "IndicatorSignal": { "indicator": "cipher_b", "signal": "gold_buy" }
    },
    "created_at": "2026-04-19T12:05:00Z"
  }
]
```

`condition` is a tagged enum with three variants:

- `{ "PriceAbove": <f64> }`
- `{ "PriceBelow": <f64> }`
- `{ "IndicatorSignal": { "indicator": "<name>", "signal": "<label>" } }`

Writes use a `.tmp` + rename sequence so readers never see a partial file.
The engine only rewrites `alerts.json` when the set changes (add, remove, or
fire) — idle bars cost zero writes.

:::note[On-disk vs wire format]
The condition keys above (`PriceAbove`, `PriceBelow`, `IndicatorSignal`) are
`PascalCase` because the `AlertCondition` enum in `src/trading/alert.rs`
uses serde's default externally-tagged encoding. The **MCP `set_alert` tool**
accepts `snake_case` (`price_above`, etc.) and translates it into the enum
at the request boundary in `src/mcp.rs`. If you are editing `alerts.json`
directly, use PascalCase; if you are calling `set_alert` over MCP, use
snake_case — see the [trading guide](/chartgen/guides/trading/#alerts).
:::

See the [trading guide](/chartgen/guides/trading/#signal-labels) for the
valid `signal` labels.

## `trades.log`

Append-only audit trail. One line per event, UTC timestamps:

```text
2026-04-19T12:34:56Z SUBMITTED buy BTCUSDT 0.01 market id=abc123
2026-04-19T12:34:57Z FILLED   buy BTCUSDT 0.01 @ 67432.5 id=abc123
2026-04-19T13:00:00Z CANCELLED id=def456
2026-04-19T13:15:22Z ALERT_TRIGGERED id=ghi789 cipher_b/gold_buy BTCUSDT 1h
```

Event types:

| Prefix | Emitted by | Fields |
|--------|------------|--------|
| `SUBMITTED` | `place_order` | side, symbol, quantity, type, `id=<order_id>` |
| `FILLED` | Exchange fill | side, symbol, quantity, `@ <price>`, `id=<order_id>` |
| `CANCELLED` | `cancel_order` | `id=<order_id>` |
| `ALERT_TRIGGERED` | Alert engine | `id=<alert_id>`, condition summary, symbol, interval |

There is no rotation — rotate or truncate externally if it grows too large.

## `subscriptions.json`

Persisted SSE push-notification filters, keyed by access token. Only
`symbols` and `alert_types` are stored; SSE senders and offline queues are
in-memory and rebuilt on reconnect.

```json
{
  "<token>": {
    "symbols": ["BTCUSDT", "ETHUSDT"],
    "alert_types": ["price_above", "indicator_signal"]
  }
}
```

Both fields may be `null`, meaning "match any". Restarting the server
preserves filters so clients keep their filters across chartgen restarts, but
queued alerts are lost.

## `chartgen.db` (OAuth clients, scripted indicators and server alerts)

Holds the registered OAuth clients, with or without `CHARTGEN_SCRIPTING=1`
([ADR-0036](/chartgen/decisions/0036-own-origin-invalid-client-redirect/)), and
the scripted indicators, which need it
([guide](/chartgen/guides/scripted-indicators/),
[ADR-0025](/chartgen/decisions/0025-user-indicator-registry/)). The path is
`CHARTGEN_DB`, default `~/.chartgen/chartgen.db`. The HTTP server opens it at
startup, creating the file and its directory, and applies pending migrations.
`PRAGMA user_version` counts the migrations applied. If it cannot be opened, the
server logs that and runs with clients in memory only. A migration runs in one transaction, so
one that fails leaves the file as it was: repair the cause and restart, and the clients and
everything else come back. A build without the default `scripting` cargo feature has no store
at all.

```sql
CREATE TABLE user_indicators (
    user_id      TEXT    NOT NULL,   -- owner: OAuth client_id, or with sign-in a users.id
    name         TEXT    NOT NULL,
    source       TEXT    NOT NULL,   -- Starlark source as registered
    meta_json    TEXT    NOT NULL,   -- {title, description, overlay, params} at register
    source_hash  TEXT    NOT NULL,   -- SHA-256 hex of source (compile-cache key)
    created_at   INTEGER NOT NULL,   -- unix seconds
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY (user_id, name)
);
CREATE TABLE oauth_clients (
    client_id      TEXT    PRIMARY KEY,
    redirect_uris  TEXT    NOT NULL, -- JSON array
    created_at     INTEGER NOT NULL,
    last_token_at  INTEGER,          -- migration 2; NULL = never got a token
    client_name    TEXT              -- migration 4; as registered, for the consent page
);
-- Migration 4, used with sign-in on (ADR-0044):
CREATE TABLE users (
    id             TEXT    PRIMARY KEY, -- github:<id> or google:<sub>
    name           TEXT    NOT NULL,    -- display only, refreshed at each login
    created_at     INTEGER NOT NULL,
    last_login_at  INTEGER NOT NULL,
    last_token_at  INTEGER              -- with the login, what keeps their alerts alive
);
CREATE TABLE sessions (
    token_hash  TEXT    PRIMARY KEY,    -- SHA-256 hex of the cookie value
    user_id     TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL
);
CREATE TABLE client_grants (            -- clients a user approved
    user_id     TEXT    NOT NULL,
    client_id   TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (user_id, client_id)
);
-- Migration 3: server alerts (ADR-0042).
CREATE TABLE server_alerts (
    id           TEXT PRIMARY KEY,
    owner        TEXT NOT NULL,      -- OAuth client_id, or with sign-in a users.id
    record_json  TEXT NOT NULL       -- definition and state, webhook URL and secret
);
CREATE TABLE server_alert_events (   -- deliveries not finished yet
    id          TEXT    PRIMARY KEY, -- the event id a receiver sees
    alert_id    TEXT    NOT NULL,
    body        TEXT    NOT NULL,    -- the JSON payload
    created_at  INTEGER NOT NULL     -- unix seconds
);
```

A fire writes the alert's new state and its delivery in one transaction, before
anything is sent, so a restart neither fires it again nor loses the delivery. A
delivery older than an hour is dropped. Spent alerts are deleted after 7 days, and those of an
owner with no token for 30 days too, unless the operator listed that owner. A user's sign-in counts as well. `record_json` holds webhook URLs,
which may contain tokens, and signing secrets in clear text, because the server needs
them to send and sign. They are never returned or logged, but treat a copy of this file
as holding them.

On Unix, a directory that does not exist yet is created owner-only (0700), and an
existing one keeps its mode. The file is created 0600, and an existing file is set to
0600 when the server opens it. These steps are best effort: a failure is not logged
and the server carries on, for example when another user owns the file or the
filesystem ignores modes. On Windows the server sets no permissions or ACLs, so the
directory and file inherit theirs. The file holds client ids, scripts and webhook secrets, so check who
can read it, especially on Windows or a mounted volume.

Clients with no token 24 hours after registering are deleted at startup and on each
registration, unless they own scripts.

`oauth_clients` keeps dynamically registered OAuth clients across restarts,
so a client keeps its `client_id`, and with it any scripts. Access tokens stay
in memory: after a restart, clients re-authorize. Clients that received a token
are only deleted when a registration at the
[1000 client limit](/chartgen/reference/oauth/#token-lifetime) evicts them, which
spares any client with a live token or a row in `user_indicators`, or, with
`CHARTGEN_SERVER_ALERTS=1`, in `server_alerts`. Back up the file like any
SQLite database (`sqlite3 chartgen.db ".backup copy.db"`).

`users`, `sessions` and `client_grants` are empty until
[sign-in](/chartgen/reference/oauth/#sign-in) is configured. Expired sessions, and
approvals of clients no longer registered, are deleted at startup. Provider tokens and
secrets are never stored, and neither are emails, except that a Google account without a
name is shown by its verified email. Deleting a user is by hand, with the server stopped:
their rows in these tables, `user_indicators` and `server_alerts`.
