# Trading System Design: Hrafn + Chartgen

**Date:** 2026-04-10
**Status:** Approved (brainstorming complete)
**Authors:** Christian Pojoni, Claude

---

## 1. Overview

A fully automated trading system that:
1. Listens to 100eyes Telegram channel for trading signals
2. Analyzes signals via Claude API (Vision) and cross-checks with own indicators
3. Executes trades via Binance (paper → live)
4. Provides manual control via claude.ai MCP interface

**Two Rust binaries. No Python. No NautilusTrader dependency.**

---

## 2. Architecture

```
┌──────────────────────────────────┐
│     100eyes Telegram Channel     │
└──────────────┬───────────────────┘
               │ grammers (user client)
               ▼                        ┌──────────────┐
┌══════════════════════════════┐        │  claude.ai   │
║         Hrafn (Rust)         ║        │  (manual)    │
║                              ║        └──────┬───────┘
║  • receives 100eyes signal   ║               │
║  • Claude Vision analysis    ║               │ MCP + OAuth 2.1
║  • calls Chartgen MCP tools  ║               │
║  • stores analysis in        ║               │
║    MuninnDB                  ║               │
║  • reports via Telegram Bot  ║               ▼
║                              ║   ┌═══════════════════════════┐
║  MCP Client ─────────────────╬──►║    Chartgen (Rust)         ║
║                              ║   ║                           ║
╚══════════════════════════════╝   ║  MCP Server (HTTP)        ║
                                   ║  ├─ generate_chart         ║
                                   ║  ├─ get_indicators         ║
                                   ║  ├─ place_order            ║
                                   ║  ├─ cancel_order           ║
                                   ║  ├─ get_orders             ║
                                   ║  ├─ get_positions          ║
                                   ║  ├─ get_balance            ║
                                   ║  ├─ set_alert              ║
                                   ║  ├─ list_alerts            ║
                                   ║  ├─ cancel_alert           ║
                                   ║  └─ OAuth 2.1 PKCE         ║
                                   ║                           ║
                                   ║  Trading Engine            ║
                                   ║  ├─ Binance WebSocket      ║
                                   ║  ├─ 33 Indicators          ║
                                   ║  ├─ Alert Engine            ║
                                   ║  ├─ Order State Machine    ║
                                   ║  └─ Position Tracker       ║
                                   ╚═══════════════════════════╝
```

### Two Control Planes

| Path | For | How |
|------|-----|-----|
| Telegram → Hrafn → Chartgen MCP | Automated (100eyes) + mobile | Hrafn analyzes, acts, reports |
| claude.ai → Chartgen MCP | Manual desktop control | Direct chart/trade interaction |

---

## 3. Components

### 3.1 Hrafn (Brain)

**Role:** Signal ingestion, analysis, decision-making.

**New feature:** `channel-telegram-user` using `grammers` crate for Telegram user client access to third-party channels.

**New files:**
```
src/channels/telegram_user.rs    # grammers user client, channel listener
```

**Configuration:**
```toml
[telegram_user]
api_id = 12345
api_hash = "abc..."
phone = "+43..."
session_file = "~/.hrafn/telegram_user.session"

[[telegram_user.watch]]
channel = "100eyes_crypto"
handler = "trading_signal"

[mcp.servers.chartgen]
name = "chartgen"
transport = "http"
url = "https://chartgen.vasudev.xyz"

[templates.trading_signal]
system = """
Du bist ein Trading-Analyst. Du erhältst Signale von 100eyes.
Nutze get_indicators um eigene Analyse zu machen.
Regeln:
- Kein Entry ohne Cipher B grüner Dot
- ADX < 20 = kein Trend, nicht handeln
- Immer eigenen Chart generieren und an User senden
- Entscheidung: place_order, set_alert, oder skip mit Begründung
"""
```

**Flow:**
1. `TelegramUserClient` watches 100eyes channel, extracts image + text
2. Injects `IncomingSignal` into agent loop as new turn
3. Agent calls Claude Vision with image + text
4. Agent calls `get_indicators` via Chartgen MCP for own analysis
5. Agent decides: `place_order`, `set_alert`, or skip
6. Agent calls `generate_chart` for visual confirmation
7. Agent reports result + chart via Telegram Bot to user
8. Analysis stored in MuninnDB

### 3.2 Chartgen (Hands)

**Role:** Trading execution, charting, alert management, MCP server.

