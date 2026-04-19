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

## Run tests

```bash
cargo test          # 33 e2e tests
cargo fmt --check
cargo clippy -- -D warnings
```
