---
title: Web frontend
description: SolidJS browser SPA for chartgen — one screen, OAuth 2.1 PKCE login, Streamable HTTP MCP client.
---

The `web/` directory holds a SolidJS single-page app that connects to
`chartgen --serve` via MCP Streamable HTTP and renders a candlestick
chart with an RSI panel. It implements
[ADR-0003](/chartgen/decisions/0003-web-frontend-stack/).

The scope is deliberately narrow: a top toolbar (symbol, timeframe,
chart type), the indicator picker, and the chart. Alerts UI, order
entry, multi-chart layouts, and LLM chat are explicitly out of scope
until the core loop is solid.

## Toolbar

The chart loads automatically as soon as sign-in finishes and reloads
whenever the symbol, timeframe, or indicator set changes. The previous
chart stays on screen while the new one loads. There is no Load button; if a load fails, use **Retry** in the status line
  or re-pick the current symbol or timeframe.

- **Symbol** — the button shows the current ticker and opens the search
  dialog. It filters a curated list of Binance USDT pairs and popular US
  stocks, ETFs, indices, FX pairs and futures by ticker or name; with an
  empty query it lists your recent symbols (last 10) first. There is no
  search backend, so any other ticker can be typed and accepted with
  Enter (it is uppercased). Tickers ending in `USDT`, `BUSD`, `BTC`,
  `ETH`, `BNB`, `USDC` or `FDUSD` are fetched from Binance, everything
  else from Yahoo Finance.
- **Timeframe** — one-click buttons for 1m, 5m, 15m, 1h, 4h, 1D and 1W,
  with the active one highlighted; the dropdown holds the rest (30m, 1M).
- **Chart type** — Candles, Hollow candles (only up candles hollow), Bars (OHLC), Line, Area.
  Heikin-Ashi is not offered because it needs client-side math
  ([ADR-0003](/chartgen/decisions/0003-web-frontend-stack/)).

Symbol, timeframe, chart type and recent symbols are saved in
`localStorage` (keys prefixed `chartgen.layout.`) and restored on reload.

### Keyboard shortcuts

Shortcuts work when focus is on the chart or page body, and are ignored
while typing in a field or while a dialog or menu is open.

| Keys | Action |
|---|---|
| any letter | Open symbol search prefilled with that letter |
| `↑` / `↓`, `Enter`, `Esc` | In symbol search: move, pick, close |
| digit | Open the timeframe popup prefilled with that digit |
| `Enter` in the popup | Switch timeframe |

The timeframe popup accepts a number of minutes (`5`, `15`, `60`, `240`)
or a number with a unit `m`, `h`, `d`, `w` (`1h`, `4H`, `1D`); an
uppercase `M` means month (`1M`), lowercase `m` minutes. A lone `D`, `W`
or `M` means one day, week or month. Only timeframes the backend serves are
accepted.

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

The first page load triggers the OAuth 2.1 PKCE flow:
discovery → dynamic client registration → redirect to
`/authorize` → token exchange. The access token stays in memory;
the `client_id`/`client_secret` pair persists in `localStorage` so
subsequent sessions skip registration.

## Managing indicators

Indicators are added from the **Indicators** picker (by name, with the
server's default parameters) and listed as chips under the toolbar. Each indicator pane
legend shows controls on hover (or keyboard focus):

- **Eye** hides the indicator. It disappears from the chart immediately and
  is no longer requested from the server; its chip stays, dimmed, with an eye button to
  show it again.
- **Gear** (or double-click on the legend row, or the gear on the chip)
  opens the settings dialog. The form is generated from the indicator's
  `configurable_params` (numbers, switches, text). **Defaults** resets the
  form to the schema defaults; **OK** validates and applies, which
  refetches the chart with the new parameters.
- **×** removes the indicator. The price pane legend cannot be removed.

The set, parameters and hidden state persist in `localStorage`
([ADR-0013](/chartgen/decisions/0013-indicator-visibility-and-instances/)).
Each indicator can be added once; two instances with different parameters
are not supported yet.

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
  `['generate_chart', ticker, timeframe, indicators]`, so switching
  timeframes on a symbol we already loaded is instant.
- **[`@modelcontextprotocol/sdk`](https://www.npmjs.com/package/@modelcontextprotocol/sdk)**
  with `StreamableHTTPClientTransport`. Bearer tokens flow through
  `requestInit.headers`; `fallbackNotificationHandler` taps into
  `notifications/alert_triggered` frames on the GET SSE stream
  (dormant logging in v0.1).
- **[Kobalte](https://kobalte.dev/)** — WAI-ARIA-correct headless
  primitives (`Dialog`, `DropdownMenu`, `Accordion`). Visual styling via Tailwind.
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
