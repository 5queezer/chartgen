//! MCP tool schema — the canonical `tools/list` response payload.
//!
//! Lives in the library (not the binary) so `examples/gen_mcp_types.rs` can
//! drive it without spinning up a real MCP server. The stdio/HTTP MCP
//! handlers in `src/mcp.rs` wrap `tools_list_result()` with the JSON-RPC
//! envelope; this module owns the single source of truth for the schemas.

use serde_json::{json, Value};

/// Return the raw `result` object for a `tools/list` JSON-RPC response.
///
/// Contains every tool chartgen exposes, each with a hand-authored
/// `inputSchema` (JSON Schema). TypeScript input types are generated from
/// these via `scripts/gen-mcp-types/`.
pub fn tools_list_result() -> Value {
    json!({
        "tools": [
            {
                "name": "generate_chart",
                "description": "Generate a financial candlestick or OHLCV chart for a given stock or crypto ticker symbol. Supports multiple timeframes (1m, 5m, 15m, 1h, 4h, 1d, 1wk) and 33 technical indicators including RSI, MACD, EMA, Bollinger Bands, Ichimoku Cloud, Stochastic, ATR, OBV, VWAP, Supertrend, and more. Returns a rendered PNG chart with price action and indicator panels by default. Set format='summary' to instead receive a compact JSON summary (OHLCV stats, last indicator values, signals, divergences, levels) — much cheaper in tokens and ideal for automated reasoning without vision. Set format='series' to receive a full per-bar time-series JSON payload (every OHLCV bar plus 1:1 aligned indicator value arrays, signals with bar timestamps, hlines, y_range) — useful for downstream numeric computation that needs raw arrays rather than last values. Use when the user asks to plot, chart, visualize, render, or analyze price action, candlesticks, or technical signals. Data sourced from Yahoo Finance (stocks like AAPL, MSFT, TSLA) and Binance (crypto like BTCUSDT, ETHUSDT). Call list_indicators first to discover all available indicator names and their configurable parameters.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "ticker": {
                            "type": "string",
                            "description": "Stock or crypto ticker symbol. Stocks: AAPL, MSFT, TSLA, GOOGL, AMZN (Yahoo Finance). Crypto: BTCUSDT, ETHUSDT, SOLUSDT (Binance). If omitted, uses random sample data."
                        },
                        "symbol": {
                            "type": "string",
                            "description": "Alias for ticker. Stock or crypto symbol."
                        },
                        "timeframe": {
                            "type": "string",
                            "description": "Candlestick timeframe / interval: 1m, 5m, 15m, 1h, 4h, 1d, 1wk",
                            "default": "4h"
                        },
                        "interval": {
                            "type": "string",
                            "description": "Alias for timeframe."
                        },
                        "indicators": {
                            "type": "array",
                            "items": {
                                "oneOf": [
                                    { "type": "string" },
                                    {
                                        "type": "object",
                                        "properties": {
                                            "name": { "type": "string" }
                                        },
                                        "required": ["name"],
                                        "additionalProperties": true
                                    }
                                ]
                            },
                            "description": "Technical indicators to render on the chart. Each element is a name string (e.g. 'rsi') or an object with 'name' and optional parameters (e.g. {\"name\": \"rsi\", \"length\": 21}). Overlays draw on the price chart, panels draw below. Call list_indicators for the full list of 33 available indicators.",
                            "default": ["ema_stack", "cipher_b", "macd"]
                        },
                        "panels": {
                            "type": "array",
                            "description": "Alias for indicators."
                        },
                        "bars": {
                            "type": "integer",
                            "description": "Number of OHLCV candlestick bars to display",
                            "default": 200
                        },
                        "width": {
                            "type": "integer",
                            "description": "Chart image width in pixels",
                            "default": 1920
                        },
                        "height": {
                            "type": "integer",
                            "description": "Chart image height in pixels",
                            "default": 1080
                        },
                        "format": {
                            "type": "string",
                            "enum": ["png", "summary", "both", "series"],
                            "description": "Output format. 'png' (default) returns a rendered chart image. 'summary' returns a compact JSON summary (OHLCV stats, last indicator values, signals, divergences, levels) — typically 10-50x fewer tokens than a PNG and ideal for automated analysis without vision. 'both' returns the PNG plus the JSON summary. 'series' returns a full per-bar time-series JSON payload (every OHLCV bar plus 1:1 aligned indicator value arrays with NaN→null for pre-warmup, signals with bar timestamps, hlines, and y_range) — ideal for downstream numeric computation.",
                            "default": "png"
                        }
                    }
                }
            },
            {
                "name": "list_indicators",
                "description": "List all available technical analysis indicators supported by the chart generation tool. Returns 33 indicators organized by category: momentum indicators (RSI, MACD, Stochastic, CCI, Williams %R, ROC), trend indicators (EMA stack, Supertrend, ADX, Ichimoku Cloud, Parabolic SAR), volatility indicators (Bollinger Bands, ATR, Keltner Channels, Donchian Channels, Historical Volatility), volume indicators (OBV, MFI, CMF, VWAP, CVD, A/D Line), and crypto-specific indicators (Funding Rate, Open Interest, Long/Short Ratio, Fear & Greed Index). Each indicator includes a description, configurable parameters with defaults, and whether it renders as an overlay on the price chart or as a separate panel. Call this before generate_chart to discover valid indicator names and their configuration options.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "get_indicators",
                "description": "Compute technical indicators for a symbol and return raw numeric values as JSON. Fetches OHLCV data (or uses cached engine data if running in --trade mode) and computes the requested indicators, returning the last values of each line, dot presence, and horizontal reference levels. This is the key tool for automated technical analysis.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbol": {
                            "type": "string",
                            "description": "Trading symbol (e.g. BTCUSDT, ETHUSDT, AAPL)"
                        },
                        "interval": {
                            "type": "string",
                            "description": "Candle interval (e.g. 1m, 5m, 15m, 1h, 4h, 1d)",
                            "default": "4h"
                        },
                        "indicators": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "List of indicator names to compute (e.g. [\"rsi\", \"macd\", \"adx\"])"
                        }
                    },
                    "required": ["symbol", "indicators"]
                }
            },
            {
                "name": "place_order",
                "description": "Place a trading order. Requires --trade mode. Submits a market or limit order for the configured symbol.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string", "description": "Trading pair (e.g. BTCUSDT)" },
                        "side": { "type": "string", "enum": ["buy", "sell"], "description": "Order side" },
                        "type": { "type": "string", "enum": ["market", "limit"], "description": "Order type" },
                        "quantity": { "type": "number", "description": "Order quantity" },
                        "price": { "type": "number", "description": "Limit price (required for limit orders)" }
                    },
                    "required": ["symbol", "side", "type", "quantity"]
                }
            },
            {
                "name": "cancel_order",
                "description": "Cancel an open order by ID. Requires --trade mode.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "order_id": { "type": "string", "description": "Order ID to cancel" }
                    },
                    "required": ["order_id"]
                }
            },
            {
                "name": "get_orders",
                "description": "List tracked orders. Requires --trade mode.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "filter": { "type": "string", "enum": ["open", "all"], "default": "all", "description": "Filter: 'open' for open orders only, 'all' for all orders" }
                    }
                }
            },
            {
                "name": "get_positions",
                "description": "Get all open positions with unrealized PnL. Requires --trade mode.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "get_balance",
                "description": "Get account balances. Requires --trade mode with exchange credentials.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "set_alert",
                "description": "Set a price or indicator alert. Fires once and is removed. Requires --trade mode.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string", "description": "Trading pair (e.g. BTCUSDT)" },
                        "condition": {
                            "type": "object",
                            "description": "Alert condition. One of: {\"price_above\": 0.85}, {\"price_below\": 0.805}, or {\"indicator_signal\": {\"indicator\": \"cipher_b\", \"signal\": \"green_dot\"}}"
                        }
                    },
                    "required": ["symbol", "condition"]
                }
            },
            {
                "name": "list_alerts",
                "description": "List all active alerts. Requires --trade mode.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "cancel_alert",
                "description": "Cancel an active alert by ID. Requires --trade mode.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "alert_id": { "type": "string", "description": "Alert ID to cancel" }
                    },
                    "required": ["alert_id"]
                }
            },
            {
                "name": "get_notifications",
                "description": "Get triggered alert notifications since last call. Drains the notification queue. Requires --trade mode.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "subscribe_notifications",
                "description": "Subscribe to push notifications for triggered alerts via SSE. Optionally filter by symbols and alert types. Requires --trade mode and SSE connection.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbols": { "type": "array", "items": { "type": "string" }, "description": "Filter by trading pairs (e.g. [\"BTCUSDT\"]). Omit for all." },
                        "alert_types": { "type": "array", "items": { "type": "string" }, "description": "Filter by alert types: price_above, price_below, indicator_signal. Omit for all." }
                    }
                }
            },
            {
                "name": "unsubscribe_notifications",
                "description": "Remove push notification subscription. Requires --trade mode.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "list_subscriptions",
                "description": "Show active notification subscription filters and offline queue size. Requires --trade mode.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            }
        ]
    })
}
