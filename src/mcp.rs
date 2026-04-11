use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as Base64Engine;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::sync::{Arc, RwLock};

use crate::fetch;
use chartgen::data;
use chartgen::engine::Engine;
use chartgen::indicator::Indicator;
use chartgen::indicators;
use chartgen::renderer;

pub fn run() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("Parse error: {}", e) }
                });
                writeln!(stdout, "{}", err).ok();
                stdout.flush().ok();
                continue;
            }
        };

        let id = req.get("id").cloned();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");

        // Notifications have no response
        if method == "initialized" || method == "notifications/initialized" {
            continue;
        }

        let response = handle_mcp_request(method, id, req.get("params"), None, None);

        writeln!(stdout, "{}", response).ok();
        stdout.flush().ok();
    }
}

/// Handle a single MCP JSON-RPC request. Shared by stdio and HTTP transports.
/// When `engine` is `Some`, trading tools are available; otherwise they return
/// an error indicating that the trading engine is not running.
/// `token` is the client's Bearer token (if any), used for subscription identity.
pub fn handle_mcp_request(
    method: &str,
    id: Option<Value>,
    params: Option<&Value>,
    engine: Option<&Arc<RwLock<Engine>>>,
    token: Option<&str>,
) -> Value {
    match method {
        "initialize" => handle_initialize(id),
        "tools/list" => handle_tools_list(id),
        "tools/call" => handle_tools_call(id, params, engine, token),
        "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("Unknown method: {}", method) }
        }),
    }
}

fn handle_initialize(id: Option<Value>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "chartgen",
                "version": "0.1.0"
            }
        }
    })
}

fn handle_tools_list(id: Option<Value>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "tools": [
                {
                    "name": "generate_chart",
                    "description": "Generate a financial candlestick or OHLCV chart image for a given stock or crypto ticker symbol. Supports multiple timeframes (1m, 5m, 15m, 1h, 4h, 1d, 1wk) and 33 technical indicators including RSI, MACD, EMA, Bollinger Bands, Ichimoku Cloud, Stochastic, ATR, OBV, VWAP, Supertrend, and more. Returns a rendered PNG chart with price action and indicator panels. Use when the user asks to plot, chart, visualize, render, or show price action, candlesticks, or technical analysis for any asset. Data sourced from Yahoo Finance (stocks like AAPL, MSFT, TSLA) and Binance (crypto like BTCUSDT, ETHUSDT). Call list_indicators first to discover all available indicator names and their configurable parameters.",
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
        }
    })
}

fn handle_tools_call(
    id: Option<Value>,
    params: Option<&Value>,
    engine: Option<&Arc<RwLock<Engine>>>,
    token: Option<&str>,
) -> Value {
    let tool_name = params
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");

    let arguments = params
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or(json!({}));

    match tool_name {
        "generate_chart" => tool_generate_chart(id, &arguments),
        "list_indicators" => tool_list_indicators(id),
        "get_indicators" => tool_get_indicators(id, &arguments, engine),
        "place_order" => tool_place_order(id, &arguments, engine),
        "cancel_order" => tool_cancel_order(id, &arguments, engine),
        "get_orders" => tool_get_orders(id, &arguments, engine),
        "get_positions" => tool_get_positions(id, engine),
        "get_balance" => tool_get_balance(id, engine),
        "set_alert" => tool_set_alert(id, &arguments, engine),
        "list_alerts" => tool_list_alerts(id, engine),
        "cancel_alert" => tool_cancel_alert(id, &arguments, engine),
        "get_notifications" => tool_get_notifications(id, engine),
        "subscribe_notifications" => tool_subscribe_notifications(id, &arguments, engine, token),
        "unsubscribe_notifications" => tool_unsubscribe_notifications(id, engine, token),
        "list_subscriptions" => tool_list_subscriptions(id, engine, token),
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{ "type": "text", "text": format!("Unknown tool: {}", tool_name) }],
                "isError": true
            }
        }),
    }
}

/// Parse panels array: each element can be a string ("rsi") or an object ({"name": "rsi", "length": 21}).
fn parse_panels(panels_value: &Value) -> (Vec<Box<dyn Indicator>>, Vec<String>) {
    let arr = match panels_value.as_array() {
        Some(a) => a,
        None => return (vec![], vec![]),
    };

    let mut result: Vec<Box<dyn Indicator>> = Vec::new();
    let mut errors = Vec::new();

    for item in arr {
        if let Some(name) = item.as_str() {
            // Simple string: "rsi"
            match indicators::by_name(name) {
                Some(ind) => result.push(ind),
                None => errors.push(name.to_string()),
            }
        } else if let Some(obj) = item.as_object() {
            // Object: {"name": "rsi", "length": 21}
            let name = match obj.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };
            match indicators::by_name_configured(name, &Value::Object(obj.clone())) {
                Some(ind) => result.push(ind),
                None => errors.push(name.to_string()),
            }
        }
    }

    (result, errors)
}

