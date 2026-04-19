---
title: Trading
description: Live WebSocket feed, order lifecycle, and price/indicator alerts.
---

`--trade` starts a live engine: a Binance WebSocket kline feed, a rolling
200-bar indicator state manager, an order tracker, and an alert engine.
Combine with `--serve` to expose 12 extra MCP tools over HTTP.

```bash
chartgen --serve --trade --testnet \
  --symbol BTCUSDT --interval 1h \
  -p cipher_b -p rsi_mfi_stoch
```

Add `--testnet` to route orders to Binance testnet (paper trading). Without it,
`place_order` hits the live exchange.

## What the engine runs

- **WebSocket feed** (`src/feed.rs`) — streams 1-minute klines for the
  configured symbol and reconnects on drop, backfilling via REST.
- **Bar aggregator** (`src/feed.rs`) — derives `5m`, `15m`, `1h`, and `4h`
  bars from the 1-minute stream so alerts can fire on higher timeframes
  without a second subscription.
- **Indicator state** (`src/indicator_state.rs`) — keeps a rolling
  200-bar window per (symbol, interval) and recomputes indicators on every
  closed bar.
- **Alert engine** (`src/trading/alert.rs`) — evaluates registered alerts
  against each new bar + indicator result.
- **Order tracker** (`src/trading/order.rs`, `position.rs`) — local state
  machine (`Pending` → `Open` → `Filled`/`Cancelled`/`Rejected`) and
  unrealized-PnL tracking.
- **Audit log** (`src/trading/persistence.rs`) — append-only `trades.log`.

All state lives under `~/.chartgen/`. See the
[persistence reference](/chartgen/reference/persistence/) for on-disk formats.

## Orders

Place a market order:

```json
{
  "name": "place_order",
  "arguments": {
    "symbol": "BTCUSDT",
    "side": "buy",
    "type": "market",
    "quantity": 0.01
  }
}
```

Limit orders require a `price`. `cancel_order` takes the `order_id` returned
by `place_order`. `get_orders` lists tracked orders (filter `open` or `all`);
`get_positions` reports open positions with unrealized PnL; `get_balance`
returns exchange balances (requires credentials).

Every order state transition is appended to `~/.chartgen/trades.log`:

```
2026-04-19T12:34:56Z SUBMITTED buy BTCUSDT 0.01 market id=abc123
2026-04-19T12:34:57Z FILLED   buy BTCUSDT 0.01 @ 67432.5 id=abc123
```

## Alerts

Alerts are **one-shot**: the first matching bar triggers the alert, enqueues
a notification, and removes the alert from the active list. The persistent
state in `~/.chartgen/alerts.json` is rewritten atomically only when an alert
fires or is added/removed — idle bars cost zero writes.

Three condition types:

```json
{ "price_above": 50000 }
{ "price_below": 45000 }
{ "indicator_signal": { "indicator": "cipher_b", "signal": "gold_buy" } }
```

### Signal labels

`indicator_signal` alerts match against a fixed label registry in
`src/trading/signals.rs`. Using anything else will never match. Current labels:

| Group | Labels |
|-------|--------|
| Shared | `buy`, `sell`, `rsi_oversold`, `rsi_overbought` |
| Cipher B | `buy_small`, `sell_small`, `wt_bull_div`, `wt_bear_div`, `rsi_bull_div`, `rsi_bear_div`, `stoch_bull_div`, `stoch_bear_div`, `any_bull_div`, `any_bear_div`, `gold_buy` |
| RSI/MFI/Stoch combo | `stoch_oversold`, `stoch_overbought` |
| WaveTrend | `cross_up_os`, `cross_down_ob`, `cross_up`, `cross_down` |
| Custom (Supertrend / Chandelier / Heikin-Ashi / momentum) | `flip_up`, `flip_down`, `buy_oversold`, `sell_overbought` |

## Consuming alerts

Two styles, pick one:

- **Poll** with [`get_notifications`](/chartgen/reference/mcp/#alerts-require---trade) — drains the triggered-alert queue on each call.
- **Push** via [`subscribe_notifications`](/chartgen/guides/notifications/) over the SSE stream. Subscriptions persist and have an offline queue (default TTL 1 h).

Triggered alerts are also logged to `trades.log` as `ALERT_TRIGGERED` lines.
