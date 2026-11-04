---
title: Web frontend
description: SolidJS browser SPA for chartgen — one screen, OAuth 2.1 PKCE login, Streamable HTTP MCP client.
---

The `web/` directory holds a SolidJS single-page app that connects to
`chartgen --serve` via MCP Streamable HTTP and renders a candlestick
chart with an RSI panel. It implements
[ADR-0003](/chartgen/decisions/0003-web-frontend-stack/).

The scope is deliberately narrow: a top toolbar (symbol, timeframe,
chart type), the indicator picker, the chart and a watchlist. Alerts UI, order
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

- **Screenshot** (camera) — downloads the chart canvas, overlays included,
  as a PNG on the chart background, named
  `<SYMBOL>_<TF>_<yyyy-mm-dd_HHMM>.png` (local time). The DOM legend
  rows above the panes are not part of the image.
- **Reset chart** (circular arrow) — scrolls to the latest bar and restores
  the default bar spacing. Same as `Alt+R`.
- **Theme** (sun/moon) — switches between the dark and light themes
  ([ADR-0016](/chartgen/decisions/0016-light-theme/)). The first visit follows
  your OS color scheme. The chart, axes, crosshair, legends and screenshot
  background all follow; colors sent by the server (indicator fills, lines)
  stay as sent.
- **Fullscreen** — hidden when the browser does not support it; toggles the browser Fullscreen API on the whole app;
  the chart resizes to fit.

Symbol, timeframe, chart type, price-scale mode, theme and recent symbols are saved in
`localStorage` (keys prefixed `chartgen.layout.`) and restored on reload.

## Chart conveniences

- **Price legend** — the top-left row of the price pane shows the symbol,
  interval, **O H L C**, the change versus the previous close (absolute
  and percent, green for up, red for down) and the volume (compact
  `K`/`M`/`B`). It follows the crosshair bar while you hover and shows the
  last bar otherwise. Price precision is derived from the data.
- **Bar-close countdown** — a badge under the last-price label on the price
  axis counts down to the close of the current bar (`MM:SS`, `HH:MM:SS`, or
  `Nd HH:MM` for long intervals) and ticks every second. It is shown only
  for symbols that trade around the clock (the Binance rule: tickers ending
  in `USDT`, `BUSD`, `BTC`, `ETH`, `BNB`, `USDC` or `FDUSD`). Stocks, FX and
  futures have trading sessions, so their bar close cannot be derived from the
  interval and the badge is hidden. It also hides once the last bar's close
  time has passed (live bar rollover is not delivered yet) or when the last
  price is scrolled out of view.
- **Price scale** — the `%`, `log` and `auto` buttons in the bottom-right
  corner of the price axis. `%` shows percent change from the first
  visible bar and `log` a logarithmic scale; they are mutually exclusive
  and clicking the active one returns to the normal scale. `auto` (on by
  default) keeps the axis fitted to the visible bars. Dragging the price
  axis scales it by hand and turns `auto` off; double-clicking the axis or
  clicking `auto` turns it back on, and `Alt+R` (reset chart) does too.
  The scale mode persists across reloads; `auto` always starts on.

### Keyboard shortcuts

Shortcuts work when focus is on the chart or page body, and are ignored
while typing in a field or while a dialog or menu is open.

| Keys | Action |
|---|---|
| any letter | Open symbol search prefilled with that letter |
| `↑` / `↓`, `Enter`, `Esc` | In symbol search: move, pick, close |
| digit | Open the timeframe popup prefilled with that digit |
| `Enter` in the popup | Switch timeframe |
| `←` / `→` | Scroll the chart by one bar |
| `↑` / `↓` or `+` / `-` | Zoom in / out |
| `Home` / `End` | Jump to the first bar / to real time |
| `Alt+R` | Reset chart (scroll to real time, default zoom) |

The timeframe popup accepts a number of minutes (`5`, `15`, `60`, `240`)
or a number with a unit `m`, `h`, `d`, `w` (`1h`, `4H`, `1D`); an
uppercase `M` means month (`1M`), lowercase `m` minutes. A lone `D`, `W`
or `M` means one day, week or month. Only timeframes the backend serves are
accepted.

