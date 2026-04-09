use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

use crate::fetch;
use chartgen::data;
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

        let response = handle_mcp_request(method, id, req.get("params"));

        writeln!(stdout, "{}", response).ok();
        stdout.flush().ok();
    }
}

/// Handle a single MCP JSON-RPC request. Shared by stdio and HTTP transports.
pub fn handle_mcp_request(method: &str, id: Option<Value>, params: Option<&Value>) -> Value {
    match method {
        "initialize" => handle_initialize(id),
        "tools/list" => handle_tools_list(id),
        "tools/call" => handle_tools_call(id, params),
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
                    "description": "Generate a trading chart PNG with candlesticks and technical indicators. Call list_indicators first to discover available indicators and their parameters. Returns base64-encoded PNG. Data from Yahoo Finance (stocks) or Binance (crypto).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "symbol": {
                                "type": "string",
                                "description": "Trading symbol. Crypto: BTCUSDT, ETHUSDT (Binance). Stocks: AAPL, MSFT, TSLA (Yahoo Finance). If omitted, uses random sample data."
                            },
                            "interval": {
                                "type": "string",
                                "description": "Candle interval: 1m, 5m, 15m, 1h, 4h, 1d, 1wk",
                                "default": "4h"
                            },
                            "panels": {
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
                                "description": "Indicators to render. Each element is a name string (e.g. 'rsi') or an object with 'name' and optional parameters (e.g. {\"name\": \"rsi\", \"length\": 21}). Call list_indicators to see available indicators and their configurable parameters.",
                                "default": ["ema_stack", "cipher_b", "macd"]
                            },
                            "bars": {
                                "type": "integer",
                                "description": "Number of OHLCV bars",
                                "default": 200
                            },
                            "width": {
                                "type": "integer",
                                "description": "Image width in pixels",
                                "default": 1920
                            },
                            "height": {
                                "type": "integer",
                                "description": "Image height in pixels",
                                "default": 1080
                            }
                        }
                    }
                },
                {
                    "name": "list_indicators",
                    "description": "List all available technical indicators with descriptions, parameters, and categories. Use this to discover what indicators are available and how to configure them.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                }
            ]
        }
    })
}

fn handle_tools_call(id: Option<Value>, params: Option<&Value>) -> Value {
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
    let symbol = args.get("symbol").and_then(|v| v.as_str());
    let interval = args
        .get("interval")
        .and_then(|v| v.as_str())
        .unwrap_or("4h");

    // Parse panels — supports both string array and mixed string/object array
    let panels_value = args
        .get("panels")
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