fn tool_generate_chart(id: Option<Value>, args: &Value) -> Value {
    let bars = args.get("bars").and_then(|v| v.as_u64()).unwrap_or(200) as usize;
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;

    // Accept both "symbol" and "ticker" parameter names
    let symbol = args
        .get("ticker")
        .or_else(|| args.get("symbol"))
        .and_then(|v| v.as_str());

    // Accept both "timeframe" and "interval" parameter names
    let interval = args
        .get("timeframe")
        .or_else(|| args.get("interval"))
        .and_then(|v| v.as_str())
        .unwrap_or("4h");

    // Accept both "indicators" and "panels" parameter names
    let panels_value = args
        .get("indicators")
        .or_else(|| args.get("panels"))
        .cloned()
        .unwrap_or_else(|| json!(["ema_stack", "cipher_b", "macd"]));

    let (panel_indicators, unknown) = parse_panels(&panels_value);

    if !unknown.is_empty() {
        return json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{
                    "type": "text",
                    "text": format!(
                        "Unknown indicator(s): {}. Call list_indicators to see available indicators.",
                        unknown.join(", ")
                    )
                }],
                "isError": true
            }
        });
    }

    let mut data = if let Some(sym) = symbol {
        let source = fetch::detect_source(sym);
        match source {
            "binance" => fetch::fetch_binance(sym, interval, bars),
            _ => fetch::fetch_yahoo(sym, interval, bars),
        }
        .unwrap_or_else(|_| data::sample_data(bars))
    } else {
        data::sample_data(bars)
    };
    data.symbol = symbol.map(|s| s.to_string());
    data.interval = Some(interval.to_string());

    // Render to a temp file, then read as base64
    let tmp_path = format!("/tmp/chartgen_{}.png", std::process::id());

    if let Err(e) = renderer::render_chart(&data, &panel_indicators, &tmp_path, width, height) {
        return json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{ "type": "text", "text": format!("Render error: {}", e) }],
                "isError": true
            }
        });
    }

    let png_bytes = match std::fs::read(&tmp_path) {
        Ok(b) => b,
        Err(e) => {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{ "type": "text", "text": format!("Read error: {}", e) }],
                    "isError": true
                }
            });
        }
    };

    std::fs::remove_file(&tmp_path).ok();

    let b64 = BASE64.encode(&png_bytes);

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "image", "data": b64, "mimeType": "image/png" }]
        }
    })
}

fn tool_list_indicators(id: Option<Value>) -> Value {
    let reg = indicators::registry();
    let indicator_list: Vec<Value> = reg
        .iter()
        .map(|info| {
            let mut entry = json!({
                "name": info.name,
                "description": info.description,
                "type": info.category,
                "overlay": info.is_overlay,
            });
            if !info.aliases.is_empty() {
                entry["aliases"] = json!(info.aliases);
            }
            if info.params != json!([]) {
                entry["configurable_params"] = info.params.clone();
            }
            entry
        })
        .collect();

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(&json!({
                    "total": indicator_list.len(),
                    "indicators": indicator_list
                })).unwrap()
            }]
        }
    })
}

// ---------------------------------------------------------------------------
// Helper: return an error when trading engine is required but not available
// ---------------------------------------------------------------------------

fn engine_not_running(id: Option<Value>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": "Trading engine not running. Start with --trade to enable trading tools." }],
            "isError": true
        }
    })
}

fn tool_ok(id: Option<Value>, payload: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&payload).unwrap() }]
        }
    })
}

fn tool_err(id: Option<Value>, msg: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": msg }],
            "isError": true
        }
    })
}

// ---------------------------------------------------------------------------
// Extract meaningful values from a PanelResult for JSON output
// ---------------------------------------------------------------------------

