---
title: Persistence
description: On-disk state written by the trading engine — alerts, audit log, subscriptions.
---

`--trade` writes state under `~/.chartgen/` (Windows: `%USERPROFILE%\.chartgen\`).
The directory is created on startup if missing. Override by setting `$HOME`
before launch.

| File | Written by | Format |
|------|------------|--------|
| `alerts.json` | `set_alert`, `cancel_alert`, fired alerts | JSON array, atomic replace |
| `trades.log` | Order lifecycle + alert triggers | Append-only text |
| `subscriptions.json` | `subscribe_notifications`, `unsubscribe_notifications` | JSON map keyed by token |

## `alerts.json`

Array of alerts awaiting evaluation. Each entry:

```json
[
  {
    "id": "8f3a…",
    "symbol": "BTCUSDT",
    "condition": { "PriceAbove": 50000 },
    "created_at": "2026-04-19T12:00:00Z"
  },
  {
    "id": "b21c…",
    "symbol": "ETHUSDT",
    "condition": {
      "IndicatorSignal": { "indicator": "cipher_b", "signal": "gold_buy" }
    },
    "created_at": "2026-04-19T12:05:00Z"
  }
]
```

`condition` is a tagged enum with three variants:

- `{ "PriceAbove": <f64> }`
- `{ "PriceBelow": <f64> }`
- `{ "IndicatorSignal": { "indicator": "<name>", "signal": "<label>" } }`

Writes use a `.tmp` + rename sequence so readers never see a partial file.
The engine only rewrites `alerts.json` when the set changes (add, remove, or
fire) — idle bars cost zero writes.

:::note[On-disk vs wire format]
The condition keys above (`PriceAbove`, `PriceBelow`, `IndicatorSignal`) are
`PascalCase` because the `AlertCondition` enum in `src/trading/alert.rs`
uses serde's default externally-tagged encoding. The **MCP `set_alert` tool**
accepts `snake_case` (`price_above`, etc.) and translates it into the enum
at the request boundary in `src/mcp.rs`. If you are editing `alerts.json`
directly, use PascalCase; if you are calling `set_alert` over MCP, use
snake_case — see the [trading guide](/chartgen/guides/trading/#alerts).
:::

See the [trading guide](/chartgen/guides/trading/#signal-labels) for the
valid `signal` labels.

## `trades.log`

Append-only audit trail. One line per event, UTC timestamps:

```text
2026-04-19T12:34:56Z SUBMITTED buy BTCUSDT 0.01 market id=abc123
2026-04-19T12:34:57Z FILLED   buy BTCUSDT 0.01 @ 67432.5 id=abc123
2026-04-19T13:00:00Z CANCELLED id=def456
2026-04-19T13:15:22Z ALERT_TRIGGERED id=ghi789 cipher_b/gold_buy BTCUSDT 1h
```

Event types:

| Prefix | Emitted by | Fields |
|--------|------------|--------|
| `SUBMITTED` | `place_order` | side, symbol, quantity, type, `id=<order_id>` |
| `FILLED` | Exchange fill | side, symbol, quantity, `@ <price>`, `id=<order_id>` |
| `CANCELLED` | `cancel_order` | `id=<order_id>` |
| `ALERT_TRIGGERED` | Alert engine | `id=<alert_id>`, condition summary, symbol, interval |

There is no rotation — rotate or truncate externally if it grows too large.

## `subscriptions.json`

Persisted SSE push-notification filters, keyed by access token. Only
`symbols` and `alert_types` are stored; SSE senders and offline queues are
in-memory and rebuilt on reconnect.

```json
{
  "<token>": {
    "symbols": ["BTCUSDT", "ETHUSDT"],
    "alert_types": ["price_above", "indicator_signal"]
  }
}
```

Both fields may be `null`, meaning "match any". Restarting the server
preserves filters so clients keep their filters across chartgen restarts, but
queued alerts are lost.
