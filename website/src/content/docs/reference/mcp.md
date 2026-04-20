---
title: MCP Integration
description: Connect chartgen to Claude Desktop or Claude.ai — 15 tools across charting, orders, alerts, and notifications.
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

Run `chartgen --serve` behind a TLS-terminating reverse proxy and add your
domain as a remote MCP connector. OAuth is handled by chartgen — see the
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
  push notifications such as `notifications/alert_triggered` as raw JSON-RPC
  notification frames — see [push notifications](/chartgen/guides/notifications/).
  Requires the Bearer token in the `Authorization` header.

`Mcp-Session-Id` is supported: if a client sends the header on a `POST /mcp`
request, the server echoes it back unchanged. chartgen's tool calls are
stateless (auth is per-request via the Bearer token), so no server-side
session state is tracked — the header is surfaced only for clients that
correlate on it.

`POST /` and `POST /message` are retained as aliases of `POST /mcp`, and
`GET /sse` is retained as an alias of `GET /mcp`, for backwards compatibility
with existing Claude.ai connector deployments that predate Streamable HTTP.

## Tool catalog

15 tools. The 12 trading tools require `--trade`; attempting to call them
without it returns an error.

### Charting (always available)

#### `generate_chart`

Render a chart for a symbol. Accepts `ticker`/`symbol`, `timeframe`/`interval`,
`indicators` (string names or objects with overrides), `bars`, `width`,
`height`, and `format`.

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
`{"name": "rsi", "length": 21}` comes back as `indicators.rsi`.

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
        { "index": 0, "label": "RSI", "values": [null, null, 68.2, 72.1] }
      ],
      "signals": [
        { "x": 3, "t": 1713430800, "y": 72.1, "label": "rsi_overbought" }
      ],
      "hlines": [{ "y": 30.0 }, { "y": 70.0 }]
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
        { "y1": [null, null, 67300.1, 67410.9], "y2": [null, null, 66802.4, 66915.3] }
      ]
    },
    "vpvr": {
      "label": "VPVR",
      "is_overlay": true,
      "hbars": [
        { "y": 67432.5, "height": 12.1, "width": 0.82, "offset": 0.0, "left": true }
      ]
    },
    "cipher_b": {
      "label": "Cipher B",
      "is_overlay": false,
      "divlines": [
        { "x1": 12, "t1": 1713427200, "y1": 30.2, "x2": 48, "t2": 1713556800, "y2": 18.7, "dashed": true }
      ]
    }
  }
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

All three channels are omitted from the per-indicator object when the
indicator emits none — matching the existing convention for `lines`,
`histogram`, `signals`, and `hlines`.

#### `list_indicators`

Return the full indicator catalog with descriptions, aliases, and configurable
parameters. Call before `generate_chart` to discover valid names.

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

### Push notifications (require `--trade`)

Instead of polling `get_notifications`, subscribe for JSON-RPC pushes over SSE:

| Tool | Purpose | Required args |
|------|---------|---------------|
| `subscribe_notifications` | Register filters for push notifications. | Optional `symbols` (array), `alert_types` (`price_above`/`price_below`/`indicator_signal`). |
| `unsubscribe_notifications` | Remove the current subscription. | — |
| `list_subscriptions` | Active filters and offline queue size. | — |

Subscriptions persist to `~/.chartgen/subscriptions.json`; queued alerts are
kept for up to one hour. See [push notifications](/chartgen/guides/notifications/).
