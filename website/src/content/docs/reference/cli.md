---
title: CLI Reference
description: Command-line flags for chartgen.
---

| Flag | Description | Default |
|------|-------------|---------|
| `--symbol` / `-s` | Ticker (AAPL, BTCUSDT) | Sample data |
| `--interval` / `-i` | 1m, 5m, 15m, 1h, 4h, 1d, 1wk | `4h` |
| `-p` | Indicator (repeat for multiple) | `cipher_b`, `macd` |
| `-n` / `--bars` | Number of bars | `120` |
| `--width` / `--height` | Image dimensions | `1920 x 1080` |
| `--output` / `-o` | Output path | `chart.png` |
| `--mcp` | MCP stdio mode | |
| `--serve` | HTTP server mode | |
| `--port` | Server port | `9315` |

## Data source auto-detection

| Suffix | Source |
|--------|--------|
| `USDT`, `BTC`, `ETH` | Binance |
| everything else | Yahoo Finance |

## Example

```bash
chartgen -s BTCUSDT -i 4h -n 200 \
  -p ema_stack -p vpvr -p wavetrend \
  --width 2560 --height 1440 \
  -o btc.png
```
