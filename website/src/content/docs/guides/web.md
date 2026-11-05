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
  dialog. Typing searches every Binance spot pair and Yahoo Finance
  instrument by ticker or name through the `search_symbols` tool
  ([reference](/chartgen/reference/mcp/#search_symbols)), 200 ms after the
  last keystroke. Rows show the ticker, name, exchange and a type badge,
  ranked like TradingView: exact ticker, prefix, contains, then name. The
  tabs **All**, **Crypto**, **Stocks**, **Funds** (ETFs and mutual
  funds), **Indices**, **Forex** and **Futures** narrow the results by
  type. While results load, or if the search fails, the built-in list of
  popular symbols answers instantly; a status line under the tabs shows
  "Searching…", an error, or a note that one source returned nothing, and
  typing is never blocked. With an empty query the dialog lists your
  recent symbols (last 10), then the popular list. The last row accepts
  whatever you typed (uppercased) with Enter, for tickers the search does
  not know. Tickers ending in `USDT`, `BUSD`, `BTC`, `ETH`, `BNB`, `USDC`
  or `FDUSD` are fetched from Binance, everything else from Yahoo
  Finance. The watchlist **+** button opens the same dialog.
- **Timeframe** — one-click buttons for 1m, 5m, 15m, 1h, 4h, 1D and 1W,
  with the active one highlighted; the dropdown holds the rest (30m, 1M).
- **Chart type** — Candles, Hollow candles (only up candles hollow), Bars (OHLC), Line, Area, Heikin Ashi. Heikin Ashi is computed on the server (the web app sends `chart_style: "heikin_ashi"` with every load, tail poll and history page), so the legend shows Heikin Ashi OHLC values and indicators are calculated on them, as on TradingView. Switching to or from it reloads the chart.
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
  interval and the badge is hidden. At the close it disappears for up to one
  poll period (2 s) until the live update brings the next bar, and it hides
  when the last price is scrolled out of view.
- **Price scale** — the `%`, `log` and `auto` buttons in the bottom-right
  corner of the price axis. `%` shows percent change from the first
  visible bar and `log` a logarithmic scale; they are mutually exclusive
  and clicking the active one returns to the normal scale. `auto` (on by
  default) keeps the axis fitted to the visible bars. Dragging the price
  axis scales it by hand and turns `auto` off; double-clicking the axis or
  clicking `auto` turns it back on, and `Alt+R` (reset chart) does too.
  The scale mode persists across reloads; `auto` always starts on.

## Live updates and history

The chart stays live while the page is open, as in TradingView
([ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/)).

- **Live candle.** The forming candle, every indicator on it, the legend,
  the last-price badge and the countdown update continuously. When the
  interval rolls over, the finished candle gets its final values and a new
  one appears. If the chart is scrolled to the newest bar it keeps
  following it; if you scrolled back, the view stays where it is.
- **Scroll-back history.** Scroll or drag to the oldest loaded bar and the
  next 500 older bars load in place, without moving what is on screen. A
  small "Loading history…" badge shows at the left edge meanwhile. If a page
  fails, the badge offers **Retry**. Paging stops when the data source has
  no older bars, or at 20,000 loaded bars, where the left edge shows
  "History limit reached" (every live update recalculates the indicators
  over all loaded bars, so the history is capped).
- **Status.** A badge after the status text shows `live`, `paused` (tab
  hidden), `live: retry in Ns` after a failed update, or `resyncing...`
  while the chart reloads to catch up.

How it works: the first load asks `generate_chart` for 500 bars with a
`warmup` of 300. The app then polls the last two bars (`bars: 2`,
`warmup: 300`) and merges them in by timestamp. A history page is
`bars: 500, warmup: 300, end_time: <oldest bar × 1000>`. 800 bars fit in
one Binance request, and a warmup of 300 covers the usual lookbacks (up to
SMA/EMA 200) so values at a page seam line up. Changing the symbol,
timeframe or indicators cancels every pending poll and page and does a
full reload.

| | 24/7 symbols (Binance rule) | Other symbols (Yahoo) |
|---|---|---|
| Poll period | 2 s | 15 s |
| After a failed poll | doubles, up to 60 s | doubles, up to 60 s |

- Polling pauses while the tab is hidden and catches up as soon as it is
  visible again. If bars were missed (a hidden tab, a sleeping laptop),
  the app fetches a longer tail to fill the gap, without moving the view.
  The gap is estimated from the clock; when that comes to more than 700
  bars a 24/7 chart reloads, which also scrolls it back to the newest bar.
  Other symbols first try a 700-bar tail, because closed hours hold no
  bars and the real gap is usually smaller.
- **Limitations.** Volume profiles (`volume_profile`/VPVR, `tpo`,
  `hvn_lvn`) keep the profile of the full load and are not recomputed for
  live bars or older pages. A divergence line that forms while the chart
  is open usually shows up only on the next full load, and a line crossing
  a page boundary starts at that boundary. Recursive and cumulative indicators (EMA, RSI,
  ATR, OBV, CVD, VWAP) can step slightly at a page seam or on the live
  bar compared with one long request, because each request warms up
  independently. Yahoo data is delayed, so its live bar moves only as
  often as Yahoo updates it.

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
| `Alt+A` | Add a price alert at the last price |

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

## Sidebar and price alerts

The right sidebar has two tabs, **Watchlist** and **Alerts**; the tab you
last used is remembered, and **»** collapses the whole sidebar. The Alerts
tab shows the number of active alerts. The watchlist polls quotes only while
its tab is showing.

An alert watches one symbol's price. The conditions follow TradingView:

| Condition | Fires when |
|---|---|
| Crossing | the price moves from one side of the level to the other, or lands on it |
| Crossing up / Crossing down | the same, in one direction only |
| Greater than / Less than | the price is above / below the level |

**Only once** alerts switch off after firing (status *triggered*).
**Every time** alerts stay active: a crossing alert fires on each new cross, and
a greater / less alert fires once, then waits until the price is back on the
other side of the level before it can fire again.

Create an alert with the bell button in the header, `Alt+A` (an alert at the
last price, ignored while typing in a field or with a dialog open), or **+** in
the Alerts tab. The dialog takes the condition, price, trigger and an optional
message. Active alerts for the chart symbol are drawn as dashed lines with a
bell label; drag a line to move the level, or double-click it to edit.

In the Alerts tab alerts are grouped by symbol with their status (*active*,
*triggered*, *paused*). Hover a row for edit, pause / resume and delete;
clicking the row switches the chart to that symbol.

When an alert fires you get a toast, a short beep and, if you allow it, a
browser notification. The browser is asked for notification permission when
you create your first alert, not on load.

Alerts are evaluated in the browser ([ADR-0020](/chartgen/decisions/0020-client-side-price-alerts/)):
the chart symbol from the live feed, other symbols from their own quotes poll
every 5 s. **They only fire while chartgen is open in a visible tab** (with several tabs open, one of them evaluates). They
are saved in `localStorage` under `chartgen.alerts.list` (at most 200;
invalid entries are dropped on load) and are not shared between browsers.

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
- **[TanStack Solid Query](https://tanstack.com/query/latest/docs/framework/solid/overview)** — cache
  and retry for the full load. Cache keys are
  `['generate_chart', ticker, timeframe, indicators]`, so switching back
  to a timeframe we already loaded is instant (the live feed then catches
  up). The query never goes stale and does not refetch on focus or
  reconnect, because the live feed keeps the data fresh.
- **Live feed** — `web/src/lib/liveFeed.ts` takes over each full load,
  polls the tail and pages in history. The merge rules live in
  `web/src/lib/seriesMerge.ts` (unit-tested): bars and every per-bar
  channel are joined by timestamp, signals and divergence lines are
  re-indexed, and anything that does not line up (another symbol or
  interval, a gap, overlapping or stale bars, a different indicator
  shape) is rejected and leads to a longer tail or a full reload.
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
