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
|  | `--port` | integer | `9315` | HTTP server port (used with `--serve`, and with `--trade` when combined with `--serve`). |
|  | `--trade` | flag | off | Start the trading engine: live WebSocket feed, indicator state manager, order tracker, and alert engine. Exposes 12 extra MCP tools. Combine with `--serve` to talk to it over HTTP. See the [trading guide](/chartgen/guides/trading/). |
|  | `--testnet` | flag | off | Route `--trade` orders to Binance testnet (paper trading). No real funds move. |

## Operating modes

`chartgen` has four modes, picked by the first matching flag:

1. **One-shot render** (no mode flag) — fetch data, render `chart.png`, exit.
2. **`--mcp`** — MCP stdio JSON-RPC, for Claude Desktop.
3. **`--serve`** — HTTP MCP server with OAuth 2.1 PKCE, for Claude.ai.
4. **`--trade`** — trading engine. Combine with `--serve` to expose the trading
   MCP tools over HTTP; on its own it runs the engine without a server.

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

Live trading engine on testnet, served over HTTP for Claude.ai:

```bash
chartgen --serve --trade --testnet \
  --symbol BTCUSDT --interval 1h \
  -p cipher_b -p rsi_mfi_stoch
```
