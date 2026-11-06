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
                "description": "Generate a financial candlestick or OHLCV chart for a given stock or crypto ticker symbol. Supports any timeframe of a number and a unit (1m, 7m, 45m, 90m, 4h, 8h, 1d, 2D, 2W, 1M, ...) and 33 technical indicators including RSI, MACD, EMA, Bollinger Bands, Ichimoku Cloud, Stochastic, ATR, OBV, VWAP, Supertrend, and more. Returns a rendered PNG chart with price action and indicator panels by default. Set format='summary' to instead receive a compact JSON summary (OHLCV stats, last indicator values, signals, divergences, levels) — much cheaper in tokens and ideal for automated reasoning without vision. Set format='series' to receive a full per-bar time-series JSON payload (every OHLCV bar plus 1:1 aligned indicator value arrays, signals with bar timestamps, hlines, y_range) — useful for downstream numeric computation that needs raw arrays rather than last values. Use when the user asks to plot, chart, visualize, render, or analyze price action, candlesticks, or technical signals. Data sourced from Yahoo Finance (stocks like AAPL, MSFT, TSLA) and Binance (crypto like BTCUSDT, ETHUSDT). Call list_indicators first to discover all available indicator names and their configurable parameters.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "ticker": {
                            "type": "string",
                            "description": "Stock or crypto ticker symbol. Stocks: AAPL, MSFT, TSLA, GOOGL, AMZN (Yahoo Finance). Crypto: BTCUSDT, ETHUSDT, SOLUSDT (Binance). If omitted, uses random sample data. If given and the upstream fetch fails, the call returns an error."
                        },
                        "symbol": {
                            "type": "string",
                            "description": "Alias for ticker. Stock or crypto symbol."
                        },
                        "timeframe": {
                            "type": "string",
                            "description": "Candlestick timeframe / interval: a number and a unit, m minutes, h hours, D days, W weeks, M months (1m, 5m, 7m, 45m, 90m, 4h, 8h, 1d, 2D, 1W, 2W, 1M). Upper bounds after normalization: under 1440m, 168h, 365D, 52W, 12M. A minute or hour interval may also take at most 60 native Binance bars per bar, so 61m is rejected and 65m is not. The response echoes the canonical form (120m becomes 2h, 2W becomes 2wk, 1M becomes 1mo).",
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
                                            "name": { "type": "string" },
                                            "id": { "type": "string", "minLength": 1, "maxLength": 64, "description": "Instance key (ADR-0028): the response keys this entry by id instead of name, so one indicator can be requested several times with different parameters." }
                                        },
                                        "required": ["name"],
                                        "additionalProperties": true
                                    }
                                ]
                            },
                            "description": "Technical indicators to render on the chart. Each element is a name string (e.g. 'rsi') or an object with 'name' and optional parameters (e.g. {\"name\": \"rsi\", \"length\": 21}). Results are keyed by name; give an object an 'id' to key it by that instead. The same indicator may appear several times (e.g. RSI 14 and RSI 21): a repeat without an id is keyed name#2, name#3, and so on; an id must not equal the name of an entry without one. At most 50 entries. Overlays draw on the price chart, panels draw below. Call list_indicators for the full list of 33 available indicators.",
                            "default": ["ema_stack", "cipher_b", "macd"]
                        },
                        "panels": {
                            "type": "array",
                            "items": {
                                "oneOf": [
                                    { "type": "string" },
                                    {
                                        "type": "object",
                                        "properties": {
                                            "name": { "type": "string" },
                                            "id": { "type": "string", "minLength": 1, "maxLength": 64, "description": "Instance key (ADR-0028): the response keys this entry by id instead of name, so one indicator can be requested several times with different parameters." }
                                        },
                                        "required": ["name"],
                                        "additionalProperties": true
                                    }
                                ]
                            },
                            "description": "Alias for indicators."
                        },
                        "bars": {
                            "type": "integer",
                            "description": "Number of OHLCV candlestick bars to display (max 5000, all formats)",
                            "default": 200
                        },
                        "end_time": {
                            "type": "integer",
                            "description": "History paging: return only bars whose open time (ms since epoch) is strictly before end_time — pass the oldest loaded bar's t × 1000 to fetch the page before it. Omit (or null) for the newest bars, including the still-forming one. Must be an integer between -2208988800000 (1900-01-01) and one year from now."
                        },
                        "warmup": {
                            "type": "integer",
                            "description": "format=series only: extra older bars (max 1000) fetched and fed to indicator computation, then trimmed, so the first returned bars carry warmed-up indicator values. A non-zero warmup is raised to what the indicators need to converge (891 for ema_stack's EMA 200). Ignored by png/summary/both.",
                            "default": 0
                        },
                        "chart_style": {
                            "type": "string",
                            "enum": ["candles", "heikin_ashi"],
                            "description": "Price series style. 'candles' (default) uses the raw OHLC bars. 'heikin_ashi' replaces them with Heikin Ashi bars (haClose = ohlc4, haOpen = (prev haOpen + prev haClose) / 2, seeded with (open+close)/2) in every format. Like TradingView, indicators are then computed on the Heikin Ashi values. The transform runs over bars+warmup before trimming, so pages and tail polls converge after warmup. format=series echoes the applied style as chart_style.",
                            "default": "candles"
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
                            "description": "Output format. 'png' (default) returns a rendered chart image. 'summary' returns a compact JSON summary (OHLCV stats, last indicator values, signals, divergences, levels) — typically 10-50x fewer tokens than a PNG and ideal for automated analysis without vision. 'both' returns the PNG plus the JSON summary. 'series' returns a full per-bar time-series JSON payload (every OHLCV bar plus 1:1 aligned indicator value arrays with NaN→null for pre-warmup, signals with bar timestamps, hlines, and y_range, a per-indicator future block with the values plotted past the last bar (Ichimoku cloud), plus has_more=true when older history may exist) — ideal for downstream numeric computation.",
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
                            "description": "Candle interval, as generate_chart's timeframe (e.g. 1m, 7m, 45m, 1h, 4h, 8h, 1d, 2D, 2W)",
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
                "name": "get_quotes",
                "description": "Get the last price and change for many symbols in one call (watchlist). Returns one entry per distinct symbol, in request order: {symbol, last, change, change_pct, prev_close, high, low, volume, source}, or {symbol, error} when that symbol fails; one bad symbol never fails the call. Stocks/ETFs/indices/FX (Yahoo): change vs the previous session's close, like TradingView. Crypto (Binance, symbols ending in USDT, BTC, ETH, ...): change over the rolling 24 h window, as Binance's own ticker reports it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbols": {
                            "type": "array",
                            "items": { "type": "string" },
                            "maxItems": 50,
                            "description": "Ticker symbols (e.g. [\"BTCUSDT\", \"AAPL\", \"^GSPC\"]). At most 50 entries; trimmed, uppercased and deduplicated."
                        }
                    },
                    "required": ["symbols"]
                }
            },
            {
                "name": "search_symbols",
                "description": "Search symbols by ticker or name across all Binance spot pairs and Yahoo Finance instruments (stocks, ETFs, funds, indices, FX, futures), like TradingView's symbol search. Returns {results: [{symbol, name, exchange, type, source}], warnings?}, ranked: exact symbol match first, then prefix, then contains, then name match. type is one of crypto, stock, etf, fund, index, forex, future; source is binance, yahoo or sample. Crypto comes from Binance only (e.g. BTCUSDT); Yahoo's own crypto listings are dropped. If one source fails its results are omitted and a warning is added; the call fails only when every queried source fails.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "minLength": 1,
                            "maxLength": 40,
                            "description": "Text to search for in tickers and names. Trimmed; 1-40 characters, no control characters."
                        },
                        "limit": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 50,
                            "default": 30,
                            "description": "Maximum number of results"
                        },
                        "types": {
                            "type": "array",
                            "items": { "type": "string", "enum": ["crypto", "stock", "etf", "fund", "index", "forex", "future"] },
                            "description": "Only return these instrument types. Omit for all types."
                        }
                    },
                    "required": ["query"]
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
            },
            {
                "name": "subscribe_chart",
                "description": "Push live updates of a chart over the caller's SSE stream (GET /mcp with the same token and Mcp-Session-Id). Each update is a notifications/chart_tick notification with params {subscription_id, series}, where series is exactly the generate_chart(format=series, bars=2, warmup) tail for these arguments: the last closed bar and the forming bar with indicators recomputed. Pushes follow the market (sub-second for active Binance symbols), at most 4 per second (2 when warmup > 300, or when an interval with no stream of its own holds more than 302 native bars). Only Binance symbols are pushable; any other symbol returns an error starting with 'not pushable', and the client should poll generate_chart instead. Limits (4 subscriptions per token, 32 per client address, 64 per server, 10 subscribe_chart calls per token per minute, 10 indicators) fail with an error starting with 'push limit reached': poll that chart instead. An error starting with 'feeds busy' passes within a minute: poll meanwhile and try again. A subscription ends with unsubscribe_chart or when its SSE stream closes. HTTP transport only; works without --trade.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "ticker": {
                            "type": "string",
                            "description": "Binance symbol, 2-20 letters or digits, e.g. BTCUSDT."
                        },
                        "timeframe": {
                            "type": "string",
                            "description": "Candlestick timeframe, as for generate_chart: a number and a unit (1m, 7m, 45m, 4h, 8h, 1d, 2D, 2W, 1M). Binance pushes intervals it has no stream for by resampling a finer one.",
                            "default": "4h"
                        },
                        "indicators": {
                            "type": "array",
                            "items": {
                                "oneOf": [
                                    { "type": "string" },
                                    {
                                        "type": "object",
                                        "properties": {
                                            "name": { "type": "string" },
                                            "id": { "type": "string", "minLength": 1, "maxLength": 64, "description": "Instance key (ADR-0028): the response keys this entry by id instead of name, so one indicator can be requested several times with different parameters." }
                                        },
                                        "required": ["name"],
                                        "additionalProperties": true
                                    }
                                ]
                            },
                            "description": "Indicators, as for generate_chart; at most 10.",
                            "default": ["ema_stack", "cipher_b", "macd"]
                        },
                        "chart_style": {
                            "type": "string",
                            "enum": ["candles", "heikin_ashi"],
                            "description": "Price series style, as for generate_chart.",
                            "default": "candles"
                        },
                        "warmup": {
                            "type": "integer",
                            "description": "Extra older bars (max 1000) the server keeps and feeds to indicator computation, as for generate_chart(format=series), and raised the same way.",
                            "default": 0
                        }
                    },
                    "required": ["ticker"]
                }
            },
            {
                "name": "unsubscribe_chart",
                "description": "End a subscribe_chart subscription. Returns {unsubscribed: false} if it was already gone (e.g. its SSE stream closed).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "subscription_id": {
                            "type": "string",
                            "description": "The subscription_id returned by subscribe_chart."
                        }
                    },
                    "required": ["subscription_id"]
                }
            }
        ]
    })
}

