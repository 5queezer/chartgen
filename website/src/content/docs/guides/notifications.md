---
title: Push notifications
description: Subscribe to triggered alerts via SSE instead of polling.
---

Under `--trade`, chartgen pushes triggered alerts to subscribed clients over
the persistent SSE stream on `/mcp` and `/sse`. This is an alternative to
polling [`get_notifications`](/chartgen/reference/mcp/#alerts-require---trade).

A subscription is bound to the access token in use, so each Claude.ai
connection has its own filters and offline queue.

## Subscribing

```json
{
  "name": "subscribe_notifications",
  "arguments": {
    "symbols": ["BTCUSDT", "ETHUSDT"],
    "alert_types": ["price_above", "indicator_signal"]
  }
}
```

Both `symbols` and `alert_types` are optional. Omit either to match all values
for that field. Valid `alert_types`:

- `price_above`
- `price_below`
- `indicator_signal`

`list_subscriptions` returns the current filters and the offline-queue size
for the calling token. `unsubscribe_notifications` removes the subscription.

## Wire format

Triggered alerts arrive as JSON-RPC **notifications** (no `id` field) on the
SSE stream:

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/alert_triggered",
  "params": {
    "alert_id": "...",
    "symbol": "BTCUSDT",
    "interval": "",
    "condition": {
      "type": "indicator_signal",
      "indicator": "cipher_b",
      "signal": "gold_buy"
    },
    "value": 67432.5,
    "triggered_at": "2026-04-19T12:34:56.123Z"
  }
}
```

`condition.type` is one of `price_above`, `price_below`, `indicator_signal`.
Price conditions include `threshold`; indicator conditions include `indicator`
and `signal`.

## Offline delivery

If the SSE stream is disconnected when an alert fires, the notification is
appended to a per-subscriber offline queue. On reconnect, queued notifications
are drained in order.

- **TTL**: queued notifications are dropped after 1 hour.
- **Capacity**: oldest entries are evicted once the queue hits 1000.
- **Persistence**: subscriptions are stored in `~/.chartgen/subscriptions.json`
  so they survive a restart. See the [persistence reference](/chartgen/reference/persistence/).

## Polling vs push

| | `get_notifications` (poll) | `subscribe_notifications` (push) |
|---|---|---|
| Transport | JSON-RPC response | JSON-RPC notification over SSE |
| Delivery | On request | Immediate |
| Offline buffer | Drained on next call | Queued up to 1 h / 1000 entries |
| Filters | None | Symbols + alert types |
| Multiple consumers | Shared queue | Per-token subscription |

Use polling for simple one-shot agents; use push for long-lived Claude.ai
sessions where you want each alert as it fires.
