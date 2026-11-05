---
title: TradingView parity
description: Feature matrix comparing TradingView's charting features with the chartgen web frontend, with status and links to the guide and ADRs.
---

chartgen's [web frontend](/chartgen/guides/web/) aims to feel like
TradingView's chart for the features it covers
([ADR-0006](/chartgen/decisions/0006-tradingview-visual-parity/)). This page
lists TradingView's charting features and says what chartgen does about each,
so you can see what is supported, what is partial and what is missing. It
covers the web app and the MCP server behind it. It is not a claim of full
parity.

Status legend:

- ✅ supported
- ◐ partial: works, with differences listed in the notes
- 🚧 in progress: being built, not on this branch yet
- ❌ not supported

## Chart & navigation

| Feature | TradingView | chartgen | Notes / links |
|---|---|---|---|
| Symbol search | ✅ | ✅ | Binance spot pairs and Yahoo instruments, type tabs, recent symbols. [Toolbar](/chartgen/guides/web/#toolbar), [`search_symbols`](/chartgen/reference/mcp/#search_symbols) |
| Timeframe buttons and keyboard entry | ✅ | ◐ | A fixed set of 14 intervals (1m, 3m, 5m, 15m, 30m, 1h, 2h, 4h, 6h, 12h, 1D, 3D, 1W, 1M); typing a digit or **Custom interval...** opens a popup that accepts any of them (`2h`, `120`, `3D`). No arbitrary multiples (7m, 2D) or seconds. [Keyboard shortcuts](/chartgen/guides/web/#keyboard-shortcuts) |
| Price legend (OHLC, change, volume) | ✅ | ✅ | Follows the crosshair. [Chart conveniences](/chartgen/guides/web/#chart-conveniences) |
| Bar-close countdown | ✅ | ◐ | Only for 24/7 symbols (the Binance rule). Hidden for stocks, FX and futures, because the session close cannot be derived from the interval. [Chart conveniences](/chartgen/guides/web/#chart-conveniences) |
| %, log and auto price scale | ✅ | ✅ | [Chart conveniences](/chartgen/guides/web/#chart-conveniences) |
| Inverted scale, lock scale to bars | ✅ | ◐ | Invert (`Alt+I`) and lock price to bar ratio. "Scale price chart only" is not offered. [Chart conveniences](/chartgen/guides/web/#chart-conveniences) |
| Date-range buttons | ✅ | ✅ | `1D 5D 1M 3M 6M YTD 1Y 5Y All`. Each range uses TradingView's interval (6M is 2h). A range is capped by the history limit. [Bottom bar](/chartgen/guides/web/#bottom-bar) |
| Time zone selector and clock | ✅ | ◐ | UTC, Exchange, Local and common IANA zones. "Exchange" is a heuristic on the ticker suffix. [Bottom bar](/chartgen/guides/web/#bottom-bar) |
| Go to date | ✅ | ✅ | `Alt+G`. [Bottom bar](/chartgen/guides/web/#bottom-bar) |
| Scroll, zoom, keyboard navigation | ✅ | ✅ | Arrow keys, `Home`/`End`, `Alt+R` reset. [Keyboard shortcuts](/chartgen/guides/web/#keyboard-shortcuts) |
| Screenshot | ✅ | ◐ | Downloads the chart canvas as PNG. The DOM legends are not in the image. [Toolbar](/chartgen/guides/web/#toolbar) |
| Fullscreen | ✅ | ✅ | Hidden when the browser lacks the Fullscreen API. [Toolbar](/chartgen/guides/web/#toolbar) |
| Light and dark theme | ✅ | ✅ | Follows the OS on first visit. Server-sent colors stay as sent. [ADR-0016](/chartgen/decisions/0016-light-theme/) |
| Custom theme editor | ✅ | ❌ | Out of scope of [ADR-0006](/chartgen/decisions/0006-tradingview-visual-parity/). |
| Chart settings dialog | ✅ | ◐ | Symbol, Status line, Scales and Appearance tabs with live preview, Cancel and Reset. Colors per theme, not part of layouts. No trading, events or alerts tabs. [Chart settings](/chartgen/guides/web/#chart-settings), [ADR-0019](/chartgen/decisions/0019-chart-settings/) |
| Right-click context menu | ✅ | ◐ | Reset view, copy price, add horizontal line, remove all drawings or indicators, settings. "Lock vertical cursor line by time" is not offered. [Chart context menu](/chartgen/guides/web/#chart-context-menu) |
| Add alert at the clicked price (context menu) | ✅ | ✅ | "Add alert at price" opens the alert dialog at that price. [Chart context menu](/chartgen/guides/web/#chart-context-menu), [Alerts](/chartgen/guides/web/#sidebar-and-price-alerts) |
| Titled dialogs (symbol search, compare, add symbol) | ✅ | ✅ | The compare and watchlist dialogs say "Compare symbol" and "Add symbol". [Compare symbols](/chartgen/guides/web/#compare-symbols), [Watchlist](/chartgen/guides/web/#watchlist) |
| Mobile and touch layout | ✅ | ❌ | The web guide describes a desktop layout only. |
| Multi-chart layouts (several charts in one view) | ✅ | ❌ | One chart per screen. [Web guide scope](/chartgen/guides/web/) |

## Chart types

| Feature | TradingView | chartgen | Notes / links |
|---|---|---|---|
| Candles | ✅ | ✅ | [Toolbar](/chartgen/guides/web/#toolbar) |
| Hollow candles | ✅ | ✅ | Up candles hollow, down candles filled, as in TradingView. |
| Bars (OHLC) | ✅ | ✅ | |
| Line | ✅ | ✅ | |
| Area | ✅ | ✅ | |
| Heikin Ashi | ✅ | ✅ | Computed on the server, so indicators run on the HA values as on TradingView. [`chart_style`](/chartgen/reference/mcp/) |
| Baseline, columns, high-low, step line, line with markers | ✅ | ◐ | Drawn in the browser from the bars. Baseline's base level is fixed at TradingView's default (50% of the visible range) and not configurable. No per-type style settings (TradingView's Baseline, Columns and High-low style tabs); the types take the line and up/down body colors, and the body/border toggles do not apply to Columns or Baseline. [Toolbar](/chartgen/guides/web/#toolbar), [ADR-0030](/chartgen/decisions/0030-custom-chart-types/) |
| Renko, Kagi, Line break, Point & figure | ✅ | ❌ | Not offered. |
| Range bars and other non-time bars | ✅ | ❌ | Not offered. |

## Indicators

| Feature | TradingView | chartgen | Notes / links |
|---|---|---|---|
| Built-in indicator library | ✅ (100+) | ◐ | 72 registered indicators: overlays, oscillators, volume and external data. [Indicator reference](/chartgen/reference/indicators/) |
| Pine v5 value semantics and TradingView defaults | ✅ | ◐ | Values equal TradingView's for the same parameters and first bar. Known differences: UTC calendar periods, pivots from loaded bars, intrabar profiles approximated, long warmups. [ADR-0018](/chartgen/decisions/0018-pine-semantics-for-indicators/), [differences list](/chartgen/reference/indicators/#values-match-tradingview) |
| Session VWAP and VWAP bands | ✅ | ◐ | Resets per session by default, but sessions are UTC days, not exchange sessions. [ADR-0018](/chartgen/decisions/0018-pine-semantics-for-indicators/) |
| Higher-timeframe pivots | ✅ | ◐ | Day, week, month, year periods. Only P, R1, S1, R2, S2 are drawn, for every period. [Pivots](/chartgen/reference/indicators/#pivot) |
| Indicator picker | ✅ | ✅ | By name, server default parameters. [Managing indicators](/chartgen/guides/web/#managing-indicators) |
| Settings dialog with Inputs and Style tabs | ✅ | ◐ | The Inputs form is generated from the server schema (numbers, switches, text; no enums). The Style tab sets visibility, color, opacity, width and line style. No per-timeframe visibility tab. [ADR-0017](/chartgen/decisions/0017-indicator-plot-styles/) |
| Hide and remove from the pane legend | ✅ | ✅ | [Managing indicators](/chartgen/guides/web/#managing-indicators), [ADR-0013](/chartgen/decisions/0013-indicator-visibility-and-instances/) |
| Several instances of one indicator (RSI 14 and RSI 21) | ✅ | ✅ | Each with its own params, style, visibility and pane. [Managing indicators](/chartgen/guides/web/#managing-indicators), [ADR-0028](/chartgen/decisions/0028-multiple-indicator-instances/) |
| Fills, hlines, volume profile, divergence lines | ✅ | ✅ | Server colors on the wire. The VPVR up/down split cannot be colored. [ADR-0007](/chartgen/decisions/0007-complete-series-serialization/), [ADR-0010](/chartgen/decisions/0010-overlay-geometry-rendering/), [ADR-0012](/chartgen/decisions/0012-geometry-colors-on-the-wire/) |
| Ichimoku cloud | ✅ | ◐ | Cloud is drawn over the bars; the projection past the last bar is not drawn yet. [Differences list](/chartgen/reference/indicators/#values-match-tradingview) |
| Geometry colors editable in Style | ✅ | ❌ | Fill, profile and divergence colors are not styleable. [ADR-0017](/chartgen/decisions/0017-indicator-plot-styles/) |
| Indicator on indicator (apply to another indicator's output) | ✅ | ❌ | Every indicator reads the price bars. |
| Indicators on Heikin Ashi values | ✅ | ✅ | [`chart_style`](/chartgen/reference/mcp/) |
| Profiles from lower-timeframe data | ✅ | ◐ | Volume and time profiles and CVD spread each bar's volume over its range. [Differences list](/chartgen/reference/indicators/#values-match-tradingview) |
| Volume in the default set | ✅ | ✅ | A fresh install starts with the EMA stack, Volume and RSI. Stored sets are never changed. [Managing indicators](/chartgen/guides/web/#managing-indicators) |
| Persisted indicator set | ✅ (cloud) | ◐ | Saved in `localStorage` per browser, not synced. [ADR-0005](/chartgen/decisions/0005-client-indicator-state/) |

## Drawing tools

| Feature | TradingView | chartgen | Notes / links |
|---|---|---|---|
| Trend line, ray, extended line | ✅ | ✅ | [Drawing tools](/chartgen/guides/web/#drawing-tools) |
| Horizontal line and ray, vertical line | ✅ | ✅ | |
| Parallel channel | ✅ | ✅ | |
| Fib retracement | ✅ | ✅ | |
| Rectangle | ✅ | ✅ | |
| Text annotation, price label | ✅ | ✅ | |
| Magnet (off, weak, strong), lock all, hide all, remove all | ✅ | ✅ | |
| Undo and redo | ✅ | ◐ | `Ctrl+Z` undoes, `Ctrl+Shift+Z` / `Ctrl+Y` redoes, 100 steps. Drawings only; resets on symbol switch. [Drawing tools](/chartgen/guides/web/#drawing-tools) |
| Per-drawing styling (color, width, line style) | ✅ | ✅ | Floating style bar; rectangle fill and text color/size too. Missing vs TradingView: style templates, extend-left/right and arrow options, text bold/italic/background, per-timeframe visibility. [Drawing tools](/chartgen/guides/web/#drawing-tools) |
| Gann, pitchfork, Elliott and chart patterns, measure, other fib tools | ✅ | ❌ | Only the eleven tools above. |
| Drawings stored per symbol | ✅ | ◐ | `localStorage`, per browser, max 500 per symbol. Shared across layouts. [Drawing tools](/chartgen/guides/web/#drawing-tools) |

## Data & live updates

| Feature | TradingView | chartgen | Notes / links |
|---|---|---|---|
| Live forming candle, crypto | ✅ | ✅ | Server push over MCP SSE, up to 4 per second, polling fallback. [ADR-0021](/chartgen/decisions/0021-realtime-chart-push/), [Live updates](/chartgen/guides/web/#live-updates-and-history) |
| Live forming candle, stocks and others | ✅ | ◐ | Polled every 15 s, and Yahoo data is delayed. [Live updates](/chartgen/guides/web/#live-updates-and-history) |
| Scroll-back history paging | ✅ | ◐ | 500 bars per page, capped at 20,000 loaded bars. [ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/) |
| Seamless indicator values at page seams | ✅ | ◐ | The server warms each request up until EMAs converge (EMA 200 within 0.01%). Running sums (OBV, CVD, A/D) can differ by a constant, VWAP until its next reset, and EMAs longer than 224 (RMAs longer than 126) still step slightly. [ADR-0032](/chartgen/decisions/0032-indicator-aware-warmup/) |
| Volume profiles on live bars | ✅ | ◐ | Keep the profile of the full load. Divergence lines that form live usually appear on the next full load. [Live updates](/chartgen/guides/web/#live-updates-and-history) |
| Extended hours and exchange sessions | ✅ | ❌ | Not described; sessions are UTC calendar periods. |
| Real-time stock quotes | ✅ | ❌ | Yahoo is delayed. |

## Watchlist & alerts

| Feature | TradingView | chartgen | Notes / links |
|---|---|---|---|
| Watchlist with last, change, change % | ✅ | ◐ | Polled every 5 s for the active list, max 50 symbols per list. Crypto change is rolling 24 h. [Watchlist](/chartgen/guides/web/#watchlist), [ADR-0011](/chartgen/decisions/0011-watchlist-quotes/) |
| Multiple named watchlists, sections, flags | ✅ | ◐ | Named lists (create, rename, duplicate, delete), collapsible sections, drag and Alt+Arrow reordering, seven flag colors and one Flagged view across lists. No separate list per flag color, no nested sections, no sync across devices; at most 20 lists and 20 sections per list. [Watchlist](/chartgen/guides/web/#watchlist), [ADR-0031](/chartgen/decisions/0031-multiple-watchlists/) |
| Price alerts (crossing, up, down, greater, less) | ✅ | ◐ | Same conditions, once or every time, toast, beep and browser notification. Evaluated in the browser. [Sidebar and price alerts](/chartgen/guides/web/#sidebar-and-price-alerts), [ADR-0020](/chartgen/decisions/0020-client-side-price-alerts/) |
| Alerts that fire with no tab open | ✅ | ❌ | Fire only while chartgen is open in a visible tab. The server alert tools exist for `--trade` users but are not wired to the web app. [ADR-0020](/chartgen/decisions/0020-client-side-price-alerts/) |
| Alerts synced across devices | ✅ | ❌ | Stored per browser profile. |
| Indicator and drawing alerts, webhooks, email, SMS | ✅ | ❌ | Price alerts only in the web app. [Push notifications](/chartgen/guides/notifications/) covers the server side. |
| Alert lines on the chart (drag to move) | ✅ | ✅ | [Sidebar and price alerts](/chartgen/guides/web/#sidebar-and-price-alerts) |

## Layouts & settings

| Feature | TradingView | chartgen | Notes / links |
|---|---|---|---|
| Named layouts (save, copy, rename, load, delete) | ✅ | ◐ | Symbol, timeframe, chart type, scale, compare symbols and indicators. Drawings are not part of a layout. Stored in `localStorage`, max 50. [Layouts](/chartgen/guides/web/#layouts), [ADR-0015](/chartgen/decisions/0015-named-layouts/) |
| Layout export and import | ✅ | ✅ | JSON file, validated on import. [Layouts](/chartgen/guides/web/#layouts) |
| Cloud sync of layouts and settings | ✅ | ❌ | [ADR-0005](/chartgen/decisions/0005-client-indicator-state/) |
| Shareable chart links | ✅ | ◐ | Query-param links for symbol, timeframe, chart type, indicators, compare symbols and time zone; no drawings, no short links. [Sharing a chart](/chartgen/guides/web/#sharing-a-chart), [ADR-0029](/chartgen/decisions/0029-shareable-chart-links/) |
| Chart settings (colors, grid, scales) | ✅ | ◐ | Saved in `localStorage` under `chartgen.chartSettings`, not synced and not in named layouts. [Chart settings](/chartgen/guides/web/#chart-settings), [ADR-0019](/chartgen/decisions/0019-chart-settings/) |
| Keyboard shortcuts | ✅ | ◐ | A documented subset with `Alt` modifiers for tools. [Keyboard shortcuts](/chartgen/guides/web/#keyboard-shortcuts) |

## Analysis tools

| Feature | TradingView | chartgen | Notes / links |
|---|---|---|---|
| Compare symbols | ✅ | ◐ | Up to five lines, percent scale. No separate-scale overlay. [Compare symbols](/chartgen/guides/web/#compare-symbols), [ADR-0022](/chartgen/decisions/0022-compare-symbols/) |
| Bar replay | ✅ | ◐ | Pick a bar or date, step, play at 1, 3 or 10 bars per second. Repainting indicators are recomputed on the server and lag while playing; only the last 500 bars before the cursor are recomputed. [Bar replay](/chartgen/guides/web/#bar-replay), [ADR-0023](/chartgen/decisions/0023-bar-replay/) |
| Data Window | ✅ | ✅ | OHLC, change, volume, each plot value, compare symbols. [Data Window](/chartgen/guides/web/#data-window) |
| Object Tree | ✅ | ✅ | Indicators, drawings, compare symbols and alerts with hide, lock and remove. [Object Tree](/chartgen/guides/web/#object-tree) |
| Paper trading and orders from the chart | ✅ | ❌ | Order entry is out of scope of the web app. The server has a `--trade` engine, see [Trading](/chartgen/guides/trading/). |
| Strategy tester and backtesting | ✅ | ❌ | Not offered. |
| Symbol fundamentals, financials, news | ✅ | ❌ | Not offered. |

## Scripting

| Feature | TradingView | chartgen | Notes / links |
|---|---|---|---|
| Pine Script indicators | ✅ | ❌ | Built-in indicators are Rust ports of Pine v5 semantics. No Pine interpreter. [ADR-0018](/chartgen/decisions/0018-pine-semantics-for-indicators/) |
| Pine editor UI | ✅ | ❌ | No script editor in the web app, and no Pine parser. |
| Scripted indicators via Starlark over MCP | n/a | ◐ | `register_indicator` and `generate_chart` accept your scripts. Phase 1: feature-flagged off by default (`CHARTGEN_SCRIPTING=1`), for trusted or single-user deployments only, since scripts run in the server process, ownership is an auto-approved OAuth client, and the limits are not a security boundary. At most 3 scripts per chart. Scripts are not computed in `subscribe_chart` pushes. [Scripted indicators](/chartgen/guides/scripted-indicators/), [ADR-0024](/chartgen/decisions/0024-starlark-script-runtime/), [ADR-0025](/chartgen/decisions/0025-user-indicator-registry/) |
| Pine-like series operators and `ta.*` functions in scripts | ✅ | ✅ | Element-wise operators, `close[1]`, comparisons, `ta.sma/ema/rsi/atr/macd/crossover` and more, with the Pine v5 code the built-ins use. Pivot tie rule is unverified against TradingView. [Series](/chartgen/guides/scripted-indicators/#series), [`ta.*`](/chartgen/guides/scripted-indicators/#ta-functions) |
| Fork a built-in as a script | ✅ (open source scripts) | ◐ | `fork_indicator` works for `wavetrend` only. [Forking](/chartgen/guides/scripted-indicators/#forking-a-built-in) |
| Script editor in the web app | ✅ | ❌ | No editor UI; scripts are registered over MCP. |
| Programmatic access (API, agents) | partial | ✅ | The MCP server exposes 19 tools, including chart generation, quotes and symbol search. [MCP reference](/chartgen/reference/mcp/) |

## Data sources

| Feature | TradingView | chartgen | Notes / links |
|---|---|---|---|
| Crypto | ✅ | ◐ | Binance spot only. [Toolbar](/chartgen/guides/web/#toolbar) |
| Stocks, ETFs, indices, FX, futures | ✅ | ◐ | Yahoo Finance, delayed. Exchange is guessed from the ticker suffix. |
| Other exchanges and data vendors | ✅ | ❌ | Only Binance and Yahoo. |
| Broker connections and trading from the chart | ✅ | ❌ | Not in the web app. See [Trading](/chartgen/guides/trading/) for the server engine. |
| Derivatives and sentiment series | partial | ✅ | `funding`, `oi`, `long_short`, `fear_greed`, `cvd` indicators. [Indicator reference](/chartgen/reference/indicators/) |

## Known differences

Deliberate deviations recorded in the ADRs. Each is a trade-off, not an
oversight.

- **Alerts run in the browser.** They fire only while chartgen is open in a
  visible tab, are per browser profile and do not sync. A crossing that
  happens and reverts while the tab is asleep is lost.
  [ADR-0020](/chartgen/decisions/0020-client-side-price-alerts/)
- **Crypto watchlist change is the rolling 24 hours** that Binance reports,
  not the change since the daily open. Stocks, ETFs, indices and FX change
  against the previous close, like TradingView.
  [ADR-0011](/chartgen/decisions/0011-watchlist-quotes/)
- **Push is capped per client address, and behind a proxy that is one
  address.** chartgen trusts no forwarding header, so behind a reverse proxy
  or tunnel every client shares the proxy's address and the cap of 8
  subscriptions becomes a second server-wide cap. Fine for a personal
  deployment. Other symbols poll.
  [ADR-0021](/chartgen/decisions/0021-realtime-chart-push/)
- **The NaN rule follows the Pine reference.** Warmup bars are `null`, not
  zero and not a partial window. Window functions skip missing values.
  EMA and RMA keep state across a missing value; that case is not yet
  verified against a live TradingView chart. Values changed for existing
  users when this landed.
  [ADR-0018](/chartgen/decisions/0018-pine-semantics-for-indicators/)
- **Calendar periods are UTC.** VWAP anchors and pivots use UTC days, ISO
  weeks, months and years, not exchange sessions. Pivots use the loaded
  bars as the higher-timeframe source.
  [Differences list](/chartgen/reference/indicators/#values-match-tradingview)
- **Running sums depend on where the series starts.** OBV, CVD and the A/D
  line can differ by a constant at a history-page seam or on a live bar,
  VWAP until its next session reset.
  The built-in EMA- and RMA-based indicators (EMA, RSI, ATR, MACD, ...) are
  warmed up until they converge, up to 1000 bars, so only EMAs longer than
  224 and RMAs longer than 126 still step slightly, as can the community
  composites (Cipher B, WaveTrend).
  [ADR-0032](/chartgen/decisions/0032-indicator-aware-warmup/)
- **History is capped at 20,000 bars**, because every live update
  recalculates the indicators over all loaded bars.
- **Bar replay recomputes only the last 500 bars** before the cursor for
  repainting indicators, and their newest bars lag while playing.
  [ADR-0023](/chartgen/decisions/0023-bar-replay/)
- **Compare costs up to five extra requests per poll.** It relies on
  klinecharts' percent axis behavior, pinned by a unit test.
  [ADR-0022](/chartgen/decisions/0022-compare-symbols/)
- **Client state is local.** Indicators, layouts, drawings, alerts and the
  watchlist live in `localStorage`: no sync, links carry only the
  chart setup, and clearing browser data wipes them.
  [ADR-0005](/chartgen/decisions/0005-client-indicator-state/)
- **Visual parity drifts.** TradingView changes its look over time, so the
  match is periodically revisited.
  [ADR-0006](/chartgen/decisions/0006-tradingview-visual-parity/)