/// Names of the server alert tools (ADR-0042). They answer on the HTTP
/// transport, where a call has an OAuth client to own the alert.
pub const SERVER_ALERT_TOOLS: [&str; 4] = [
    "create_server_alert",
    "list_server_alerts",
    "update_server_alert",
    "delete_server_alert",
];

/// Tools for alerts that run on the server (ADR-0042). `tools/list` appends them
/// only with `CHARTGEN_SERVER_ALERTS=1`; the generated TypeScript types always
/// include them, because the web app handles both cases.
pub fn server_alert_tools() -> Vec<Value> {
    let condition = json!({
        "type": "string",
        "enum": ["crossing", "crossing_up", "crossing_down", "greater", "less"],
        "description": "crossing: the price goes from one side of the level to the other, or onto it. crossing_up / crossing_down: only that way. greater / less: the price is above / below the level. A crossing needs a previous reading, so the first price the alert sees never fires one."
    });
    let trigger = json!({
        "type": "string",
        "enum": ["once", "every"],
        "description": "once: switch off after firing. every: stay active; greater and less wait until the price is back on the other side."
    });
    vec![
        json!({
            "name": "create_server_alert",
            "description": "Create a price alert that runs on the server, with no browser open, and optionally calls a webhook when it fires. It belongs to your OAuth client and survives restarts. Binance symbols only. Drawing and indicator alerts cannot run here. The server's operator decides which clients may create alerts and use webhooks (see can_create and webhooks in list_server_alerts). The webhook must be https on port 443 at a public address. With sign: true the response holds webhook_secret once; it cannot be read again. Returns the alert as list_server_alerts does.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Binance symbol, e.g. BTCUSDT." },
                    "interval": { "type": "string", "description": "Bar interval, e.g. 1m, 15m, 1h, 1d. With evaluate_on: close the alert reads the closes of these bars, which need a Binance kline stream; with live it only labels the alert.", "default": "1m" },
                    "evaluate_on": { "type": "string", "enum": ["live", "close"], "description": "live: every price update, like a browser alert. close: only when a bar of the interval closes.", "default": "live" },
                    "condition": condition.clone(),
                    "level": { "type": "number", "description": "The price level; greater than 0." },
                    "trigger": trigger.clone(),
                    "message": { "type": "string", "maxLength": 200, "description": "Your note, sent in the webhook payload." },
                    "webhook_url": { "type": "string", "maxLength": 512, "description": "https URL to POST to when the alert fires." },
                    "sign": { "type": "boolean", "description": "Sign deliveries with a per-alert secret (HMAC-SHA256). Needs webhook_url.", "default": false }
                },
                "required": ["symbol", "condition", "level", "trigger"]
            }
        }),
        json!({
            "name": "list_server_alerts",
            "description": "List your server alerts with what your client may do (can_create, webhooks) and each alert's state: active, disarmed, last_triggered_at, fire_count, feed (live or waiting), last_price and the last webhook delivery (status and error only). A webhook shows its host and whether it signs, never its URL or secret. On a server without server alerts this returns {enabled: false, alerts: []}.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "update_server_alert",
            "description": "Change one of your server alerts. A new condition, level or trigger switches it on and re-arms it; a new message or webhook does not, so a spent alert stays spent. active: false pauses it and active: true resumes it, re-armed. webhook_url replaces the webhook, an empty string removes it; a new URL does not keep the old signing secret. With sign: true the response holds a new webhook_secret once. Symbol, interval and evaluate_on cannot change: delete and create instead.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "alert_id": { "type": "string", "description": "The alert's id." },
                    "active": { "type": "boolean", "description": "Pause (false) or resume (true)." },
                    "condition": condition,
                    "level": { "type": "number", "description": "The price level; greater than 0." },
                    "trigger": trigger,
                    "message": { "type": "string", "maxLength": 200, "description": "Your note; an empty string removes it." },
                    "webhook_url": { "type": "string", "maxLength": 512, "description": "New https URL, or an empty string to remove the webhook." },
                    "sign": { "type": "boolean", "description": "true: sign with a new secret. false: stop signing." }
                },
                "required": ["alert_id"]
            }
        }),
        json!({
            "name": "delete_server_alert",
            "description": "Delete one of your server alerts, with any delivery still pending. Returns {deleted: false} if you have no alert with that id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "alert_id": { "type": "string", "description": "The alert's id." }
                },
                "required": ["alert_id"]
            }
        }),
    ]
}