**New files:**
```
src/
├── feed.rs                  # Binance WebSocket live feed
├── trading/
│   ├── mod.rs
│   ├── exchange.rs          # Exchange trait + Binance impls
│   ├── order.rs             # Order types + state machine
│   ├── position.rs          # Position tracker
│   └── alert.rs             # AlertEngine trait + impl
└── engine.rs                # Top-level orchestrator
```

**New CLI mode:** `chartgen --trade [--testnet]`

---

## 4. Core Abstractions

### Exchange Trait

```rust
#[async_trait]
pub trait Exchange: Send + Sync {
    async fn submit_order(&self, req: &OrderRequest) -> Result<OrderId>;
    async fn cancel_order(&self, id: &OrderId) -> Result<()>;
    async fn open_orders(&self) -> Result<Vec<Order>>;
    async fn positions(&self) -> Result<Vec<Position>>;
    async fn account_balance(&self) -> Result<Balance>;
}
```

Implementations:
- `BinanceTestnet` (Phase 1: paper trading)
- `BinanceLive` (Phase 2: live crypto)
- Future: `NautilusAdapter` (if IB/DEX execution needed)

### Order Model

```rust
pub struct OrderRequest {
    pub symbol: String,
    pub side: Side,            // Buy | Sell
    pub order_type: OrderType, // Market | Limit { price }
    pub quantity: f64,
}

pub enum OrderState {
    Pending,
    Open,
    Filled { fill_price: f64, filled_at: DateTime<Utc> },
    Cancelled,
    Rejected { reason: String },
}
```

### Alert Engine

```rust
pub enum AlertCondition {
    PriceAbove(f64),
    PriceBelow(f64),
    IndicatorSignal {
        indicator: String,    // "cipher_b", "adx", ...
        signal: String,       // "green_dot", "crossover", ...
    },
}

pub trait AlertEngine: Send + Sync {
    fn add(&mut self, alert: Alert) -> AlertId;
    fn remove(&mut self, id: &AlertId) -> bool;
    fn list(&self) -> &[Alert];
    fn evaluate(
        &self,
        bar: &Bar,
        indicators: &HashMap<String, PanelResult>,
    ) -> Vec<TriggeredAlert>;
}
```

---

## 5. MCP Tools (10 total)

### Chart & Analysis
| Tool | Status | Description |
|------|--------|-------------|
| `generate_chart` | exists | PNG chart with indicators |
| `list_indicators` | exists | 33 indicators with params |
| `get_indicators` | **new** | Raw indicator values as JSON |

### Trading
| Tool | Status | Description |
|------|--------|-------------|
| `place_order` | **new** | Submit buy/sell order |
| `cancel_order` | **new** | Cancel open order |
| `get_orders` | **new** | List orders with state |
| `get_positions` | **new** | Positions with PnL |
| `get_balance` | **new** | Account balance |

### Alerts
| Tool | Status | Description |
|------|--------|-------------|
| `set_alert` | **new** | Create price/indicator alert |
| `list_alerts` | **new** | List active alerts |
| `cancel_alert` | **new** | Remove alert |

---

## 6. Data Flow (`--trade` mode)

```
Binance WebSocket
    │ new bar
    ▼
  feed.rs → Bar
    │
    ▼
  engine.rs
    1. update indicator state
    2. alert_engine.evaluate(bar, indicators)
    3. triggered? → notify callback
    4. open orders? → check fills
    │
    │ Arc<RwLock<Engine>> shared with
    ▼
  MCP Server (axum) ← claude.ai / Hrafn
```

---

## 7. Persistenz

| What | Where | Why |
|------|-------|-----|
| Trade history, analyses | MuninnDB (via Hrafn) | Searchable, linkable memories |
| Open orders, positions | In-memory (Chartgen) | Rebuilt from Binance API on restart |
| Active alerts | `~/.chartgen/alerts.json` | Survives restart |
| Audit trail | `~/.chartgen/trades.log` | Append-only, one line per event |

### Audit Log Format
```
2026-04-10T14:32:01Z FILLED BUY XRPUSDT 100 @ 0.8120 id=abc123
2026-04-10T15:01:44Z ALERT_TRIGGERED cipher_b green_dot XRPUSDT 1h
2026-04-10T15:01:45Z SUBMITTED BUY XRPUSDT 100 MARKET id=def456
```

---

## 8. Error Handling