fn panel_result_to_json(pr: &chartgen::indicator::PanelResult) -> Value {
    let mut out = json!({ "label": pr.label, "is_overlay": pr.is_overlay });

    // Lines: last y-value for each
    if !pr.lines.is_empty() {
        let lines: Vec<Value> = pr
            .lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let last_val = line.y.iter().rev().find(|v| !v.is_nan()).copied();
                json!({
                    "index": i,
                    "label": line.label,
                    "last_value": last_val,
                })
            })
            .collect();
        out["lines"] = json!(lines);
    }

    // Dots: last bar dots
    if !pr.dots.is_empty() {
        let dots: Vec<Value> = pr
            .dots
            .iter()
            .filter(|d| !d.y.is_nan())
            .map(|d| json!({ "x": d.x, "y": d.y }))
            .collect();
        out["dots"] = json!(dots);
    }

    // Hlines: horizontal reference levels
    if !pr.hlines.is_empty() {
        let hlines: Vec<Value> = pr.hlines.iter().map(|h| json!({ "y": h.y })).collect();
        out["hlines"] = json!(hlines);
    }

    // Bars (histogram): last value
    if !pr.bars.is_empty() {
        let bars_info: Vec<Value> = pr
            .bars
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let last = b.y.last().copied().unwrap_or(f64::NAN);
                json!({ "index": i, "last_value": if last.is_nan() { Value::Null } else { json!(last) } })
            })
            .collect();
        out["histogram"] = json!(bars_info);
    }

    out
}

// ---------------------------------------------------------------------------
// get_indicators (#32)
// ---------------------------------------------------------------------------

fn tool_get_indicators(
    id: Option<Value>,
    args: &Value,
    engine: Option<&Arc<RwLock<Engine>>>,
) -> Value {
    let symbol = args.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
    let interval = args
        .get("interval")
        .and_then(|v| v.as_str())
        .unwrap_or("4h");
    let indicator_names: Vec<&str> = args
        .get("indicators")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    if symbol.is_empty() {
        return tool_err(id, "symbol is required");
    }
    if indicator_names.is_empty() {
        return tool_err(id, "indicators array is required and must not be empty");
    }

    // Validate indicator names
    let mut unknown = Vec::new();
    for name in &indicator_names {
        if indicators::by_name(name).is_none() {
            unknown.push(*name);
        }
    }
    if !unknown.is_empty() {
        return tool_err(
            id,
            &format!(
                "Unknown indicator(s): {}. Call list_indicators to see available indicators.",
                unknown.join(", ")
            ),
        );
    }

    // Try to get cached results from the engine first
    if let Some(eng) = engine {
        let e = eng.read().unwrap();
        if let Some(results) = e.indicator_state.get_results(symbol, interval) {
            let mut output =
                json!({ "symbol": symbol, "interval": interval, "source": "engine_cache" });
            let mut ind_results = json!({});
            for name in &indicator_names {
                if let Some(pr) = results.get(*name) {
                    ind_results[name] = panel_result_to_json(pr);
                } else {
                    ind_results[name] = json!({ "error": "not computed in engine" });
                }
            }
            output["indicators"] = ind_results;
            return tool_ok(id, output);
        }
    }

    // Fetch data and compute from scratch
    let bars = 200;
    let source = fetch::detect_source(symbol);
    let fetch_result = match source {
        "binance" => fetch::fetch_binance(symbol, interval, bars),
        _ => fetch::fetch_yahoo(symbol, interval, bars),
    };
    let mut ohlcv = match fetch_result {
        Ok(d) => d,
        Err(e) => return tool_err(id, &format!("Failed to fetch data: {e}")),
    };
    ohlcv.symbol = Some(symbol.to_uppercase());
    ohlcv.interval = Some(interval.to_string());

    let mut output = json!({ "symbol": symbol, "interval": interval, "bars": ohlcv.bars.len(), "source": "fetched" });
    let mut ind_results = json!({});
    for name in &indicator_names {
        if let Some(ind) = indicators::by_name(name) {
            let pr = ind.compute(&ohlcv);
            ind_results[name] = panel_result_to_json(&pr);
        }
    }
    output["indicators"] = ind_results;
    tool_ok(id, output)
}

// ---------------------------------------------------------------------------
// place_order (#33)
// ---------------------------------------------------------------------------

