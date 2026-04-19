# chartgen-web

A tiny browser SPA that talks to chartgen's HTTP MCP surface. Renders
candlesticks + an RSI pane from the `generate_chart` (`format=series`) tool
and logs `notifications/alert_triggered` pushes from the SSE stream to the
console.

Stack: Vite + vanilla TypeScript + [klinecharts](https://github.com/liihuu/KLineChart)
+ [`@modelcontextprotocol/sdk`](https://github.com/modelcontextprotocol/typescript-sdk).

## v0 scope

- OAuth 2.1 PKCE (S256) against chartgen's built-in authorization server,
  using dynamic client registration (RFC 7591).
- One screen: symbol input + interval dropdown + Load button.
- Access token in memory only — re-auth on 401.
- SSE stream auto-connects; notifications go to `console.log`, no UI yet.

Explicitly out of scope: alerts UI, order/position panels, LLM chat,
multi-chart layout, drawing tools, desktop wrappers.

## Dev

Requires Node 20+ and a running `chartgen --serve` on `localhost:9315`
(the web UI does not need the server to boot, only to Load a chart).

```bash
cd web
cp .env.example .env.local    # optional — defaults are fine locally
npm install
npm run dev
```

Open http://localhost:5173.

Vite proxies `/mcp`, `/sse`, `/.well-known/*`, `/register`, `/authorize`,
`/token`, `/message`, and `/oauth/*` to `VITE_CHARTGEN_BASE_URL`, so the SPA
runs same-origin in dev and OAuth redirects land back on `localhost:5173`.

## Build

```bash
npm run build
```

Emits a static bundle to `web/dist/` — deploy to any static host
(Cloudflare Pages, Vercel, S3) or serve it from chartgen itself behind the
same reverse proxy.

## Config

| Env var | Default | Notes |
|---|---|---|
| `VITE_CHARTGEN_BASE_URL` | `http://localhost:9315` | Base URL of the chartgen server. Must be reachable from the browser. Read at build time for `import.meta.env`. |

## Files

```
web/
├── index.html
├── package.json
├── tsconfig.json
├── vite.config.ts
├── .env.example
├── .gitignore
└── src/
    ├── main.ts         Bootstrap + form handling
    ├── oauth.ts        RFC 7591 registration + PKCE S256 flow
    ├── mcp.ts          SDK Client over SSEClientTransport with Bearer
    ├── chart.ts        klinecharts render (candles + custom RSI pane)
    ├── types.ts        Hand-written response types for format=series
    └── style.css
```

## Not covered here

Live end-to-end verification against a running chartgen instance (OAuth round
trip + SSE subscription + `notifications/alert_triggered` receipt) has not
been performed by the scaffolding commit — that needs a reviewer with a
running server.
