---
title: MCP Integration
description: Connect chartgen to Claude Desktop or Claude.ai — 19 tools across charting, live chart push, orders, alerts, and notifications, plus optional scripted-indicator and server-alert tools.
---

chartgen speaks the Model Context Protocol over both stdio (Claude Desktop)
and HTTP (Claude.ai). Both transports share the same tool definitions.

The HTTP transport implements **MCP Streamable HTTP, spec
[2025-03-26](https://spec.modelcontextprotocol.io/specification/2025-03-26/basic/transports/)**.
See [ADR-0002](/chartgen/decisions/0002-mcp-transport-streamable-http/) for
the migration rationale.

## Transports

### Claude Desktop (stdio)

Add chartgen to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "chartgen": {
      "command": "/path/to/chartgen",
      "args": ["--mcp"]
    }
  }
}
```

### Claude.ai — Streamable HTTP (2025-03-26) + OAuth 2.1 PKCE

Run `chartgen --serve` behind a TLS-terminating reverse proxy and add
`https://<your-domain>/mcp` (for the hosted instance,
`https://chartgen.vasudev.xyz/mcp`) as a remote MCP connector. OAuth is
handled by chartgen — see the
[OAuth reference](/chartgen/reference/oauth/) for the endpoint catalog and
token lifetime.

The connector uses a single endpoint — `/mcp` — in two modes:

- **`POST /mcp`** — JSON-RPC 2.0 request/response. The server negotiates
  `Content-Type` based on the client's `Accept` header. Streamable HTTP
  clients send `Accept: application/json, text/event-stream`; chartgen's
  synchronous tool calls return `Content-Type: application/json` with the
  JSON-RPC response in the body. A client that explicitly excludes JSON
  (e.g. `Accept: text/plain`) receives `406 Not Acceptable`. A missing
  `Accept` header is tolerated and treated as `application/json`.
- **`GET /mcp`** — optional server-initiated SSE stream. Used to deliver
  push notifications such as `notifications/alert_triggered` and
  `notifications/chart_tick` as raw JSON-RPC notification frames — see
  [push notifications](/chartgen/guides/notifications/) and
  [`subscribe_chart`](#subscribe_chart--unsubscribe_chart). Requires the
  Bearer token in the `Authorization` header.

`Mcp-Session-Id` is supported: if a client sends the header on a `POST /mcp`
request, the server echoes it back unchanged. An `initialize` request without
one gets a freshly minted id, which spec-compliant clients then send on every
request, including `GET /mcp`. chartgen's tool calls are stateless (auth is
per-request via the Bearer token). The only state keyed on the session is
chart push: `subscribe_chart` pushes over the SSE stream opened with the same
token and session id. A POST made only of notifications or responses (one, or a
batch), such as `notifications/initialized` or `notifications/cancelled`, is
answered with `202 Accepted` and no body.

`POST /` and `POST /message` are retained as deprecated aliases of `POST /mcp`, and
`GET /sse` is retained as an alias of `GET /mcp`, for backwards compatibility
with existing Claude.ai connector deployments that predate Streamable HTTP or
were added with the root URL. `GET /` is not MCP: it serves the
[web app](/chartgen/guides/web/) to browsers and the health JSON to
everyone else ([ADR-0027](/chartgen/decisions/0027-serve-web-app-same-origin/)).

## Tool catalog

