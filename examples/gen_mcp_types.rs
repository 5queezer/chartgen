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
    let envelope = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": chartgen::mcp_schema::tools_list_result(),
    });
    println!("{}", serde_json::to_string_pretty(&envelope).unwrap());
}
