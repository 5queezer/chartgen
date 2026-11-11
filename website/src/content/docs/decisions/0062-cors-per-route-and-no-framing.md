---
title: "0062 — Other origins get an answer on the MCP and OAuth client routes only, and the app is not framed"
description: "The permissive CORS answer stays on the routes an MCP client in a browser uses and goes from every other route and from unknown paths. The web app's pages forbid framing, and every response forbids content sniffing."
sidebar:
  order: 62
---

**Status:** Accepted
**Date:** 2026-11-11
**Amends:** [ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/) (the headers of the sign-in and consent pages are now a tested contract, and the routes that read the session cookie no longer answer another origin) and [ADR-0027](/chartgen/decisions/0027-serve-web-app-same-origin/) (the web app's pages forbid framing)

## Context and Problem Statement

Since its first HTTP transport the server answered every request, on every path, with
`Access-Control-Allow-Origin: *` and allowed every method and header. An MCP client
that runs in a browser on another origin needs that: it registers, exchanges a code
and calls `/mcp` from there.

The server has since grown routes no other origin has a reason to call: pages that
read the session cookie (sign-in, consent, sign-out), `/oauth/me` and
`/workspace/sync`, which only the web app asks, and the app's own files. A read-only
page for the operator is planned (ADR-0063, not yet written).

A review of the production server from outside found two things. Every path carried
the permissive CORS answer. And the web app's pages said nothing about framing, so
another site could show the app in a frame.

Neither was exploitable as found. A bearer token is never attached by a browser on
its own, the session cookie is `SameSite=Lax` and is not sent in a request from
another site, and the consent and sign-out handlers refuse a request that is not from
their own origin. The pages the server writes itself (sign-in, consent, errors)
already forbade framing. But "no other origin may read this" then rested on each
handler, not on the routes, and one framing case was real.

Which routes answer another origin, and what do pages say about framing?

## Decision Drivers

- A route that reads a cookie must never answer another origin, by construction and
  not by each handler's care.
- An MCP client in a browser must see no difference.
- **With sign-in off the app gets a token with nobody present, so a framed app is a
  working app:** another site can show it and steer a visitor's clicks in it. With
  sign-in on a framed app is signed out, because the cookie is not sent into a frame
  of another site. The first case is the main reason to forbid framing.
- The operator page to come must not be readable from, or framed by, another origin,
  and should not need a special case for that.
- No new dependency and nothing to configure.

## Considered Options

1. Keep the permissive answer everywhere and rely on bearer tokens and `SameSite`.
2. The permissive answer on the MCP and OAuth client routes only; no CORS answer
   anywhere else, and none on an unknown path.
3. A list of allowed origins in a setting.

For framing: `X-Frame-Options: DENY` with `frame-ancestors 'none'` on the app's pages,
or a full Content-Security-Policy for the app now.

## Decision Outcome

Chosen: **option 2**, with `X-Frame-Options: DENY` and
`Content-Security-Policy: frame-ancestors 'none'` on the web app's pages, and
`X-Content-Type-Options: nosniff` on every response.

### Which routes answer another origin

| Routes | Another origin |
|---|---|
| `/mcp`, and its aliases `/`, `/message`, `/sse` | answered: any origin, method and header, never with credentials |
| `/register`, `/token`, `/revoke` | answered, the same |
| `/.well-known/oauth-authorization-server`, `/.well-known/oauth-protected-resource`, `/.well-known/oauth-protected-resource/mcp` | answered, the same |
| `/authorize`, `/oauth/signin`, `/oauth/login/{provider}`, `/oauth/callback/{provider}`, `/oauth/cancel`, `/oauth/consent`, `/oauth/logout`, `/oauth/signed-out` | no CORS headers |
| `/oauth/me`, `/workspace/sync` | no CORS headers |
| `/favicon.svg`, `/favicon.ico`, the web app's files, any unknown path | no CORS headers |

The answered routes take a bearer token or nothing. None reads a cookie. Their answer
to a preflight and to a request is byte for byte what it was, and a test pins it.

"No CORS headers" is not a refusal the server sends. A preflight to such a route gets
what any `OPTIONS` to it gets (`405`, or `404` for an unknown path), and a browser on
another origin then does not make the request or does not hand over the answer. A
page on the server's own origin needs no CORS headers and sees no change.

`GET /` is the web app or the health document, and `POST /` is an alias of `/mcp`. A
path cannot be answered for one method and not for another, so `/` stays answered.
The health document says nothing a stranger may not know.

### Framing and sniffing

- The web app's pages (`index.html`, at `/` and at every path that falls back to it)
  carry `X-Frame-Options: DENY` and `Content-Security-Policy: frame-ancestors 'none'`.
  Both, because older browsers know only the first.
- The pages the server writes keep what [ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/)
  gave them, and it is now tested as a contract on the consent page:
  `X-Frame-Options: DENY`, `Content-Security-Policy: default-src 'none'; style-src
  'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'`, `Referrer-Policy:
  same-origin`, `X-Content-Type-Options: nosniff`, `Cache-Control: no-store`.
- Every response carries `X-Content-Type-Options: nosniff`, from one place.

A full Content-Security-Policy for the app is not part of this decision. The chart
and component libraries set inline styles, and what else the app needs has not been
measured. It comes later, first in report-only mode, with the test suite listening for
violations in the page; the server gets no endpoint for reports.

### What changes on the wire

- `Access-Control-Allow-Origin`, `Access-Control-Expose-Headers`,
  `Access-Control-Allow-Methods`, `Access-Control-Allow-Headers` and the `Vary` line
  that came with them are gone from every route in the lower three rows of the table,
  and from unknown paths.
- A preflight to those routes is no longer `200`.
- **Who could notice:** a browser page on another origin, not ours, that asked
  `/oauth/me` who is signed in or called `/workspace/sync`. None is known, and such a
  page could not have used `/workspace/sync`, which needs the web app's own token.
  The web app, the development server (it proxies to the server, so the browser sees
  one origin), command-line and server-side MCP clients send no preflight and are not
  affected.
- The app's pages gain two headers; every response gains one, or keeps the one it had.

### Consequences

- Good: no cookie-reading route answers another origin, whatever a handler does.
- Good: the app cannot be framed, with sign-in on or off.
- Good: the operator page needs no exception: its route and its page fall under "no
  CORS headers" and "not framed" like the rest.
- Good: MCP clients in browsers see the same answers.
- Bad: a page on another origin that used `/oauth/me` stops working, without a
  setting to bring it back.
- Bad: nobody can embed the app in a page of their own. A site that wants a chart in
  a frame has no way to ask for it.
- Neutral: `/` keeps the permissive answer for its `POST`.
- Neutral: `Strict-Transport-Security` is not sent by the server. It speaks plain
  HTTP behind a proxy and cannot know that the public side is HTTPS; the proxy sends
  it.

### Confirmation

End-to-end tests against the running server: the preflight and request answers of the
ten answered routes, compared whole; no `Access-Control-` header on any other route,
for a preflight and for a request, with and without the web app; the two framing
headers on the app's pages at `/`, `/index.html` and a fallback path; exactly one
`nosniff` on answers of every kind; the five headers of the consent page.

Not validated here: a real browser client on another origin against a deployed
server, and what a proxy in front adds or removes.

## Pros and Cons of the Options

### 1. Permissive everywhere

- Good: nothing changes for anyone.
- Bad: the safety of every cookie route rests on `SameSite` and on each handler's own
  origin check, and each new route has to remember it.

### 2. Per route

- Good: the rule is in the router, where a new route is put on one side or the other.
- Bad: a wire-visible change for pages on other origins.

### 3. A list of allowed origins

- Good: an operator could let one page of their own in.
- Bad: MCP clients in browsers come from origins the operator does not know, so the
  MCP routes would still need "any". It would only serve the routes that should
  answer nobody.
