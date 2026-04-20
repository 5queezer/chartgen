# chartgen-web

SolidJS browser frontend for chartgen. Implements [ADR-0003](../website/src/content/docs/decisions/0003-web-frontend-stack.md).

## Stack

- SolidJS + Vite + TypeScript (strict + `noUncheckedIndexedAccess`)
- `@tanstack/solid-query` for async server state
- `@modelcontextprotocol/sdk` (`StreamableHTTPClientTransport`)
- Kobalte for accessible UI primitives
- Tailwind v4 (CSS-first config)
- klinecharts for candles + RSI panel
- Zod for runtime validation at the MCP boundary

## Dev

```bash
npm install
npm run dev
```

Vite serves at `http://localhost:5173` and proxies `/mcp`, `/sse`,
`/register`, `/authorize`, `/token`, `/.well-known/*`, `/oauth/*` to
`http://localhost:9315` (chartgen's default port). Start chartgen in a
separate terminal:

```bash
cargo run -- --serve
```

Opening the dev server triggers the OAuth 2.1 PKCE flow on the first
`Load` click: discovery → dynamic client registration → authorize
redirect → token exchange. The resulting access token stays in memory
only; `client_id`/`client_secret` persist in `localStorage`.

## Build

```bash
npm run build
```

Emits a static bundle in `dist/`. Deploy to any static host
(Cloudflare Pages, Netlify, S3+CloudFront, etc.). Configure the hosted
origin with `VITE_CHARTGEN_BASE_URL` at build time if the frontend and
chartgen are on different origins; otherwise leave it unset for
same-origin.

## Typecheck

```bash
npm run typecheck
```

## Configuration

| Variable | Purpose | Default |
|---|---|---|
| `VITE_CHARTGEN_BASE_URL` | chartgen origin | unset → same-origin |

See `.env.example`. Copy to `.env.local` to override for local dev.
