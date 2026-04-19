---
title: Quick Start
description: Build chartgen and generate your first chart.
---

## Prerequisites

- Rust 1.76+
- `fontconfig` and `freetype` development libraries

On Debian/Ubuntu:

```bash
sudo apt-get install libfontconfig1-dev libfreetype-dev
```

## Build

```bash
git clone https://github.com/5queezer/chartgen.git
cd chartgen
cargo build --release
```

## Generate a chart

```bash
cargo run -- --symbol AAPL --interval 1d \
  -p ema_stack -p vpvr -p rsi -p macd \
  --output chart.png
```

Symbols ending in `USDT`, `BTC`, or `ETH` are fetched from Binance; everything
else falls back to Yahoo Finance.

## Run as an MCP server

For Claude Desktop (stdio):

```bash
cargo run -- --mcp
```

For Claude.ai (HTTP + OAuth 2.1 PKCE):

```bash
cargo run -- --serve --port 9315
```

## Run in trading mode

`--trade` streams a live Binance WebSocket feed, runs the alert engine, and
exposes the order / position / alert MCP tools over the same HTTP + OAuth
server as `--serve`:

```bash
cargo run -- --trade --testnet --symbol BTCUSDT --interval 1h --port 9315
```

`--trade` and `--serve` are mutually exclusive — mode dispatch in
`src/main.rs` picks the first match and returns. Use `--trade` when you want
the trading engine (it already serves HTTP); use `--serve` when you only need
the charting tools. State lives under `~/.chartgen/` (`alerts.json`,
`trades.log`, `subscriptions.json`).

## Run tests

```bash
cargo test
cargo fmt --check
cargo clippy -- -D warnings
```

## Next steps

- [CLI reference](/chartgen/reference/cli/) — every flag.
- [Indicators](/chartgen/reference/indicators/) — all 38 with sample renders and parameters.
- [MCP integration](/chartgen/reference/mcp/) — the 15 tools exposed over stdio and HTTP.
- [Trading](/chartgen/guides/trading/) — alerts, orders, and the live engine.
- [Push notifications](/chartgen/guides/notifications/) — SSE subscriptions for triggered alerts.
- [Deployment](/chartgen/guides/deploy/) — Docker, Coolify, release binaries, reverse proxy.
- [OAuth](/chartgen/reference/oauth/) — Claude.ai connector flow.
- [Persistence](/chartgen/reference/persistence/) — on-disk state formats.
- [Contributing](/chartgen/guides/contributing/) — dev loop, CI jobs, keeping docs in sync.
