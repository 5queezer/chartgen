---
title: "0027 — Serve the web app from the MCP server's origin"
description: "chartgen --serve serves the built SolidJS app from CHARTGEN_WEB_DIR: browsers get the app at /, everyone else keeps the JSON health document, and /mcp is the canonical MCP endpoint."
sidebar:
  order: 27
---

**Status:** Accepted; amended by [ADR-0062](/chartgen/decisions/0062-cors-per-route-and-no-framing/) (the app's pages forbid framing, and its files answer no other origin)
**Date:** 2026-11-05

## Context and Problem Statement

The SolidJS app in `web/` ([ADR-0003](/chartgen/decisions/0003-web-frontend-stack/))
talks to chartgen over MCP and OAuth. ADR-0003 assumed the production app
and the backend would share one origin, but nothing deployed the app. The
Docker image that Coolify builds contained only the Rust binary, so opening
`https://chartgen.vasudev.xyz/` in a browser showed the JSON health
document, not the chart app.

`GET /` is also probed by health checks and by MCP clients, and existing
connectors were added with the root URL and send `POST /`. Any change has
to keep those requests working exactly as before.

## Decision Drivers

- One deploy (the root `Dockerfile` on Coolify) without new Coolify
  settings or a second hosting service.
- Same origin for the app and the API: no CORS, and OAuth `redirect_uri`
  is just `origin + "/"`.
- Non-browser requests to `/` must keep getting the same JSON.
- Running `chartgen --serve` without the web build must work as before.
- Connectors must discover OAuth from the MCP URL they were given.

## Considered Options

**Same origin, served by chartgen (chosen).** The image builds `web/dist`
and chartgen serves it as static files next to its MCP and OAuth routes.

**Separate static host (Cloudflare Pages).** The app is built and hosted
on its own origin and calls the API through `VITE_CHARTGEN_BASE_URL`. This
needs a second deploy pipeline, CORS for every route, and OAuth redirects
across origins, and the two deploys can drift apart.

**Backend only.** Keep the image API-only and leave the app to local
`npm run dev`. The deployed URL would never show the app.

## Decision Outcome

Chosen option: same origin, served by chartgen.

- `CHARTGEN_WEB_DIR` points at a built `web/dist`. Unset, chartgen
  serves no static files and behaves as before. The Dockerfile builds the
  app in a `node:22-bookworm-slim` stage with an empty
  `VITE_CHARTGEN_BASE_URL`, copies it to `/usr/share/chartgen/web`, and
  sets `CHARTGEN_WEB_DIR` there.
- Routes:
  - `/` is the web app for browsers and the JSON health document for
    everyone else. A `GET /` whose `Accept` header contains `text/html`
    gets `index.html`; any other `GET /` gets the JSON. The OAuth callback
    (`/?code=…&state=…`) is a browser navigation, so it loads the app.
  - `/mcp` is the canonical MCP endpoint (`POST` for JSON-RPC, `GET` for
    the SSE stream). Documentation names `https://chartgen.vasudev.xyz/mcp`
    as the connector URL.
  - `POST /` is a deprecated compatibility alias for connectors added with
    the root URL, like the older `/message` and `/sse` aliases. It is kept
    and not advertised.
  - All explicit routes (`/mcp`, `/sse`, `/message`, `/register`,
    `/authorize`, `/token`, `/.well-known/oauth-authorization-server`,
    `/.well-known/oauth-protected-resource`,
    `/.well-known/oauth-protected-resource/mcp`, `/favicon.*`) take
    precedence. Static files are the router fallback, served by
    tower-http `ServeDir`, which rejects `..` paths. An unknown `GET` that
    accepts `text/html` gets `index.html` (SPA routing). Other unknown
    paths, including missing `/assets/*` files and non-GET requests, stay
    404. Paths with a dot-prefixed segment (`/.env`, `/.git/…`) or a
    leading `//` are refused with 404 before `ServeDir`. `CHARTGEN_WEB_DIR`
    must hold only the build output: symlinks inside it are followed.
- Cache headers: hashed files under `/assets/` get
  `public, max-age=31536000, immutable`. Everything else, including
  `index.html`, gets `no-cache`. All static responses carry
  `X-Content-Type-Options: nosniff`.
- OAuth discovery from `/mcp` follows RFC 9728.
  `/.well-known/oauth-protected-resource/mcp` describes the resource
  `<base>/mcp`, and `/.well-known/oauth-protected-resource` describes
  `<base>` for the root alias. Both name `<base>` as the authorization
  server. A 401 from an MCP endpoint carries
  `WWW-Authenticate: Bearer resource_metadata="…"` pointing at the
  matching document.

### Consequences

- Good: one image, one origin, one deploy. Opening the public URL shows
  the app, and the OAuth redirect stays on the same origin.
- Good: health checks and MCP probes see no change. Root-URL connectors
  keep working, but 401s from `/`, `/sse` and `/message` now carry a
  `WWW-Authenticate: Bearer resource_metadata="…"` hint pointing at the
  root document (`resource` = base URL).
- Good: connectors starting at `/mcp` find the authorization server
  through the 401 hint or the path-suffixed metadata.
- Bad: the image build now also needs Node and `npm ci`, which makes it
  slower and adds the web dependencies to the build.
- Bad: telling browsers from other clients relies on `Accept: text/html`.
  A client that sends `text/html` to `GET /` gets HTML instead of JSON.
- Neutral: the app and the server ship together and cannot be deployed
  separately. `VITE_CHARTGEN_BASE_URL` still supports a separate host for
  anyone who wants one.

## Links

- Web stack: [ADR-0003](/chartgen/decisions/0003-web-frontend-stack/)
- MCP transport and aliases: [ADR-0002](/chartgen/decisions/0002-mcp-transport-streamable-http/)
- [RFC 9728: OAuth 2.0 Protected Resource Metadata](https://www.rfc-editor.org/rfc/rfc9728)
