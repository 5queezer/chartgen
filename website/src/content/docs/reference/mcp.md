---
title: MCP Integration
description: Connect chartgen to Claude Desktop or Claude.ai — 15 tools across charting, orders, alerts, and notifications.
---

chartgen speaks the Model Context Protocol over both stdio (Claude Desktop)
and HTTP (Claude.ai). Both transports share the same tool definitions.

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

### Claude.ai (HTTP + OAuth 2.1 PKCE)

Run `chartgen --serve` behind a TLS-terminating reverse proxy and add your
domain as a remote MCP connector. OAuth is handled by chartgen — see the
[OAuth reference](/chartgen/reference/oauth/) for the endpoint catalog and
token lifetime. A persistent SSE stream on `/mcp` and `/sse` delivers push
notifications — see [notifications](/chartgen/guides/notifications/).

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
