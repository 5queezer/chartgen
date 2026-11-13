//! Emit the canonical `tools/list` JSON-RPC result on stdout so the Node
//! codegen in `scripts/gen-mcp-types/` can convert each tool's
//! `inputSchema` into a TypeScript interface.
//!
//! Drives `chartgen::mcp_schema::tools_list_result()` in-process — no HTTP
//! server, no `--trade` mode, no runtime state.
//!
//! Usage: `cargo run --quiet --example gen_mcp_types > /tmp/tools.json`

use serde_json::json;

fn main() {
    let mut result = chartgen::mcp_schema::tools_list_result();
    result["tools"]
        .as_array_mut()
        .unwrap()
        .extend(chartgen::mcp_schema::server_alert_tools());
    result["tools"]
        .as_array_mut()
        .unwrap()
        .extend(chartgen::mcp_schema::setup_tools());
    result["tools"]
        .as_array_mut()
        .unwrap()
        .extend(chartgen::mcp_schema::event_tools());
    let envelope = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": result,
    });
    println!("{}", serde_json::to_string_pretty(&envelope).unwrap());
}
