---
title: CLI Reference
description: Command-line flags for chartgen.
---

| Short | Long | Type | Default | Description |
|-------|------|------|---------|-------------|
| `-s` | `--symbol` | string | sample data | Ticker (e.g. `AAPL`, `BTCUSDT`). If omitted, chartgen uses random sample data. |
| `-i` | `--interval` | string | `4h` | Candle interval: `1m`, `5m`, `15m`, `1h`, `4h`, `1d`, `1wk`. |
| `-p` | `--panels` | string (repeat) | `cipher_b`, `macd` | Indicator names, bottom to top. Repeat the flag for multiple indicators. |
| `-n` | `--bars` | integer | `120` | Number of bars. |
| `-o` | `--output` | string | `chart.png` | Output file path. |
|  | `--width` | integer | `1920` | Image width in pixels. |
|  | `--height` | integer | `1080` | Image height in pixels. |
|  | `--source` | string | `auto` | Data source: `auto`, `binance`, or `yahoo`. `auto` picks by ticker suffix. |
|  | `--mcp` | flag | off | Run as MCP stdio JSON-RPC server (for Claude Desktop). |
|  | `--serve` | flag | off | Run as remote MCP HTTP server with OAuth 2.1 PKCE. |
|  | `--port` | integer | `9315` | HTTP server port (used with `--serve` and `--trade`). |
|  | `--trade` | flag | off | Run in trading mode with a live WebSocket feed and alert engine. |
|  | `--testnet` | flag | off | Use Binance testnet (paper trading) when in `--trade` mode. |

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