fn tool_place_order(
    id: Option<Value>,
    args: &Value,
    engine: Option<&Arc<RwLock<Engine>>>,
) -> Value {
    let engine = match engine {
        Some(e) => e,
        None => return engine_not_running(id),
    };

    let symbol = args
        .get("symbol")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let side_str = args.get("side").and_then(|v| v.as_str()).unwrap_or("");
    let type_str = args
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("market");
    let quantity = args.get("quantity").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let price = args.get("price").and_then(|v| v.as_f64());

    if symbol.is_empty() {
        return tool_err(id, "symbol is required");
    }

    let side = match side_str {
        "buy" => chartgen::trading::order::Side::Buy,
        "sell" => chartgen::trading::order::Side::Sell,
        _ => return tool_err(id, "side must be 'buy' or 'sell'"),
    };

    let order_type = match type_str {
        "market" => chartgen::trading::order::OrderType::Market,
        "limit" => match price {
            Some(p) => chartgen::trading::order::OrderType::Limit { price: p },
            None => return tool_err(id, "price is required for limit orders"),
        },
        _ => return tool_err(id, "type must be 'market' or 'limit'"),
    };

    if quantity <= 0.0 {
        return tool_err(id, "quantity must be > 0");
    }

    let order_id = uuid::Uuid::new_v4().to_string();
    let mut e = engine.write().unwrap();
    let order = e.order_tracker.create(
        order_id.clone(),
        symbol.clone(),
        side.clone(),
        order_type.clone(),
        quantity,
    );
    let state = order.state.to_string();

    // Log the submission
    let side_label = match &side {
        chartgen::trading::order::Side::Buy => "BUY",
        chartgen::trading::order::Side::Sell => "SELL",
    };
    let type_label = match &order_type {
        chartgen::trading::order::OrderType::Market => "MARKET".to_string(),
        chartgen::trading::order::OrderType::Limit { price } => format!("LIMIT@{price}"),
    };
    e.audit_log
        .log_submitted(&symbol, side_label, quantity, &type_label, &order_id);

    tool_ok(id, json!({ "order_id": order_id, "status": state }))
}

// ---------------------------------------------------------------------------
// cancel_order (#33)
// ---------------------------------------------------------------------------

fn tool_cancel_order(
    id: Option<Value>,
    args: &Value,
    engine: Option<&Arc<RwLock<Engine>>>,
) -> Value {
    let engine = match engine {
        Some(e) => e,
        None => return engine_not_running(id),
    };

    let order_id = args.get("order_id").and_then(|v| v.as_str()).unwrap_or("");

    if order_id.is_empty() {
        return tool_err(id, "order_id is required");
    }

    let mut e = engine.write().unwrap();
    match e.order_tracker.mark_cancelled(order_id) {
        Ok(()) => {
            e.audit_log.log_cancelled(order_id);
            tool_ok(id, json!({ "ok": true }))
        }
        Err(msg) => tool_err(id, &msg),
    }
}

// ---------------------------------------------------------------------------
// get_orders (#33)
// ---------------------------------------------------------------------------

fn tool_get_orders(id: Option<Value>, args: &Value, engine: Option<&Arc<RwLock<Engine>>>) -> Value {
    let engine = match engine {
        Some(e) => e,
        None => return engine_not_running(id),
    };

    let filter = args.get("filter").and_then(|v| v.as_str()).unwrap_or("all");

    let e = engine.read().unwrap();
    let orders: Vec<Value> = if filter == "open" {
        e.order_tracker
            .open_orders()
            .iter()
            .map(|o| order_to_json(o))
            .collect()
    } else {
        e.order_tracker.all().iter().map(order_to_json).collect()
    };

    tool_ok(id, json!(orders))
}

fn order_to_json(o: &chartgen::trading::order::Order) -> Value {
    let type_str = match &o.order_type {
        chartgen::trading::order::OrderType::Market => "market".to_string(),
        chartgen::trading::order::OrderType::Limit { price } => format!("limit@{price}"),
    };
    let side_str = match &o.side {
        chartgen::trading::order::Side::Buy => "buy",
        chartgen::trading::order::Side::Sell => "sell",
    };
    json!({
        "id": o.id,
        "symbol": o.symbol,
        "side": side_str,
        "type": type_str,
        "quantity": o.quantity,
        "state": o.state.to_string(),
    })
}

// ---------------------------------------------------------------------------
// get_positions (#34)
// ---------------------------------------------------------------------------

fn tool_get_positions(id: Option<Value>, engine: Option<&Arc<RwLock<Engine>>>) -> Value {
    let engine = match engine {
        Some(e) => e,
        None => return engine_not_running(id),
    };

    let e = engine.read().unwrap();
    let positions: Vec<Value> = e
        .position_tracker
        .positions()
        .iter()
        .map(|p| {
            let side_str = match &p.side {
                chartgen::trading::order::Side::Buy => "buy",
                chartgen::trading::order::Side::Sell => "sell",
            };
            json!({
                "symbol": p.symbol,
                "side": side_str,
                "quantity": p.quantity,
                "entry_price": p.entry_price,
                "current_price": p.current_price,
                "unrealized_pnl": p.unrealized_pnl(),
            })
        })
        .collect();

    tool_ok(id, json!(positions))
}

