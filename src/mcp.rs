use std::io::{self, BufRead, Write};
use serde_json::{json, Value};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::{data, indicators, renderer};

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

        let response = match method {
            "initialize" => handle_initialize(id),
            "initialized" => continue, // notification, no response
            "notifications/initialized" => continue,
            "tools/list" => handle_tools_list(id),
            "tools/call" => handle_tools_call(id, req.get("params")),
            "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Unknown method: {}", method) }
            }),
        };

        writeln!(stdout, "{}", response).ok();
        stdout.flush().ok();
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
                    "description": "Generate a trading chart PNG with candlesticks and indicator panels. Returns base64-encoded PNG. Available indicators: macd, wavetrend, rsi, cipher_b.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "panels": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Indicator panels to render (bottom to top). Available: macd, wavetrend, rsi, cipher_b",
                                "default": ["cipher_b", "macd"]
                            },
                            "bars": {
                                "type": "integer",
                                "description": "Number of OHLCV bars to generate",
                                "default": 120
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
                            },
                            "output": {
                                "type": "string",
                                "description": "Optional file path to save the PNG to disk"
                            }
                        }
                    }
                },
                {
                    "name": "list_indicators",
                    "description": "List all available indicator names.",
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
        .unwrap_or_else(|| vec!["cipher_b".into(), "macd".into()]);

    let bars = args.get("bars").and_then(|v| v.as_u64()).unwrap_or(120) as usize;
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;
    let output_path = args.get("output").and_then(|v| v.as_str()).map(|s| s.to_string());

    let data = data::sample_data(bars);

    let mut unknown = Vec::new();
    let panel_indicators: Vec<_> = panel_names.iter().filter_map(|name| {
        let ind = indicators::by_name(name);
        if ind.is_none() {
            unknown.push(name.clone());
        }
        ind
    }).collect();

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
    let tmp_path = output_path.clone().unwrap_or_else(|| {
        format!("/tmp/chartgen_{}.png", std::process::id())
    });

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

    // Clean up temp file if we created it
    if output_path.is_none() {
        std::fs::remove_file(&tmp_path).ok();
    }

    let b64 = BASE64.encode(&png_bytes);

    let mut content = vec![
        json!({ "type": "image", "data": b64, "mimeType": "image/png" }),
    ];

    if let Some(ref path) = output_path {
        content.push(json!({ "type": "text", "text": format!("Saved to {}", path) }));
    }

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "content": content }
    })
}

fn tool_list_indicators(id: Option<Value>) -> Value {
    let list = indicators::available();
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{
                "type": "text",
                "text": format!("Available indicators: {}", list.join(", "))
            }]
        }
    })
}
