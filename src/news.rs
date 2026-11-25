//! The tool `get_news` (ADR-0086): its arguments, then the `chartgen-news` crate's answer,
//! from the server's record of the news while it is current (ADR-0088), else from the
//! feeds.

use chartgen_news::{MAX_HOURS, MAX_LIMIT};
use serde_json::Value;

use crate::mcp::{tool_err, tool_ok};

fn integer(args: &Value, key: &str, default: u64, max: u64) -> Result<u64, String> {
    match args.get(key).filter(|v| !v.is_null()) {
        None => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=max).contains(n))
            .ok_or_else(|| format!("{key} must be an integer from 1 to {max}")),
    }
}

/// The store the news is recorded in, if this process has one.
fn store() -> Option<&'static dyn chartgen_news::Store> {
    #[cfg(feature = "scripting")]
    if let Some(store) = chartgen::scripting::persistence::global() {
        return Some(store);
    }
    None
}

/// `get_news`: one symbol's recent headlines. A feed that cannot be read is named under
/// `sources_failed` and never fails the call.
pub fn call_tool(id: Option<Value>, args: &Value) -> Value {
    if crate::replay::global().is_some() {
        return tool_err(
            id,
            "This is a replay server: get_news is not available, because it reads what the fixtures do not hold",
        );
    }
    let Some(text) = args.get("symbol").and_then(Value::as_str) else {
        return tool_err(id, "symbol is required, e.g. \"BTCUSDT\"");
    };
    let Some(symbol) = crate::mcp::normalize_quote_symbol(text) else {
        return tool_err(id, &format!("{text:?} is not a symbol"));
    };
    let (hours, limit) = match (
        integer(args, "hours", 24, MAX_HOURS),
        integer(args, "limit", 10, MAX_LIMIT),
    ) {
        (Ok(h), Ok(l)) => (h, l as usize),
        (Err(e), _) | (_, Err(e)) => return tool_err(id, &e),
    };
    match chartgen_news::read(store(), &symbol, hours, limit) {
        Ok(v) => tool_ok(id, v),
        Err(e) => tool_err(id, &e),
    }
}

/// Record the news into the store (ADR-0088), unless turned off or without a store.
#[cfg(feature = "scripting")]
pub fn start_recording() {
    match chartgen::scripting::persistence::global() {
        Some(store) => chartgen_news::record::start(store),
        None => eprintln!("[news-record] the store is unavailable; the news is not recorded"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Refused before any feed is read, so no test reaches an outlet.
    #[test]
    fn bad_arguments_are_refused() {
        let err = |args: Value| {
            let r = call_tool(None, &args);
            assert_eq!(r["result"]["isError"], true, "{args}");
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert!(err(json!({})).contains("symbol is required"));
        assert!(err(json!({ "symbol": "BTC USDT" })).contains("is not a symbol"));
        assert!(err(json!({ "symbol": "BTCUSDT", "hours": 0 })).contains("hours"));
        assert!(err(json!({ "symbol": "BTCUSDT", "hours": 169 })).contains("hours"));
        assert!(err(json!({ "symbol": "BTCUSDT", "limit": 51 })).contains("limit"));
    }
}
