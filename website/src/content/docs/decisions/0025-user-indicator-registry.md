---
title: "0025 — Per-user indicator registry and MCP surface"
description: "User scripts are stored in SQLite (rusqlite, bundled) keyed by the OAuth client id, resolved before built-ins through indicators::by_name_for, managed with four feature-flagged MCP tools, and fail per indicator via indicator_errors."
sidebar:
  order: 25
---

**Status:** Accepted
**Date:** 2026-11-05
**Amended by:** [ADR-0036](/chartgen/decisions/0036-own-origin-invalid-client-redirect/)
(registered OAuth clients persist without `CHARTGEN_SCRIPTING=1`),
[ADR-0041](/chartgen/decisions/0041-script-editor-in-the-web-app/) (adds `get_my_indicator`)
**Implements:** [Proposal 0001](/chartgen/proposals/0001-python-indicators/) (Phase 1, registry half; the runtime is [ADR-0024](./0024-starlark-script-runtime/))

## Context and Problem Statement

With a script runtime in place (ADR-0024), user indicators need an owner,
storage that survives restarts, a way in through MCP, and a place in the
indicator dispatch, so that `generate_chart` and `list_indicators` treat
them like built-ins. Several choices here have long-term consequences:

- what "user" means on a server whose OAuth flow auto-approves;
- where the scripts are stored;
- how lookup combines user scripts with built-ins;
- how a broken script shows up in a chart response without breaking the
  wire contract;
- how the tools are gated while the feature is new.

## Decision Drivers

- No new account system: reuse the existing OAuth 2.1 flow
  ([oauth reference](/chartgen/reference/oauth/)).
