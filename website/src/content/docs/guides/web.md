---
title: Web frontend
description: SolidJS browser SPA for chartgen — one screen, OAuth 2.1 PKCE login, Streamable HTTP MCP client.
---

The `web/` directory holds a SolidJS single-page app that connects to
`chartgen --serve` via MCP Streamable HTTP and renders a candlestick
chart with an RSI panel. It implements
[ADR-0003](/chartgen/decisions/0003-web-frontend-stack/).

The v0.1 scope is deliberately narrow: ticker + timeframe + a Load
button, and that's it. Alerts UI, order entry, multi-chart layouts,
and LLM chat are explicitly out of scope until the core loop is
solid.

## Getting started

```bash
# Terminal 1: run chartgen
cargo run -- --serve

# Terminal 2: run the frontend
cd web
npm install
npm run dev
```

Vite serves at `http://localhost:5173` and proxies OAuth + MCP
endpoints to chartgen on port 9315, so no CORS or
`VITE_CHARTGEN_BASE_URL` configuration is needed for local dev.

Clicking **Load** triggers the OAuth 2.1 PKCE flow on first use:
discovery → dynamic client registration → redirect to
`/authorize` → token exchange. The access token stays in memory;
the `client_id`/`client_secret` pair persists in `localStorage` so
subsequent sessions skip registration.

## Build and deploy

```bash
cd web
npm run build      # static bundle in dist/
npm run typecheck  # strict + noUncheckedIndexedAccess
```

The build output is a static bundle ready for any CDN or object
store. The recommended target is **Cloudflare Pages**:

1. Connect the GitHub repository in the Pages dashboard.
2. Set **Build command** to `cd web && npm install && npm run build`.
3. Set **Build output directory** to `web/dist`.
4. Add `VITE_CHARTGEN_BASE_URL=https://chartgen.example.com` as an
   environment variable if the API is on a different origin.

For same-origin deployments (frontend and backend behind one
domain) leave `VITE_CHARTGEN_BASE_URL` unset — the app will hit
`/mcp`, `/register`, `/token` etc. on the current origin.

## Architecture

- **[SolidJS](https://www.solidjs.com/)** — fine-grained reactivity,
  ~7 KB runtime. Candle ticks update only the DOM nodes that depend
  on them. See
  [ADR-0003](/chartgen/decisions/0003-web-frontend-stack/#decision-outcome)
  for why Solid over React/Svelte/Vue.
- **[TanStack Solid Query](https://tanstack.com/query/latest/docs/framework/solid/overview)** — cache,
  retry, stale-while-revalidate. Cache keys are
  `['generate_chart', ticker, timeframe]`, so switching timeframes
  on a symbol we already loaded is instant.
- **[`@modelcontextprotocol/sdk`](https://www.npmjs.com/package/@modelcontextprotocol/sdk)**
  with `StreamableHTTPClientTransport`. Bearer tokens flow through
  `requestInit.headers`; `fallbackNotificationHandler` taps into
  `notifications/alert_triggered` frames on the GET SSE stream
  (dormant logging in v0.1).
- **[Kobalte](https://kobalte.dev/)** — WAI-ARIA-correct headless
  primitives (`Select`, `TextField`). Visual styling via Tailwind.
- **[Tailwind v4](https://tailwindcss.com/)** — CSS-first config
  (no `postcss.config.js`). OKLCH dark palette.
- **[klinecharts](https://klinecharts.com/)** — candles + RSI
  panel. The RSI values come from chartgen; the browser doesn't
  recompute them. A custom indicator reads per-bar stashed values
  and returns them verbatim; `calc`'s `null` outputs become gaps in
  the panel.
- **[Zod](https://zod.dev/)** — every MCP response is parsed with
  the schemas in `web/src/types/responses.ts`
  ([ADR-0004](/chartgen/decisions/0004-mcp-type-safety/)).
  Tool-argument types come from
  `web/src/types/generated-input.ts`, regenerated via
  `./scripts/gen-mcp-types.sh`.

## Configuration

| Variable | Purpose | Default |
|---|---|---|
| `VITE_CHARTGEN_BASE_URL` | chartgen origin | unset → same-origin |

Copy `web/.env.example` to `web/.env.local` for local overrides.
Production builds bake the value at build time (standard Vite
behavior for `VITE_*` variables).
