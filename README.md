# chartgen

A Rust-based trading chart generator and MCP server. Fetches OHLCV data from Yahoo Finance (stocks) and Binance (crypto), computes 33 technical indicators, and renders publication-quality PNG candlestick charts. Integrates with Claude.ai as a remote MCP tool via OAuth 2.1 PKCE.

## Key Features

- 33 technical indicators: momentum, trend, volatility, volume, and crypto-specific
- Real-time data from Yahoo Finance and Binance
- VuManChu Cipher B full port from Pine Script
- VPVR (Volume Profile Visible Range) with horizontal histogram bars
- LLM-optimized chart rendering with panel labels and value annotations
- MCP server with OAuth 2.1 PKCE for Claude.ai integration
- Self-describing indicator registry with configurable parameters
- 2-stage Docker build (93MB runtime image)

## Table of Contents

- [Tech Stack](#tech-stack)
- [Prerequisites](#prerequisites)
- [Getting Started](#getting-started)
- [Usage](#usage)
- [Architecture](#architecture)
- [Indicators](#indicators)
- [MCP Integration](#mcp-integration)
- [Environment Variables](#environment-variables)
- [Testing](#testing)
- [Deployment](#deployment)
- [Troubleshooting](#troubleshooting)
- [License](#license)

## Tech Stack

- **Language**: Rust (2021 edition)
- **Chart Rendering**: plotters 0.3 (bitmap backend, TTF fonts)
- **Technical Analysis**: ta 0.5 (streaming indicators)
- **HTTP Server**: axum 0.8 + tokio
- **Data Fetching**: reqwest 0.12 (blocking)
- **Auth**: OAuth 2.1 PKCE (SHA-256, in-memory store)
- **CLI**: clap 4 (derive)
- **Deployment**: Docker + Coolify on Hetzner
- **CI/CD**: GitHub Actions

## Prerequisites

- Rust 1.76+ (install via [rustup](https://rustup.rs))
- System libraries for font rendering:
  ```bash
  # Ubuntu/Debian
  sudo apt-get install libfontconfig1-dev libfreetype-dev

  # macOS (via Homebrew)
  brew install fontconfig freetype

  # Fedora
  sudo dnf install fontconfig-devel freetype-devel
  ```
- Docker (optional, for containerized deployment)

## Getting Started

### 1. Clone the Repository

```bash
git clone https://github.com/5queezer/chartgen.git
cd chartgen
```

### 2. Build

```bash
cargo build --release
```

The binary is at `target/release/chartgen`.

### 3. Generate a Chart

```bash
# Sample data with default indicators (EMA stack + Cipher B + MACD)
cargo run -- --output chart.png

# Real stock data
cargo run -- --symbol AAPL --interval 1d --output aapl.png

# Crypto with custom indicators
cargo run -- --symbol BTCUSDT --interval 4h --panels rsi,macd,bbands --output btc.png
```

## Usage

### CLI Options

| Flag | Description | Default |
|------|-------------|---------|
| `--symbol` | Ticker symbol (AAPL, BTCUSDT, etc.) | Sample data |
| `--interval` | Timeframe: 1m, 5m, 15m, 1h, 4h, 1d, 1wk | `4h` |
| `--panels` | Comma-separated indicator names | `cipher_b,macd` |
| `--bars` | Number of candlestick bars | `120` |
| `--width` | Image width in pixels | `1920` |
| `--height` | Image height in pixels | `1080` |
| `--output` | Output PNG file path | `chart.png` |
| `--source` | Data source: `auto`, `yahoo`, `binance` | `auto` |
| `--mcp` | Run as MCP stdio server | - |
| `--serve` | Run as HTTP server with OAuth | - |
| `--port` | HTTP server port | `9315` |

### Modes

**File output** (default):
```bash
chartgen --symbol TSLA --interval 1d --panels ema_stack,rsi,vpvr --output tsla.png
```

**MCP stdio server** (for local Claude Desktop):
```bash
chartgen --mcp
```

**HTTP server** (for remote Claude.ai):
```bash
chartgen --serve --port 9315
```

### Data Source Auto-Detection

Symbols ending in `USDT`, `BTC`, `ETH`, `BNB`, `BUSD` route to Binance. Everything else routes to Yahoo Finance. Override with `--source`.

## Architecture

### Directory Structure

```
src/
├── main.rs              # CLI entry point, mode dispatch
├── lib.rs               # Library re-exports
├── data.rs              # Bar, OhlcvData structs, sample data generator
├── indicator.rs         # Indicator trait, PanelResult, rendering primitives
├── indicators/
│   ├── mod.rs           # Registry, by_name(), by_name_configured(), available()
│   ├── ta_bridge.rs     # 12 indicators wrapping the ta crate
│   ├── custom.rs        # 13 custom indicators (VPVR, ADX, Ichimoku, etc.)
│   ├── cipher_b.rs      # VuManChu Cipher B (full Pine Script port)
│   ├── macd.rs          # MACD + Signal + Histogram
│   ├── rsi.rs           # RSI with OB/OS levels
│   ├── wavetrend.rs     # WaveTrend oscillator
│   └── external.rs      # 5 indicators requiring API calls (Funding, OI, etc.)
├── fetch.rs             # Yahoo Finance + Binance data fetching
├── renderer.rs          # Chart rendering with plotters
├── mcp.rs               # MCP JSON-RPC protocol (stdio + shared handler)
└── server.rs            # HTTP server, OAuth 2.1 PKCE, SSE transport
tests/
└── e2e.rs               # 33 end-to-end tests
```

### Data Flow

```
User Request
    │
    ├─ CLI: --symbol AAPL --panels rsi,macd
    ├─ MCP: {"method":"tools/call","params":{"name":"generate_chart",...}}
    └─ HTTP: POST /mcp (Bearer token)
    │
    ▼
fetch.rs ──── Yahoo Finance / Binance API ──── OhlcvData (Vec<Bar>)
    │
    ▼
indicators/mod.rs ──── by_name("rsi") ──── Box<dyn Indicator>
    │
    ▼
indicator.compute(data) ──── PanelResult { lines, fills, bars, dots, hbars, ... }
    │
    ▼
renderer.rs ──── plotters ──── PNG (file or base64)
```

### Indicator Trait

Every indicator implements:

```rust
pub trait Indicator {
    fn name(&self) -> &str;
    fn compute(&self, data: &OhlcvData) -> PanelResult;
    fn description(&self) -> &str;          // for MCP tool discovery
    fn params(&self) -> Value;              // configurable parameters as JSON
    fn configure(&mut self, params: &Value); // apply custom config
}
```

`PanelResult` contains rendering primitives:

| Field | Type | Description |
|-------|------|-------------|
| `lines` | `Vec<Line>` | Time-series lines (y values, color, width, label) |
| `fills` | `Vec<Fill>` | Shaded regions between two series |
| `bars` | `Vec<Bars>` | Vertical histogram bars |
| `dots` | `Vec<Dot>` | Scatter points (signals, divergences) |
| `hlines` | `Vec<HLine>` | Horizontal reference lines |
| `hbars` | `Vec<HBar>` | Horizontal bars for VPVR (left or right aligned) |
| `is_overlay` | `bool` | Draw on price chart vs separate panel |

## Indicators

### 33 Available Indicators

**Trend (Overlays)**

| Name | Aliases | Description |
|------|---------|-------------|
| `ema_stack` | `ema` | EMA 8/21/50/200 stack |
| `bbands` | `bb`, `bollinger` | Bollinger Bands (20, 2.0) |
| `keltner` | `kc` | Keltner Channels |
| `donchian` | `dc` | Donchian Channels |
| `supertrend` | `st` | Supertrend (10, 3.0) |
| `sar` | `psar` | Parabolic SAR |
| `ichimoku` | `ichi` | Ichimoku Cloud |
| `heikin_ashi` | `ha` | Heikin Ashi candles |
| `vwap` | - | Volume Weighted Average Price |
| `vwap_bands` | - | VWAP with standard deviation bands |
| `pivot` | `pivot_points` | Pivot Points (P, R1, R2, S1, S2) |
| `vpvr` | `vp`, `volume_profile` | Volume Profile Visible Range |

**Momentum (Panels)**

| Name | Aliases | Description |
|------|---------|-------------|
| `rsi` | - | Relative Strength Index (14) |
| `macd` | - | MACD (12, 26, 9) |
| `stoch` | `stochastic` | Slow Stochastic (14, 3, 3) |
| `cci` | - | Commodity Channel Index (20) |
| `williams_r` | `willr` | Williams %R (14) |
| `roc` | - | Rate of Change (12) |
| `wavetrend` | `wt` | WaveTrend oscillator |
| `cipher_b` | `cipher`, `cb` | VuManChu Cipher B composite |
| `adx` | - | Average Directional Index (14) |

**Volatility (Panels)**

| Name | Aliases | Description |
|------|---------|-------------|
| `atr` | - | Average True Range (14) |
| `histvol` | `hv` | Historical Volatility (20) |
| `kalman_volume` | `kv` | Kalman Volume Filter |

**Volume (Panels)**

| Name | Aliases | Description |
|------|---------|-------------|
| `obv` | - | On-Balance Volume |
| `mfi` | - | Money Flow Index (14) |
| `cmf` | - | Chaikin Money Flow (20) |
| `ad` | `adline` | Accumulation/Distribution Line |
| `cvd` | - | Cumulative Volume Delta |

**Crypto-Specific (Panels)**

| Name | Aliases | Description |
|------|---------|-------------|
| `funding` | `funding_rate` | Binance Futures Funding Rate |
| `oi` | `open_interest` | Open Interest History |
| `long_short` | `ls_ratio` | Long/Short Account Ratio |
| `fear_greed` | `fng` | Fear & Greed Index |

### Configurable Parameters

Indicators accept parameters via MCP or the registry:

```json
{"name": "rsi", "length": 21}
{"name": "bbands", "length": 30, "mult": 2.5}
{"name": "vpvr", "bins": 32, "side": "right"}
{"name": "supertrend", "length": 14, "factor": 2.0}
```

Call `list_indicators` via MCP to see all parameters with defaults.

## MCP Integration

### Claude Desktop (stdio)

Add to `claude_desktop_config.json`:

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

### Claude.ai (remote HTTP)

Connect to `https://chartgen.vasudev.xyz` as a custom MCP connector. The server handles OAuth 2.1 PKCE automatically.

### MCP Tools

**`generate_chart`** — Renders a PNG chart and returns it as base64.

```json
{
  "ticker": "BTCUSDT",
  "timeframe": "4h",
  "indicators": ["ema_stack", "rsi", {"name": "bbands", "length": 30}],
  "bars": 200,
  "width": 1920,
  "height": 1080
}
```

**`list_indicators`** — Returns metadata for all 33 indicators including descriptions, parameters, and overlay/panel classification.

### OAuth 2.1 Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/.well-known/oauth-authorization-server` | GET | RFC 8414 metadata |
| `/register` | POST | Dynamic client registration (RFC 7591) |
| `/authorize` | GET | Authorization with PKCE S256 challenge |
| `/token` | POST | Token exchange (form or JSON) |
| `/` | GET | Health check (no auth required) |
| `/`, `/mcp`, `/message` | POST | MCP JSON-RPC endpoint |
| `/sse`, `/mcp` | GET | SSE transport |

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `CHARTGEN_BASE_URL` | Public URL for OAuth issuer | `http://localhost:9315` |

### GitHub Actions Secrets

| Secret | Description |
|--------|-------------|
| `COOLIFY_API_TOKEN` | Coolify API bearer token |
| `COOLIFY_APP_UUID` | Coolify application UUID |
| `COOLIFY_BASE_URL` | Coolify instance URL |

## Testing

### Run All Tests

```bash
cargo test
```

### Test Structure (33 tests)

| Category | Count | Description |
|----------|-------|-------------|
| Helper functions | 8 | EMA, SMA, lowest, highest, stoch edge cases |
| All indicators (200 bars) | 1 | Every indicator produces valid output |
| All indicators (5 bars) | 1 | Minimal data handling |
| All indicators (1 bar) | 1 | Single-bar edge case |
| All indicators (0 bars) | 1 | Empty data handling |
| Overlay classification | 1 | Overlays vs panels correctly flagged |
| Alias resolution | 1 | All aliases resolve to indicators |
| Chart rendering | 7 | Empty, single overlay/panel, mixed, all indicators |
| External indicators | 2 | Graceful failure without network |
| Data edge cases | 1 | Sample data sizes |
| Registry meta | 3 | available() count, by_name(), unknown returns None |

### Linting

```bash
cargo fmt --check    # formatting
cargo clippy -- -D warnings  # lints
```

Both are enforced by a pre-commit hook and CI.

## Deployment

### Docker

```bash
docker build -t chartgen .
docker run -p 9315:9315 -e CHARTGEN_BASE_URL=https://your-domain.com chartgen
```

The image uses a 2-stage build:
- **Build**: `rust:1.86-slim` with dev libraries
- **Runtime**: `debian:bookworm-slim` (~93MB) with `fonts-dejavu-core`

### Coolify

The repo includes auto-deploy: CI passes on `master` → GitHub Actions triggers Coolify restart via API. Configure the three secrets (`COOLIFY_API_TOKEN`, `COOLIFY_APP_UUID`, `COOLIFY_BASE_URL`) and it works automatically.

### Manual

```bash
cargo build --release
./target/release/chartgen --serve --port 9315
```

The release profile is optimized for size: `opt-level="s"`, LTO, stripped symbols.

## Troubleshooting

### Build fails with font/freetype errors

```
error: could not find system library 'fontconfig'
```

Install system dependencies:
```bash
sudo apt-get install libfontconfig1-dev libfreetype-dev  # Debian/Ubuntu
```

### Yahoo Finance returns empty data

Yahoo may rate-limit or block certain intervals. The 4h timeframe is synthesized by fetching 1h data and aggregating every 4 bars. If fetching fails, the chart falls back to sample data.

### Binance API errors for non-crypto symbols

Binance only supports crypto pairs (BTCUSDT, ETHUSDT, etc.). Use `--source yahoo` for stocks, or let auto-detection handle it.

### MCP "Authorization failed" in Claude.ai

1. Verify the server is reachable: `curl https://your-domain.com/`
2. Check OAuth metadata: `curl https://your-domain.com/.well-known/oauth-authorization-server`
3. Disconnect and reconnect the connector in Claude.ai settings
4. `tools/list` and `initialize` work without auth; `tools/call` requires a Bearer token

### Chart renders but panels are empty

Some external indicators (Funding Rate, Open Interest, Long/Short Ratio) require a crypto symbol on Binance Futures. They return empty panels for stock symbols.

## License

MIT