## Drawing tools

The vertical toolbar on the left of the chart holds the drawing tools:
trend line, ray, extended line, horizontal line, horizontal ray,
vertical line, parallel channel, fib retracement, rectangle, text
annotation and price label. Below them are a magnet toggle (off, weak,
strong), lock all, hide all and remove all (asks for confirmation). The
button at the top switches the cursor between crosshair and arrow.

A tool is armed for exactly one drawing: pick it, click the chart to
place the points, and the tool is released. Pick it again or press `Esc`
to cancel. Drawings pan and zoom with the chart.

- **Select** a drawing by clicking it, then press `Delete` or
  `Backspace` to remove it.
- **Right-click** a drawing for Remove and Lock/Unlock. Locked drawings
  cannot be dragged but still answer right-click, so they can be
  unlocked there. Right-clicking while a drawing is in progress cancels
  it. Hide is only available for all drawings at once (toolbar eye).
- **Double-click** a text annotation to edit its text in a dialog.

Drawings are saved per symbol in `localStorage` under
`chartgen.drawings.<SYMBOL>` as time and price points, so they survive
reloads and timeframe changes. Points right of the last bar are stored
as the last bar's time plus whole bar intervals. A drawing with a point
before the oldest loaded bar stays hidden and untouched until older
bars are loaded. Stored entries are validated on load and malformed ones
are dropped. Each symbol is capped at 200 KB and the 100 most recently
used symbols are kept. A red dot at the bottom of the toolbar means the
drawing limit (500 per symbol) was reached or saving failed (storage
full); hover it for details.

| Keys | Action |
|---|---|
| `Alt+T` | Trend line |
| `Alt+H` | Horizontal line |
| `Alt+J` | Horizontal ray |
| `Alt+V` | Vertical line |
| `Alt+F` | Fib retracement |
| `Esc` | Cancel the armed tool or close the drawing menu |
| `Delete` / `Backspace` | Remove the selected drawing |
| `Ctrl+Z` / `Cmd+Z` | Undo the last drawing action |

The shortcuts use modifiers so they do not clash with the letter and
digit shortcuts above.

## Watchlist