// ---------------------------------------------------------------------------
// get_balance (#34)
// ---------------------------------------------------------------------------

fn tool_get_balance(id: Option<Value>, engine: Option<&Arc<RwLock<Engine>>>) -> Value {
    let _engine = match engine {
        Some(e) => e,
        None => return engine_not_running(id),
    };

    // Balance requires an exchange connection which is async. Since handle_mcp_request
    // is synchronous and the exchange trait is async, we return the locally-tracked
    // position data as a balance proxy. A full exchange balance query would need
    // the exchange to be stored in Engine and called from an async context.
    tool_err(
        id,
        "get_balance requires an exchange connection. Use get_positions for locally-tracked balances.",
    )
}

// ---------------------------------------------------------------------------
// set_alert (#35)
// ---------------------------------------------------------------------------

fn tool_set_alert(id: Option<Value>, args: &Value, engine: Option<&Arc<RwLock<Engine>>>) -> Value {
    let engine = match engine {
        Some(e) => e,
        None => return engine_not_running(id),
    };

    let symbol = args
        .get("symbol")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if symbol.is_empty() {
        return tool_err(id, "symbol is required");
    }

    let condition_val = match args.get("condition") {
        Some(c) => c,
        None => return tool_err(id, "condition is required"),
    };

    let condition = if let Some(price) = condition_val.get("price_above").and_then(|v| v.as_f64()) {
        chartgen::trading::alert::AlertCondition::PriceAbove(price)
    } else if let Some(price) = condition_val.get("price_below").and_then(|v| v.as_f64()) {
        chartgen::trading::alert::AlertCondition::PriceBelow(price)
    } else if let Some(sig) = condition_val.get("indicator_signal") {
        let indicator = sig
            .get("indicator")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let signal = sig
            .get("signal")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if indicator.is_empty() || signal.is_empty() {
            return tool_err(
                id,
                "indicator_signal requires 'indicator' and 'signal' fields",
            );
        }
        chartgen::trading::alert::AlertCondition::IndicatorSignal { indicator, signal }
    } else {
        return tool_err(
            id,
            "condition must contain one of: price_above, price_below, or indicator_signal",
        );
    };

    let mut e = engine.write().unwrap();
    let alert_id = e.add_alert(symbol, condition);
    tool_ok(id, json!({ "alert_id": alert_id }))
}

// ---------------------------------------------------------------------------
// list_alerts (#35)
// ---------------------------------------------------------------------------

fn tool_list_alerts(id: Option<Value>, engine: Option<&Arc<RwLock<Engine>>>) -> Value {
    let engine = match engine {
        Some(e) => e,
        None => return engine_not_running(id),
    };

    let e = engine.read().unwrap();
    let alerts: Vec<Value> = e
        .alert_engine
        .list()
        .iter()
        .map(|a| {
            let cond = match &a.condition {
                chartgen::trading::alert::AlertCondition::PriceAbove(p) => {
                    json!({ "price_above": p })
                }
                chartgen::trading::alert::AlertCondition::PriceBelow(p) => {
                    json!({ "price_below": p })
                }
                chartgen::trading::alert::AlertCondition::IndicatorSignal { indicator, signal } => {
                    json!({ "indicator_signal": { "indicator": indicator, "signal": signal } })
                }
            };
            json!({
                "id": a.id,
                "symbol": a.symbol,
                "condition": cond,
                "created_at": a.created_at.to_rfc3339(),
            })
        })
        .collect();

    tool_ok(id, json!(alerts))
}

// ---------------------------------------------------------------------------
// cancel_alert (#35)
// ---------------------------------------------------------------------------

fn tool_cancel_alert(
    id: Option<Value>,
    args: &Value,
    engine: Option<&Arc<RwLock<Engine>>>,
) -> Value {
    let engine = match engine {
        Some(e) => e,
        None => return engine_not_running(id),
    };

    let alert_id = args.get("alert_id").and_then(|v| v.as_str()).unwrap_or("");

    if alert_id.is_empty() {
        return tool_err(id, "alert_id is required");
    }

    let mut e = engine.write().unwrap();
    if e.remove_alert(alert_id) {
        tool_ok(id, json!({ "ok": true }))
    } else {
        tool_err(id, &format!("alert not found: {alert_id}"))
    }
}

// ---------------------------------------------------------------------------
// get_notifications (#37)
// ---------------------------------------------------------------------------

