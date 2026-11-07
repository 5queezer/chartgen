---
title: Web frontend
description: SolidJS browser SPA for chartgen — one screen, OAuth 2.1 PKCE login, Streamable HTTP MCP client.
---

The `web/` directory holds a SolidJS single-page app that connects to
`chartgen --serve` via MCP Streamable HTTP and renders a candlestick
chart with an RSI panel. It implements
[ADR-0003](/chartgen/decisions/0003-web-frontend-stack/).

The Docker image serves the app at `/`, on the same origin as `/mcp`
([ADR-0027](/chartgen/decisions/0027-serve-web-app-same-origin/)). It has a
top toolbar (symbol, timeframe including custom intervals, chart type), the
indicator picker, drawings, a watchlist, price alerts, layouts, a
[grid of charts](#chart-grid) and a compact layout for phones. With
`CHARTGEN_SCRIPTING=1` it also has a [script editor](#script-editor), and with
`CHARTGEN_SERVER_ALERTS=1` alerts can run on the server. Order entry and LLM
chat are out of scope.

## Toolbar

The chart loads automatically as soon as sign-in finishes and reloads
whenever the symbol, timeframe, or indicator set changes. The previous
chart stays on screen while the new one loads. There is no Load button.

If a load fails (a mistyped symbol, a provider outage), the header shows a
compact red `error` badge (the full message is its tooltip) and the chart
area shows "Couldn't load SYMBOL — symbol not found" with **Retry** and, when
another chart loaded before, **Back to PREVIOUS**. The failed symbol stays
active, as on TradingView, and is not saved to your recents.

The header has a fixed height and never scrolls the page sideways: 38 px with
a mouse, so the chart gets the height, and taller on a touch screen, where its
buttons stay comfortable to tap. As the window narrows, lower-priority items
collapse:

- below 1400 px the **Replay** and **Indicators** labels become icon-only;
- below 1280 px screenshot, copy link, chart settings and fullscreen move
  into the **⋯** menu, and the timeframe buttons collapse into the timeframe
  dropdown below 1200 px;
- below 900 px compare, reset chart and the theme toggle join the **⋯** menu;
- below 640 px the wordmark is dropped and **Bar replay** joins the menu too.

The status text stays on one line (the live badge reads `live`, `polling`,
`paused`...); the layout name truncates, and its unsaved-changes dot always
stays visible.

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
  with the active one highlighted (below 1200 px they all move into the
  dropdown, which then lists every timeframe). The dropdown holds the rest, grouped as
  on TradingView: Minutes (3m, 30m), Hours (2h, 6h, 12h) and Days (3D, 1M),
  then **Recent** with your last five custom intervals, and ends with
  **Custom interval...**, which opens the same popup as the keyboard entry (see
  [Keyboard shortcuts](#keyboard-shortcuts)). While a custom interval is
  active, the dropdown's button shows it.
- **Chart type** — Candles, Hollow candles (only up candles hollow), Bars (OHLC), Line, Line with markers, Step line, Area, Baseline, Columns, High-low, Heikin Ashi. Heikin Ashi is computed on the server (the web app sends `chart_style: "heikin_ashi"` with every load, tail poll and history page), so the legend shows Heikin Ashi OHLC values and indicators are calculated on them, as on TradingView. Switching to or from it reloads the chart. The other types are drawn in the browser from the same bars, so switching between them needs no reload, and the legend keeps showing the bar's OHLC:
  - **Line with markers** — the close line with a dot on every bar.
  - **Step line** — the closes joined by horizontal-then-vertical steps.
  - **Baseline** — the close line and fill, in the up color above a base
    level and the down color below it. The base level is the middle of the
    visible close range (TradingView's default of 50%) and follows the view.
  - **Columns** — a column from the bottom of the pane to each close, in
    the up color when the bar closed at or above the previous close, else
    in the down color.
  - **High-low** — one bar from each bar's low to its high, no open or close.

  Line with markers, Step line and High-low use the line color of the
  [chart settings](#chart-settings); Baseline and Columns use the up and down
  body colors. Close-based types scale the price axis to the closes, High-low
  to the highs and lows
  ([ADR-0030](/chartgen/decisions/0030-custom-chart-types/)).

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
  last bar otherwise. Price precision is derived from the data (float noise
  such as `385.19000244` is ignored, so equities get two decimals). Overlay
  indicator values in the legends and the Data Window use the same price
  precision; oscillators keep their own, and volume-type values and axes use
  `K`/`M`/`B` with at most two decimals. Legend rows have a translucent
  background, and light plot colors are darkened on a light theme so the
  text stays readable.
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
  The scale mode persists across reloads; `auto` starts on, unless the
  price-to-bar ratio is locked.
  `inv` (or `Alt+I`) inverts the price pane's axis so higher prices are at
  the bottom (indicator panes keep their own direction); labels, crosshair,
  drawings and alert lines all follow. **Lock price to bar ratio** (chart
  context menu or Settings, Scales) keeps the candle aspect fixed while you
  zoom horizontally: the visible price range scales with the bar width, and
  when zoomed in the window slides to keep the last visible close in view.
  The ratio is taken when you turn the lock on, and again for a new symbol
  or timeframe; after a window resize it keeps the price range on screen.
  While compare symbols are shown the lock is paused and the axis fits every
  series, because a fixed price range would crop lines rebased to the first
  visible bar; the context menu item then reads "(paused while comparing)", and
  the lock resumes from a fresh fit when the last one is removed.
  Locking turns `auto` off; clicking `auto`, double-clicking the axis,
  dragging the axis or `Alt+R` releases the lock. While it is locked the
  price axis cannot be panned vertically (TradingView allows it). Invert
  and lock are saved per chart, like the scale mode, and are part of named
  layouts. "Scale price chart only" is not offered.

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
  | 6M | 2h | | |

  The range ends at the newest bar. Month ranges are calendar months, and YTD starts on
  1 January 00:00 in the selected time zone. A range with more bars than
  pixels is shown at the minimum bar width, so only its newest part is
  visible. Paging respects the history limit (see below); "All" loads as
  much as the limit allows.
- **Time zone** — the dropdown offers **UTC**, **Exchange**, **Local** (the
  browser's zone) and a short list of common IANA zones. It sets the time
  axis labels, the crosshair time label, the time in the price legend and
  the clock beside it (shown with its UTC offset). The choice persists
  across reloads and is part of a named layout. The default is Local.
  Daily, weekly and monthly bars show the date alone in the crosshair label.
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
  regression channel, Ichimoku, chart patterns and Auto Fib. While playing that happens at most twice a
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
  `polling`, `paused` (tab hidden), `live: retry in Ns` after a
  failed poll, or `resyncing...` while the chart reloads to catch up.

How it works: the first load asks `generate_chart` for 500 bars with a
`warmup` of 300. The app then polls the last two bars (`bars: 2`,
`warmup: 300`) and merges them in by timestamp. A history page is
`bars: 500, warmup: 300, end_time: <oldest bar × 1000>`. A warmup of 300
covers the usual lookbacks (up to SMA 200), and the server raises it for
indicators that need more to converge, such as the EMA 200 of `ema_stack`
(891 bars) or an RSI with a long length
([ADR-0032](/chartgen/decisions/0032-indicator-aware-warmup/)), so values
at a page seam and on the live bar line up. A custom interval on a Binance symbol can
fall short of that ([Keyboard shortcuts](#keyboard-shortcuts) has the limit). Changing the symbol,
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
then polls and tries push again every 5 minutes. "Feeds busy", when too many
feeds are starting across the server, also means polling, but the app tries
again sooner, with the backoff below.

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
  a page boundary starts at that boundary. Running sums (OBV, CVD, A/D
  line) can differ by a constant at a page seam or on the live bar
  compared with one long request, because each request starts its sum
  independently. VWAP can differ until its next session reset. The
  built-in EMA- and RMA-based indicators (EMA, RSI, ATR, MACD, ...) match
  it, except an EMA longer than 224 bars or an RMA longer than 126, which
  converge beyond the 1000-bar warmup cap; the community composites
  (Cipher B, WaveTrend) can still step slightly. Yahoo data is delayed, so its live bar moves only as
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
| `Alt+I` | Invert the price scale |
| `Alt+A` | Add a price alert at the last price |
| `Alt+↑` / `Alt+↓` | Watchlist: move the focused row or section up / down |
| `Shift+→` | Bar replay: forward one bar |
| `Shift+↓` | Bar replay: play / pause |
| `Esc` | Bar replay: jump to real time |

The timeframe popup accepts a number of minutes (`5`, `7`, `45`, `120`) or a
number with a unit `m`, `h`, `d`, `w` (`7m`, `45m`, `8h`, `4H`, `2D`, `2W`);
an uppercase `M` means month (`3M`), lowercase `m` minutes. A lone `D`, `W`
or `M` means one day, week or month. `1`, `2`, `0` then `Enter` selects 2h.
This works for Binance and Yahoo symbols alike. Lengths are bounded at under 1440
minutes, 168h, 365D, 52W and 12M, and above 60 minutes the length must be a round
one (`65m` works, `61m` does not). The field turns red for anything else, including
seconds, and says so. [MCP reference](/chartgen/reference/mcp/#generate_chart)
lists how bars line up and what each source builds them from.

A custom interval works like a preset everywhere: live updates, history paging,
bar replay, alerts on it, layouts and share links. On a stock, an interval built
from Yahoo's finest bars has as much history as they do: about 30 days for `7m`.

A custom interval on a Binance symbol computes indicators over at most 20,000 native
bars, however much warmup they ask for. A `59m` chart holds about 338 bars, while
EMA 200 needs 891 to converge, so its live values and page seams can differ from a
longer history. Scrolling back to load more does not change that: each page, poll and
push computes over its own capped read. [MCP reference](/chartgen/reference/mcp/#generate_chart)
gives the rule.

## Drawing tools

The vertical toolbar on the left of the chart holds the drawing tools in
six groups, one button each: **Lines** (trend line, ray, extended line,
horizontal line, horizontal ray, vertical line, parallel channel), **Fib**
(retracement, trend-based extension), **Pitchfork**, **Shapes and text**
(rectangle, text annotation, price label), **Measure** (price range, date
range, date and price range) and **Forecasting** (long and short position).
A group button shows the tool you used last and arms it on click. Open the
flyout with the small chevron at its corner, or press and hold the button,
to see every tool of the group by name; arrow keys move, `Enter` picks and
`Esc` closes it. The last tool of each group is remembered in the browser,
and the `Alt` shortcuts below update it too. Below the groups, always
visible, are a magnet toggle (off, weak, strong), lock all, hide all, undo,
redo and remove all (asks for confirmation). The button at the top switches
the cursor between crosshair and arrow.

A tool is armed for exactly one drawing: pick it, click the chart to
place the points, and the tool is released. Pick it again or press `Esc`
to cancel. Drawings pan and zoom with the chart.

Hold `Ctrl` (`Cmd` on Mac) while placing or dragging a point to invert the
magnet temporarily: off becomes strong, weak or strong becomes off. The magnet
button shows the state in effect, and releasing the key restores your setting.

Hold `Shift` while placing or dragging a point of a trend line, ray or extended
line to snap the line to horizontal, vertical or 45°, whichever is nearest. The
snap wins over the magnet on the axis it fixes. The 45° is a screen angle at that
moment, so zooming later changes it. A vertical line has no single price to
cross, so alerts are not offered on it.

- **Measure tools** take two clicks. Price range shows a vertical arrow
  and `+1,234.50 (+2.31%)`; date range a horizontal arrow and
  `12 bars, 3d 4h`; date and price range a box with both. The band is
  blue when price (date range: time) moves up or right, red otherwise. The
  style bar shows that color and a fill color you can change. Labels move
  below the band when there is no room above it.
- **Fib extension (trend-based)** takes three clicks (A, B, C) and draws
  the levels 0, 0.236, 0.382, 0.5, 0.618, 0.786, 1, 1.618, 2.618, 3.618
  and 4.236 from C, offset by the A to B move, each labelled
  `level (price)`. Colors are lighter in the dark theme.
- **Pitchfork** (Andrews) takes three clicks. The median line runs from
  the first point through the midpoint of the second and third; parallels
  run through those two. All three lines extend along the median's
  direction to the edge of the pane.
- **Long position / Short position** take one click, placing the entry.
  The reward (target) is 10% of the price span visible when you pick the
  tool and the stop is half of that, so the ratio starts at 2.00 (with no
  usable span: +2% and -1%, mirrored for a short). The box is 10% of the
  visible bars wide, at least 20. Drag the entry, target or stop handle; target and stop share the
  right edge, and each stays on its side of the entry. Labels show price
  and percent, and `Risk/Reward Ratio: 2.00` updates while you drag. The
  zones are green (reward) and red (risk); the labels stay centered on the
  part of the box that is on screen. The style bar's color is the entry line.
- **Select** a drawing by clicking it, then press `Delete` or
  `Backspace` to remove it.
- **Right-click** a drawing for its [menu](#right-click-menus): Style, Clone,
  Lock/Unlock, Hide and Remove, and on lines and channels **Add alert on …**
  ([drawing alerts](#drawing-and-indicator-alerts)). Locked drawings cannot be
  dragged but still answer right-click, so they can be unlocked there. A hidden
  drawing no longer answers on the chart; show it again from its Object Tree
  row or the toolbar eye. Right-clicking while a tool is armed cancels the tool,
  with or without points placed, and opens no menu.
- **Double-click** a text annotation to edit its text in a dialog.
- **Style** a selected drawing with the bar at the top-right of the
  chart (below the legend when the chart is too narrow): color, width
  (1 to 4 px) and line style (solid, dashed, dotted); a rectangle also has
  a fill color with opacity, a text annotation its text color and size.
  The bar is disabled while the drawing is locked and hides on `Esc`, when
  you click elsewhere or when you hide all drawings. Unstyled lines keep
  klinecharts' default blue in both themes; unstyled text follows the
  theme. The style is saved with the drawing (`style` field, optional, so
  older stored drawings load unchanged).
- **Undo and redo** (keys, or the arrow buttons above the trash button)
  cover creating, moving, editing, styling, locking, hiding and removing
  drawings (also remove all), up to 100 steps. The selection stays on the
  drawing. Drawings only, not indicators or settings; the history resets
  when you switch symbol. The shortcuts follow the typed letter, so they
  work on QWERTZ and AZERTY layouts.

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
| `Ctrl+Shift+Z` / `Ctrl+Y` | Redo (`Cmd+Shift+Z` on macOS) |

The shortcuts use modifiers so they do not clash with the letter and
digit shortcuts above.

## Chart settings

The gear button in the top toolbar (or **Settings…** in the chart context
menu) opens the chart settings dialog with four tabs:

- **Symbol** — body, borders and wick of the candles, each with an on/off
  checkbox and an up and a down color, plus the line/area color (also used
  by Line with markers, Step line and High-low). Border and wick follow the
  body color until you pick one for them.
- **Status line** — show or hide the OHLC values, the bar change, the volume
  and the indicator values in the legends.
- **Scales** — last price line, last price label, high and low price labels,
  the price scale placement (right or left), and the invert and lock price to
  bar ratio switches. Those two belong to the chart's scale state rather than
  to the settings: they apply at once, **Cancel** puts them back and
  **Reset settings** turns both off.
- **Appearance** — background color, vertical and horizontal grid lines and
  grid color, crosshair style (dashed, dotted, solid) and color.

Changes preview live on the chart. **OK** keeps them, **Cancel** (or `Esc`)
reverts, and **Reset settings** puts the options and the current theme's colors
back to the defaults (still previewed until OK; the other theme is untouched). Colors and the candle part toggles are stored per
theme, so the light and dark themes can look different; the other options are
shared. Everything is saved in `localStorage` under `chartgen.chartSettings`
and validated on load. Settings are not part of named layouts (invert and lock, which are, are not stored there). See
[ADR-0019](/chartgen/decisions/0019-chart-settings/).

## Right-click menus

Right-clicking opens a menu for what is under the pointer (mouse only, see
[Phones and tablets](#phones-and-tablets) for touch). Arrow keys and `Enter`
drive it. `Esc` or a scroll closes it. A click outside closes it and does nothing
else, so it cannot start a replay, place a drawing point or press what is under
it. A right click elsewhere moves the menu there. Anywhere not listed here, and in every text field,
the browser's own menu stays.

### Chart context menu

On the chart, and on the price legend:

| Item | Action |
|---|---|
| Reset chart view (`Alt+R`) | Scroll to real time at the default zoom |
| Copy price | Copy the price under the cursor, in the chart's precision |
| Add alert at | Open the alert dialog at that price |
| Add horizontal line at | Add a horizontal line at that price; it is saved like any drawing |
| Invert scale (`Alt+I`) | Flip the price axis; a check mark shows when it is on |
| Lock price to bar ratio | Keep the candle aspect while zooming horizontally; a check mark shows when it is on |
| Copy link to chart | Copy a [shareable link](#sharing-a-chart) |
| Remove all drawings | Asks for confirmation |
| Remove all indicators | Asks for confirmation |
| Settings… | Open the chart settings dialog |

The price is the one under the pointer on the regular, log, percent and inverted
scales alike. TradingView's "Lock vertical cursor line by time" is not offered
because klinecharts has no pinned crosshair.

In a sub-pane the price items and the price scale's options make no sense, so it
offers Reset chart view, **Copy value** (on that pane's own scale), that indicator's
items (see below), Copy link and Settings.

### Everything else

| Where | Items |
|---|---|
| Price axis, and its `%` `log` `inv` `auto` buttons | Reset price scale, Auto, Lock price to bar ratio, Regular, Percent, Logarithmic, Invert scale, and at the click's height Copy price, Add alert at, Add horizontal line at |
| Time axis | Reset time scale, Go to date… (`Alt+G`) |
| Drawing, or its Object Tree row | Add alert on … (lines and channels), Style…, Clone, Lock/Unlock, Hide/Show, Remove |
| Alert line, Alerts panel row (a server alert's too), or its Object Tree row | Edit…, Pause/Resume, Open *symbol* in chart (another symbol's alert), Delete |
| Indicator legend row, chip, Object Tree row | Add alert on … (with a plot to read), Settings for …, Hide/Show, Remove |
| Compare symbol's legend row or Object Tree row | Remove |
| Watchlist row | Open in chart, Add alert on …, Flag…, Copy symbol, Remove from watchlist (not in the Flagged view) |
| Watchlist section | Collapse/Expand, Rename…, Remove section |

Clone puts the copy just below the drawing; alerts on the original stay on the
original. In a [chart grid](#chart-grid) a right click makes that chart the active
one, and the menu acts on it.

## Chart grid

The grid button left of the layout menu shows 1 chart, 2 side by side, 2 stacked or
4. Each chart has its own symbol, interval, chart type, scale, compare symbols,
indicators and bar replay, and its own live data.

- **Active chart.** Pressing a chart makes it active, with a colored frame. The
  toolbar, indicator chips and picker, watchlist clicks, sidebar panels, bottom bar,
  drawing toolbar and keyboard shortcuts act on it. Only one drawing toolbar shows,
  the active chart's. Each chart's legend shows its symbol and interval.
- **Global.** Theme, chart settings, the time zone, watchlists and recent symbols
  are the same for all charts.
- **New charts.** A chart added by a bigger grid is the chart saved for that place
  in the active layout, else starts from the active chart's symbol, interval, chart
  type and indicators.
- **Fewer charts** remove the last ones. If one holds something the active layout
  could not bring back, a prompt asks first.
- **Drawings** belong to the symbol, as everywhere else: two charts on one symbol
  show the same drawings, and a change in one appears in the other. That chart's
  undo history for that symbol is cleared, so undo cannot overwrite the change.
  Different symbols never share drawings.
- **Live data.** Each chart subscribes on its own. Past the server's push limits
  (4 subscriptions per token, 10 subscribe calls per minute, 32 per client address,
  which everyone behind one proxy shares) a chart polls instead and tries push again
  every 5 minutes. The status line shows the active chart's state.
- **Alerts.** Lines show on every chart of their symbol. Price alerts run as usual.
  Indicator and sloped-drawing alerts only run while the **active** chart shows
  their symbol and interval, and otherwise read *inactive: open AAPL 1h*. Server alerts
  run whatever the charts show. Clicking an alert puts its symbol on the active chart.
- **Scripts.** The editor's Add to chart and the error marks read the active chart.
  Saving a script reloads every chart using it, and deleting one takes it off all of them.
- **Sub-panes** shrink to fit a smaller chart, and get their height back with one chart.
- **The compact layout** ([phones and tablets](#phones-and-tablets)) shows the active
  chart only. The grid is kept and comes back on a wider screen.

There is no symbol or crosshair sync between charts yet. Storage and the design are
in [ADR-0043](/chartgen/decisions/0043-multi-chart-grid/).

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
| `interval` | Timeframe (`4h`, `240`, `1d`, `7m`, `2W`, ...) |
| `type` | Chart type: `candles`, `hollow`, `bars`, `line`, `line_markers`, `step_line`, `area`, `baseline`, `columns`, `high_low`, `heikin_ashi` |
| `ind` | Indicators, comma separated (at most 50, 2 KB). Non-default parameters go in parentheses, separated by `;`: `macd(fast:8;slow:20)`; characters outside letters, digits, `.`, `_` and `-` in a text value are percent-encoded. Visible indicators only; `ind=` means none. A repeated name is another instance: `rsi,rsi(length:21)` shows two RSIs |
| `compare` | Compare symbols, comma separated; always written, `compare=` clears them |
| `tz` | One of the time zones in the time-zone menu: `utc`, `exchange`, `local` or a listed IANA name such as `Europe/Berlin` |

Every parameter is optional. The app reads them once on load, saves them like a
manual change and removes them from the address bar. An invalid value (unknown
interval, empty symbol) is ignored and the stored value stays.
Indicators from a link are checked against the server's indicator list before
the first chart request: names it does not know and parameters it rejects
(wrong type, outside min/max, not a parameter of that indicator) are left out
with a toast, so the indicator uses its default. Only what came from the link is
checked; your other indicators are never removed. If the list cannot be loaded,
the link's indicators are kept as they are. A link carries the chart setup only,
not drawings, alerts, colors or indicator styles. The parameters survive the
sign-in redirect. If the browser blocks the clipboard API (plain HTTP), the link
is copied through a hidden text field instead. In a [chart grid](#chart-grid) a link
carries the active chart only, and opening one applies it to the active chart.
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
- **×** (shown on hover, always on a touch screen, like the row's other buttons) removes a row.
- **»** collapses the panel to a thin strip on the right edge; the list
  icon there expands it again.

Stock, ETF, index and FX rows change against the previous session's
close, like TradingView. Crypto rows (Binance) change over the rolling
24 hours, as Binance reports it, so for crypto the number can differ from
TradingView's change since the daily open
([ADR-0011](/chartgen/decisions/0011-watchlist-quotes/)). A symbol that
fails to load shows dashes; hover the row for the reason.

### Lists, sections and flags

The name in the panel header opens the list switcher. It lists your
watchlists, **Flagged**, and the actions **Create new list…**, **Rename…**,
**Duplicate** and **Delete…** (which asks first). The last list cannot be
deleted. The list you are viewing is remembered. Quotes are polled for that
list only, so each list holds up to 50 symbols; you can have up to 20 lists.

- **Add section** (the button next to **+**) appends a named divider. The
  symbols under a divider belong to that section. Click the section name to
  collapse or expand it; hover it to rename it (✎) or remove it (×, which keeps
  its symbols). Up to 20 sections per list.
- **Reorder** by dragging a row or section by its grip (⋮⋮, shown on hover), or
  focus a row and press **Alt+ArrowUp** / **Alt+ArrowDown**. A section moves
  together with its symbols.
- **Flags**: right-click a row and pick **Flag…**, or use the flag button shown on hover, to pick
  one of seven colors (red, orange, yellow, green, cyan, blue, purple) or **No
  flag**. A flagged row has a colored mark at its left edge. A flag belongs to
  the symbol, so it shows in every list that holds it.
- **Flagged** in the switcher is a read-only view of every flagged symbol across
  all lists. You can still change flags there; a symbol disappears from the view
  when you clear its flag. It shows at most 50 symbols, the quotes limit, and
  there is no separate list per color.

The lists, sections, flags and the active list are saved as one versioned
document in `localStorage` under `chartgen.watchlist.v2`, and the collapsed
state under `chartgen.watchlist.collapsed`
([ADR-0031](/chartgen/decisions/0031-multiple-watchlists/)). Changes made in
another tab of the same browser show up here without a reload; each tab keeps
its own active list. A list from an
earlier version (`chartgen.watchlist.symbols`, default: BTCUSDT, ETHUSDT,
SOLUSDT, AAPL, MSFT, NVDA, SPY) becomes the list "Watchlist" with no sections
the first time you open the new version, and the old key is left in place. On
load, symbols are trimmed, uppercased and deduplicated per list, and invalid
entries are dropped.

## Sidebar and price alerts

The right sidebar has four icon tabs, **Watchlist**, **Alerts**, **Data
Window** and **Object Tree**; the tab you last used is remembered. Click the
selected tab again to collapse the sidebar into a narrow rail of the four icons
(the Alerts icon shows the number of active alerts). Click an icon in the rail
to open the sidebar on that tab. The collapsed state is remembered. On a narrow
screen the sidebar is a sheet closed with **×**. The Alerts tab shows the number of
active alerts. The watchlist polls quotes only while its tab is showing.

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
the Alerts tab. The dialog takes the source, condition, price, trigger and an
optional message. Active alerts for the chart symbol are drawn as dashed lines with a
bell label; drag a line to move the level, or double-click it to edit.

In the Alerts tab alerts are grouped by symbol with what they watch and their
status (*active*, *triggered*, *paused*, or *inactive* with the reason). Hover
a row for edit, pause / resume and delete; clicking the row switches the chart
to that symbol.

When an alert fires you get a toast, a short beep and, if you allow it, a
browser notification. The browser is asked for notification permission when
you create your first alert, not on load.

Alerts are evaluated in the browser ([ADR-0020](/chartgen/decisions/0020-client-side-price-alerts/)):
the chart symbol from the live feed, other symbols from their own quotes poll
every 5 s. **They only fire while chartgen is open in a visible tab** (with several tabs open, one of them evaluates). They
are saved in `localStorage` under `chartgen.alerts.list` (at most 200;
invalid entries are dropped on load) and are not shared between browsers.

### Alerts on the server

On a server started with `CHARTGEN_SERVER_ALERTS=1`, a price alert can run on the
server ([ADR-0042](/chartgen/decisions/0042-server-side-alerts/)): it fires with no
browser open and can call a webhook. Tick **Run on server** in the dialog. On a
server without them, or if its operator did not allow your sign-in, the dialog says the
alert runs in this browser.

- **Only price alerts on Binance symbols.** Drawing and indicator alerts run in the
  browser, and the dialog says so when one is the source. A stock is refused with
  the reason.
- **Check on** is *Every tick*, like a browser alert, or *Bar close* of the chart's
  interval, which reads only the closes of those bars. An interval with no Binance
  stream of its own can only use *Every tick*.
- **Webhook** is optional, and only offered if the server sends them for your sign-in. It
  must be `https` on port 443 at a public address: local and private addresses, other ports
  and redirects are refused, and so are hosts this server itself uses. **Sign webhook**
  shows a secret once, when the alert is saved. Copy it then. See the
  [payload and signature](/chartgen/reference/mcp/#webhook-payload).

The conditions and triggers work as above, with one difference: changing a fired alert's
message or webhook leaves it fired, and only a new condition, level or trigger brings it
back. A crossing needs a previous reading, so a crossing that happens while the server is
down is not reported. What already fired does not fire again after a restart.

In the Alerts tab a server alert is marked **server**, with its status, the webhook's
host and the last result: *webhook HTTP 200*, the error, or *waiting for prices*. Edit,
pause / resume and delete act on the server. In the dialog a new webhook URL replaces
the old one and drops its secret unless you sign again, and an empty field keeps it. When
one fires, the tab that leads shows the toast, within 5 s, once however many tabs are open;
a fire while no tab was open is reported when you next open one. A webhook over the server's
budget waits its turn, so it can arrive later than the toast.

**Notifications** reach this device with chartgen closed
([ADR-0045](/chartgen/decisions/0045-web-push-for-server-alerts/)). Press **Enable notifications**
in the Alerts tab or the dialog, which is the only thing that asks for the browser's permission,
then tick **Send a notification** on an alert; new alerts start ticked once this device is on. It
works in desktop browsers and on Android. On an iPhone or iPad, add chartgen to the Home Screen first
(Share, then Add to Home Screen) and open it from there, because iOS only lets an installed app
notify; the control says so. If notifications are blocked, or the browser cannot do it, it says that
too. The notification shows the symbol, the condition and the price, and clicking it opens the chart
on that symbol. With push on, the notification is the one report of that fire: the tab shows no toast
for it, unless the push failed or you first see the fire more than an hour later, when the notification may
never have arrived. **Turn off** stops this device. Signing out does not, so alarms keep working.

Without [sign-in](/chartgen/reference/oauth/#sign-in) each browser is its own, so **an alert notifies only the
browser that made it**: to get an alert on your phone, make it on your phone. On iOS the Home Screen app probably
counts as its own browser. With sign-in an alert notifies every device of your account, so one made on the desktop
reaches the phone. Each tab goes by how its own device fared, and tells you itself if that one's push failed.

Signing out does not stop notifications to that device, so your alerts keep reaching it. A device belongs to one
account at a time: when someone else signs in on that browser with notifications on, it becomes theirs and shows only
their alerts. Your alerts keep running and reach your other devices, and signing in there again takes it back.

A server alert belongs to the sign-in of this browser, so clearing its site data loses
access to it while it keeps running. It has no line on the chart and is not in the
Object Tree yet. Limits: 20 alerts and 2 symbol and interval pairs per sign-in, and an *Every
time* alert fires at most 6 times a minute. A fired *Only once* alert is deleted after 7 days.
On a server open to anyone, alerts are deleted once their sign-in has been unused for 30 days;
one its operator allowed keeps them.

### Drawing and indicator alerts

An alert can also watch a drawing or an indicator plot
([ADR-0034](/chartgen/decisions/0034-drawing-and-indicator-alerts/)), with the
same conditions and triggers. In the dialog, **Source** is what is compared
(the symbol's price, a drawing, or an indicator plot) and **Level** what it is
compared with (a value, or a plot).

- **Drawings.** Right-click a trend line, ray, extended line, horizontal line,
  horizontal ray or parallel channel and pick **Add alert on …**, or choose
  the drawing as the source in the dialog. The alert compares the live close
  with the line's value at the current bar, where the chart draws it (gaps such
  as nights and weekends take no room; on a log scale the line is straight in
  log price). A sloped line is extended past its points (a ray only from its
  start on). A channel has two lines: crossing either one fires, *greater
  than* means above both and *less than* below both. The Alerts tab reads
  *Close crossing Trend line*. A sloped line's alert only runs while the chart
  shows its symbol at the interval it was created on (otherwise *inactive:
  open AAPL 1h*); horizontal lines and rays are watched for any symbol.
  Deleting the drawing does not delete its alerts: they switch off and read
  *inactive: drawing deleted* until the drawing is back (undo). Alerts are not
  offered on a vertical line, nor on one whose two points fall on a single bar
  of the interval on screen (a line drawn on 1m, viewed on 1h). An alert whose
  line is like that on its creation interval reads *inactive: line is vertical*
  there. Changing the interval does not bring it back to life: elsewhere it
  reads *inactive: open AAPL 1h*, and the line must no longer be vertical on its
  creation interval before the alert can run. Each window reports this for the
  chart it shows.
- **Indicators.** Hover an indicator's legend row and click the bell
  (**Add alert on RSI**). For an overlay such as EMA the dialog opens as the
  price crossing its first plot (pick another one, e.g. EMA 200, under
  **Level**: *Close crossing EMA 200*). For an oscillator such as RSI it
  opens as the plot against a value (*RSI (14) crossing up 70*); **Level** can
  also be another plot of the same indicator (*MACD crossing Signal*). The
  values are those of the forming bar, checked on every live update.
  Indicator alerts only run while the chart shows that symbol at that interval
  with the same indicator settings; otherwise the Alerts tab reads *inactive:
  open BTCUSDT 1h*, *indicator not on the chart* or *indicator changed* (after
  you changed its parameters: create a new alert for the new ones).
- **Patterns.** See [pattern indicators](#pattern-indicators). A detection has
  no condition or level: the alert fires when a bar closes with a new one.

Only price alerts are drawn as lines on the chart. With several chartgen tabs
open, one evaluates the alerts and the Alerts tab of the others says so. An
indicator or sloped drawing alert depends on that tab's chart: while it shows
something else, the others read *inactive: open BTCUSDT 1h in the other tab*.

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
compare symbols and the indicator set (including hidden and edited ones), for each
chart of its [grid](#chart-grid). A layout saved before grids existed opens as one
chart. Drawings are not part of a layout: they stay per symbol and look the same in every
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
| Load layout... | A searchable list of layouts; click one to switch, or the x on its row to delete it. Clicking the open layout goes back to its saved state. The open layout cannot be deleted: switch to another one first. |
| Export saved layout / Import layout... | Download the saved layout as a JSON file, or add one from a file. Imports are validated (files over 256 KB, other versions and unknown fields are refused or dropped); invalid files are refused. |

Switching layouts applies all of the above. If the current layout has
unsaved changes, a dialog offers Save, Don't save or Cancel, also when
you pick the open layout itself to discard them. Unsaved
changes survive a reload like any other chart state, and so does the
active layout, so there is no "leave page?" prompt. Saving with nothing
changed still shows "Layout saved". If the browser refuses the save (storage
full or blocked), an error says so, the layout stays unsaved and the dialog stays
open. Hiding an indicator counts as a
change; opening an indicator's settings and pressing OK without edits
does not.

If you use several tabs, each tab keeps its own open layout; saving in
one tab does not discard layouts created in another.

A layout with compare symbols also remembers the scale mode to return to when
the last one is removed, so layouts with the same compare list each go back to
their own mode. A layout saved before this has none, so opening it falls back to
the normal scale, and saving it then keeps normal: saving does not bring back a
mode it has already lost. The mode is taken when the first compare symbol is
added, so to set it, remove the layout's compare symbols, pick the mode you want,
add them back, then save.

Layouts are stored in `localStorage` under `chartgen.layouts` (at most
50, validated on load, malformed ones dropped). See
[ADR-0015](/chartgen/decisions/0015-named-layouts/) and
[ADR-0040](/chartgen/decisions/0040-layout-compare-restore/).

## Phones and tablets

The app runs in a phone's browser without a separate mobile site
([ADR-0039](/chartgen/decisions/0039-compact-layout/)). Rotating or
resizing only moves the layout: the symbol, indicators and drawings stay, the
chart is not reloaded and no data is fetched again.

Windows narrower than 768 px get the compact layout. So do touch screens
narrower than 1024 px or shorter than 500 px, which covers phones in both
orientations and small tablets held upright. A tablet held sideways keeps the
desktop layout with larger touch targets.

In the compact layout:

- **The chart takes the whole width.** The header keeps the symbol, the
  interval, the chart type, the indicators and a `⋯` menu with the rest (replay,
  compare, reset, theme, screenshot, link, settings, fullscreen), plus the
  layout menu. The indicator chips and the clock go away.
- **A dock at the bottom** opens the drawing tools, the watchlist, the alerts,
  the Data Window and the Object Tree. The panels open as a bottom sheet, or as
  a side panel when the phone is sideways, over the chart and without resizing
  it. Tap outside the sheet or its `×` to close it. Picking a symbol in the
  watchlist closes it too.
- **The drawing toolbar** opens from the dock's pencil button and closes when
  you pick a tool, so you can tap the tool's points on the chart. The button
  stays highlighted while a tool is armed.
- **Dialogs** fit the screen and shrink to what the on-screen keyboard leaves
  visible, with their buttons always in view. They also stay clear of notches
  and the home indicator.
- **On a short screen** the price pane takes about two thirds of the height, and
  the indicator panes shrink to fit. They return to their usual height when you
  rotate back. In the compact layout any resize or indicator change resets pane
  heights dragged with a mouse, and touch cannot drag pane separators at all.
- **The range buttons** scroll sideways when they do not fit. On a phone held
  sideways the time zone select is hidden, so hold it upright to change the time
  zone.

On any touch screen:

| Gesture | What it does |
| --- | --- |
| One-finger drag | Pans the chart |
| Pinch | Zooms the chart |
| Tap, or long press | Shows the crosshair; the legend reads that bar |
| Drag after a long press | Moves the crosshair instead of panning |
| Tap elsewhere | Hides the crosshair |
| Tap an indicator's legend row | Shows its alert, hide, settings and remove buttons |

The page itself does not scroll or pull to refresh, so gestures on the chart
only reach the chart. A drawing dragged past its pane stops at the pane's edge, and
lifting your finger on the separator there keeps the drag. Dragging an axis does nothing on touch, because klinecharts
only scales axes for the mouse. Use a pinch for the time axis, and the `auto` and
`log` buttons for the price axis.

Touch has no right-click, and a long press belongs to the crosshair, so the
[right-click menus](#right-click-menus) open only with a mouse. Their actions are
elsewhere on touch:

- Reset view, copy link and settings are in the `⋯` menu. Invert scale and lock
  ratio are in **Chart settings**.
- `%`, `log`, `inv` and `auto` are the buttons on the price axis. Go to date is its
  button in the bottom bar.
- Remove all drawings is in the drawing toolbar.
- Hide, lock or remove one drawing, hide, set up or remove one indicator, and
  delete an alert, are buttons on their Object Tree rows.
- A watchlist row's flag and remove are the buttons on the row; tapping it opens it in the chart.
- Edit, pause or delete an alert with the buttons on its row in the Alerts panel.
- Add an alert from the Alerts panel's `+`. Type the price, or pick a drawing or
  indicator as the source.
- Copy price, copy symbol, cloning a drawing, and adding an alert or a horizontal
  line at a tapped price, have no touch equivalent. Use the horizontal line tool
  instead of the last one.

Most targets are at least 40 px. These are smaller, because they sit on the
chart or inside a row: the legend buttons (32 px), the `%`, `log`, `inv` and
`auto` buttons (32 by 28 px), the indicator chip buttons (28 px), a watchlist row's
buttons (20 px) and the arrow
on a drawing tool group (20 px; holding the tool's button opens the same list).

This layout has been tested in Chromium's device emulation only (iPhone 13,
Pixel 7, iPad Mini and a 360 px phone), not on real phones or in Safari. The
on-screen keyboard was simulated.

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

A server without persistent storage forgets every registered client when
it restarts, and a server at its client limit can evict an idle one. The app
notices that the stored `client_id` is no longer known, registers a new one and
signs in again by itself, also with several tabs open, and a share link opened
at that moment still lands on its chart
([ADR-0036](/chartgen/decisions/0036-own-origin-invalid-client-redirect/)).
This needs the app served by chartgen itself with `CHARTGEN_BASE_URL` set to
its public URL; with the Vite dev server, clear `localStorage` by hand.

### Signing in with GitHub or Google

On a server with [sign-in](/chartgen/reference/oauth/#sign-in) configured, the first load
without a session leads to a page offering each provider, or straight to the only one.
After the provider, the app loads where it would have, and a
[share link](#sharing-a-chart) still lands on its chart. Reloads and new tabs take no provider
round trip while the session lasts, up to 30 days. A GitHub and a Google account are two
separate users, each with their own scripts.

The header shows the signed-in user at its right end: an initial, with the name on wider
windows. Its menu says which provider it was, and has **Sign out**. On the compact shell,
where the symbol needs that room, the same entries are at the bottom of the **More tools**
menu. A server without sign-in shows none of this.

**Sign out** ends the session in this browser and leads to a page of its own, with
**Sign in again**. Every other tab of the app in this browser follows. It does not sign you
out at the provider, and tokens given to other applications, such as Claude, keep working
until they expire.

If a provider denies, fails or cannot be reached, the page says so and offers **Try again**
and **Cancel**. Cancel returns to the app, which stops at `sign-in failed: you cancelled it`
with **Sign in again**. A token that went stale signs in again by itself, and if the session
is gone as well, the app lands on the sign-in page.

Other applications are not trusted the way the app is: the first time one asks, a page shows
its name and where it will send you, and only the address is reliable.

Only what is stored on the server follows the account: scripted indicators and
[alerts on the server](#alerts-on-the-server), which keep running after signing out. Watchlists,
layouts, drawings, chart settings and alerts in the browser stay in this browser, whoever signs
in on it, and remain there after signing out.

## Managing indicators

Indicators are added from the **Indicators** picker (by name, with the
server's default parameters) and listed as chips under the toolbar.
Picking an indicator that is already on the chart (marked **added**) adds
another instance with the default parameters, so RSI 14 and RSI 21, or EMA
20 and EMA 50 as two `ma` instances, can sit side by side. Each instance has
its own parameters, style, visibility and pane. When an indicator is on the
chart more than once, its legends, chips, Object Tree rows and Data Window
sections show the parameters of each instance (`RSI (14)`, `RSI (21)`)
([ADR-0028](/chartgen/decisions/0028-multiple-indicator-instances/)).
Each indicator pane legend shows controls on hover (or keyboard focus):

- **Bell** opens the alert dialog on the indicator's plots
  ([indicator alerts](#drawing-and-indicator-alerts)).
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
  Below the plots the Style tab lists the indicator's other colors, each
  with a visibility checkbox and a color swatch (opacity included): every
  fill (**Fill 1**, ...; two swatches where the fill has an above and a
  below color, like the Ichimoku cloud), every horizontal level (**Level
  70**, with a line style), the volume-profile bars (outside the value
  area, inside it where the profile has one, and the point of control), and
  the divergence lines (bullish and bearish, one visibility toggle). A
  histogram with a per-bar palette (Volume, MACD) shows one swatch per
  server color instead of a single one; a color you pick applies to that
  server color wherever it appears, so it stays on the up bars when the
  server lists the colors in another order. These overrides sit on top of
  the server colors at draw time: live ticks and theme switches keep them,
  and colors you did not touch still follow the server and the theme. They
  are stored in the indicator's `style.geometry` (histogram palettes in the
  plot's `palette`) next to the plot styles, so layouts keep them too
  ([ADR-0035](/chartgen/decisions/0035-styleable-geometry/)). A fill or
  level override applies only while the payload still has the same fill
  color at that position (and, outside volume profiles, the same level
  value); otherwise it is ignored and dropped when the dialog opens. Hidden
  fills do not scale the price axis.
- **×** removes the indicator. Other instances of the same indicator stay.
  The price pane legend cannot be removed.

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
Panes follow the stored indicator order, also after a reload. Symbols with no
traded volume at all (Yahoo FX pairs) show no Volume pane; the indicator stays
in the set.

The set, parameters and hidden state persist in `localStorage`
([ADR-0013](/chartgen/decisions/0013-indicator-visibility-and-instances/)),
and so do the plot styles
([ADR-0017](/chartgen/decisions/0017-indicator-plot-styles/)).
Every instance after the first is stored with an instance id
(`{ "name": "rsi", "id": "rsi#2", "length": 21 }`). Sets and layouts saved
before instances existed load unchanged.

### Funding, Open Interest and Long/Short panels

These three panels read Binance futures data, so they have values for crypto
symbols that Binance trades as a perpetual future. A bar with no value is left
empty, not drawn as zero. When a panel has no values at all, its legend says why:

| Legend | Why |
|---|---|
| `Funding (Binance futures only)` | The symbol is not on Binance: a stock, index, FX pair, or `BTC-USD`. |
| `Funding (no futures market)` | Binance has no futures rows for it: a spot-only pair such as `BTCFDUSD`, a spot symbol whose future has another name (`SHIBUSDT` is `1000SHIBUSDT` there), or a futures market that did not exist yet for these bars. |
| `Funding (unavailable)` | The request failed, or the server's request budget for Binance was used up. The server log has the reason (`[futures] ...`). |
| `OI (last 30 days only)` | Every bar is older than the 30 days Binance keeps. |

A panel that loaded as `unavailable` while Binance was refusing requests is
loaded again by the web app once values return (a bounded reload, up to three,
which also brings the new label), so it does not stay blank.

How far back the values go:

- **Funding** has full history at Binance. The server reads back as far as the
  visible bars need, newest first, up to 3000 rows (about 2.7 years at one payment
  every 8 hours). Older bars stay empty. Scrolling back reads the older window
  the same way.
- **Open interest** and the **long/short ratio** are kept by Binance for the last
  30 days only. Bars older than that are empty on any chart. Reading goes back up
  to 4000 rows (about 14 days of 5 minute rows) in one chart, so a zoomed-out
  5 minute chart can have empty bars before that.
- A bar takes the nearest row within two bar lengths (funding: within 8 hours).
  The data periods are 5m to 1d, so a 1m chart repeats each 5 minute row over its
  bars.

Cost, in requests to Binance:

- Opening a chart in the web app (500 bars and 300 bars of warmup) reads each
  series once, and the live updates that follow reuse it. Funding, open interest
  and long/short together: **5** on a 5m chart (1 + 2 + 2), **7** on 1D (5 + 1 + 1),
  **8** on 1W (6 + 1 + 1). A request for thousands of bars, as an MCP client can
  make, reads at most 6 pages of funding and 8 of each of the others, **22** in all.
- After that nothing is asked for until a new row can exist: funding once when the
  next payment is due (about every 8 hours per symbol), the others once per data
  period (5 minutes and up). A window of older bars, from scrolling back, is read
  once for its span and then kept.
- Binance allows 500 `fundingRate` requests per 5 minutes per IP, and 1000 per 5
  minutes for each of the two data endpoints. The server counts them itself and
  refuses at 400 and 800, so Binance never answers 429 for them, which would stop
  every futures request for a while. A refusal in the middle of a read keeps the
  rows read so far, and the older bars stay empty.
- One series is read for at most 20 seconds, and a second chart asking for it meanwhile
  waits for that read.

## Pattern indicators

Three indicators in the picker find patterns on the server
([ADR-0046](/chartgen/decisions/0046-pattern-detection/)). They work on any data
source, and on Heikin Ashi bars, where they read the Heikin Ashi candles. Gap
patterns never fire on those, and rarely on 24/7 markets, where a bar opens at the
last close.

- **Candlestick Patterns** (`candlestick_patterns`) labels 50 patterns: blue
  bullish, red bearish, grey neutral, above or below the bar. Hover a label for its
  name and meaning. **pattern** chooses all, one side or one pattern; search the
  picker by a pattern's name to find the indicator. **trend** is the prior trend for
  patterns that need one, such as a hammer against a hanging man: the close against
  its SMA 50, also the SMA 50 against the SMA 200, or no check.
- **Chart Patterns** (`chart_patterns`) searches the last 600 bars for head and
  shoulders and its inverse, double and triple tops and bottoms, ascending,
  descending and symmetrical triangles, rising and falling wedges, channels, flags,
  pennants and rectangles. It draws the outline with a label that names the status:
  forming, confirmed or failed. Hover it for the measured move and what fails it.
  **depth** is the bars on each side of a swing, **deviation** the least move between
  swings in ATR(10), **tolerance** how close counts as the same level, as a percentage of
  the pattern's height. There are no Elliott waves: an honest count needs a degree and
  alternatives, not one rule check.
- **Auto Fib Retracement** (`auto_fib`) draws levels over the last significant swing,
  from the same **depth** and **deviation**. **level1** to **level8** set the ratios
  (0 is the end of the swing, 1 its start; **reverse** swaps them). On the Style tab each
  level is a plot with its own color, width and visibility. Changing a level's value
  starts its color over, because a plot's style follows its name.

**What can change.** Each indicator's settings say this too.

| | On the forming bar | After it closed |
|---|---|---|
| Candlestick label | Outlined: can change or vanish | Never |
| Chart pattern | Nothing | A forming one can reshape or vanish as swings confirm. In the newer half of the 600 bars, a confirmed one stays, and can still fail. In the older half, confirmed ones can appear and disappear as the window slides |
| Fib levels | Nothing | Redrawn when a newer swing confirms or a close breaks the swing's start. Older bars show the new levels too |

A swing is known only **depth** bars after its high or low, so a chart pattern appears that
much after its last turn, and is confirmed only by a later bar's close past its line. The
window's start decides its first swings, so moving it by a bar can change what the older half
of the 600 bars shows: on a random walk, over 11,200 closes, 271 confirmed patterns appeared
there and 360 disappeared, against 1 and 1 in the newer half. The status of a pattern more than
300 bars old is a reading of the window, not a record. A daily bar counts as forming for 24
hours from its open, so a label on it stays outlined after its exchange closes.

**Alerts.** The bell on a pattern indicator's legend row, or **Source** in the dialog, offers
*Bullish*, *Bearish* and, for candlesticks, *Neutral*. To alert on one pattern, choose it in
the settings, then add the alert to that instance. Each new detection on a closed bar fires
once; those already on the chart do not, nor do older ones that only appear because the window
moved. For a fib level, pick the level under **Level**:
*Close crossing 0.618*. A level moving over the price with a new swing does not fire it.

**What alerts cannot do.** They run in this browser, only while a tab shows that symbol,
interval and settings (in a grid, on its active chart): not on the server, and not by webhook. A pattern cannot fire on the
forming bar, and there is no alert on a pattern failing or on a forming one.

**Over MCP**, `set_alert` with `indicator_signal` matches a pattern's key on the newest closed
bar, such as `hammer` or `double_top_confirmed`. It needs a server started with `--trade`, and
two things it does not check when you set the alert. The indicator must be in that server's
`--panels` under the name you pass, since the engine computes nothing else; any other name is
accepted and never fires. And the engine's window starts empty and holds 200 bars at most,
with no history loaded: nothing fires until enough bars have closed since the start (about 50
for the SMA 50 trend rule, 200 for SMA 50 and 200), and `chart_patterns` reads only those 200,
so it can differ from the web chart's 600.

**How much to trust them.** A label says a shape occurred, not that it predicts anything. On a
random walk, which holds no pattern at all, the defaults still find about 238 candlestick
labels and 10 chart patterns per 1000 bars, right after 10 bars about half of the time, as
chance would be. `cargo run --example pattern_stats` prints the table per pattern. It was not
run on real market data.

## Script editor

On a server started with `CHARTGEN_SCRIPTING=1`, a **Scripts** button (`{ }` on
smaller windows) sits next to **Indicators**, and the indicator picker links to
the same panel. Without that flag none of this appears
([ADR-0041](/chartgen/decisions/0041-script-editor-in-the-web-app/)). The
panel opens under the chart. On the compact layout it takes the whole screen, clear
of notches and fitted above the on-screen keyboard, and opens from the indicator
picker, since the header has no room for its button. Any other window below 720 px
of height gets the full screen too. See
[Scripted indicators](/chartgen/guides/scripted-indicators/) for the language
and for why scripting is for trusted deployments only.

- **New** starts from a template that registers as it is. Name the script
  (lowercase letters, digits and `_`, starting with a letter) and **Save**, or
  press Ctrl/Cmd+S. A saved script keeps its name: to rename one, paste it into a
  new script and delete the old one.
- **Errors** from validation are listed under the code with their line and
  step, and the line is marked. Click one to jump to it. The server reports
  lines, not columns. A script that fails is not saved.
- **Add to chart** adds the saved script as an indicator instance, and is off
  while there are unsaved changes because the chart runs the saved text. Saving a
  script the chart uses reloads the chart. If it fails on the real chart, which
  has more bars than the dry run, the error appears here too, the indicator's chip
  turns red with the reason as its tooltip, and the other indicators stay. A chart
  runs at most 3 scripts, and is polled rather than pushed. A chart that names a
  script you no longer have works the same way: remove its chip, or save it again.
- **Delete** asks first, and also removes the script from the chart, unless a
  built-in of that name takes its place again.
- **Unsaved text** is kept per script in this browser's localStorage, so closing
  the panel or the tab loses nothing, and open tabs keep each other's drafts. Two
  tabs editing the same script share one draft: the last one to write wins. It
  comes back when you reopen the script; **Revert** returns to the saved text, and
  Ctrl/Cmd+Z undoes that.
- **A script changed elsewhere**, in another tab or by an MCP client, is not
  replaced unasked. If the saved text differs from what your draft started from,
  the panel says so and Save waits until you pick **Use the saved version** or
  **Keep my draft**. A draft whose script is gone stays in the list as
  **Draft, not saved**, and saving it creates the script again.
- **A new script** never replaces a saved one of the same name, even from a tab
  whose list is out of date.
- **Tab** indents by four spaces. To leave the editor with the keyboard, press Esc
  and then Tab.

Scripts belong to the OAuth client of this browser. Another browser, or this one
after clearing its site data, signs in as a new client and does not see them.
With [sign-in](/chartgen/reference/oauth/#sign-in) they belong to the signed-in user
instead, and follow you to any browser.

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
- **Charts of a grid** — `web/src/components/ChartPane.tsx` holds what belongs to one
  chart (query, live feed, replay, compare feed). The stores keep one instance per pane,
  and their module-level exports read the active pane
  ([ADR-0043](/chartgen/decisions/0043-multi-chart-grid/)).
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
    where Senkou A and Senkou B cross, and runs `displacement - 1` bars
    past the last bar as on TradingView. The chart widens its right
    margin to make room, and the legend and Data Window show the
    projected spans with the future bar's time when you hover there
    ([ADR-0033](/chartgen/decisions/0033-indicator-projection-past-last-bar/)).
    Oscillator panes always include
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
