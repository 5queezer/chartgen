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

### Consequences

- Good: redeploys no longer strand browsers, with or without a volume.
- Good: with a volume, external MCP clients keep their `client_id` across a
  deploy. They re-authorize, because tokens stay in memory.
- Bad: clients that received a token are never collected, and a restart no longer
  clears them. Sign-in is auto-approved, so 1000 such clients, from real use or
  from someone scripting the flow, leave `/register` at `503` until an operator
  deletes rows. Clients already registered keep working. Before this, a deploy
  emptied the table.
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
