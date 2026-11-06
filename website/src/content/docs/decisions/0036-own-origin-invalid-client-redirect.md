---
title: "0036 — Redirect an unknown OAuth client back to the server's own web app"
description: "GET /authorize answers an unknown client_id with a redirect only when redirect_uri is on the server's own origin and it serves the web app, so the app can re-register after a redeploy. Everyone else still gets the 400. Registered clients now persist without CHARTGEN_SCRIPTING."
sidebar:
  order: 36
---

**Status:** Accepted
**Date:** 2026-11-06
**Amends:** [ADR-0025](/chartgen/decisions/0025-user-indicator-registry/) (client storage no longer needs `CHARTGEN_SCRIPTING=1`)

## Context and Problem Statement

The web app keeps its `client_id` in `localStorage`. The server keeps clients
in memory unless `CHARTGEN_SCRIPTING=1` persists them
([ADR-0025](/chartgen/decisions/0025-user-indicator-registry/)), and a deployment
without a volume forgets them on every redeploy.

After a redeploy a returning browser therefore navigates to `/authorize` with a
stale id and lands on a raw JSON page,
`{"error":"invalid_client","error_description":"Unknown client_id"}`. It is a
top-level navigation, so the app's JavaScript never runs again and the user is
stuck until they clear `localStorage` by hand.

RFC 6749 section 4.1.2.1 says the authorization server must not automatically
redirect when the client is invalid: an unverified `redirect_uri` is what makes
an open redirector. Any recovery by redirect has to be a deliberate exception.

## Decision Drivers

- The app recovers with no user action, and share links survive it.
- `/authorize` must not become an open redirector.
- External MCP clients get no redirect, and keep their registration across a
  deploy.
- Script tools stay off unless `CHARTGEN_SCRIPTING=1`.
- Works whatever the persistence setup of the deployment is.

## Considered Options

**Redirect to the server's own origin only (chosen).** The server tells the app
through its own callback, under the conditions below.

**Persist clients only.** This removes the cause on a deployment with a volume,
and we do it as well (below). Alone it does nothing for a deployment without a
volume, and a browser can still hold an id the server has collected.

**Client-side probe.** `fetch` `/authorize` before navigating and re-register on
a 400. For a valid client this GET has a side effect: it issues an auth code
that is never used, on every sign-in.

**Server-rendered error page.** A page with a "Sign in again" link instead of
JSON. It needs the user to act and has to clear `localStorage` from a page that
is not the app, so it only makes the dead end look better.

## Decision Outcome

Chosen option: **redirect to the server's own origin only**, because the target
of the redirect is then fixed by server configuration and not by the requester,
which is what the RFC's rule protects against.

For an unknown `client_id`, `GET /authorize` answers `302` to `redirect_uri` with
`error=invalid_client` and the original `state` added to its query string when all
of these hold. In every other case it keeps the `400` JSON.

- The server serves the web app (`CHARTGEN_WEB_DIR` is set).
- `redirect_uri` parses to exactly the origin (scheme, host, port) of
  `base_url()`. Origins are compared after parsing, never as string prefixes, so
  `https://example.com.evil.com` and `https://example.com@evil.com` fail.
- `redirect_uri` has no userinfo and no fragment.

`base_url()` is `CHARTGEN_BASE_URL`, falling back to `http://localhost:<port>`.
It never comes from the `Host` header or a forwarding header, so a client cannot
influence the origin it is compared with. The `Location` is the re-serialized
parsed URL, so a browser cannot read it differently from the check.

The web app acts on `?error=invalid_client` only if `state` equals the pending
PKCE state in `sessionStorage`. It then clears the stored client and the PKCE
state, removes the parameters from the URL and restarts the flow, which registers
a new client. A crafted link does not know the state and is ignored, so it cannot
wipe the stored client.

This counts as a failed redirect in the app's existing one-retry guard. A second
failure in the tab stops at `sign-in failed: <reason>` with **Sign in again**,
which registers again because the client is already gone. Share parameters are
written to the stores before any redirect
([ADR-0029](/chartgen/decisions/0029-shareable-chart-links/)), so a share link
opened with a stale client still lands on its chart.

### Persisting clients without scripting

External MCP clients cannot use the redirect, so they also need the cause
removed. In a build with the `scripting` cargo feature, the default, the HTTP
server now stores and restores registered clients in `CHARTGEN_DB` whether or not
`CHARTGEN_SCRIPTING=1`. ADR-0025 tied this to scripting because the `client_id`
owns scripts. A build without the feature has no store and keeps clients in
memory.

The env var still alone decides the script tools, in `tools/list` and in
dispatch, because turning it on just to keep clients would expose them. A separate
env var for client storage was rejected: it adds a switch with no use for off,
since a store that cannot be opened is already logged and not fatal.

### Eviction at the client limit

Persisting clients would make the 1000 client limit permanent: clients that
received a token are never collected, and sign-in is auto-approved. A restart
used to empty the table; now a full table would leave `/register` at `503` for
good.

So when the table is full, `/register` evicts instead of refusing, from memory
and from the store:

1. Clients that never received a token, oldest registration first.
2. Then the least recently tokened client. The in-memory table tracks the last
   token time for this, restored from `last_token_at`.
3. Never a client that owns stored scripts, whether or not
   `CHARTGEN_SCRIPTING=1` is set now, because the `client_id` is the owner's only
   identity. If every client owns scripts, or the owners cannot be read,
   `/register` still answers `503` and evicts nobody.

Eviction also drops the client's live tokens and pending codes. A surviving token
would keep working for a client on no record, and could register scripts for an
owner that nothing protects. An open SSE stream is left alone, as it is when a
token expires, because streams are only checked when they connect.

An evicted client recovers as after a restart without a volume: the web app
through the redirect above, an MCP client by registering again. The registration
rate limit runs first, so a limited caller cannot evict anyone.

Refusing with `503` needs an operator to recover. Expiry by age alone would not
stop the table filling quickly, and would log out clients that are merely idle
even when there is room.

### Consequences

- Good: redeploys no longer strand browsers, with or without a volume.
- Good: with a volume, external MCP clients keep their `client_id` across a
  deploy. They re-authorize, because tokens stay in memory.
- Good: registration keeps working at the client limit, with no operator action.
- Bad: at the limit, anyone can push out idle clients by scripting the flow, at
  the pace of the rate limit. Those clients must sign in again, and an MCP client
  gets a new `client_id`. No scripts are lost, because their owners are spared.
- Bad: at the limit, a client that just registered is the first to go until it
  has a token, so a concurrent registration can evict it mid sign-in. It then
  recovers like any other evicted client. Registrations without a token can evict
  at most one tokened client this way, since each evicts the previous one.
- Bad: 1000 clients that own scripts still leave `/register` at `503`. Script tools
  are off by default, so this needs scripting turned on.
- Neutral: every default `--serve` now creates `~/.chartgen/chartgen.db`.
- Good: external MCP clients with their own redirect URIs, and API-only servers,
  still get the `400`.
- Bad: a deliberate deviation from RFC 6749 section 4.1.2.1. Anyone can make
  `/authorize` redirect to a path on the server's own origin, with these two
  parameters added. No other origin can be reached this way, and the app ignores
  a `state` it did not issue.
- Bad: each recovery registers a client. Unused ones are collected after 24 hours
  and registration is rate-limited as before.
- Neutral: the recovery needs `CHARTGEN_BASE_URL` to match the origin the app is
  served from. If it does not, sign-in is broken already, because the discovery
  document points elsewhere. The Vite dev server is such a case: its origin is
  not the server's, so a stale client there still gets the `400`.
