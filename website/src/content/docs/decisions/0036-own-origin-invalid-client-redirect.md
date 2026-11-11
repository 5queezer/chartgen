---
title: "0036 — Redirect an unknown OAuth client back to the server's own web app"
description: "GET /authorize answers an unknown client_id with a redirect only when redirect_uri is on the server's own origin and it serves the web app, so the app can re-register after a redeploy. Everyone else still gets the 400. Registered clients now persist without CHARTGEN_SCRIPTING."
sidebar:
  order: 36
---

**Status:** Accepted
**Date:** 2026-11-06
**Amends:** [ADR-0025](/chartgen/decisions/0025-user-indicator-registry/) (client storage no longer needs `CHARTGEN_SCRIPTING=1`)
**Amended by:** [ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/) (with sign-in on, a client's liveness is limited per user, so the lockout costs accounts), [ADR-0058](/chartgen/decisions/0058-refresh-tokens-for-headless-clients/) (external clients refresh their token after a restart, and a refresh token keeps its client from being evicted)

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
- The new redirect must add no open redirect. `/authorize` already follows any
  `redirect_uri` a registered client names, and registration is open and
  auto-approved, so this path must not reach anywhere that does not.
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
PKCE state in `sessionStorage`. It then clears the PKCE state, removes the
parameters from the URL and restarts the flow. A crafted link does not know the
state, so it cannot wipe the stored client: the app removes the parameters and
stops at **Sign in again**. It does not restart, because where `sessionStorage`
does not survive a navigation the state can never match and a restart would loop.

Tabs of one browser share `localStorage` but not `sessionStorage`, so the PKCE state
records the `client_id` that asked. The stored client is cleared only if it still
equals the rejected id, so a tab bounced late cannot remove the client another tab
has just registered. The code exchange uses the recorded id, because the stored one
may have changed since. Entries saved before this have no id and use the stored
client as before.

Registering and removing the stored client both take a Web Lock, re-checking the
stored client inside it, so tabs that find it empty end up sharing one client instead
of each registering. Without Web Locks, on an insecure origin or an old browser, tabs
may each register. The last one stored wins and each tab keeps working as its own
client, though with scripting on a script saved in a losing tab is not listed after a
reload.

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
received a token are never collected, and sign-in is auto-approved. A restart used
to empty the table; now a full table would leave `/register` at `503` for good.

So when the table is full, `/register` evicts where it can, from memory and from
the store:

1. A client with a live token is never evicted. A stranger must not be able to end
   a session in use.
2. A client that owns stored scripts is never evicted, whether or not
   `CHARTGEN_SCRIPTING=1` is set now, because the `client_id` is the owner's only
   identity.
3. Of the rest, clients that never received a token go first, oldest registration
   first, then the client longest out of use. Last use is an in-memory field, set
   when a token is issued and on each authenticated call, with no store write per
   request. After a restart it starts from `last_token_at`.
4. If nothing is left, or the owners cannot be read, `/register` answers `503` and
   evicts nobody.

The owners are read from the store before taking the OAuth lock, because every
authenticated call needs that lock and the store may be busy. A client's first script
can land after the read, but writing one takes a live token, so that client is not
evictable anyway unless the token expires in that moment.

Eviction drops the client's pending codes, so a code cannot mint a token for a
client on no record. An evicted client recovers as after a restart without a volume:
the web app through the redirect above, an MCP client by registering again.

Rejected:

- **Evict by oldest token and revoke it.** The first version did this. A client
  working daily on a 6-day-old token was first in line, and a stranger repeating
  register, authorize and token emptied the table of everyone else's sessions: 1000
  rounds took 1.7 s on a loopback peer.
- **Always refuse with `503`.** Then every full table needs an operator, even one
  filled by ordinary use with idle clients. With eviction, only a deliberate lockout
  does.
- **Expiry by age alone.** It would not stop the table filling quickly, and would
  drop idle clients even when there is room.

### Consequences

- Good: redeploys no longer strand browsers, with or without a volume.
- Good: with a volume, external MCP clients keep their `client_id` across a
  deploy. They re-authorize, because tokens stay in memory.
- Good: nobody can lose a session in use to a stranger. A client with a live token
  sees no change, however full the table gets.
- Good: registration keeps working at the limit while any client is evictable.
- Bad: anyone scripting the flow can push out every client with no live token. That
  means clients idle past the 7-day token lifetime, and right after a restart every
  client that has not signed in again, because tokens are memory-only. The rate limit
  does not bound this in general: loopback peers are exempt, so a same-host reverse
  proxy has no limit, and the limit is per address, so many addresses get around it.
  Those clients must sign in again, and an MCP client gets a new `client_id`. No
  scripts are lost, because their owners are spared.
- Bad: this does not stop a deliberate lockout. Someone holding 1000 live tokens gets
  `503` for all new registrations, and it lasts until an operator acts. Tokens expire
  after 7 days and a restart drops them, but the clients persist, and `/authorize` and
  `/token` issue new tokens with no `/register` and no rate limit: re-tokening 999
  clients after a restart took 1.6 s on a loopback peer. Clients already registered
  keep working. New browsers, browsers recovering from a stale client and new MCP
  clients cannot sign in. Refusals are logged at most once a minute, and the
  [recovery](/chartgen/reference/oauth/#when-register-stays-at-503) deletes every client
  that owns no scripts. While registration is open and auto-approved, the server
  cannot tell a stranger's client from a user's. Requiring approval or authentication
  at registration is the real alternative, and a separate project.
- Bad: at the limit, a client that just registered is the first to go until it has a
  token, so a concurrent registration can evict it mid sign-in. It then recovers like
  any other evicted client. Registrations without a token can evict at most one other
  client this way, since each evicts the previous one.
- Bad: 1000 clients that own scripts still leave `/register` at `503`. Script tools
  are off by default, so this needs scripting turned on.
- Neutral: every default `--serve` now creates `~/.chartgen/chartgen.db`.
- Good: external MCP clients with their own redirect URIs, and API-only servers,
  still get the `400`.
- Bad: a deliberate deviation from RFC 6749 section 4.1.2.1. Anyone can make
  `/authorize` redirect to a path on the server's own origin, with these two
  parameters added. No other origin can be reached this way, and the app does
  nothing with a `state` it did not issue beyond stopping at **Sign in again**. A
  crafted link can therefore show that screen to someone who follows it.
- Bad: each recovery registers a client. Unused ones are collected after 24 hours
  and registration is rate-limited as before.
- Neutral: the recovery needs `CHARTGEN_BASE_URL` to match the origin the app is
  served from. If it does not, sign-in is broken already, because the discovery
  document points elsewhere. The Vite dev server is such a case: its origin is
  not the server's, so a stale client there still gets the `400`.
