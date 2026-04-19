---
title: CLI Reference
description: Command-line flags for chartgen.
---

| Short | Long | Type | Default | Description |
|-------|------|------|---------|-------------|
| `-s` | `--symbol` | string | sample data | Ticker (e.g. `AAPL`, `BTCUSDT`). If omitted, chartgen uses random sample data. In `--trade` mode, defaults to `BTCUSDT`. |
| `-i` | `--interval` | string | `4h` | Candle interval: `1m`, `5m`, `15m`, `1h`, `4h`, `1d`, `1wk`. |
| `-p` | `--panels` | string (repeat) | `cipher_b`, `macd` | Indicator names, bottom to top. Repeat the flag for multiple indicators. See the [indicator reference](/chartgen/reference/indicators/). |
| `-n` | `--bars` | integer | `120` | Number of bars. |
| `-o` | `--output` | string | `chart.png` | Output file path (one-shot render mode only). |
|  | `--width` | integer | `1920` | Image width in pixels. |
|  | `--height` | integer | `1080` | Image height in pixels. |
|  | `--source` | string | `auto` | Data source: `auto`, `binance`, or `yahoo`. `auto` picks by ticker suffix (see below). |
|  | `--mcp` | flag | off | Run as MCP stdio JSON-RPC server. Use with Claude Desktop. |
|  | `--serve` | flag | off | Run as remote MCP HTTP server with OAuth 2.1 PKCE. See [MCP integration](/chartgen/reference/mcp/) and [OAuth](/chartgen/reference/oauth/). |
|  | `--port` | integer | `9315` | HTTP server port, used by both `--serve` and `--trade`. |
|  | `--trade` | flag | off | Start the trading engine: live WebSocket feed, indicator state manager, order tracker, alert engine, and the same HTTP + OAuth server as `--serve`. Exposes 12 extra MCP tools on top of the 3 charting tools. Mutually exclusive with `--serve`. See the [trading guide](/chartgen/guides/trading/). |
|  | `--testnet` | flag | off | Intended to route `--trade` orders to Binance testnet. Currently a no-op — see the caution in the [trading guide](/chartgen/guides/trading/). |

## Operating modes

`chartgen` has four modes, picked by the **first** matching flag in this
order (`src/main.rs` dispatches sequentially and returns early):

1. **`--mcp`** — MCP stdio JSON-RPC, for Claude Desktop.
2. **`--serve`** — HTTP MCP server with OAuth 2.1 PKCE, for Claude.ai
   (charting tools only).
3. **`--trade`** — trading engine plus the same HTTP + OAuth server as
   `--serve`, with all 15 tools exposed.
4. **One-shot render** (no mode flag) — fetch data, render `chart.png`, exit.

`--serve` and `--trade` are mutually exclusive; passing both runs only
`--serve`. Use `--trade` whenever you want the engine — it already serves HTTP.

## Data source auto-detection

With `--source auto` (the default), the ticker suffix picks the provider:

| Suffix | Source |
|--------|--------|
| `USDT`, `BTC`, `ETH` | Binance |
| anything else | Yahoo Finance |

Force a provider with `--source binance` or `--source yahoo`.

## Example

```bash
chartgen -s BTCUSDT -i 4h -n 200 \
  -p ema_stack -p vpvr -p wavetrend \
  --width 2560 --height 1440 \
  -o btc.png
```

Live trading engine served over HTTP for Claude.ai:

```bash
chartgen --trade --testnet \
  --symbol BTCUSDT --interval 1h \
  --port 9315 \
  -p cipher_b -p rsi_mfi_stoch
```