- Scripts survive restarts and redeploys, so they need to be on disk.
- One dispatch seam ([proposal decision 12](/chartgen/proposals/0001-python-indicators/#decision-log)):
  the renderer and the serializers keep working with `dyn Indicator`.
- The fork model: a user's script may shadow a built-in for that user only.
- Backward-compatible responses ([ADR-0004](./0004-mcp-type-safety/)).
- The feature can be turned off and is off by default.

## Considered Options

**Identity.**
1. *The OAuth `client_id` of the bearer token (chosen).* Tokens are
   issued per client, and the token map already records the client.
2. The token itself. Tokens expire after 7 days and are lost on restart, so
   scripts would vanish.
3. A human account (email, IdP subject). chartgen has none, and its
   `/authorize` auto-approves.

**Storage.**
1. *SQLite through `rusqlite` with the `bundled` feature (chosen).*
2. A JSON file per user, like `alerts.json`. It needs atomic-replace code,
   offers no concurrent-writer safety, and needs a new file format per
   future table.
3. `sled` or `redb`. Embedded key-value stores without SQL; migrations
   and listing by user are hand-written.
4. In memory only. Scripts would be lost on restart.

**Dispatch.**
1. *`indicators::by_name_for(name, user)` and
   `by_name_configured_for(..., user)`: the user's script first, then the
   built-in (chosen).* `by_name(name)` keeps its signature and stays the
   built-ins-only lookup for the CLI, the trading engine, `gen_docs` and
   tests.
2. Change `by_name(name)` to `by_name(name, ctx)` everywhere, as the
   proposal sketched. 36 call sites change, and all of them except
   the MCP path would pass "no user".
3. A separate script namespace (`user:foo`). This breaks the fork model.

**Per-indicator failure.**
1. *Leave the failed indicator out of `indicators` and report it under a
   new optional top-level `indicator_errors` map (chosen).*
2. Put an `{error}` object inside `indicators`, as `get_indicators` does.
   Clients that iterate `indicators` expecting the series shape would
   break.
3. Fail the whole call.

## Decision Outcome

**User = OAuth client id.** `server.rs` resolves the bearer token to the
`client_id` it was issued to (`OAuthStore::client_for_token`) and passes it
to `handle_mcp_request` as `user`. The stdio transport and the CLI have no
user and see only built-ins. Registration is dynamic and auto-approved, so
a "user" is an MCP client registration, not a verified person: two MCP
clients of one person get two namespaces, and anyone who can register a
client gets a namespace. While scripting is enabled, registered OAuth
clients are also stored (`oauth_clients` table) and loaded at startup.
Otherwise every restart would drop the in-memory client list, clients
would re-register with new ids, and their scripts would be orphaned.
Tokens stay in memory, so after a restart a client re-authorizes with the
same `client_id`.

**Threat model: ownership is bound to a client registration, not a
person.** The `client_id` is not a secret: it is sent in the clear in
`/authorize` URLs and token requests, and public clients have no secret
(`token_endpoint_auth_method: none`). `/authorize` approves without asking
anyone. So anyone who knows a `client_id` and can receive a redirect to
one of its registered redirect URIs (for example a loopback URI such as
`http://localhost/cb`, which any local process can serve) can obtain a
token for that client and read, replace or delete its scripts. Scripts
are therefore only as private as client ids are, and that is not very.
**Proper user login (an authenticating `/authorize` with real user
accounts) is required before multi-user public use**; until then the
feature is meant for trusted or single-user deployments. To limit the
damage, the server no longer logs client ids, codes or verifiers, and the
database file is owner-only (0600 in a 0700 directory).

**Registration limits.** Because `/register` is open, it is bounded
whether or not scripting is on: 1–5 redirect URIs of at most 512
characters, a client name of at most 256, 20 registrations per peer
address per 10 minutes (loopback peers are exempt, so local tests and a
reverse proxy on the same host are not throttled; then only the total cap
applies), and at most 1000 clients. A client that never receives a token
within 24 hours is forgotten, in memory and, with scripting on, in the
database (`last_token_at`, migration 2).

**SQLite store.** `CHARTGEN_DB` sets the file, default
`~/.chartgen/chartgen.db`. Migrations run on first open, at server startup
when scripting is on, and are tracked in `PRAGMA user_version`. The
`user_indicators` table has primary key `(user_id, name)`. It stores the
source, a `meta_json` snapshot of `PARAMS` / `META`, a SHA-256
`source_hash`, and `created_at` / `updated_at`. Why `rusqlite` with
`bundled`: SQL gives migrations and per-user listing without hand-written
storage code, and the database file is a plain SQLite file that standard
tools can open. `bundled` compiles SQLite into the binary, so no system
library or musl/ARM cross-compile setup is needed (about 1 MB). The
connection is a `Mutex<Connection>`; script access is low-rate.

**Compile cache.** Frozen modules are cached in process, LRU with 1024
entries and at most 64 MiB in total (a module keeping over 1 MiB alive is
rejected at compile), keyed by `(user, name, source_hash)`.
Re-registering changes the hash, so a stale entry is never served.

**Concurrency.** At most 4 script evaluations (compile, dry run, compute)
run at once, and at most one per user; a caller waits up to its deadline
for a slot, then gets "server busy". A chart computes at most 3 scripted
indicators, all sharing one 1 s budget, so a request cannot multiply the
per-script limit by its panel count.

**MCP surface.** Four tools, listed in `tools/list` and dispatched only
when `CHARTGEN_SCRIPTING=1`:

| Tool | Input | Result |
|---|---|---|
| `register_indicator` | `name`, `source` | `{ok: true, name, title, overlay, configurable_params, replaced, shadows_builtin}`, or `isError` with `{ok: false, errors: [{step, line, message}]}` |
| `fork_indicator` | `builtin_name` | `{name, source, configurable_params}` from `src/scripting/refimpl/`; an error listing the forkable built-ins otherwise |
| `list_my_indicators` | — | `{total, indicators: [... list_indicators entry, title, created_at, updated_at]}` |
| `delete_indicator` | `name` | `{ok: true, name}`, or an error when there is no such script |

`update_indicator` from the proposal is `register_indicator` with an
existing name. Names follow the built-in shape, `^[a-z][a-z0-9_]{0,47}$`.
A user may store 50 scripts. `PARAMS` becomes the same schema the
built-ins declare, so `validate_params` checks chart requests against it
unchanged.

**Listing and resolution.** `list_indicators` appends the caller's scripts
with `source: "user"`; a script replaces the built-in entry it shadows.
`generate_chart` resolves names through `by_name_for`. Aliases of a
shadowed built-in still resolve to the built-in.

**Failure in a chart.** `Indicator` gains
`try_compute(&self, data) -> Result<PanelResult, ScriptError>`, which
defaults to `Ok(compute(data))`. `generate_chart` now computes every
indicator once with it, for all formats (the PNG renders from those
results through `renderer::render_results`). A failing indicator is
dropped from `indicators`, and the payload gets
`indicator_errors: {<request key>: {step: "runtime", line, message}}`
(summary, series and both). For `format=png` it is an extra text item. The
Zod schemas mark the field optional, and `list_indicators` gains an
optional `source`.

**Feature flag and codegen.** `mcp_schema::tools_list_result()` (the input
of `scripts/gen-mcp-types.sh`) never contains the scripting tools;
`mcp_schema::scripting_tools()` holds them and `tools/list` appends them
when the flag is on. So the generated TypeScript input types do not
depend on the environment that runs codegen, and the `mcp-types-drift`
job needs no flag. The web client gets typed inputs for these tools when
a frontend editor needs them (Phase 2).

### Consequences

- Good: no new identity or account system, and the per-user namespace
  works for Claude.ai and the web client alike.
- Good: one SQLite file, migrated in place, which also holds the OAuth
  clients, and later can hold other per-user state.
- Good: built-in callers and outputs are unchanged. The only wire
  additions are optional fields.
- Good: a broken script cannot take down a chart.
- Bad: the identity is a client registration, not a person, and is weak
  because `/authorize` auto-approves (see the threat model). Public
  multi-user use needs real user login, and sharing scripts across a
  person's clients needs a real account model.
- Bad: behind a reverse proxy that is not on the same host, every client
  shares the proxy's address, so the per-address registration limit is
  effectively global.
- Bad: `subscribe_chart` pushes (ADR-0021) compute built-ins only; a
  user script in a pushed chart is unknown there.
- Bad: with scripting on, `/register` writes to disk, and clients that
  never come back are never cleaned up.
- Bad: `get_indicators` and the trading engine's indicator state still
  resolve built-ins only.
- Bad: `generate_chart` now always computes the indicators before
  rendering a PNG, where the renderer used to do it. The total work is
  the same, but a PNG request holds all `PanelResult`s at once.

## Links

- [Proposal 0001](/chartgen/proposals/0001-python-indicators/)
- [ADR-0024: Starlark script runtime](./0024-starlark-script-runtime/)
- [MCP reference: scripted indicators](/chartgen/reference/mcp/#scripted-indicators-chartgen_scripting1)
- [Guide: scripted indicators](/chartgen/guides/scripted-indicators/)
