---
title: Web frontend
description: SolidJS browser SPA for chartgen — one screen, OAuth 2.1 PKCE login, Streamable HTTP MCP client.
---

The `web/` directory holds a SolidJS single-page app that connects to
`chartgen --serve` via MCP Streamable HTTP and renders a candlestick
chart with an RSI panel. It implements
[ADR-0003](/chartgen/decisions/0003-web-frontend-stack/).

The scope is deliberately narrow: a top toolbar (symbol, timeframe,
chart type), the indicator picker, the chart and a watchlist. Order
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

- **Replay** (rewind) — starts and ends [bar replay](#bar-replay).
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

Symbol, timeframe, chart type, price-scale mode, theme, time zone and recent symbols are saved in
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
  interval and the badge is hidden. At the close it disappears until the
  live update brings the next bar (a moment with push, up to one 2 s poll
  period without), and it hides when the last price is scrolled out of view.
- **Price scale** — the `%`, `log` and `auto` buttons in the bottom-right
  corner of the price axis. `%` shows percent change from the first
  visible bar and `log` a logarithmic scale; they are mutually exclusive
  and clicking the active one returns to the normal scale. `auto` (on by
  default) keeps the axis fitted to the visible bars. Dragging the price
  axis scales it by hand and turns `auto` off; double-clicking the axis or
  clicking `auto` turns it back on, and `Alt+R` (reset chart) does too.
  The scale mode persists across reloads; `auto` always starts on.

## Bottom bar

The bar under the chart has TradingView's date-range buttons, a **Go to
date** button, a clock and the time zone selector.

- **Date ranges** — `1D 5D 1M 3M 6M YTD 1Y 5Y All`. A range switches to
  TradingView's interval for it, loads older history until the range start
  is covered, then fits the range into the width and scrolls to the newest
  bar.

  | Range | Interval | Range | Interval |
  |---|---|---|---|
  | 1D | 1m | YTD | 1d |
  | 5D | 5m | 1Y | 1d |
  | 1M | 30m | 5Y | 1wk |
  | 3M | 1h | All | 1mo |
  | 6M | 4h | | |

  TradingView uses 2h for 6M; chartgen has no 2h timeframe. The range ends
  at the newest bar. Month ranges are calendar months, and YTD starts on
  1 January 00:00 in the selected time zone. A range with more bars than
  pixels is shown at the minimum bar width, so only its newest part is
  visible. Paging respects the history limit (see below); "All" loads as
  much as the limit allows.
- **Time zone** — the dropdown offers **UTC**, **Exchange**, **Local** (the
  browser's zone) and a short list of common IANA zones. It sets the time
  axis labels, the crosshair time label, the time in the price legend and
  the clock beside it (shown with its UTC offset). The choice persists
  across reloads and is part of a named layout. The default is Local.
  The series payload carries no exchange, so **Exchange** is a heuristic
  on the symbol: crypto and `=X` pairs are UTC, `.DE` is Europe/Berlin,
  `.L` Europe/London, `.TO` America/Toronto, and everything else is
  treated as America/New_York.
- **Go to date** — the calendar button or `Alt+G` opens a dialog with a date
  and an optional time (read in the selected time zone). OK pages history
  back if the date is older than the loaded bars, then centers the nearest
  bar. A date before the start of history goes to the first bar and says so
  in the bar; at the history limit it goes to the oldest loaded bar.

## Compare symbols

The **Compare** button (two overlapping circles) in the toolbar opens the
symbol search; pick a symbol and it is drawn as a line over the price
series. Add up to five. The first one switches the price axis to percent,
so every line starts from the same point: the axis shows each symbol's own
percent change since the first visible bar, and the lines move with scroll
and zoom. Removing the last compare symbol restores the scale mode you had
before, unless you picked another one meanwhile. Outside percent mode the
lines stay anchored at the first loaded bar.

Each compare symbol has a legend row under the indicator rows with its
name, its close (at the crosshair bar, or the last bar at rest), the percent
change since the first visible bar, and a **×** on hover to remove it. The
lines follow the main chart: the same loaded range, older history when you
scroll back, the live tail, and a refetch on a timeframe change. A compare
symbol has bars on its own schedule, so each main bar shows the last close
of a bar that opened before the main bar ended (a daily Binance bar matches the
Yahoo bar of the same session). Each symbol keeps its color until removed, and
choosing a compare symbol as the main symbol removes it from the list.

The compare list is saved with the rest of the chart state
(`chartgen.layout.compare`) and is part of a [layout](#layouts). See
[ADR-0022](/chartgen/decisions/0022-compare-symbols/).

## Bar replay

The **Replay** button (rewind icon) in the toolbar replays the chart from a
past bar, as TradingView's Bar Replay does. Live updates stop while
replaying, and a control bar floats over the bottom of the chart.

1. **Pick the start.** After **Replay**, a vertical line follows the mouse
   and dims the bars to its right. Click a bar to start there; dragging still
   scrolls the chart and the wheel zooms, so you can scroll back first.
   Picking cancels an armed drawing tool and never places a drawing point. Or use the calendar
   button (**Select date**) to start at the bar of a date and time, read in
   the bottom bar's time zone; older history is loaded first if needed.
   **Select bar** picks a new start at any time.
2. **Replay.** Bars after the start are hidden. **Forward** (`Shift+→`)
   adds the next bar. **Play** / **Pause** (`Shift+↓`) adds bars at the
   speed of the `1x` menu: 1, 3 or 10 bars per second. Playback stops at
   the newest loaded bar.
3. **Back to live.** **Jump to real time**, the **×** or `Esc` shows the
   whole chart again and restarts live updates, which catch up the bars
   that formed meanwhile. With a drawing tool armed, `Esc` first cancels
   the tool.

While replaying:

- Indicators show what they showed at the replay bar. Most are cut at it.
  Those whose values change with later bars are recomputed on the server up
  to the replay bar: volume and time profiles (`volume_profile`/VPVR,
  `session_vp`, `hvn_lvn`, `naked_poc`, `tpo`), ZigZag, the linear
  regression channel and Ichimoku. While playing that happens at most twice a
  second and again on pause, so these lag a little and their newest bars are
  briefly blank. Divergence lines and signals appear once they are
  confirmed: Cipher B's two bars after the bar they mark. A moving average
  with a negative offset leaves its newest values blank.
- Scrolling back still loads older history. Drawings work as usual; ones
  placed after the replay bar are drawn past the last bar. Compare lines
  are cut at the replay bar too, and the Data Window shows the replay bar
  (or the one under the crosshair), never a later one.
- Changing the symbol, timeframe or indicators keeps the replay time: the
  new chart shows every bar that had closed by then, loading older history
  if needed. If there is none, the chart stays empty and the replay bar says
  so (or that the history failed to load).
- Price alerts keep checking live prices (the chart symbol is checked through
  the watchlist quotes meanwhile) and never fire on replayed bars.

See [ADR-0023](/chartgen/decisions/0023-bar-replay/).

## Live updates and history

The chart stays live while the page is open, as in TradingView
([ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/),
[ADR-0021](/chartgen/decisions/0021-realtime-chart-push/)). Binance symbols
are pushed by the server and tick several times a second; other symbols are
polled.

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
- **Status.** A badge after the status text shows `live` (server push),
  `live (polling)`, `paused` (tab hidden), `live: retry in Ns` after a
  failed poll, or `resyncing...` while the chart reloads to catch up.

How it works: the first load asks `generate_chart` for 500 bars with a
`warmup` of 300. The app then polls the last two bars (`bars: 2`,
`warmup: 300`) and merges them in by timestamp. A history page is
`bars: 500, warmup: 300, end_time: <oldest bar × 1000>`. 800 bars fit in
one Binance request, and a warmup of 300 covers the usual lookbacks (up to
SMA/EMA 200) so values at a page seam line up. Changing the symbol,
timeframe or indicators cancels every pending poll and page and does a
full reload.

**Push.** After each full load the app calls `subscribe_chart` with the
same arguments and `warmup: 300`. For a Binance symbol the server then
pushes the same two-bar tail over the MCP SSE stream (`GET /mcp`) whenever
the market moves, at most 4 times a second, and the app merges it exactly
like a polled tail. Polling stops while pushes arrive. The server answers
"not pushable" for other symbols, which simply keep polling, and "push
limit reached" when a server limit (subscriptions per token or address,
subscribes per minute, at most 10 indicators) refuses the chart, which
then polls until the next chart change.

- **Fallback.** No push for 6 s (three times the 2 s Binance kline
  cadence), or a dropped SSE stream, which ends the subscription on the
  server, puts the chart back on polling at once. The app then retries the
  subscription after 2 s, doubling up to 60 s, and stops polling again once
  pushes resume. A failed subscribe backs off the same way.
- **Chart changes.** Changing the symbol, timeframe, indicators or chart
  type unsubscribes and subscribes the new chart.
- **Hidden tab.** A hidden tab unsubscribes, as it stops polling, and
  subscribes again (after a catch-up poll) when visible. A background tab
  costs the server nothing.
- **Gaps.** A pushed tail that does not line up with the chart (bars
  missed while the stream stalled) triggers one poll, which catches up or
  reloads as described below. Any other poll that lands while pushes are
  on is dropped, so it cannot put an older value on the forming candle.

| | 24/7 symbols (Binance rule) | Other symbols (Yahoo) |
|---|---|---|
| Updates | pushed (polled every 2 s as fallback) | polled every 15 s |
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
| `Alt+G` | Go to date |
| `Alt+A` | Add a price alert at the last price |
| `Shift+→` | Bar replay: forward one bar |
| `Shift+↓` | Bar replay: play / pause |
| `Esc` | Bar replay: jump to real time |

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

## Chart settings

The gear button in the top toolbar (or **Settings…** in the chart context
menu) opens the chart settings dialog with four tabs:

- **Symbol** — body, borders and wick of the candles, each with an on/off
  checkbox and an up and a down color, plus the line/area color. Border and
  wick follow the body color until you pick one for them.
- **Status line** — show or hide the OHLC values, the bar change, the volume
  and the indicator values in the legends.
- **Scales** — last price line, last price label, high and low price labels,
  and the price scale placement (right or left).
- **Appearance** — background color, vertical and horizontal grid lines and
  grid color, crosshair style (dashed, dotted, solid) and color.

Changes preview live on the chart. **OK** keeps them, **Cancel** (or `Esc`)
reverts, and **Reset settings** puts the options and the current theme's colors
back to the defaults (still previewed until OK; the other theme is untouched). Colors and the candle part toggles are stored per
theme, so the light and dark themes can look different; the other options are
shared. Everything is saved in `localStorage` under `chartgen.chartSettings`
and validated on load. Settings are not part of named layouts. See
[ADR-0019](/chartgen/decisions/0019-chart-settings/).

## Chart context menu

Right-clicking the chart (outside a drawing) opens a menu:

| Item | Action |
|---|---|
| Reset chart view (`Alt+R`) | Scroll to real time at the default zoom |
| Copy link to chart | Copy a [shareable link](#sharing-a-chart) |
| Copy price | Copy the price under the cursor, in the chart's precision |
| Add horizontal line at | Add a horizontal line at that price; it is saved like any drawing |
| Add alert at | Open the alert dialog at that price |
| Remove all drawings | Asks for confirmation |
| Remove all indicators | Asks for confirmation |
| Settings… | Open the chart settings dialog |

Copy price, Add horizontal line and Add alert only appear when the click is in the price
pane. Right-clicking a drawing opens the drawing menu instead. TradingView's
"Lock vertical cursor line by time" is not offered because klinecharts has no
pinned crosshair.

## Sharing a chart

The link icon in the toolbar ("Copy link to chart", also in the chart context
menu) puts a URL on the clipboard and shows a "Link copied" toast. Opening it
reproduces the chart:

```text
https://chartgen.vasudev.xyz/?symbol=ETHUSDT&interval=4h&type=candles&ind=rsi(length:21),ema_stack&compare=BTCUSDT&tz=UTC
```

| Parameter | Meaning |
|---|---|
| `symbol` | Chart symbol |
| `interval` | Timeframe (`4h`, `240`, `1d`, ...) |
| `type` | Chart type: `candles`, `hollow`, `bars`, `line`, `area`, `heikin_ashi` |
| `ind` | Indicators, comma separated. Non-default parameters go in parentheses, separated by `;`: `macd(fast:8;slow:20)`. Visible indicators only; `ind=` means none |
| `compare` | Compare symbols, comma separated |
| `tz` | Time zone (`utc`, `exchange`, `local` or an IANA name) |

Every parameter is optional. The app reads them once on load, saves them like a
manual change and removes them from the address bar. An invalid value (unknown
interval, empty symbol) is ignored and the stored value stays, and indicator
names the server does not know are dropped. A link carries the chart setup only,
not drawings, alerts, colors or indicator styles. The parameters survive the
sign-in redirect. If the browser blocks the clipboard API (plain HTTP), the link
is copied through a hidden text field instead.
[ADR-0029](/chartgen/decisions/0029-shareable-chart-links/)

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

The right sidebar has four icon tabs, **Watchlist**, **Alerts**, **Data
Window** and **Object Tree**; the tab you last used is remembered, and **»**
collapses the whole sidebar. The Alerts tab shows the number of active alerts. The watchlist polls quotes only while
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

### Data Window

The **Data Window** tab lists everything the legend knows about one bar: the
symbol and interval, the bar's date and time, open, high, low, close, change
(absolute and percent) and volume, then one section per visible indicator with
each plot's value in its plot color (plots hidden in the Style tab are left
out), then the compare symbols with their value and percent change. The values
follow the crosshair and show the last bar while the crosshair is off the
chart. The time uses the time zone chosen in the bottom bar; daily, weekly
and monthly bars show the date only.

### Object Tree

The **Object Tree** tab lists what is on the chart:

- **Main series**: the symbol and interval.
- **Compare**: each compare symbol, with remove.
- **Indicators**: hide / show, settings and remove, like the legend controls.
- **Drawings**: each with a short description (for example
  `Trend line 2026-11-03 → 11-04`) and hide / show, lock / unlock and remove.
  Clicking a row scrolls the chart to the drawing if it is out of view and
  selects it, so Delete removes it.
- **Alerts**: the chart symbol's alerts; clicking one opens it for editing.

## Layouts

A layout is a named bundle of symbol, timeframe, chart type, scale mode,
compare symbols and the indicator set (including hidden and edited ones). Drawings are
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

The app removes `?code=…&state=…` from the URL as soon as it reads
them, so a refresh never replays a used code. If the token exchange
fails (for example an expired code or a network error), the app starts
the flow over once by itself. If that fails too, the status line shows
`sign-in failed: <reason>` with a **Sign in again** button, and the
chart does not load until you use it. The app never redirects in a
loop.

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

Lines and histograms are drawn in the colors of the PNG renderer, which
the server sends with the series. Line widths come from the server too
([ADR-0026](/chartgen/decisions/0026-line-and-histogram-colors/)).
Histograms are colored per bar, as on TradingView: Volume is green on up
candles and red on down candles, MACD green above zero and red below,
and the Awesome Oscillator green while rising. The legend and the Data
Window show a histogram's value in the color of that bar, and label it
by name (`Volume`, MACD's `Histogram`). A color set in the Style tab
wins over the server's. For a histogram it recolors every bar, and a
width set there wins over the server's line width. Clicking **Defaults**
goes back to the server's colors. With a server older than the web app
the chart falls back to its own rotating palette.

A fresh install starts with the EMA stack, Volume and RSI, like a new
TradingView chart with its Volume pane. Stored sets are never changed.

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

The production deployment serves the app from chartgen itself
([ADR-0027](/chartgen/decisions/0027-serve-web-app-same-origin/)). The
root `Dockerfile` builds `web/dist` with an empty `VITE_CHARTGEN_BASE_URL`,
copies it into the image and sets `CHARTGEN_WEB_DIR`, so one container
serves both on the same origin:

- `GET /` from a browser (`Accept: text/html`) returns the app, as does
  the OAuth callback `/?code=…`. Other `GET /` requests keep returning
  the JSON health document.
- `/mcp`, `/register`, `/authorize`, `/token`, `/favicon.*` and the three
  explicit `/.well-known/oauth-authorization-server`,
  `/.well-known/oauth-protected-resource` and
  `/.well-known/oauth-protected-resource/mcp` routes are the server's own
  and take precedence over static files.
- `CHARTGEN_WEB_DIR` must contain only the build output. Dotfiles and
  paths starting with `//` are refused with 404; symlinks are followed,
  so do not put any in the directory.
- Other browser paths that match no file return `index.html`.
- `/assets/*` (content-hashed) is cached as `immutable`, and
  `index.html` as `no-cache`.

To serve a local build the same way:

```bash
(cd web && npm run build)
CHARTGEN_WEB_DIR=web/dist cargo run -- --serve   # open http://localhost:9315/
```

The bundle also works on any static host (CDN, object store, Cloudflare
Pages). Set `VITE_CHARTGEN_BASE_URL=https://chartgen.example.com` at build
time when the API is on a different origin. Leave it unset for
same-origin hosting; the app then calls `/mcp`, `/register`, `/token`
etc. on the current origin.

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
  applies pushed tails or polls the tail, and pages in history. The merge rules live in
  `web/src/lib/seriesMerge.ts` (unit-tested): bars and every per-bar
  channel are joined by timestamp, signals and divergence lines are
  re-indexed, and anything that does not line up (another symbol or
  interval, a gap, overlapping or stale bars, a different indicator
  shape) is rejected and leads to a longer tail or a full reload.
- **[`@modelcontextprotocol/sdk`](https://www.npmjs.com/package/@modelcontextprotocol/sdk)**
  with `StreamableHTTPClientTransport`. Bearer tokens flow through
  `requestInit.headers`; `fallbackNotificationHandler` receives the frames
  of the GET SSE stream. `notifications/chart_tick` is validated with Zod
  and handed to the live feed; `notifications/alert_triggered` is only
  logged (dormant in v0.1).
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
    payload says so. Each line carries a server-assigned color
    ([ADR-0012](/chartgen/decisions/0012-geometry-colors-on-the-wire/)); only for older servers that omit it
    is the kind inferred from the slope (bullish green, bearish red).

  Colors come from the wire
  ([ADR-0012](/chartgen/decisions/0012-geometry-colors-on-the-wire/));
  the per-indicator themes above are the fallback for servers that send
  none. The geometry code is pure TypeScript in
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
