//! MCP tool schema — the canonical `tools/list` response payload.
//!
//! Lives in the library (not the binary) so `examples/gen_mcp_types.rs` can
//! drive it without spinning up a real MCP server. The stdio/HTTP MCP
//! handlers in `src/mcp.rs` wrap `tools_list_result()` with the JSON-RPC
//! envelope; this module owns the single source of truth for the schemas.

use crate::drawings::{DASHES, FONT_SIZES, MAX_DRAWINGS, MAX_TEXT, TOOLS};
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
                "description": "Generate a financial candlestick or OHLCV chart for a given stock or crypto ticker symbol. Supports any timeframe of a number and a unit (1m, 7m, 45m, 90m, 4h, 8h, 1d, 2D, 2W, 1M, ...) and 33 technical indicators including RSI, MACD, EMA, Bollinger Bands, Ichimoku Cloud, Stochastic, ATR, OBV, VWAP, Supertrend, and more. Returns a rendered PNG chart with price action and indicator panels by default. Set format='summary' to instead receive a compact JSON summary (OHLCV stats, last indicator values, signals, divergences, levels) — much cheaper in tokens and ideal for automated reasoning without vision. Set format='series' to receive a full per-bar time-series JSON payload (every OHLCV bar plus 1:1 aligned indicator value arrays, signals with bar timestamps, hlines, y_range) — useful for downstream numeric computation that needs raw arrays rather than last values. Use when the user asks to plot, chart, visualize, render, or analyze price action, candlesticks, or technical signals. Data sourced from Yahoo Finance (stocks like AAPL, MSFT, TSLA) and Binance (crypto like BTCUSDT, ETHUSDT). Call list_indicators first to discover all available indicator names and their configurable parameters. To annotate the chart (trend lines, zones, fib levels, text, long/short positions), pass `drawings`: call inspect_chart first to anchor points to real bars, and see the draw-on-chart prompt.",
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
                            "description": "Candlestick timeframe / interval: a number and a unit, s seconds, m minutes, h hours, D days, W weeks, M months (1s, 5s, 1m, 5m, 7m, 45m, 90m, 4h, 8h, 1d, 2D, 1W, 2W, 1M). Seconds need a Binance spot symbol: Yahoo has none. Upper bounds after normalization: under 60s, 1440m, 168h, 365D, 52W, 12M. A minute or hour interval may also take at most 60 native Binance bars per bar, so 61m is rejected and 65m is not. The response echoes the canonical form (120m becomes 2h, 2W becomes 2wk, 1M becomes 1mo).",
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
                        "profile_intrabars": {
                            "type": "boolean",
                            "description": "format=series only: compute volume_profile, fixed_range_vp and anchored_vp from lower-timeframe bars, for a chart's full load: up to 6 more Binance requests. The series then says which bars in profile_interval. Leave it off on pages and tails, which keep the load's profile. Default: false, each bar's volume spread over its range.",
                            "default": false
                        },
                        "chart_style": {
                            "type": "string",
                            "enum": ["candles", "heikin_ashi", "renko", "kagi", "point_figure", "line_break", "range"],
                            "description": "Price series style. 'candles' (default) uses the raw OHLC bars. 'heikin_ashi' replaces them with Heikin Ashi bars (haClose = ohlc4, haOpen = (prev haOpen + prev haClose) / 2, seeded with (open+close)/2) in every format. Like TradingView, indicators are then computed on the Heikin Ashi values. The transform runs over bars+warmup before trimming, so pages and tail polls converge after warmup. 'renko', 'kagi', 'point_figure', 'line_break' and 'range' are price-based: built from the bars of `timeframe`, so they only approximate tick data, with times that are not evenly spaced. The last bar is the open one; only closed source bars confirm a bar. Each reads a fixed window of source bars (up to 5000, within the 20,000 native bar limit), `warmup` does nothing for them, and a window may hold fewer bars than `bars`. Indicators run on these bars; ones that project past the last bar return no `future`. format=series echoes the applied style as chart_style. See ADR-0047.",
                            "default": "candles"
                        },
                        "chart_params": chart_params_schema(),
                        "width": {
                            "type": "integer",
                            "description": "Chart image width in pixels. width x height counts towards what one call may hold: 4000 x 4000 fits with little on it, and a call past that is refused (-32602).",
                            "default": 1920
                        },
                        "height": {
                            "type": "integer",
                            "description": "Chart image height in pixels. See width.",
                            "default": 1080
                        },
                        "format": {
                            "type": "string",
                            "enum": ["png", "summary", "both", "series"],
                            "description": "Output format. 'png' (default) returns a rendered chart image. 'summary' returns a compact JSON summary (OHLCV stats, last indicator values, signals, divergences, levels) — typically 10-50x fewer tokens than a PNG and ideal for automated analysis without vision. 'both' returns the PNG plus the JSON summary. 'series' returns a full per-bar time-series JSON payload (every OHLCV bar plus 1:1 aligned indicator value arrays with NaN→null for pre-warmup, signals with bar timestamps, hlines, and y_range, a per-indicator future block with the values plotted past the last bar (Ichimoku cloud), plus has_more=true when older history may exist) — ideal for downstream numeric computation.",
                            "default": "png"
                        },
                        "drawings": drawings_schema()
                    }
                }
            },
            {
                "name": "list_drawing_tools",
                "description": "List the 18 drawing tools generate_chart's `drawings` takes: the web app's own tools (trend line, ray, extended, horizontal and vertical lines, parallel channel, fib retracement and trend-based extension, rectangle, text, price label, pitchfork, price/date ranges, long and short positions). For each: name, label, what each of its points is, which style fields apply and whether it takes text. Read-only and free. The `draw-on-chart` prompt walks through drawing.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "inspect_chart",
                "description": "Read-only facts of a chart, to place drawings on real bars instead of guessing prices: the first and last bar (time, open, high, low, close), the bar count, the highest high and lowest low with their times, the last close, the price range the PNG shows, and the most recent swing highs and lows (zigzag pivots with swing_depth bars on each side), each with its time and price and the `snap` that anchors a drawing point to it. Take the same ticker, timeframe, bars and end_time as the generate_chart call that will draw, so the bars match. Cheap: no indicators, no image.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "ticker": { "type": "string", "description": "Stock or crypto symbol, as generate_chart takes it. Required." },
                        "symbol": { "type": "string", "description": "Alias for ticker." },
                        "timeframe": { "type": "string", "description": "As generate_chart's timeframe.", "default": "4h" },
                        "interval": { "type": "string", "description": "Alias for timeframe." },
                        "bars": { "type": "integer", "description": "Bars to read, newest last, 1 to 1000.", "default": 200 },
                        "end_time": { "type": "integer", "description": "As generate_chart's end_time: only bars that open before it (ms since epoch)." },
                        "swing_depth": { "type": "integer", "description": "Bars on each side a swing high or low must top (or undercut), 1 to 50. Larger finds fewer, bigger swings.", "default": 5 }
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
                "name": "get_footprint",
                "description": "Volume footprint of chart bars: buy and sell volume per price row of each bar, built from lower-timeframe bars (not from trades). `intrabar_interval` says which; null means nothing finer exists and the bar is its own only intrabar. `side` says how volume was split: 'taker_volume' uses Binance's taker buy volume per kline, 'close_vs_open' gives each intrabar to one side (Yahoo has no aggressor data). Row placement is always an estimate: each intrabar's volume is spread over its high-low range. The forming bar repaints as data arrives. Costs up to 7 Binance requests on a cold load, and is cached.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "ticker": { "type": "string", "description": "Symbol, as generate_chart's ticker." },
                        "timeframe": { "type": "string", "description": "Chart interval, as generate_chart's timeframe.", "default": "1h" },
                        "bars": { "type": "integer", "minimum": 1, "maximum": 200, "default": 60, "description": "Newest bars to return." },
                        "end_time": { "type": "integer", "description": "Exclusive upper bound on bar open time, ms since epoch." },
                        "row_size": { "type": "number", "minimum": 0, "description": "Row height in price. Raised to a tick, lowered to the highest price, and widened if a bar would have over 200 rows; row_widened says so. Omit for row_ticks or auto." },
                        "row_ticks": { "type": "integer", "minimum": 0, "description": "Row height in ticks. Omit for auto: 0.2 x ATR on round numbers." },
                        "atr_length": { "type": "integer", "minimum": 1, "maximum": 500, "default": 14, "description": "ATR length of the auto row size." },
                        "value_area": { "type": "number", "minimum": 1, "maximum": 100, "default": 70, "description": "Value area, percent of the volume." },
                        "imbalance": { "type": "number", "minimum": 100, "maximum": 100000, "default": 300, "description": "Imbalance, percent: buy against sell one row below, sell against buy one row above." },
                        "stacked": { "type": "integer", "minimum": 1, "maximum": 100, "default": 3, "description": "Imbalances in a row that make a stacked imbalance. A level counts only where at least 2 intrabars traded, on it and on the level it is compared with: one kline has one split, so its rows would all agree." },
                        "intrabar_interval": { "type": "string", "description": "For a poll of a chart already loaded: the intrabar_interval its load returned, or 'none' if that was null, so the few newest bars are read as the load's were. Falls back to what these bars can afford if they cannot have it. Omit to choose by the window." }
                    },
                    "required": ["ticker"]
                }
            },
            {
                "name": "get_tpo",
                "description": "Time Price Opportunity (market profile) of periods: which blocks traded at each price row, with POC, value area, poor highs and lows, single prints, initial balance and rotation factor. Built from block-length klines. Costs up to 3 Binance requests for a block Binance has (5m, 15m, 30m, 1h, 2h, 4h); any other is built from finer klines and can cost up to 20.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "ticker": { "type": "string", "description": "Symbol, as generate_chart's ticker." },
                        "period": { "type": "string", "default": "1D", "description": "Length of a profile: a count and D, W or M, such as 1D, 1W or 1M." },
                        "block": { "type": "string", "default": "30m", "description": "Length of a block, from 5m to 4h. If 3000 cannot hold a whole period, the next that can: the reply's block is that one, and block_widened says so. A period 4h blocks cannot cover is refused." },
                        "periods": { "type": "integer", "minimum": 1, "maximum": 60, "default": 10, "description": "Newest periods to return, whole ones within 3000 blocks; `periods_cut` says if that left some out." },
                        "end_time": { "type": "integer", "description": "Exclusive upper bound on block open time, ms since epoch." },
                        "row_size": { "type": "number", "minimum": 0, "description": "Row height in price. Omit for row_ticks or auto." },
                        "row_ticks": { "type": "integer", "minimum": 0, "description": "Row height in ticks. Omit for auto: a 24th of the median period range, on round numbers." },
                        "value_area": { "type": "number", "minimum": 1, "maximum": 100, "default": 70, "description": "Value area, percent of the blocks." },
                        "ib_blocks": { "type": "integer", "minimum": 1, "maximum": 100, "default": 2, "description": "Blocks of the initial balance." },
                        "sessions": { "type": "array", "items": { "type": "string" }, "maxItems": 5, "description": "Up to 5 HH:MM-HH:MM in the exchange's time (UTC for crypto): one profile per session of each period." }
                    },
                    "required": ["ticker"]
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
                            "description": "Candlestick timeframe, as for generate_chart: a number and a unit (1s, 1m, 7m, 45m, 4h, 8h, 1d, 2D, 2W, 1M). Binance pushes intervals it has no stream for by resampling a finer one.",
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
                            "enum": ["candles", "heikin_ashi", "renko", "kagi", "point_figure", "line_break", "range"],
                            "description": "Price series style, as for generate_chart. Price-based tails carry up to 64 bars, as one closed source bar can confirm several.",
                            "default": "candles"
                        },
                        "chart_params": chart_params_schema(),
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
pub const SERVER_ALERT_TOOLS: [&str; 6] = [
    "create_server_alert",
    "list_server_alerts",
    "update_server_alert",
    "delete_server_alert",
    "subscribe_push",
    "unsubscribe_push",
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
                    "sign": { "type": "boolean", "description": "Sign deliveries with a per-alert secret (HMAC-SHA256). Needs webhook_url.", "default": false },
                    "push": { "type": "boolean", "description": "Notify your devices (subscribe_push) when it fires. May go with a webhook.", "default": false }
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
                    "sign": { "type": "boolean", "description": "true: sign with a new secret. false: stop signing." },
                    "push": { "type": "boolean", "description": "Notify your devices when it fires, or stop." }
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
        json!({
            "name": "subscribe_push",
            "description": "Register this browser's Web Push subscription, so that server alerts with push: true notify it. Subscribe with the public_key from list_server_alerts. The endpoint must be a known push service (FCM, Mozilla, Apple, Windows). Registering the same endpoint again updates its keys. A device over the limit replaces your oldest. Devices get the alerts of this client only.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "endpoint": { "type": "string", "maxLength": 1024, "description": "PushSubscription.endpoint." },
                    "p256dh": { "type": "string", "maxLength": 88, "description": "The subscription's p256dh key, base64url." },
                    "auth": { "type": "string", "maxLength": 24, "description": "The subscription's auth secret, base64url." }
                },
                "required": ["endpoint", "p256dh", "auth"]
            }
        }),
        json!({
            "name": "unsubscribe_push",
            "description": "Stop notifying this browser. Signing out does not do this, so that alarms keep working. Returns {removed: false} if you have no such device.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "endpoint": { "type": "string", "description": "PushSubscription.endpoint." }
                },
                "required": ["endpoint"]
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

/// `generate_chart`'s `chart_params`: apart, as `json!` runs out of recursion on
/// one literal this deep.
fn chart_params_schema() -> Value {
    json!({
        "type": "object",
        "description": "Inputs of a price-based chart_style, refused for the others. Echoed in format=series and summary with the box_size the build used. Send that box_size back on later polls and pages to keep the same bars. subscribe_chart requires it for an atr or percent box, so pushed bars line up with the chart that was loaded.",
        "properties": {
            "box": {
                "type": "string",
                "enum": ["atr", "fixed", "percent"],
                "description": "How the box is set, for renko, kagi (the reversal amount), point_figure and range (the range size). 'atr' (default): ATR of the closed source bars. 'fixed': box_size. 'percent': a percentage of the last price. ATR and percent are rounded to two significant digits.",
                "default": "atr"
            },
            "atr_length": {
                "type": "integer",
                "description": "ATR length, 1 to 500.",
                "default": 14
            },
            "percent": {
                "type": "number",
                "description": "Percentage of the last price, above 0 and at most 100.",
                "default": 1
            },
            "box_size": {
                "type": "number",
                "description": "The size of a fixed box, or the resolved size of an atr or percent box to keep. A size that would make over 20,000 bars is refused."
            },
            "reversal": {
                "type": "integer",
                "description": "point_figure: boxes the close must come back to start a new column, 1 to 20.",
                "default": 3
            },
            "lines": {
                "type": "integer",
                "description": "line_break: how many lines a close must pass to reverse, 1 to 20.",
                "default": 3
            }
        },
        "additionalProperties": false
    })
}

/// `generate_chart`'s `drawings` (ADR-0054), its tool list from `drawings::TOOLS`.
fn drawings_schema() -> Value {
    let names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
    let tools: Vec<String> = TOOLS
        .iter()
        .map(|t| format!("{} ({})", t.name, t.points.join("; ")))
        .collect();
    json!({
        "type": "array",
        "maxItems": MAX_DRAWINGS,
        "description": format!(
            "Drawings to put on the price chart, as a person draws them in the web app (ADR-0054). Each is {{tool, points, style?, text?}} with exactly the tool's points: {}. A point is {{time, price}} or an anchor {{time, snap}} that takes that bar's open, high, low or close; time is unix seconds, unix ms or an ISO 8601 date or date-time (UTC unless it has an offset) and resolves to the bar containing it, or past the last bar to that bar's grid. All drawings are checked together: if any is invalid the call fails, names each bad drawing by index with a reason and a hint, and draws nothing. On success the answer adds a JSON text item: each drawing with its resolved points (time, timestamp in ms, price, bar), warnings (a point cut off past the right edge or outside the visible price range), and open_url, a link that opens the chart in the web app with these drawings as normal, editable drawings (when the server has CHARTGEN_BASE_URL). Call inspect_chart first to anchor points to real swing highs and lows; list_drawing_tools describes every tool. At most {} drawings.",
            tools.join(", "),
            MAX_DRAWINGS
        ),
        "items": {
            "type": "object",
            "properties": {
                "tool": { "type": "string", "enum": names },
                "points": {
                    "type": "array",
                    "description": "Exactly the tool's points, in its order (1 to 3).",
                    "items": {
                        "type": "object",
                        "properties": {
                            "time": {
                                "oneOf": [{ "type": "number" }, { "type": "string" }],
                                "description": "Unix seconds, unix ms, or ISO 8601 (2026-10-12 or 2026-10-12T14:00:00Z)."
                            },
                            "price": { "type": "number", "description": "The point's price. Give price or snap." },
                            "snap": {
                                "type": "string",
                                "enum": ["open", "high", "low", "close"],
                                "description": "Anchor to this value of the bar at time, instead of a price. Only on loaded bars."
                            }
                        },
                        "required": ["time"],
                        "additionalProperties": false
                    }
                },
                "style": {
                    "type": "object",
                    "description": "The look, as the web app's style bar sets it; absent fields take the tool's own. cgText takes color and fontSize; the others color, width and dash, and the filled ones (cgRect and the three ranges) fill.",
                    "properties": {
                        "color": { "type": "string", "description": "#rgb, #rrggbb, #rrggbbaa or rgba(r, g, b, a)." },
                        "width": { "type": "integer", "minimum": 1, "maximum": 4, "description": "Line width in px." },
                        "dash": { "type": "string", "enum": DASHES },
                        "fill": { "type": "string", "description": "Fill color, as color; may carry alpha." },
                        "fontSize": { "type": "integer", "enum": FONT_SIZES }
                    },
                    "additionalProperties": false
                },
                "text": { "type": "string", "maxLength": MAX_TEXT, "description": "cgText's text (required there); no other tool takes text." }
            },
            "required": ["tool", "points"],
            "additionalProperties": false
        }
    })
}

/// `prompts/list`'s result (ADR-0054).
pub fn prompts_list() -> Value {
    json!({
        "prompts": [{
            "name": "draw-on-chart",
            "description": "How to draw trendlines, zones, fib levels, text and positions on a chart and check the result: inspect_chart, anchors, generate_chart with drawings, warnings, open_url.",
            "arguments": [
                { "name": "ticker", "description": "Symbol to draw on, e.g. BTCUSDT.", "required": false },
                { "name": "timeframe", "description": "Timeframe, e.g. 4h.", "required": false }
            ]
        }]
    })
}

/// `prompts/get`'s result for `name`, or None for a prompt there is not.
pub fn prompt_get(name: &str) -> Option<Value> {
    (name == "draw-on-chart").then(|| {
        json!({
            "description": "Drawing on a chart with chartgen",
            "messages": [{ "role": "user", "content": { "type": "text", "text": DRAW_ON_CHART } }]
        })
    })
}

const DRAW_ON_CHART: &str = r##"You can draw on chartgen charts with the same tools a person uses in the web app. Work like this:

1. Inspect, don't guess. Call inspect_chart with the ticker, timeframe and bars you will chart. It returns the first and last bar, the price range, and recent swing highs and lows with their times and prices. Never compute coordinates from memory or from the image.
2. Anchor points to bars. A point is {"time": T, "price": P} or {"time": T, "snap": "high"} (open, high, low or close). Prefer snap on swing points: the price then is the bar's own value. Times may be unix seconds, unix ms or ISO 8601; each resolves to the bar containing it.
3. Draw. Call generate_chart with the same ticker, timeframe and bars, and drawings: [{tool, points, style?, text?}]. list_drawing_tools says what each tool's points are. All drawings are checked together: if one is invalid, the call fails, lists every bad drawing by index with a hint, and draws nothing. Fix them and call again.
4. Verify. Look at the image, and read the JSON item: the resolved points (time, price, bar) and warnings, e.g. a point cut off past the right edge or a price outside the visible range. Fix what is off and draw again.
5. Share. When the answer has open_url, give it to the user: it opens the chart in the web app with your drawings as normal drawings they can move, restyle, lock or delete.

Example 1, a trendline between two swing lows. inspect_chart returned swings including {"kind": "low", "time": "2026-10-14T08:00:00Z"} and {"kind": "low", "time": "2026-10-20T16:00:00Z"}:
{"tool": "rayLine", "points": [{"time": "2026-10-14T08:00:00Z", "snap": "low"}, {"time": "2026-10-20T16:00:00Z", "snap": "low"}], "style": {"color": "#089981", "width": 2}}
(segment ends at the second point; rayLine runs on to the right edge.)

Example 2, a support zone with cgRect, from the first swing low to the last bar, between two prices:
{"tool": "cgRect", "points": [{"time": "2026-10-14T08:00:00Z", "price": 61850}, {"time": "2026-10-28T20:00:00Z", "price": 62400}], "style": {"fill": "rgba(8, 153, 129, 0.2)", "color": "#089981"}}
Add a label: {"tool": "cgText", "points": [{"time": "2026-10-14T08:00:00Z", "price": 62600}], "text": "Support 61.9k-62.4k"}

Example 3, a fib retracement from a swing high down to a swing low. fibonacciLine's first point is the 100% level and its second the 0% level, so this puts 0% at the low:
{"tool": "fibonacciLine", "points": [{"time": "2026-10-09T12:00:00Z", "snap": "high"}, {"time": "2026-10-14T08:00:00Z", "snap": "low"}]}

Positions: cgLong and cgShort take entry, target and stop; the target and stop share one time, the box's right edge, and must be on the profit and loss side of the entry."##;
