use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

use crate::data;
use crate::fetch;
use crate::indicators;
use crate::renderer;

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
                    "description": "Generate a trading chart PNG with candlesticks and technical indicator panels. Supports 33 indicators. Returns base64-encoded PNG image. Data from Yahoo Finance (stocks) or Binance (crypto).",
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
                                "items": { "type": "string" },
                                "description": "Indicators to render. Overlays draw on price chart, panels below. Available: ema_stack, bbands, keltner, donchian, vwap, vwap_bands, supertrend, sar, ichimoku, heikin_ashi, pivot, volume_profile (overlays); cipher_b, macd, rsi, wavetrend, stoch, atr, obv, cci, roc, mfi, williams_r, cmf, adx, ad, histvol, cvd, funding, oi, long_short, fear_greed, kalman_volume (panels)",
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
                    "description": "List all available technical indicator names with descriptions.",
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

fn tool_generate_chart(id: Option<Value>, args: &Value) -> Value {
    let panel_names: Vec<String> = args
        .get("panels")
        .and_then(|p| serde_json::from_value(p.clone()).ok())
        .unwrap_or_else(|| vec!["ema_stack".into(), "cipher_b".into(), "macd".into()]);

    let bars = args.get("bars").and_then(|v| v.as_u64()).unwrap_or(200) as usize;
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;
    let symbol = args.get("symbol").and_then(|v| v.as_str());
    let interval = args
        .get("interval")
        .and_then(|v| v.as_str())
        .unwrap_or("4h");

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

    let mut unknown = Vec::new();
    let panel_indicators: Vec<_> = panel_names
        .iter()
        .filter_map(|name| {
            let ind = indicators::by_name(name);
            if ind.is_none() {
                unknown.push(name.clone());
            }
            ind
        })
        .collect();

    if !unknown.is_empty() {
        return json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{
                    "type": "text",
                    "text": format!("Unknown indicator(s): {}. Available: {:?}", unknown.join(", "), indicators::available())
                }],
                "isError": true
            }
        });
    }

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
    let info = json!({
        "overlays": [
            "ema_stack", "bbands", "keltner", "donchian", "vwap", "vwap_bands",
            "supertrend", "sar", "ichimoku", "heikin_ashi", "pivot", "volume_profile"
        ],
        "panels": [
            "cipher_b", "macd", "rsi", "wavetrend", "stoch", "atr", "obv", "cci",
            "roc", "mfi", "williams_r", "cmf", "adx", "ad", "histvol", "cvd",
            "kalman_volume"
        ],
        "external_api": [
            "funding", "oi", "long_short", "fear_greed"
        ]
    });
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(&info).unwrap()
            }]
        }
    })
}