19 tools. The 12 trading tools require `--trade`; attempting to call them
without it returns an error. With `CHARTGEN_SCRIPTING=1` the server also
lists the 5 [scripted-indicator tools](#scripted-indicators-chartgen_scripting1),
and with `CHARTGEN_SERVER_ALERTS=1` the 4
[server alert tools](#server-alerts-chartgen_server_alerts1).

### Charting (always available)

#### `generate_chart`

Render a chart for a symbol. Accepts `ticker`/`symbol`, `timeframe`/`interval`,
`indicators` (string names or objects with overrides), `bars` (default 200),
`width`, `height`, `format`, `end_time`, `warmup`, and `chart_style`.

| Parameter | Meaning |
|-----------|---------|
| `bars` | Integer, default 200. Capped at 5000 in every format (`png`, `summary`, `both`, `series`); larger values are silently clamped. On Binance each 1000 bars is one upstream request. |
| `end_time` | Integer, ms since epoch, between `-2208988800000` (1900-01-01; index histories predate 1970) and one year from now. Only bars whose open time is strictly before `end_time` are returned. To load the page before the oldest bar you have, pass that bar's `t × 1000`. Omit it (or send `null`) to get the newest bars, including the candle that is still forming. Applies to every format. A string, a float or an out-of-range value is a tool error. |
| `warmup` | Integer, default 0, max 1000. `format=series` only. Fetches this many extra older bars and feeds them to the indicators, then trims them from the response, so the first returned bars have warmed-up values instead of `null`. `png`, `summary`, and `both` ignore it and render exactly `bars`. |
| `chart_style` | `candles` (default) or `heikin_ashi`. `heikin_ashi` replaces the price bars with Heikin Ashi bars in every format: `haClose = (o+h+l+c)/4`, `haOpen = (prev haOpen + prev haClose)/2` (the first bar is seeded with `(o+c)/2`), `haHigh = max(h, haOpen, haClose)`, `haLow = min(l, haOpen, haClose)`; volume and timestamps are unchanged. Like TradingView, indicators are computed on the Heikin Ashi values, not the raw bars. The transform runs over `bars + warmup` before trimming, and because each `haOpen` is an exponentially decaying average, pages and tail polls with the same warmup agree after a few bars. `format=series` echoes the applied style as `chart_style`. Any other value is an error. |

`timeframe` is a number and a unit: `m` minutes, `h` hours, `d` or `D` days,
`w`, `W` or `wk` weeks, and `M` or `mo` months (`7m`, `45m`, `90m`, `8h`, `2D`,
`2W`, `3M`). Lowercase `m` is minutes and uppercase `M` months; the other units
take either case. Anything else is a tool error that says what is wrong.
[ADR-0038](/chartgen/decisions/0038-custom-intervals/) explains the limits:

- Upper bounds, after normalization: under 1440 minutes, then `168h`, `365D`, `52W`
  and `12M`. Not every length below a bound parses, as the next rule says. `1440m`
  is `1d`, and seconds are not offered.
- A minute or hour interval may take at most 60 native bars per bar, counted on
  Binance's klines for every symbol. Every length up to 60 minutes qualifies; `61m`
  needs 61 `1m` klines and is rejected, while `65m` needs 13 `5m` klines and is
  fine, and `1439m` is rejected. Days, weeks and months have no such rule: `365D`
  takes 365 `1d` klines.

The response's `interval` is the canonical form, and the same bars have one
name: `120m` is `2h`, `24h` is `1d`, `7d` is `1wk`, `2W` is `2wk`, `1M` is
`1mo`.

Where a source has the interval, its own bars are served. Otherwise bars come
from the coarsest native interval that divides it and are resampled:

| Source | Native | Built from |
|---|---|---|
| Binance | `1m 3m 5m 15m 30m 1h 2h 4h 6h 8h 12h 1d 3d 1w 1M` | `7m` from `1m`, `45m` from `15m`, `90m` from `30m`, `10h` from `2h`, `6d` from `3d`, `2W` from `1w`, `3M` from `1M` |
| Yahoo | `1m 2m 5m 15m 30m 1h 90m 1d 1wk 1mo` | `7m` from `1m`, `6m` from `2m`, `45m` from `15m`, `8h` from `1h`, `2D` from `1d`, `2W` from `1wk`, `2M` and `3M` from `1mo` |

Bars land on a stable grid, so a reload gives the same bars:

- **24/7 symbols, minutes and hours.** A grid counted from the unix epoch.
  It agrees with Binance's klines at every interval Binance has.
- **Exchange-traded symbols, minutes and hours.** Buckets start at each
  session's open, and a session's last bucket may be shorter, as on TradingView.
- **Days.** Calendar days counted from epoch day 1, where Binance's `3d` klines
  open. A stock's `2D` bucket with no trading day in it has no bar, and one
  with a single trading day holds that day alone.
- **Weeks.** Counted from Monday 1970-01-05, so `2W` opens every second Monday.
- **Months.** Counted from January 1970, so `3M` is a calendar quarter and
  `12M` a calendar year.

A custom interval on Binance reads at most 20 requests' worth of native bars per
call (20,000). That caps the bars fetched, which also supply `warmup`; the response
returns at most `bars` of them. A `59m` call fetches about 338 bars, so one asking
for `bars: 2` returns 2, computed over those 338, and one asking for `bars: 500`
returns the 338 with `has_more: true`, though it asked for more, and older bars come
from paging. Warmup past the cap is simply shorter, and paging does not restore it: a
page, a poll and a push each compute over their own capped read. With `warmup: 1000` on
`59m`, an indicator with a very long window stays `null` or unconverged on every one of
them. Calls for the newest bars share native
klines read in the last 2 minutes, so after the first, polling a tail costs one
request whatever its warmup.

The server holds one budget for all its Binance requests. After Binance answers 429,
418 or 403 nothing is sent until its `Retry-After` has passed, and nothing starts once
80% of the weight for the minute is used. Such a call is a tool error starting
`Binance rate limit:` that names the wait. Retry after it.

With a `ticker`, a failed upstream fetch (network error, unknown symbol, rate
limit) is a tool error in every format. Only a call without a ticker renders
the synthetic demo series.

`format` values:

| Value | Output |
|-------|--------|
| `png` (default) | Base64-encoded PNG of the rendered chart. |
| `summary` | Compact JSON: OHLCV stats, last indicator values, signal dots, divergences, and horizontal reference levels. Typically 10–50× fewer tokens than a PNG — ideal for automated analysis without vision. |
| `both` | PNG plus the JSON summary. |
| `series` | Full per-bar time-series JSON: every OHLCV bar plus indicator value arrays aligned 1:1 with bars (NaN → JSON `null` for pre-warmup), signals with bar timestamps, hlines, and y_range. Useful for downstream numeric computation that needs raw arrays rather than last values. |

Example request:

```json
{
  "name": "generate_chart",
  "arguments": {
    "ticker": "BTCUSDT",
    "timeframe": "1h",
    "indicators": [
      "ema_stack",
      { "name": "rsi", "length": 21 },
      { "name": "vpvr", "bins": 32, "split_up_down": true }
    ],
    "bars": 200,
    "format": "summary"
  }
}
```

Summary payload shape (for `format=summary` or `format=both`):

```json
{
  "symbol": "BTCUSDT",
  "interval": "1h",
  "price": {
    "bars": 200,
    "first": { "date": "...", "open": 66900.1, "close": 66950.2 },
    "last":  { "date": "...", "open": 67400.0, "high": 67500.0,
               "low": 67320.0, "close": 67432.5, "volume": 128.4 },
    "range": { "high": 67980.0, "low": 65010.5 },
    "total_volume": 18342.7,
    "change_pct": 0.79
  },
  "indicators": {
    "rsi": {
      "label": "RSI(21)",
      "is_overlay": false,
      "y_range": [0.0, 100.0],
      "lines": [{ "index": 0, "label": "RSI", "last_value": 62.4 }],
      "signals": [{ "x": 197, "y": 72.1, "label": "rsi_overbought" }],
      "hlines": [{ "y": 30.0 }, { "y": 70.0 }]
    }
  }
}
```

Fields are keyed by the name each indicator was requested with, so
`{"name": "rsi", "length": 21}` comes back as `indicators.rsi`. A line's
`last_value` is its last finite value. Lines that break at a trend flip
(Supertrend's and Parabolic SAR's up and down lines) report their value at
the last bar instead, `null` when they have none there: only the active one
has a value. In `format=series` those lines carry `"breaks": true`, which the
web legend uses the same way.

##### Several instances of one indicator

The same indicator may be requested more than once, with different
parameters ([ADR-0028](/chartgen/decisions/0028-multiple-indicator-instances/)).
Each entry gets its own key in `indicators` and `indicator_errors` (every
format), so results map back to their entries whatever the order:

- An object entry with an `id` (a string of 1 to 64 characters) is keyed
  by its id. The id only names the result and is not a parameter.
- An entry without an id is keyed by its name. A name that is already
  taken gets `name#2`, `name#3`, and so on. Explicit ids are reserved
  first, so an automatic key never takes one.
- A repeated id, or an id that is not a 1 to 64 character string, is a
  tool error (`Duplicate indicator id: …`, `Invalid indicator id: …`).
- An id equal to the name of an entry without an id is a tool error, since
  both would get that key (`Indicator id macd is also the name of an entry
  without an id; give that entry an id too`).
- `generate_chart` takes at most 50 entries (`At most 50 indicators per
  chart.`); `subscribe_chart` at most 10.

```json
"indicators": [
  { "name": "rsi", "length": 14 },
  { "name": "rsi", "length": 21, "id": "rsi#2" },
  { "name": "ma", "type": "ema", "length": 20 },
  { "name": "ma", "type": "ema", "length": 50 }
]
```

comes back keyed `rsi`, `rsi#2`, `ma` and `ma#2`. The web app sends
`<name>#<n>` ids for every instance after the first. Push tails
(`subscribe_chart`) use the same keys.

Series payload shape (for `format=series`):

```json
{
  "symbol": "BTCUSDT",
  "interval": "1h",
  "bars": [
    { "t": 1713420000, "o": 66900.1, "h": 67100.0, "l": 66850.0, "c": 67050.5, "v": 128.4 },
    { "t": 1713423600, "o": 67050.5, "h": 67220.0, "l": 67010.0, "c": 67180.0, "v":  96.2 },
    { "t": 1713427200, "o": 67180.0, "h": 67340.0, "l": 67120.0, "c": 67290.5, "v": 104.8 },
    { "t": 1713430800, "o": 67290.5, "h": 67500.0, "l": 67240.0, "c": 67432.5, "v": 128.4 }
  ],
  "indicators": {
    "rsi": {
      "label": "RSI(21)",
      "is_overlay": false,
      "y_range": [0.0, 100.0],
      "lines": [
        { "index": 0, "label": "RSI", "values": [null, null, 68.2, 72.1], "color": "rgba(195,62,225,0.8)", "width": 2 }
      ],
      "signals": [
        { "x": 3, "t": 1713430800, "y": 72.1, "label": "rsi_overbought", "color": "rgba(225,62,62,1)" }
      ],
      "hlines": [
        { "y": 30.0, "color": "rgba(62,225,69,0.3)" },
        { "y": 70.0, "color": "rgba(225,62,62,0.3)" }
      ]
    },
    "volume": {
      "label": "Volume",
      "is_overlay": false,
      "lines": [
        { "index": 0, "label": "Volume MA", "values": [null, null, null, 114.5], "color": "rgba(41,98,255,1)", "width": 1 }
      ],
      "histogram": [
        {
          "index": 0,
          "label": "Volume",
          "values": [128.4, 96.2, 104.8, 128.4],
          "palette": ["rgba(38,166,154,0.5)", "rgba(239,83,80,0.5)"],
          "color_idx": [0, 0, 0, 0]
        }
      ]
    },
    "bbands": {
      "label": "BB",
      "is_overlay": true,
      "lines": [
        { "index": 0, "label": "upper", "values": [null, null, 67400.5, 67511.7] },
        { "index": 1, "label": "middle", "values": [null, null, 67051.3, 67163.1] },
        { "index": 2, "label": "lower", "values": [null, null, 66702.1, 66814.5] }
      ],
      "fills": [
        { "y1": [null, null, 67300.1, 67410.9], "y2": [null, null, 66802.4, 66915.3], "color": "rgba(33,150,243,0.1)" }
      ]
    },
    "vpvr": {
      "label": "VPVR",
      "is_overlay": true,
      "hbars": [
        { "y": 67432.5, "height": 12.1, "width": 0.82, "offset": 0.0, "left": true, "color": "rgba(41,98,255,0.4)" }
      ]
    },
    "cipher_b": {
      "label": "Cipher B",
      "is_overlay": false,
      "divlines": [
        { "x1": 12, "t1": 1713427200, "y1": 30.2, "x2": 48, "t2": 1713556800, "y2": 18.7, "dashed": true, "color": "rgba(255,0,0,0.8)" }
      ]
    }
  },
  "has_more": true
}
```

`bars[i]` aligns 1:1 with every `indicators.*.lines[*].values[i]` (and
`indicators.*.histogram[*].values[i]` when an indicator emits histogram
bars, e.g. MACD). Pre-warmup indicator values are `null` (not truncated).
Each signal carries both `x` (bar index) and `t` (unix seconds from the
source feed, or `null` for synthetic sample data). `format=series` is
exclusive — no PNG companion, a single text content item.

Three additional channels round out the payload so the full set of
`PanelResult` data an indicator emits is renderable on the wire — see
[ADR-0007](/chartgen/decisions/0007-complete-series-serialization/):

- **`fills`** — shaded regions between two y-series (Bollinger Bands,
  Keltner Channels, Ichimoku cloud, VWAP bands). `y1` and `y2` are arrays
  aligned 1:1 with `bars[]`; pre-warmup values are `null`.
- **`hbars`** — horizontal bars at price levels, used by volume-profile
  indicators (VPVR, Session VP, HVN/LVN, Naked POC, TPO). Each bar has a
  price-space `y` and `height`, axis-relative `width` and `offset`
  (0.0–1.0 as a fraction of chart width), and a boolean `left` that tells
  the renderer which edge of the chart to anchor against.
- **`divlines`** — two-point diagonal segments for divergences
  (WaveTrend/RSI divergences in Cipher B, for example). Each endpoint
  carries both `x` (bar index) and `t` (unix seconds, or `null` for
  synthetic sample data), mirroring the signal shape, plus `dashed`.
- **`labels`** — text labels on bars (`candlestick_patterns`): `x`, `t`, `y`
  (the bar's high or low), `above`, `text`, `key`, `name`, `bias`, `color` and
  `tooltip`. `tentative: true` marks one on the forming bar, which can change or
  vanish until the bar closes.
- **`patterns`** — detected shapes (`chart_patterns`, the swing of `auto_fib`):
  `key`, `name`, `bias`, `status` (`forming`, `confirmed` or `failed`), `strokes`
  (two-point segments as in `divlines`), `label`, `tooltip`, and optional `target`,
  `invalidation` and `confirmed_t`.
- **`levels`** — `lines[line]` is flat at `y` from bar `x` on (`auto_fib`).
- **`events`** — what an alert can wait for: `key`, `name` and `t`, the bar time of
  the newest occurrence on a closed bar, or `null`.

`patterns`, `levels` and `events` are re-detected as a whole
([ADR-0046](/chartgen/decisions/0046-pattern-detection/)), so every response holds all
of them, tails included: an index before the response's first bar is negative, and `t`
still names the bar. Take them from your newest response and ignore those on older
history pages. A new swing moves a level on every bar, so rewrite that plot from `levels`
instead of merging its values. These indicators raise `warmup` to their lookback.
`signals` only hold detections on the newest closed bar, for `indicator_signal` alerts
(see the [conditions](#alerts-require---trade) that needs).
As the 600-bar window slides, `chart_patterns`' older patterns can change, and an event's `t`
can fall back to an older bar or to `null` and return. To tell a new detection, take a `t`
newer than any you saw and no older than the newest bar of your previous response.
`format=summary` and `png` take no warmup, so `chart_patterns` and `auto_fib` read only the
`bars` you ask for: ask for 600 to match the web chart.

An indicator that plots past the last bar also carries a **`future`**
block ([ADR-0033](/chartgen/decisions/0033-indicator-projection-past-last-bar/)).
Today that is `ichimoku`: TradingView draws its cloud `displacement - 1`
bars ahead of price.

```json
"future": {
  "bars": 25,
  "t": [1713571200, 1713585600, ...],
  "lines": [{ "index": 2, "values": [67120.4, ...] }, { "index": 3, "values": [66410.0, ...] }],
  "fills": [{ "index": 0, "y1": [67120.4, ...], "y2": [66410.0, ...] }]
}
```

- `bars` is the number of bars after the last one, at most 500. Value `j` of every
  array belongs to the `j`-th of them.
- `lines[k].index` and `fills[k].index` name the line or fill the values
  continue: Senkou A (line 2), Senkou B (line 3) and the cloud (fill 0).
- `t` is those bars' open times: the last bar's time plus `j + 1`
  intervals, on 24/7 symbols. Month-based bars (`1M`, `3M`, `2mo`) step by
  calendar months. Exchange-traded symbols follow the exchange's regular session,
  from Yahoo's metadata, in its time zone: intraday bars stay between open and
  close and restart at the next open, and day bars stay on trading days. Saturday and
  Sunday each count as closed when no loaded bar falls on them, and a Sunday with
  neither a Friday nor a Saturday means a Friday and Saturday weekend. Without a
  session, as for crypto, a day closes only if the bars could have shown otherwise:
  7 days of `1d` bars, or 5 cycles of 7 longer ones. US-listed equities, ETFs, indices and mutual funds also skip
  the holidays of the NYSE's published calendar, for 2026 to 2028, on intraday and day
  bars. Weekly and monthly bars stay on their calendar grid, so one can fall on a
  holiday. An early-close day is not skipped: its bars stop at the 13:00 New York close. Other exchanges' holidays,
  US indices quoted from Chicago (`^VIX`, `^TNX`), and US dates past 2028 are not skipped. Futures step
  through their breaks. See
  [ADR-0038](/chartgen/decisions/0038-custom-intervals/). `t` is absent when the
  bars have no timestamps.
- The per-bar arrays are unchanged, so clients that ignore `future` keep
  working. Every series response carries the block, tails and pushes
  included: the forming bar moves the newest projected values. Take it
  from your newest response and ignore the one on older history pages.

Every item of `fills`, `hbars`, `divlines` and `hlines` also carries a
`color`: a CSS `"rgba(r,g,b,a)"` string with the color the PNG renderer
uses, so clients can match the PNG exactly
([ADR-0012](/chartgen/decisions/0012-geometry-colors-on-the-wire/)).
A fill may also carry `color_below`, used where `y1 < y2` (`color`
applies where `y1 >= y2`); it is omitted when the fill has one color.
Older servers omit `color`;
clients should fall back to their own theme.

Plots carry their colors too
([ADR-0026](/chartgen/decisions/0026-line-and-histogram-colors/)):

- Each item of `lines` has `color` (same CSS form) and `width`, the
  stroke width in px.
- Each item of `histogram` has a `label` for legends. It is the
  indicator's label (`"Volume"`, `"AO"`), or `"Histogram"` when a line
  already has that name (MACD), numbered when there are several.
- Each histogram also has per-bar colors in compact form: `palette`
  lists each distinct color once, in order of first appearance, and
  `color_idx[i]` is the index into `palette` of bar `i`'s color, aligned
  1:1 with `values`. It is `null` where the value is `null` or the bar
  has no color. Palettes are per response, so a client joining two
  responses (a history page, a tail) remaps the indices rather than
  concatenating them.
- Each signal has the `color` of its dot.

All of these are omitted by older servers.

All three channels are omitted from the per-indicator object when the
indicator emits none — matching the existing convention for `lines`,
`histogram`, `signals`, and `hlines`.

##### Live tail and history paging

See [ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/).
For Binance symbols over HTTP, [`subscribe_chart`](#subscribe_chart--unsubscribe_chart)
pushes the same tail instead of you polling for it.

- **Live tail.** Poll `{ "format": "series", "bars": 2, "warmup": 200 }`
  every few seconds. The last bar is the candle still forming, and the one
  before it is the last closed bar. Merge both into your arrays by `t`.
  Yahoo stamps its live bar with the last trade time; chartgen snaps it
  back onto its period start, so `t` always lands on the bar grid.
- **Older history.** Request `{ "format": "series", "bars": 500,
  "warmup": 200, "end_time": <bars[0].t of your oldest page × 1000> }`.
  The new page ends right before `end_time`, so pages join without gaps or
  duplicates.
- **`has_more`** (boolean, always present in `format=series`) is `true`
  when older history may exist before `bars[0]`. For Binance and the
  sample source that means the read got all it asked for: the full `bars + warmup`
  count or, for an interval built from finer bars, the 20,000-kline cap. So a capped
  page can be shorter than `bars` and still report `true`. For
  Yahoo it means the request window was not cut off by Yahoo's lookback
  limit or the symbol's first trade date, so a short page (for example
  2000 1m bars when one 7-day window only holds about 1950) still reports
  `true`. It is `false` when history ran out (the page can be shorter or
  empty) and for the no-ticker demo series, which has no timestamps.
- **Volume profiles on partial responses.** `vpvr`, `hvn_lvn`, and `tpo`
  are computed over the bars of the response they arrive in. Keep their
  `hbars` and profile-derived `hlines` (POC/VAH/VAL, HVN/LVN nodes) from
  the initial full load, and ignore them on tail-poll and history-page
  responses, which cover only a slice of the chart.

How `warmup` trims the channels: `bars[]`, `lines`, `histogram` (with
its `color_idx`), and `fills` keep only their last `bars` entries. Signal `x` values are shifted
to the trimmed window, and signals inside the warmup are dropped. Divlines
that end at or before the first returned bar are dropped. Divlines that
cross into the window are clipped to start at bar 0, with `y1`
interpolated. For windowed profiles (`vpvr`, `hvn_lvn`, `tpo`), `hbars`
and `hlines` are recomputed over the returned bars only. `y_range` and the
`hlines` of every other indicator don't change.

A non-zero `warmup` is raised to what the requested indicators need to
converge, up to 1000
([ADR-0032](/chartgen/decisions/0032-indicator-aware-warmup/)). `ema_stack`
needs 891 bars, so its EMA 200 is computed over at least that many and
matches one long request to within 0.01% at a page seam or on a tail. The exception is
a Binance interval built from finer bars when the 20,000-kline cap leaves fewer
than that: a `59m` read holds about 338 bars, so these guarantees do not hold there. The
other EMA/RMA-based built-ins (`ma`, `ma_ribbon`, `envelopes`, `keltner`,
`rsi`, `atr`, `adx`, `supertrend`, `macd`, `stoch_rsi`, `trix`, `ppo`,
`tsi`, `smi_ergodic`, `elder_force_index`, `volume_oscillator`,
`connors_rsi`, `kalman_volume`) declare it from their parameters. The
community composites (`cipher_b`, `wavetrend`, `rsi_mfi_stoch`) and user
scripts keep the `warmup` you send. `ichimoku` declares its longest window
plus `displacement - 1` (77 bars at the defaults), so a tail's leading
spans and `future` block match one long request. `warmup: 0` is never raised.
Smoothings beyond the cap (an EMA longer than 224, an RMA longer than 126)
can still differ slightly at a seam, running sums (OBV, CVD, A/D line) by a
constant offset, and VWAP until its next session reset. Finite-window
indicators (SMA, Bollinger Bands, Donchian) match exactly once `warmup`
covers their lookback. When history runs out, `has_more` stays `true` as
long as the response fetched more bars than it returned, so the next page
brings the trimmed warmup bars. `has_more` is never `true` with an empty
`bars`: if trimming would leave no bar (`bars: 0`), the fetched bars come
back instead, with indicator values `null` where they have not converged,
and `has_more` is `false`.

Upstream limits: Binance requests are paginated at 1000 bars per call.
Yahoo intraday history ends after about 30 days for 1m, 60 days for other
sub-hour intervals, and 730 days for 1h, and an interval built from one of them
ends with it: `7m` has 30 days of history on a stock, `45m` 60 and `8h` 730.
Each upstream request times out after 10 s, and a paginated Binance fetch
gives up after 20 s.

Daily and longer Yahoo bars (`1d`, `3d`, `5d`, `1wk`, `1mo`, `3mo`) are
stamped at 00:00 UTC of the exchange-local date, using the exchange's
`gmtoffset`. Yahoo itself stamps them at the session open, which for ASX in
AEDT or NZX falls on the previous UTC day, so a UTC date read from the stamp
would be one day early. Binance date bars already open at 00:00 UTC. A
history page's `end_time` is compared against these stamps.

Yahoo has no native 3m, 2h, 4h, 6h, 12h or 3d, so chartgen builds them from
1m, 1h and 1d bars (Binance serves all of them natively):

- **2h, 4h, 6h, 12h** are anchored at each session's open, like TradingView:
  a US stock gets 09:30–13:30 and 13:30–16:00 ET at 4h, the second one
  shorter; at 12h each session is one bar. A session ends where two
  consecutive source bars are more than an hour apart, so the overnight gap,
  a futures daily break (ES=F trades 18:00–17:00 ET and restarts its buckets
  at 18:00) and a lunch break that leaves a gap in Yahoo's rows each start
  a new session, so such a market anchors its afternoon separately. Tokyo, Shanghai
  and Hong Kong keep null rows through lunch, so theirs stays on the morning's grid.
  Each bucket's `t` is its slot on the session's grid. Crypto (`BTC-USD`)
  and FX (`EURUSD=X`) trade around the clock and use the UTC grid instead,
  with `t` at the grid start.
- **3m** uses the UTC grid for every symbol (exchanges open on 15-minute
  boundaries, so it matches the session grid). It inherits the 1m limits:
  about 30 days of history and 7 days per request, so the first load of a
  stock covers roughly 4 to 5 sessions.
- **3d** groups three calendar days counted from the Unix epoch, on the same
  grid as Binance's 3d klines (epoch days with `day % 3 == 1`). For stocks
  this means calendar-day buckets that can hold fewer than three trading
  days. TradingView likely buckets by trading sessions, a known divergence.
- A zero-volume 1m or 1h print outside the regular session (Yahoo's phantom
  print at the close) is merged into the previous bar instead of becoming a
  bar of its own.

##### Per-indicator errors

A built-in never fails, but a [scripted indicator](#scripted-indicators-chartgen_scripting1)
can, for example by hitting its time limit. A failing indicator is left out of
`indicators`, the rest of the chart renders, and the payload (`summary`,
`series`, `both`) gets a top-level map keyed like `indicators`:

```json
"indicator_errors": {
  "my_osc": { "step": "runtime", "line": 7, "message": "time limit exceeded" }
}
```

`line` is 1-based, or `null` when unknown. With `format=png` the map comes
as a second text content item, `{"indicator_errors": {...}}`. The field is
absent when every indicator computed
([ADR-0025](/chartgen/decisions/0025-user-indicator-registry/)).

#### `list_indicators`

Return the full indicator catalog with descriptions, aliases, and configurable
parameters. Call before `generate_chart` to discover valid names. With
scripting on, the caller's own scripts are included, tagged `"source": "user"`;
a script replaces the entry of the built-in it shadows.

#### `get_indicators`

Compute indicators and return raw numeric values as JSON. Fetches OHLCV (or
uses cached engine data under `--trade`) and reports each indicator's last
line values, dot presence (signals), and horizontal reference levels.

Required: `symbol`, `indicators` (array of names). Optional: `interval` (default `4h`).

Response shape:

```json
{
  "symbol": "BTCUSDT",
  "interval": "4h",
  "source": "engine_cache",
  "bars": 200,
  "indicators": {
    "rsi":  { "label": "RSI(14)", "lines": [...], "signals": [...], "hlines": [...] },
    "macd": { "label": "MACD",    "lines": [...], "histogram": [...] }
  }
}
```

`source` is `engine_cache` when hitting the running trading engine's rolling
window, `fetched` when the tool fell back to fresh OHLCV. The per-indicator
object is the same shape `generate_chart` returns under `format=summary`.

#### `get_quotes`

Last price and change for many symbols in one call. The web watchlist
polls it every 5 s.

Required: `symbols` (array of at most 50 strings). Symbols are trimmed,
uppercased and deduplicated. A valid symbol is 1–32 characters of
`A-Z 0-9 . ^ = - _` with at least one letter or digit. These are tool
errors: an array of more than 50 entries (checked before any entry is
read, so duplicates count too), a missing `symbols` and a non-string
entry.

Response shape: one entry per distinct symbol, in request order:

```json
{
  "quotes": [
    {
      "symbol": "AAPL",
      "last": 349.51,
      "change": 17.61,
      "change_pct": 5.3058,
      "prev_close": 331.9,
      "high": 354.47,
      "low": 340.99,
      "volume": 88958176.0,
      "source": "yahoo"
    },
    { "symbol": "NOPEUSDT", "error": "Binance: Invalid symbol." }
  ]
}
```

A symbol that fails gets `{symbol, error}` and never fails the call. That
covers a malformed ticker (`"invalid symbol"`), an unknown ticker and an
unreachable upstream. `change = last - prev_close` and
`change_pct = change / prev_close × 100`. `high`, `low` and `volume` are
`null` when upstream omits them. Numbers that are not finite are `null`.

What `prev_close` means depends on `source` (see
[ADR-0011](/chartgen/decisions/0011-watchlist-quotes/)):

| `source` | Symbols | `last` / `prev_close` | `high` / `low` / `volume` |
|---|---|---|---|
| `yahoo` | stocks, ETFs, indices, FX | `regularMarketPrice` / previous session close (`chartPreviousClose`, v8 chart `range=1d`) | regular-session day values |
| `binance` | tickers ending in `USDT`, `BTC`, `ETH`, … | `lastPrice` / `openPrice` of the **rolling 24 h** ticker | rolling 24 h; volume in the base asset |
| `sample` | any (`CHARTGEN_DATA_SOURCE=sample`) | forming daily bar's close / previous daily close | forming daily bar |

Live quotes are cached per symbol for 3 s, and the cache is shared by
all clients, so tabs polling the same symbols share one upstream fetch
per symbol at most every 3 s. Per-symbol errors are cached the same way.
The Binance and Yahoo fetches for one call run at the same time.

- **Binance** symbols are fetched with one batched `/api/v3/ticker/24hr`
  request. Binance rejects the whole batch with HTTP 400 if any symbol is
  unknown (`-1121`) or malformed (`-1100`). Only then is each symbol
  requested on its own, so only the bad one fails. The bad symbol is
  remembered for 5 minutes: it gets its error from memory and stays out
  of the batch. Other failures, including rate limits (429/418), fail
  every Binance symbol of that call and are never retried per symbol.
- **Yahoo** symbols cost one request each, run by a queue of at most 8
  workers. The symbol is percent-encoded as a single URL path segment.

Each request times out after 10 s.

#### `search_symbols`

Search tickers and names across all Binance spot pairs and Yahoo Finance
instruments, like TradingView's symbol search. The web symbol search dialog
calls it.

Required: `query` (string). Optional: `limit` (integer 1–50, default 30),
`types` (array of `crypto`, `stock`, `etf`, `fund`, `index`, `forex`,
`future`; omitted or empty means all). The query is trimmed and must be 1–40
characters without control characters. A bad query, `limit` or `types` is a
tool error.

```json
{
  "results": [
    {
      "symbol": "SPY",
      "name": "State Street SPDR S&P 500 ETF Trust",
      "exchange": "NYSEArca",
      "type": "etf",
      "source": "yahoo"
    },
    {
      "symbol": "BTCUSDT",
      "name": "BTC / USDT",
      "exchange": "Binance",
      "type": "crypto",
      "source": "binance"
    }
  ],
  "warnings": ["yahoo: Yahoo API returned status 429 Too Many Requests"]
}
```

`warnings` is present only when a source failed. Results are ranked: exact
match first, then symbol prefix, then symbol contains, then name match. An
exact match is the symbol itself or its root: the base of a crypto pair
(`BTC` for `BTCUSDT`), the base currency of `EURUSD=X`, the `ES` of `ES=F`
and the `GSPC` of `^GSPC`. Among exact matches the order is coins (crypto
with a non-fiat base), futures, forex (USD quote first), indices, crypto
pairs with a fiat base (`EURUSDT`), then stocks, ETFs and funds, so `btc`
gives `BTCUSDT`, `BTC=F`, then the Grayscale ETF `BTC`. After that USDT/USD
quotes come first, then shorter symbols. Remaining ties keep Binance before
Yahoo and Yahoo's own order. Duplicate symbols are dropped.
`symbol` can be passed straight to `generate_chart` or `get_quotes`.

| `source` | Where | Notes |
|---|---|---|
| `binance` | `/api/v3/exchangeInfo`, `TRADING` spot pairs only (about 3000) | fetched once and cached in memory for 6 h, searched locally; `type` is `crypto`, `name` is `"BASE / QUOTE"` |
| `yahoo` | `/v1/finance/search`, 20 quotes | cached per query for 60 s; `type` from `quoteType` (`EQUITY` → `stock`, `ETF` → `etf`, `INDEX` → `index`, `CURRENCY` → `forex`, `FUTURE` → `future`, `MUTUALFUND` → `fund`), `exchange` from `exchDisp` |
| `sample` | built-in list of 30 symbols (`CHARTGEN_DATA_SOURCE=sample`) | no network |

Yahoo results of other kinds (including crypto such as `BTC-USD`, which
Binance covers) are dropped, and so are symbols outside `A-Z 0-9 . ^ = - _`
(the symbols are uppercased first).

Binance and Yahoo run at the same time, and a `types` filter skips a
source that cannot match (for example, `["crypto"]` never calls Yahoo).
When one source fails, the other's results are returned and `warnings`
names the failure (`"binance: …"` or `"yahoo: …"`). The call is an error
only when every queried source fails.

Upstream protection:

- Concurrent identical lookups share one upstream request, so a cold cache
  costs one `exchangeInfo` download however many searches arrive.
- A failed lookup is remembered for 60 s instead of retried on every
  keystroke.
- After the 6 h TTL the Binance list is still served while it refreshes in
  the background; a failed refresh keeps the old list and retries after
  60 s.
- Yahoo search is limited to 2 requests per second (burst 2) for the whole
  server, since it shares a host with chart and quote requests. Over the
  limit the call returns the Binance results plus a `yahoo: rate limited`
  warning, and the rejection is not remembered. Cache hits cost no token.
- Each source has a 4 s deadline. A source that misses it is reported as
  `"<source>: timed out"`, its late answer still fills the cache, and the
  other source's results are not held back.

The Yahoo cache holds at most 500 queries and drops the oldest first. Its
key is the query lowercased with inner whitespace collapsed. Each upstream
request times out after 10 s.

#### `subscribe_chart` / `unsubscribe_chart`

Server push of the live tail, so a chart updates as the market trades
instead of once per poll. See
[ADR-0021](/chartgen/decisions/0021-realtime-chart-push/). HTTP transport
only (stdio returns an error); works without `--trade`.

`subscribe_chart` takes the chart's `generate_chart` arguments: `ticker`
(required, 2-20 letters or digits), `timeframe` (default `4h`),
`indicators` (at most 10, with [instance ids](#several-instances-of-one-indicator)
as for `generate_chart`), `chart_style` and `warmup` (default 0, max
1000). Validation and error messages are otherwise the same as for
`generate_chart`, except that a chart using your scripts
([`CHARTGEN_SCRIPTING=1`](#scripted-indicators-chartgen_scripting1)) answers
`not pushable`, because pushed tails run without a user. Poll `generate_chart` for
those. An interval Binance has no stream for follows the coarsest
stream that divides it (`7m` follows `1m`), and each tail is resampled from
that stream's bars as a fetch would be, so a live bar is the bar a reload
shows. The exception is a reconnect short enough to fit inside one native kline whose
REST backfill fails, as during a rate-limit cooldown: that kline's last updates are
lost, and the bar stays wrong until it leaves the window
([ADR-0038](/chartgen/decisions/0038-custom-intervals/)). It returns:

```json
{ "subscription_id": "cs-0b7f…" }
```

From then on the SSE stream (`GET /mcp`) opened with the same Bearer token
and `Mcp-Session-Id` receives:

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/chart_tick",
  "params": { "subscription_id": "cs-0b7f…", "series": { "…": "…" } }
}
```

`series` is exactly what `generate_chart(format=series, bars=2, warmup)`
returns for the same arguments at that moment: the last closed bar and the
forming bar, with every indicator recomputed. Merge it like a polled tail.

- **Pushable symbols.** Binance symbols only. Any other symbol fails with
  an error that starts with `not pushable`: poll `generate_chart` instead.
  With `CHARTGEN_DATA_SOURCE=sample` the same symbols are pushable and a
  synthetic feed ticks every ~500 ms.
- **Cadence.** The server follows Binance's kline stream (every 2 s) and
  its aggTrade stream (each trade), and pushes at most 4 tails per second
  per chart (2 per second when `warmup` is above 300, or when an interval with no
  stream of its own holds more than 302 native bars). The first tail is
  pushed right after subscribing.
- **Sharing.** Subscriptions with equal arguments share one rolling window
  and one recompute. Every chart of a symbol and feed interval shares one Binance
  WebSocket, so `1m` and `7m` of a symbol use the same one.
- **Limits.** At most 4 subscriptions per token, 32 per client address
  (the TCP peer; behind a reverse proxy that is the proxy, unless
  [a header is named](/chartgen/guides/deploy/#sign-in-through-github-and-google)) or with
  [sign-in](/chartgen/reference/oauth/#sign-in) per user, and 64 per
  server, and 10 `subscribe_chart` calls per token per minute. Over a
  limit the call fails with an error starting with `push limit reached`;
  poll that chart instead. Without an open SSE stream for the session it
  fails too, and can be retried once the stream is open. An error starting
  with `feeds busy` means too many feeds are starting across the server (40 new
  ones a minute, 96 at once): it passes within a minute, so poll meanwhile and
  try again.
- **Lifetime.** A subscription ends with `unsubscribe_chart`, when its
  SSE stream closes, or when the stream falls 256 frames behind (the stream
  is then unlinked). A WebSocket closes 30 s after its last chart goes.

`unsubscribe_chart` takes `subscription_id` and returns
`{ "unsubscribed": true }`, or `false` if the subscription was already gone
(or belongs to another token).

### Server alerts (`CHARTGEN_SERVER_ALERTS=1`)

Price alerts that run on the server with no browser open, and can call a webhook
([guide](/chartgen/guides/web/#alerts-on-the-server),
[ADR-0042](/chartgen/decisions/0042-server-side-alerts/)). These tools are listed only
when the server runs with `CHARTGEN_SERVER_ALERTS=1` and answer only on the HTTP
transport: an alert belongs to the OAuth client of the bearer token, or with
[sign-in](/chartgen/reference/oauth/#sign-in) to its user, and another client or user
cannot see or change it. They are unrelated to the `--trade` alert tools above. On a
server without them `list_server_alerts` returns `{enabled: false, alerts: []}` and the
others an error. The operator decides who may use them
([`CHARTGEN_SERVER_ALERT_CLIENTS`](/chartgen/guides/deploy/#server-alerts)): by default anyone
may create alerts and nobody gets webhooks. `can_create`, `webhooks` and `push` in the list say
what your client may do.

| Tool | Arguments | Result |
|---|---|---|
| `create_server_alert` | `symbol`, `condition`, `level`, `trigger`; optional `interval` (default `1m`), `evaluate_on`, `message`, `webhook_url`, `sign`, `push` | The alert; `webhook_secret` too, once, with `sign: true` |
| `list_server_alerts` | — | `{enabled, can_create, webhooks, push, limits, alerts: [...]}` |
| `update_server_alert` | `alert_id`; optional `active`, `condition`, `level`, `trigger`, `message`, `webhook_url`, `sign`, `push` | The alert; a new `webhook_secret`, once, with `sign: true` |
| `delete_server_alert` | `alert_id` | `{deleted}`, with any delivery still pending |
| `subscribe_push` | `endpoint`, `p256dh`, `auth` | `{devices}`: registers this browser's push subscription |
| `unsubscribe_push` | `endpoint` | `{removed, devices}` |

- `symbol`: a Binance symbol. A stock is refused. Drawing and indicator alerts cannot
  run here.
- `condition`: `crossing`, `crossing_up`, `crossing_down`, `greater`, `less`. A crossing
  needs a previous reading, so the first price after creating, resuming, editing or a
  server restart never fires one. `greater` and `less` can fire on it.
- `trigger`: `once` switches off after firing. `every` stays on; `greater` and `less`
  wait until the price is back on the other side.
- `evaluate_on`: `live` (default) reads every price update, and `interval` only labels
  the alert, so it can be any interval. `close` reads only the closes of `interval`'s bars,
  which must have a Binance kline stream, so a custom interval is refused.
- A symbol new to the server is checked with one request, so `create_server_alert` can fail
  with `Binance rate limit:` like any fetch. Nothing is stored; retry after the wait it names.
- `update_server_alert`: a new `condition`, `level` or `trigger` switches the alert on and
  re-arms it. A new `message` or webhook leaves its state alone, so a spent alert stays spent.
  `active: false` pauses, `active: true` resumes re-armed. `webhook_url: ""` removes the webhook,
  and a new URL drops the signing secret unless `sign: true` is sent with it. `sign: false` stops
  signing. Symbol, interval and `evaluate_on` cannot change.

An alert in a list:

```json
{
  "id": "61c779cc-a2f2-418c-8965-95f1241994f2",
  "symbol": "BTCUSDT",
  "interval": "1h",
  "evaluate_on": "live",
  "condition": "crossing",
  "level": 65000.0,
  "trigger": "every",
  "message": "breakout",
  "active": true,
  "disarmed": false,
  "created_at": 1793965350122,
  "last_triggered_at": 1793965351000,
  "fire_count": 1,
  "webhook": { "host": "hooks.example.com", "signed": true },
  "last_delivery_at": 1793965351131,
  "last_status": 200,
  "last_error": null,
  "feed": "live",
  "last_price": 65012.5
}
```

Times are unix milliseconds. `webhook` shows the host only, never the URL or the secret.
`feed` is `waiting` when no price arrived for two minutes. `last_status` is the webhook's
HTTP status, and nothing else of its response is kept. `last_error` is why the last
delivery failed or was skipped, or that fires are being limited.

Limits, per user where there is sign-in: 20 alerts per client and 200 not yet spent in total, 16 symbol and interval pairs
watched and 2 per client, 10 creates or updates per client per minute, 6 fires per `every` alert
per minute, 30 webhooks per client and 120 in total per minute. A delivery over those waits its
turn, up to 50 per client and for up to an hour, with the client that waited longest going first;
past that the fire still counts and `last_error` says the webhook was skipped.

A spent alert is deleted 7 days after it fired. An alert is deleted once its owner has had no
token for 30 days, unless the operator listed that client.

#### Webhook URL

Refused when creating, and checked again before every attempt:

- anything but `https` on port 443, or a URL with a user or password, or over 512 bytes;
- an address that is loopback, private, link-local, CGNAT, multicast, reserved, documentation
  or a cloud metadata address, on IPv4 or IPv6, in any spelling (`2130706433`,
  `[::ffff:127.0.0.1]`), IPv6 outside global unicast or in the 6to4, Teredo and NAT64 ranges, or
  in a prefix the operator blocked;
- a name with no dot, or under a private suffix such as `.internal` or `.docker`;
- a host this server fetches from (Binance, Yahoo, alternative.me) or its own host;
- a name with any refused address among its answers. That reads the same as a name that does not
  resolve: `webhook host does not resolve to a public address`.

The connection is pinned to the addresses checked, unless the operator sends webhooks through a
proxy, which then resolves and connects itself. Redirects are not followed: a 3xx is
a failure. Each attempt has 5 s; a timeout, connection failure, 5xx or 429 is retried
twice, after 2 s and 8 s; other answers are final.

#### Webhook payload

`POST` with `Content-Type: application/json`:

```json
{
  "id": "d99a10e1-d742-4b79-9ccc-f75169944c6a",
  "type": "alert.fired",
  "alert_id": "61c779cc-a2f2-418c-8965-95f1241994f2",
  "symbol": "BTCUSDT",
  "interval": "1h",
  "evaluate_on": "live",
  "condition": "crossing",
  "level": 65000.0,
  "price": 65010.5,
  "trigger": "every",
  "message": "breakout",
  "bar_time": 1793962800,
  "fired_at": 1793965351000,
  "fire_count": 3
}
```

`bar_time` is the bar's open in unix seconds, `fired_at` unix milliseconds, `message` may
be `null`. Delivery is at least once: a restart resumes an unfinished delivery for up to
an hour, so deduplicate on `id`. Any 2xx counts as received; the body of the response is
never read.

| Header | Value |
|---|---|
| `X-Chartgen-Event` | The event `id` |
| `X-Chartgen-Timestamp` | Unix seconds of this attempt |
| `X-Chartgen-Signature` | `sha256=<hex>`, only with `sign: true` |
| `User-Agent` | `chartgen-webhook/1` |

The signature is HMAC-SHA256 with the alert's secret over `<timestamp>.<raw body>`. To
verify, compute it over the bytes you received, compare in constant time, and refuse an
old timestamp to stop replays:

```python
import hashlib, hmac

def verify(secret: str, timestamp: str, body: bytes, signature: str) -> bool:
    signed = timestamp.encode() + b"." + body
    mac = hmac.new(secret.encode(), signed, hashlib.sha256).hexdigest()
    return hmac.compare_digest(f"sha256={mac}", signature)
```

The secret is shown once and is never logged or returned again; `sign: true` on
`update_server_alert` replaces it.

**Push** ([ADR-0045](/chartgen/decisions/0045-web-push-for-server-alerts/)). `push: true` on an alert,
with or without a webhook, sends the same JSON to each of your devices through Web Push, encrypted under
RFC 8291. `push` in `list_server_alerts` is `null` where the server will not notify for your client,
else `{public_key, devices}`: subscribe in the browser with that key as `applicationServerKey`, then
pass the subscription to `subscribe_push`. The endpoint must be `https` at a known push service
(`fcm.googleapis.com`, `updates.push.services.mozilla.com`, `*.push.apple.com`,
`*.notify.windows.com`), `p256dh` at most 88 characters and `auth` at most 24. Registering the same
endpoint again updates its keys. You may have 5 devices, and a new one over that replaces the oldest; at
most 30 pushes per minute. On a server whose 2000 devices are full, a new one replaces another client's,
and a client's alerts only notify that client's devices: with
[sign-in](/chartgen/reference/oauth/#sign-in) those are all of its user's, on every client. `subscribe_push` answers
`devices`, the count, and `device`, this one's id. A 404 or 410 from the push service removes the device. Delivery is at least once, so deduplicate on `id`; the app's worker uses it
as the notification's tag. Each alert also returns `push`, `last_push_at`, `last_push_status` and
`last_push_error`, the last outcome across your devices, and `last_push_devices`: the devices its last fire went
to, by id, each with `{at, status, error}`, or `null` while its outcome is not known. A device not there was not sent
that fire.

### Scripted indicators (`CHARTGEN_SCRIPTING=1`)

User-authored Starlark indicators
([guide](/chartgen/guides/scripted-indicators/),
[ADR-0024](/chartgen/decisions/0024-starlark-script-runtime/),
[ADR-0025](/chartgen/decisions/0025-user-indicator-registry/)). These tools
are listed and served only when the server runs with
`CHARTGEN_SCRIPTING=1`. Scripts belong to the OAuth client of the bearer
token, or with [sign-in](/chartgen/reference/oauth/#sign-in) to its user, so the
tools that store or read scripts need the HTTP transport
(stdio has no user). Once registered, a script's name works in
`generate_chart` (resolved before built-ins, so it can shadow one for its
owner) and shows up in `list_indicators`. The generated TypeScript input
types do not include these tools.

| Tool | Arguments | Result |
|---|---|---|
| `register_indicator` | `name` (`^[a-z][a-z0-9_]{0,47}$`), `source` (≤ 64 KiB), `create_only` (default `false`) | `{ok: true, name, title, overlay, configurable_params, replaced, shadows_builtin}` |
| `fork_indicator` | `builtin_name` | `{name, source, configurable_params}` for a built-in with a reference implementation (`wavetrend`); otherwise an error listing the forkable ones |
| `list_my_indicators` | — | `{total, indicators: [{name, title, description, type, overlay, configurable_params?, source: "user", created_at, updated_at}]}` |
| `get_my_indicator` | `name` | `{name, source, updated_at}`: the stored Starlark source. With no such script: `isError: true` with `{ok: false, error: "not_found", message}`, which a failed read never has |
| `delete_indicator` | `name` | `{ok: true, name}`; an error if you have no such script |

`register_indicator` validates in four steps: `parse`, `static`, `presence`,
and `dry_run` (100 synthetic bars, 200 ms). A failure returns `isError: true`
with:

```json
{ "ok": false, "errors": [{ "step": "static", "line": 3, "message": "`len` shadows a builtin; pick another name" }] }
```

Registering an existing name replaces the script (`"replaced": true`), unless
`create_only` is `true`: then the store's insert refuses it, and the error has
`"step": "exists"`. Each
user may store 50 scripts. Scripts are stored in SQLite at `CHARTGEN_DB`
(default `~/.chartgen/chartgen.db`), see the
[persistence reference](/chartgen/reference/persistence/#chartgendb-oauth-clients-and-scripted-indicators).

A `generate_chart` call computes at most 3 scripted indicators, sharing one
1 s budget; further ones fail with an `indicator_errors` entry. At most 4
script evaluations run on the server at once and one per user; a caller
waits for a slot until its deadline, then gets "server busy".
`subscribe_chart` pushes compute built-ins only.

Scripting is meant for trusted or single-user deployments: without sign-in a
script's owner is an OAuth client registration and `/authorize` approves
automatically, and with it anyone with an account can run scripts in the server
process (threat model in
[ADR-0025](/chartgen/decisions/0025-user-indicator-registry/) and
[ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/)).

### Orders & positions (require `--trade`)

| Tool | Purpose | Required args |
|------|---------|---------------|
| `place_order` | Submit a market or limit order. | `symbol`, `side` (`buy`/`sell`), `type` (`market`/`limit`), `quantity`; `price` for limits. |
| `cancel_order` | Cancel an open order. | `order_id`. |
| `get_orders` | List tracked orders. | Optional `filter`: `open` or `all` (default). |
| `get_positions` | Open positions with unrealized PnL. | — |
| `get_balance` | **Stub.** Always returns an error; use `get_positions`. | — |

:::caution
`place_order` currently updates local state and the audit log only — the
engine is not yet wired to submit to an exchange, so `--testnet` and the
absence of `--testnet` behave the same. See the [trading guide](/chartgen/guides/trading/)
for the full picture and the `trades.log` audit-trail format.
:::

### Alerts (require `--trade`)

| Tool | Purpose | Required args |
|------|---------|---------------|
| `set_alert` | Register a one-shot alert. Fires once, then is removed. | `symbol`, `condition` (see below). |
| `list_alerts` | List all active alerts. | — |
| `cancel_alert` | Remove an alert before it fires. | `alert_id`. |
| `get_notifications` | Drain the triggered-alert queue (polling). | — |

Alert conditions:

```json
{ "price_above": 50000 }
{ "price_below": 45000 }
{ "indicator_signal": { "indicator": "cipher_b", "signal": "gold_buy" } }
```

Signal labels come from a fixed registry in `src/trading/signals.rs` — see
the [trading guide](/chartgen/guides/trading/#signal-labels) for the full list.
`candlestick_patterns` and `chart_patterns` use their own: a pattern's key such as `hammer`, or
`double_top_confirmed`, on the newest closed bar.

`indicator_signal` is not checked when set. It only fires if the indicator is in the server's
`--panels` under that exact name, since the engine computes nothing else; any other name is
accepted and never fires. The engine's window starts empty and holds at most 200 bars, with no
history loaded, so an indicator needing more bars stays silent until they have closed, and
`chart_patterns` reads those 200 rather than the web chart's 600.

### Push notifications (require `--trade`)

Instead of polling `get_notifications`, subscribe for JSON-RPC pushes over SSE:

| Tool | Purpose | Required args |
|------|---------|---------------|
| `subscribe_notifications` | Register filters for push notifications. | Optional `symbols` (array), `alert_types` (`price_above`/`price_below`/`indicator_signal`). |
| `unsubscribe_notifications` | Remove the current subscription. | — |
| `list_subscriptions` | Active filters and offline queue size. | — |

Subscriptions persist to `~/.chartgen/subscriptions.json`; queued alerts are
kept for up to one hour. See [push notifications](/chartgen/guides/notifications/).