fn tool_get_notifications(id: Option<Value>, engine: Option<&Arc<RwLock<Engine>>>) -> Value {
    let engine = match engine {
        Some(e) => e,
        None => return engine_not_running(id),
    };

    let mut e = engine.write().unwrap();
    let notifications = e.drain_notifications();
    let items: Vec<Value> = notifications
        .iter()
        .map(|t| {
            let cond = match &t.alert.condition {
                chartgen::trading::alert::AlertCondition::PriceAbove(p) => {
                    json!({ "price_above": p })
                }
                chartgen::trading::alert::AlertCondition::PriceBelow(p) => {
                    json!({ "price_below": p })
                }
                chartgen::trading::alert::AlertCondition::IndicatorSignal { indicator, signal } => {
                    json!({ "indicator_signal": { "indicator": indicator, "signal": signal } })
                }
            };
            json!({
                "alert_id": t.alert.id,
                "symbol": t.alert.symbol,
                "condition": cond,
                "triggered_at": t.triggered_at.to_rfc3339(),
                "value": t.value,
            })
        })
        .collect();

    tool_ok(id, json!(items))
}

fn tool_subscribe_notifications(
    id: Option<Value>,
    args: &Value,
    engine: Option<&Arc<RwLock<Engine>>>,
    token: Option<&str>,
) -> Value {
    let engine = match engine {
        Some(e) => e,
        None => return engine_not_running(id),
    };
    let token = match token {
        Some(t) => t,
        None => {
            return tool_ok(
                id,
                json!({"error": "No authentication token — subscribe via authenticated HTTP endpoint"}),
            )
        }
    };

    let symbols: Option<HashSet<String>> = args
        .get("symbols")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<HashSet<_>>()
        })
        .and_then(|s| if s.is_empty() { None } else { Some(s) });

    let alert_types: Option<HashSet<String>> = args
        .get("alert_types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<HashSet<_>>()
        })
        .and_then(|s| if s.is_empty() { None } else { Some(s) });

    let mut e = engine.write().unwrap();
    e.subscription_registry
        .subscribe(token, symbols.clone(), alert_types.clone());
    if let Err(err) = e.subscription_registry.save() {
        eprintln!("[MCP] Failed to persist subscription: {err}");
    }

    tool_ok(
        id,
        json!({
            "status": "subscribed",
            "symbols": symbols,
            "alert_types": alert_types,
        }),
    )
}

fn tool_unsubscribe_notifications(
    id: Option<Value>,
    engine: Option<&Arc<RwLock<Engine>>>,
    token: Option<&str>,
) -> Value {
    let engine = match engine {
        Some(e) => e,
        None => return engine_not_running(id),
    };
    let token = match token {
        Some(t) => t,
        None => {
            return tool_ok(
                id,
                json!({"error": "No authentication token — unsubscribe via authenticated HTTP endpoint"}),
            )
        }
    };

    let mut e = engine.write().unwrap();
    let removed = e.subscription_registry.unsubscribe(token);
    if let Err(err) = e.subscription_registry.save() {
        eprintln!("[MCP] Failed to persist subscription: {err}");
    }

    tool_ok(
        id,
        json!({ "status": if removed { "unsubscribed" } else { "not_found" } }),
    )
}