The panel on the right of the chart lists symbols with their **Last**
price, absolute change (**Chg**) and percentage change (**Chg%**).
Changes are green when up and red when down, and the Last cell flashes
green or red briefly whenever the price ticks. Quotes come from the
[`get_quotes`](/chartgen/reference/mcp/#get_quotes) tool, polled every
5 s while the browser tab is visible. Polling pauses while the tab is
hidden or the panel is collapsed, and resumes within 5 s when you come back.

- **Click a row** to show that symbol on the chart. The row of the
  current chart symbol is highlighted.
- **+** opens the symbol search dialog. The symbol you pick is added at
  the bottom of the list, and the chart symbol stays unchanged.
- **×** (shown on hover) removes a row.
- **»** collapses the panel to a thin strip on the right edge; the list
  icon there expands it again.

Stock, ETF, index and FX rows change against the previous session's
close, like TradingView. Crypto rows (Binance) change over the rolling
24 hours, as Binance reports it, so for crypto the number can differ from
TradingView's change since the daily open
([ADR-0011](/chartgen/decisions/0011-watchlist-quotes/)). A symbol that
fails to load shows dashes; hover the row for the reason.

The list (default: BTCUSDT, ETHUSDT, SOLUSDT, AAPL, MSFT, NVDA, SPY) and
the collapsed state are saved in `localStorage` under
`chartgen.watchlist.symbols` and `chartgen.watchlist.collapsed`. On load,
entries are trimmed, uppercased and deduplicated, and invalid ones are
dropped. The list holds at most 50 symbols, the `get_quotes` limit.

## Layouts

A layout is a named bundle of symbol, timeframe, chart type, scale mode,
and the indicator set (including hidden and edited ones). Drawings are
not part of a layout: they stay per symbol and look the same in every
layout. The layout menu sits at the right end of the top bar and shows the active
layout's name. A dot next to the name means the chart differs from the
saved layout. The first start creates one layout, `Unnamed`, from your
current chart.

| Menu entry | Action |
|---|---|
| Save layout (`Ctrl+S` / `Cmd+S`) | Store the current chart in the active layout. Works from anywhere, including the chart, and replaces the browser's save dialog. |
| Rename... | Rename the active layout. A name already in use gets a suffix: `Name (2)`. |
| Make a copy / Save as... | Store the current chart as a new layout and switch to it. |
| Create new layout... | A new layout on the current symbol and timeframe; chart type, scale mode and indicators go back to their defaults. |
| Load layout... | A searchable list of layouts; click one to switch, or the x on its row to delete it. The open layout cannot be deleted: switch to another one first. |
| Export saved layout / Import layout... | Download the saved layout as a JSON file, or add one from a file. Imports are validated (files over 256 KB, other versions and unknown fields are refused or dropped); invalid files are refused. |

Switching layouts applies all of the above. If the current layout has
unsaved changes, a dialog offers Save, Don't save or Cancel. Unsaved
changes survive a reload like any other chart state, and so does the
active layout, so there is no "leave page?" prompt. Saving with nothing
changed still shows "Layout saved". Hiding an indicator counts as a
change; opening an indicator's settings and pressing OK without edits
does not.

If you use several tabs, each tab keeps its own open layout; saving in
one tab does not discard layouts created in another.

Layouts are stored in `localStorage` under `chartgen.layouts` (at most
50, validated on load, malformed ones dropped). See
[ADR-0015](/chartgen/decisions/0015-named-layouts/).

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
  refetches the chart with the new parameters. The **Style** tab lists each
  plotted line and histogram with a visibility checkbox, a color swatch
  (palette grid, opacity slider and custom color), a width (1-4 px) and a line
  style (solid, dashed, dotted). Style changes apply instantly on the client,
  never refetch, and also recolor the legend; hidden plots are not drawn and
  their legend value is omitted. **Defaults** resets the tab being shown.
- **×** removes the indicator. The price pane legend cannot be removed.

The set, parameters and hidden state persist in `localStorage`
([ADR-0013](/chartgen/decisions/0013-indicator-visibility-and-instances/)),
and so do the plot styles
([ADR-0017](/chartgen/decisions/0017-indicator-plot-styles/)).
Each indicator can be added once; two instances with different parameters
are not supported yet.

## Build and deploy

```bash
cd web
npm run build      # static bundle in dist/
npm run typecheck  # strict + noUncheckedIndexedAccess
npm test           # Vitest unit tests
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
- **Overlay geometry** — the `fills`, `hbars` and `divlines` channels
  of the series payload
  ([ADR-0007](/chartgen/decisions/0007-complete-series-serialization/))
  render in the chart and follow pan, zoom and rescale:
  - *fills* — semi-transparent bands between two series (Bollinger,
    Keltner, Donchian, Stochastic, Cipher B / WaveTrend, the Kalman
    Volume Filter bands and OB/OS zones). The band breaks where either
    series is `null`. The Ichimoku cloud switches between green and red
    where Senkou A and Senkou B cross. Oscillator panes always include
    the indicator's `y_range` (e.g. RSI 0–100) and every fill edge, so
    zones at the pane edge stay visible.
  - *hbars* — volume-profile rows (`volume_profile`, `tpo`), anchored
    to the left or right pane edge as the payload's `left` flag says,
    with width proportional to volume. The widest row (the POC) is gold,
    and rows inside the value area are brighter.
  - *divlines* — straight segments between two bars, dashed when the
    payload says so. Bullish divergences are green and bearish ones red.
    The kind is inferred from the slope, because the wire carries no color.

  The wire drops server-side colors, so the frontend mirrors the Rust
  palette per indicator. The geometry code is pure TypeScript in
  `web/src/components/geometry.ts` (unit-tested), and klinecharts
  `draw` callbacks paint it. See
  [ADR-0010](/chartgen/decisions/0010-overlay-geometry-rendering/).
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
