---
title: Web frontend
description: Browser SPA for chartgen's MCP HTTP surface — OAuth 2.1 PKCE + klinecharts.
---

`web/` ships a small single-page app that consumes chartgen's HTTP MCP
endpoints directly. It has no backend of its own — it drives the same
[OAuth 2.1 PKCE flow](/chartgen/reference/oauth/) and
[`tools/call` JSON-RPC](/chartgen/reference/mcp/) that Claude.ai uses, and
listens to the persistent SSE stream for
[push notifications](/chartgen/guides/notifications/).

See `web/README.md` in the repo for the full file layout.

## Scope (v0)

- OAuth 2.1 PKCE login with dynamic client registration (RFC 7591). The
  `client_id`/`client_secret` returned by `POST /register` are cached in
  `localStorage`; the access token is kept **in memory only**. On 401 the
  full auth flow is re-run.
- One screen: ticker + interval + Load button.
- On Load, calls `generate_chart` with `format=series` and renders the
  resulting bars as klinecharts candles plus an RSI pane.
- The SSE stream at `GET /sse` is kept open with `Authorization: Bearer
  <token>` (header, not query param); `notifications/alert_triggered` is
  logged to the browser console. No UI wiring yet.

## Dev setup

Requires Node 20+ and a chartgen server reachable at
`VITE_CHARTGEN_BASE_URL` (default `http://localhost:9315`).

```bash
cd web
cp .env.example .env.local
npm install
npm run dev
```

Open `http://localhost:5173`. Vite proxies every chartgen path
(`/mcp`, `/sse`, `/message`, `/.well-known/*`, `/register`, `/authorize`,
`/token`, `/oauth/*`) to the server so the SPA runs same-origin and the
OAuth redirect lands back on `localhost:5173`.

## Build

```bash
cd web
npm run build
```

Emits a static bundle in `web/dist/`. Deploy to any static host (Cloudflare
Pages, Vercel, S3) — or serve the built assets from chartgen itself behind
the same reverse proxy.

In production, the SPA points at whatever `VITE_CHARTGEN_BASE_URL` was set
to at build time; there is no runtime config lookup. Rebuild when the base
URL changes.

## Stack

- Vite + TypeScript
- [`klinecharts`](https://github.com/liihuu/KLineChart) 9.x — candlesticks
  and the RSI pane are rendered from chartgen's own values (a custom
  indicator pulls from a field stashed on each `KLineData` bar), so
  pre-warmup `null`s become rendered gaps.
- [`@modelcontextprotocol/sdk`](https://github.com/modelcontextprotocol/typescript-sdk)
  `Client` over `SSEClientTransport` — JSON-RPC `tools/call` and the SSE
  notification channel share the same authenticated transport. The Bearer
  is passed via `requestInit.headers` and an `eventSourceInit.fetch`
  override so it never appears as a query parameter.

## Out of scope for v0

- Alerts UI (creation/listing), order placement, position display.
- LLM chat integration.
- Multi-chart layout, drawing tools.
- Desktop wrappers (Tauri / Electron).
- Persistent user preferences beyond the cached client registration.

See `web/README.md` for the file layout and a short tour of each module.