/// Tools for user-authored Starlark indicators (ADR-0025). `tools/list`
/// appends them only when `CHARTGEN_SCRIPTING=1`; `tools_list_result()`
/// (and so the generated TypeScript input types) never includes them.
pub fn scripting_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "register_indicator",
            "description": "Register (or replace) one of your own indicators, written in Starlark (a Python dialect; Pine Script-like). The source defines PARAMS = [{\"name\", \"type\": int|float|bool|string, \"default\", \"min\"?, \"max\"?, \"desc\"?}], META = {\"title\", \"overlay\": bool, \"description\"?} and def compute(ohlcv, params, chart). ohlcv.open/high/low/close/volume/time/hl2/hlc3/ohlc4 are series; series support + - * / and comparisons (giving bool series), & | ~, and close[1] for the previous bar. ta.* has sma, ema, wma, rma, rsi, stoch, atr, macd, highest, lowest, stdev, change, roc, crossover, crossunder, pivot_high, pivot_low and element-wise math. Draw with chart.line/fill/bars/dot/hline/hbar/divline/set_y_range/set_label. The script is parsed, statically checked and dry-run on 100 synthetic bars; a failure returns {ok: false, errors: [{step, line, message}]}. Once registered, use the name in generate_chart like a built-in; it shadows a built-in of the same name for you only. Call fork_indicator to start from a built-in's source.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "pattern": "^[a-z][a-z0-9_]{0,47}$", "description": "Indicator name: lowercase letters, digits and _, starting with a letter." },
                    "source": { "type": "string", "maxLength": 65536, "description": "Starlark source defining PARAMS, META and compute(ohlcv, params, chart)." },
                    "create_only": { "type": "boolean", "default": false, "description": "Refuse instead of replacing when you already have an indicator with this name." }
                },
                "required": ["name", "source"]
            }
        }),
        json!({
            "name": "fork_indicator",
            "description": "Get the Starlark source of a built-in indicator, to edit and register under your account with register_indicator (keep the name to replace the built-in for you, or pick a new one). Only built-ins with a reference implementation can be forked; the error lists them.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "builtin_name": { "type": "string", "description": "Built-in indicator name, e.g. wavetrend." }
                },
                "required": ["builtin_name"]
            }
        }),
        json!({
            "name": "list_my_indicators",
            "description": "List the indicators you registered: name, title, description, overlay, configurable_params, created_at and updated_at (unix seconds). list_indicators also includes them, tagged source: \"user\".",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "get_my_indicator",
            "description": "Get the stored Starlark source of one of your registered indicators, to edit and register again.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of your indicator." }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "delete_indicator",
            "description": "Delete one of your registered indicators. A built-in it shadowed becomes visible again.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of your indicator." }
                },
                "required": ["name"]
            }
        }),
    ]
}