fn tool_list_subscriptions(
    id: Option<Value>,
    engine: Option<&Arc<RwLock<Engine>>>,
    token: Option<&str>,
) -> Value {
    let engine = match engine {
        Some(e) => e,
        None => return engine_not_running(id),
    };
    let token = match token {
        Some(t) => t,
        None => return tool_ok(id, json!({"error": "No authentication token"})),
    };

    let e = engine.read().unwrap();
    match e.subscription_registry.list(token) {
        Some(info) => tool_ok(
            id,
            json!({
                "subscribed": true,
                "symbols": info.symbols,
                "alert_types": info.alert_types,
                "offline_queue_size": info.offline_queue_size,
                "connected": info.connected,
            }),
        ),
        None => tool_ok(id, json!({ "subscribed": false })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chartgen::data::Bar;
    use chartgen::engine::{Engine, EngineConfig};
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("chartgen_mcp_test_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn make_engine(data_dir: PathBuf) -> Arc<RwLock<Engine>> {
        Arc::new(RwLock::new(Engine::new(EngineConfig {
            symbol: "BTCUSDT".to_string(),
            interval: "1h".to_string(),
            indicators: vec!["rsi".to_string()],
            data_dir,
            notification_ttl_secs: 3600,
            max_queue_size: 1000,
        })))
    }

    fn make_bar(close: f64) -> Bar {
        Bar {
            open: close,
            high: close + 1.0,
            low: close - 1.0,
            close,
            volume: 100.0,
            date: "2026-01-01".to_string(),
        }
    }

    /// Helper to extract the text content from a tool response.
    fn response_text(resp: &Value) -> String {
        resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    fn is_error(resp: &Value) -> bool {
        resp["result"]["isError"].as_bool().unwrap_or(false)
    }

    // --- get_indicators tests ---

    #[test]
    fn get_indicators_returns_rsi_structure() {
        // Feed some bars so the engine has cached results
        let dir = temp_dir("get_ind_rsi");
        let engine = make_engine(dir.clone());
        {
            let mut e = engine.write().unwrap();
            for i in 0..20 {
                e.on_bar(make_bar(100.0 + i as f64));
            }
        }

        let args = json!({ "symbol": "BTCUSDT", "interval": "1h", "indicators": ["rsi"] });
        let resp = tool_get_indicators(Some(json!(1)), &args, Some(&engine));
        assert!(!is_error(&resp));

        let text = response_text(&resp);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["source"], "engine_cache");
        assert!(parsed["indicators"]["rsi"]["lines"].is_array());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- set_alert / list_alerts / cancel_alert roundtrip ---

    #[test]
    fn alert_roundtrip() {
        let dir = temp_dir("alert_rt");
        let engine = make_engine(dir.clone());

        // Set alert
        let args = json!({
            "symbol": "BTCUSDT",
            "condition": { "price_above": 50000.0 }
        });
        let resp = tool_set_alert(Some(json!(1)), &args, Some(&engine));
        assert!(!is_error(&resp));
        let text = response_text(&resp);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        let alert_id = parsed["alert_id"].as_str().unwrap().to_string();
        assert!(!alert_id.is_empty());

        // List alerts
        let resp = tool_list_alerts(Some(json!(2)), Some(&engine));
        assert!(!is_error(&resp));
        let text = response_text(&resp);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        let alerts = parsed.as_array().unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0]["id"], alert_id);
        assert_eq!(alerts[0]["symbol"], "BTCUSDT");

        // Cancel alert
        let args = json!({ "alert_id": alert_id });
        let resp = tool_cancel_alert(Some(json!(3)), &args, Some(&engine));
        assert!(!is_error(&resp));
        let text = response_text(&resp);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["ok"], true);

        // List should be empty now
        let resp = tool_list_alerts(Some(json!(4)), Some(&engine));
        let text = response_text(&resp);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- get_notifications drains queue ---

    #[test]
    fn get_notifications_drains_queue() {
        let dir = temp_dir("notif_drain_mcp");
        let engine = make_engine(dir.clone());

        // Set an alert and trigger it
        {
            let mut e = engine.write().unwrap();
            e.add_alert(
                "BTCUSDT".to_string(),
                chartgen::trading::alert::AlertCondition::PriceAbove(100.0),
            );
            e.on_bar(make_bar(105.0));
        }

        // First call should return 1 notification
        let resp = tool_get_notifications(Some(json!(1)), Some(&engine));
        assert!(!is_error(&resp));
        let text = response_text(&resp);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        let items = parsed.as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["symbol"], "BTCUSDT");

        // Second call should return empty
        let resp = tool_get_notifications(Some(json!(2)), Some(&engine));
        let text = response_text(&resp);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Trading tools require engine ---

    #[test]
    fn trading_tools_return_error_without_engine() {
        let resp = tool_set_alert(Some(json!(1)), &json!({}), None);
        assert!(is_error(&resp));
        assert!(response_text(&resp).contains("not running"));

        let resp = tool_list_alerts(Some(json!(2)), None);
        assert!(is_error(&resp));

        let resp = tool_get_positions(Some(json!(3)), None);
        assert!(is_error(&resp));

        let resp = tool_get_notifications(Some(json!(4)), None);
        assert!(is_error(&resp));

        let resp = tool_place_order(Some(json!(5)), &json!({}), None);
        assert!(is_error(&resp));

        let resp = tool_cancel_order(Some(json!(6)), &json!({}), None);
        assert!(is_error(&resp));

        let resp = tool_get_orders(Some(json!(7)), &json!({}), None);
        assert!(is_error(&resp));
    }

    // --- place_order / get_orders ---

    #[test]
    fn place_and_get_orders() {
        let dir = temp_dir("place_orders");
        let engine = make_engine(dir.clone());

        let args = json!({
            "symbol": "BTCUSDT",
            "side": "buy",
            "type": "market",
            "quantity": 0.5
        });
        let resp = tool_place_order(Some(json!(1)), &args, Some(&engine));
        assert!(!is_error(&resp));
        let text = response_text(&resp);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["status"], "Pending");
        assert!(parsed["order_id"].is_string());

        // Get all orders
        let resp = tool_get_orders(Some(json!(2)), &json!({}), Some(&engine));
        assert!(!is_error(&resp));
        let text = response_text(&resp);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        let orders = parsed.as_array().unwrap();
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0]["side"], "buy");
        assert_eq!(orders[0]["quantity"], 0.5);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- handle_mcp_request with engine ---

    #[test]
    fn handle_mcp_tools_list_includes_trading_tools() {
        let resp = handle_mcp_request("tools/list", Some(json!(1)), None, None, None);
        let tools = resp["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"get_indicators"));
        assert!(names.contains(&"place_order"));
        assert!(names.contains(&"cancel_order"));
        assert!(names.contains(&"get_orders"));
        assert!(names.contains(&"get_positions"));
        assert!(names.contains(&"get_balance"));
        assert!(names.contains(&"set_alert"));
        assert!(names.contains(&"list_alerts"));
        assert!(names.contains(&"cancel_alert"));
        assert!(names.contains(&"get_notifications"));
        assert!(names.contains(&"subscribe_notifications"));
        assert!(names.contains(&"unsubscribe_notifications"));
        assert!(names.contains(&"list_subscriptions"));
    }

    #[test]
    fn subscribe_notifications_with_filters() {
        let dir = temp_dir("sub_filter");
        let engine = make_engine(dir.clone());

        let args = json!({
            "symbols": ["BTCUSDT"],
            "alert_types": ["price_above", "price_below"]
        });
        let resp = tool_subscribe_notifications(Some(json!(1)), &args, Some(&engine), Some("tok1"));
        assert!(!is_error(&resp));
        let text = response_text(&resp);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["status"], "subscribed");
        assert!(parsed["symbols"]
            .as_array()
            .unwrap()
            .contains(&json!("BTCUSDT")));
        assert_eq!(parsed["alert_types"].as_array().unwrap().len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subscribe_notifications_no_token() {
        let dir = temp_dir("sub_no_tok");
        let engine = make_engine(dir.clone());

        let args = json!({});
        let resp = tool_subscribe_notifications(Some(json!(1)), &args, Some(&engine), None);
        assert!(!is_error(&resp));
        let text = response_text(&resp);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert!(parsed["error"]
            .as_str()
            .unwrap()
            .contains("No authentication token"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsubscribe_notifications_removes() {
        let dir = temp_dir("unsub_remove");
        let engine = make_engine(dir.clone());

        let args = json!({});
        tool_subscribe_notifications(Some(json!(1)), &args, Some(&engine), Some("tok1"));

        let resp = tool_unsubscribe_notifications(Some(json!(2)), Some(&engine), Some("tok1"));
        assert!(!is_error(&resp));
        let text = response_text(&resp);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["status"], "unsubscribed");

        let resp = tool_unsubscribe_notifications(Some(json!(3)), Some(&engine), Some("tok1"));
        let text = response_text(&resp);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["status"], "not_found");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_subscriptions_shows_state() {
        let dir = temp_dir("list_sub_state");
        let engine = make_engine(dir.clone());

        let resp = tool_list_subscriptions(Some(json!(1)), Some(&engine), Some("tok1"));
        assert!(!is_error(&resp));
        let text = response_text(&resp);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["subscribed"], false);

        let args = json!({ "symbols": ["ETHUSDT"] });
        tool_subscribe_notifications(Some(json!(2)), &args, Some(&engine), Some("tok1"));

        let resp = tool_list_subscriptions(Some(json!(3)), Some(&engine), Some("tok1"));
        let text = response_text(&resp);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["subscribed"], true);
        assert!(parsed["symbols"]
            .as_array()
            .unwrap()
            .contains(&json!("ETHUSDT")));
        assert_eq!(parsed["offline_queue_size"], 0);
        assert_eq!(parsed["connected"], false);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subscription_tools_return_error_without_engine() {
        let resp = tool_subscribe_notifications(Some(json!(1)), &json!({}), None, Some("tok1"));
        assert!(is_error(&resp));
        assert!(response_text(&resp).contains("not running"));

        let resp = tool_unsubscribe_notifications(Some(json!(2)), None, Some("tok1"));
        assert!(is_error(&resp));

        let resp = tool_list_subscriptions(Some(json!(3)), None, Some("tok1"));
        assert!(is_error(&resp));
    }
}
