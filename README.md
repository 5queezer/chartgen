# chartgen

Trading chart generator and MCP server in Rust. Fetches live data from Yahoo Finance and Binance, renders 33 technical indicators as PNG candlestick charts, and integrates with Claude.ai via OAuth 2.1 PKCE.

![AAPL 1D with EMA Stack, VPVR, RSI, MACD](docs/screenshot.png)

## Quick Start

```bash
# Prerequisites: Rust 1.76+, fontconfig/freetype dev libs
sudo apt-get install libfontconfig1-dev libfreetype-dev  # Debian/Ubuntu

git clone https://github.com/5queezer/chartgen.git && cd chartgen
cargo build --release

# Generate a chart
cargo run -- --symbol AAPL --interval 1d -p ema_stack -p vpvr -p rsi -p macd --output chart.png

# Run as MCP server for Claude Desktop
cargo run -- --mcp

# Run as HTTP server for Claude.ai
cargo run -- --serve
```

## CLI

| Flag | Description | Default |
|------|-------------|---------|
| `--symbol` / `-s` | Ticker (AAPL, BTCUSDT) | Sample data |
| `--interval` / `-i` | 1m, 5m, 15m, 1h, 4h, 1d, 1wk | `4h` |
| `-p` | Indicator (repeat for multiple) | `cipher_b`, `macd` |
| `-n` / `--bars` | Number of bars | `120` |
| `--width` / `--height` | Image dimensions | 1920 x 1080 |
| `--output` / `-o` | Output path | `chart.png` |
| `--mcp` | MCP stdio mode | |
| `--serve` | HTTP server mode | |
| `--port` | Server port | `9315` |

Auto-detection: `USDT`/`BTC`/`ETH` suffixes → Binance, everything else → Yahoo Finance.

## Indicators

33 indicators in 5 categories. All configurable via MCP parameters.

<details>
<summary><strong>Overlays</strong> (12) — drawn on price chart</summary>

| Name | Aliases |
|------|---------|
| `ema_stack` | `ema` |
| `bbands` | `bb`, `bollinger` |
| `keltner` | `kc` |
| `donchian` | `dc` |
| `supertrend` | `st` |
| `sar` | `psar` |
| `ichimoku` | `ichi` |
| `heikin_ashi` | `ha` |
| `vwap` | |
| `vwap_bands` | |
| `pivot` | `pivot_points` |
| `vpvr` | `vp`, `volume_profile` |

</details>

<details>
<summary><strong>Momentum</strong> (9)</summary>

| Name | Aliases |
|------|---------|
| `rsi` | |
| `macd` | |
| `stoch` | `stochastic` |
| `cci` | |
| `williams_r` | `willr` |
| `roc` | |
| `wavetrend` | `wt` |
| `cipher_b` | `cipher`, `cb` |
| `adx` | |

</details>

<details>
<summary><strong>Volatility</strong> (3) · <strong>Volume</strong> (5) · <strong>Crypto</strong> (4)</summary>

| Name | Aliases | Category |
|------|---------|----------|
| `atr` | | Volatility |
| `histvol` | `hv` | Volatility |
| `kalman_volume` | `kv` | Volatility |
| `obv` | | Volume |
| `mfi` | | Volume |
| `cmf` | | Volume |
| `ad` | `adline` | Volume |
| `cvd` | | Volume |
| `funding` | `funding_rate` | Crypto |
| `oi` | `open_interest` | Crypto |
| `long_short` | `ls_ratio` | Crypto |
| `fear_greed` | `fng` | Crypto |

</details>

### Parameter Examples (MCP)

```json
{"name": "rsi", "length": 21}
{"name": "bbands", "length": 30, "mult": 2.5}
{"name": "vpvr", "bins": 32, "side": "right"}
```

## MCP Integration

### Claude Desktop

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

### Claude.ai

Add `https://chartgen.vasudev.xyz` as a remote MCP connector. OAuth 2.1 PKCE is handled automatically.

### Tools

- **`generate_chart`** — render a chart (returns base64 PNG)
- **`list_indicators`** — discover all 33 indicators with parameters

## Architecture

```
src/
├── main.rs           CLI + mode dispatch
├── data.rs           Bar, OhlcvData
├── indicator.rs      Indicator trait, PanelResult, Line/Fill/HBar
├── indicators/
│   ├── mod.rs        Registry (by_name, available, registry)
│   ├── ta_bridge.rs  12 ta-rs wrappers
│   ├── custom.rs     13 custom indicators
│   ├── cipher_b.rs   Cipher B (Pine Script port)
│   ├── external.rs   5 API-based indicators
│   ├── macd.rs       MACD
│   ├── rsi.rs        RSI
│   └── wavetrend.rs  WaveTrend
├── fetch.rs          Yahoo Finance + Binance
├── renderer.rs       plotters-based chart rendering
├── mcp.rs            MCP JSON-RPC (stdio + HTTP shared)
└── server.rs         axum HTTP, OAuth 2.1 PKCE
```

## Deployment

### Docker

```bash
docker build -t chartgen .
docker run -p 9315:9315 -e CHARTGEN_BASE_URL=https://your-domain.com chartgen
```

2-stage build: `rust:1.86-slim` → `debian:bookworm-slim` (~93MB).

### CI/CD

Push to `master` → GitHub Actions runs check/test/clippy/fmt → on success, Coolify auto-deploys.

Required secrets: `COOLIFY_API_TOKEN`, `COOLIFY_APP_UUID`, `COOLIFY_BASE_URL`.

## Testing

```bash
cargo test          # 33 e2e tests
cargo fmt --check   # enforced by pre-commit hook
cargo clippy -- -D warnings
```

## License

MIT
