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

Example:

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

#### `list_indicators`

Return the full indicator catalog with descriptions, aliases, and configurable
parameters. Call before `generate_chart` to discover valid names.

#### `get_indicators`

Compute indicators and return raw numeric values as JSON. Fetches OHLCV (or
uses cached engine data under `--trade`) and reports each indicator's last
line values, dot presence (signals), and horizontal reference levels.

Required: `symbol`, `indicators` (array of names). Optional: `interval` (default `4h`).

### Orders & positions (require `--trade`)

| Tool | Purpose | Required args |
|------|---------|---------------|
| `place_order` | Submit a market or limit order. | `symbol`, `side` (`buy`/`sell`), `type` (`market`/`limit`), `quantity`; `price` for limits. |
| `cancel_order` | Cancel an open order. | `order_id`. |
| `get_orders` | List tracked orders. | Optional `filter`: `open` or `all` (default). |
| `get_positions` | Open positions with unrealized PnL. | — |
| `get_balance` | Account balances. Needs exchange credentials. | — |

With `--testnet`, orders route to Binance testnet (paper trading). See the
[trading guide](/chartgen/guides/trading/) for the order state machine and
the `trades.log` audit trail format.

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