### Binance WebSocket Disconnect
- Exponential backoff reconnect (1s → 2s → 4s → max 60s)
- REST backfill for missed bars on reconnect
- Telegram alert to user if disconnect > 60s

### Chartgen MCP Unreachable (Hrafn side)
- Hrafn MCP client has built-in retry logic
- Signal queued, not dropped
- After 3 failures: Telegram message "Chartgen offline, signal skipped: [text]"

### Order Rejected
- `OrderState::Rejected { reason }` returned to caller
- Hrafn reports via Telegram
- No automatic retry on rejections

---

## 9. Testing Strategy

| Level | What | How |
|-------|------|-----|
| Unit | Order state machine, alert evaluation | `#[cfg(test)]` with mock bars |
| Integration | MCP tools end-to-end | Extend existing e2e.rs with Engine in testnet mode |
| Paper Trading | Full flow without real money | `chartgen --trade --testnet` with Binance Testnet |
| Hrafn E2E | Signal in → Telegram message out | Mock MCP server with defined responses |

```bash
# Paper trading with Binance Testnet
chartgen --trade --testnet

# Run trading tests
cargo test --features trading
```

---

## 10. Phases

| Phase | Scope | Exchange |
|-------|-------|----------|
| 1 | Paper trading + alerts | Binance Testnet |
| 2 | Live crypto | Binance |
| 3 | Stocks | Interactive Brokers (via NautilusAdapter if needed) |
| 4 | DEXes | dYdX, Hyperliquid (via NautilusAdapter if needed) |

Phases 3-4 may require introducing NautilusTrader as an `Exchange` implementation. The trait abstraction ensures ~75% of code survives that migration.

---

## 11. Estimated Costs

| Item | Cost |
|------|------|
| Claude API (signal analysis, ~10-50 signals/day) | $15-75/month |
| Binance Testnet | Free |
| Hetzner server (existing) | Already running |
| 100eyes subscription | Existing |

---

## 12. Non-Goals

- No custom frontend/dashboard (claude.ai is the UI)
- No own signal generation (100eyes provides signals)
- No HFT/low-latency (seconds are fine, not milliseconds)
- No multi-user support (single trader system)

---

## Decision Log

| # | Decision | Alternatives Considered | Why |
|---|----------|------------------------|-----|
| 1 | Fully automated (Claude API) over semi-automated | Semi-auto (subscription only, user in loop) | User wants hands-off operation. API cost ($15-75/mo) is acceptable. |
| 2 | Pure Rust, no Python | Python for Telegram (Telethon), NautilusTrader (Python/Rust) | Avoid language boundary complexity. All components have Rust alternatives. |
| 3 | No NautilusTrader (for now) | Full NT adoption, NT Rust crates only | NT's value is execution (IB/DEX adapters). For Binance paper/live, custom ~2000 LOC is simpler. Trait abstraction preserves migration path. |
| 4 | Hrafn as Brain, Chartgen as Hands | Single binary, NautilusTrader monolith | Hrafn already has Telegram + Claude API + MCP client + MuninnDB. Chartgen already has indicators + charts + MCP server + OAuth. Clean separation. |
| 5 | grammers for 100eyes channel access | Telethon (Python), manual forwarding, Telegram Premium auto-forward | grammers is Telethon ported to Rust. Cleanest long-term solution. Feature-flagged in Hrafn. |
| 6 | Modular with Redis (initial) → single binary (revised) | Always Redis, always monolith | Redis unnecessary when Hrafn calls Chartgen via MCP (HTTP). Two processes, but no message broker needed. |
| 7 | Chartgen extended (not replaced) | New project, port to NautilusTrader | 33 indicators, chart renderer, MCP server, OAuth 2.1 all production-ready. ~2000 LOC addition vs. rewrite. |
| 8 | In-memory state + JSON alerts + append-only log | SQLite, PostgreSQL, Redis | Minimal persistence needs. Orders/positions rebuilt from exchange on restart. Alerts are the only persistent state. |
| 9 | `Exchange` + `AlertEngine` traits | Direct Binance API calls, NautilusTrader types | Trait boundary enables future migration to NautilusTrader or other venues without rewriting business logic. |
| 10 | `get_indicators` tool (JSON data) over chart-only analysis | Always render charts for analysis | Raw numbers are cheaper, faster, and more precise for automated analysis. Charts reserved for human review. |
