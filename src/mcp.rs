use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as Base64Engine;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::sync::{Arc, RwLock};

use crate::chart_push::{ChartRequest, PushCtx, MAX_INDICATORS, NOT_PUSHABLE, PUSH_LIMIT};
use crate::fetch;
use crate::intrabars;
use crate::search;
use chartgen::data;
use chartgen::engine::Engine;
use chartgen::indicator::Indicator;
use chartgen::indicators;
use chartgen::renderer;
#[cfg(feature = "scripting")]
use chartgen::scripting;

/// `CHARTGEN_SCRIPTING=1` in a build with the `scripting` feature.
fn scripting_enabled() -> bool {
    #[cfg(feature = "scripting")]
    return scripting::enabled();
    #[cfg(not(feature = "scripting"))]
    false
}

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

        let response = handle_mcp_request(method, id, req.get("params"), None, None, None, None);

        writeln!(stdout, "{}", response).ok();
        stdout.flush().ok();
    }
}

/// Handle a single MCP JSON-RPC request. Shared by stdio and HTTP transports.
/// When `engine` is `Some`, trading tools are available; otherwise they return
/// an error indicating that the trading engine is not running.
/// `token` is the client's Bearer token (if any), used for subscription identity.
/// `push` is set on the HTTP transport, for the chart push tools (ADR-0021).
/// `user` owns the caller's scripted indicators (the OAuth client id, ADR-0025);
/// `None` on stdio and whenever scripting is disabled.
pub fn handle_mcp_request(
    method: &str,
    id: Option<Value>,
    params: Option<&Value>,
    engine: Option<&Arc<RwLock<Engine>>>,
    token: Option<&str>,
    push: Option<&PushCtx>,
    user: Option<&str>,
) -> Value {
    match method {
        "initialize" => handle_initialize(id),
        "tools/list" => handle_tools_list(id, scripting_enabled()),
        "tools/call" => handle_tools_call(id, params, engine, token, push, user),
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
            "protocolVersion": "2025-03-26",
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

/// The scripting tools are listed only when `scripting` (CHARTGEN_SCRIPTING=1).
fn handle_tools_list(id: Option<Value>, scripting: bool) -> Value {
    let mut result = chartgen::mcp_schema::tools_list_result();
    if scripting {
        if let Some(tools) = result["tools"].as_array_mut() {
            tools.extend(chartgen::mcp_schema::scripting_tools());
        }
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn handle_tools_call(
    id: Option<Value>,
    params: Option<&Value>,
    engine: Option<&Arc<RwLock<Engine>>>,
    token: Option<&str>,
    push: Option<&PushCtx>,
    user: Option<&str>,
) -> Value {
    let tool_name = params
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");

    let arguments = params
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or(json!({}));

    call_tool(
        tool_name,
        id,
        arguments,
        engine,
        token,
        push,
        user,
        scripting_enabled(),
    )
}

/// Dispatch one tool call. The scripting tools exist only when `scripting`.
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(feature = "scripting"), allow(unused_variables))]
fn call_tool(
    tool_name: &str,
    id: Option<Value>,
    arguments: Value,
    engine: Option<&Arc<RwLock<Engine>>>,
    token: Option<&str>,
    push: Option<&PushCtx>,
    user: Option<&str>,
    scripting: bool,
) -> Value {
    match tool_name {
        "generate_chart" => tool_generate_chart(id, &arguments, user),
        "list_indicators" => tool_list_indicators(id, user),
        "get_indicators" => tool_get_indicators(id, &arguments, engine),
        "get_quotes" => tool_get_quotes(id, &arguments),
        "get_footprint" => tool_get_footprint(id, &arguments),
        "get_tpo" => tool_get_tpo(id, &arguments),
        "search_symbols" => tool_search_symbols(id, &arguments),
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
        "subscribe_chart" => tool_subscribe_chart(id, &arguments, push, token, user),
        "unsubscribe_chart" => tool_unsubscribe_chart(id, &arguments, push, token),
        #[cfg(feature = "scripting")]
        "register_indicator" if scripting => tool_register_indicator(id, &arguments, user),
        #[cfg(feature = "scripting")]
        "fork_indicator" if scripting => tool_fork_indicator(id, &arguments),
        #[cfg(feature = "scripting")]
        "list_my_indicators" if scripting => tool_list_my_indicators(id, user),
        #[cfg(feature = "scripting")]
        "get_my_indicator" if scripting => tool_get_my_indicator(id, &arguments, user),
        #[cfg(feature = "scripting")]
        "delete_indicator" if scripting => tool_delete_indicator(id, &arguments, user),
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
/// Returns (indicators, instance-keys-in-order, unknown-names, invalid-parameter
/// messages, per-key errors). With a `user`, a name nobody knows may be a script
/// that was deleted or belongs to a previous client, and a script's params may no
/// longer fit its new schema: those fail only their key, under `indicator_errors`
/// (ADR-0041). Without one, and for a built-in's params, they fail the request.
type ParsedPanels = (
    Vec<Box<dyn Indicator>>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<(String, Value)>,
);

fn key_error(step: &str, message: String) -> Value {
    json!({ "step": step, "line": null, "message": message })
}

fn unknown_key_error(name: &str) -> Value {
    key_error(
        "unknown",
        format!("Unknown indicator: {name}. It is not one of your scripts or a built-in."),
    )
}

/// Longest accepted instance `id` (ADR-0028).
const MAX_INSTANCE_ID: usize = 64;

/// Most indicator entries one `generate_chart` call accepts (the web app's
/// cap too).
const MAX_CHART_INDICATORS: usize = 50;

/// Response key of every entry (ADR-0028): an object's `id` when it has one,
/// else its name; a name requested again without an id gets `name#2`,
/// `name#3`... so no instance overwrites another. `None` for an entry
/// without a name; `Err` for an invalid or repeated `id`, or an `id` equal
/// to the name of an entry without one (that entry's key).
fn instance_keys(arr: &[Value]) -> Result<Vec<Option<String>>, String> {
    let entry_name = |item: &Value| -> Option<String> {
        item.as_str()
            .or_else(|| item.get("name").and_then(|n| n.as_str()))
            .map(str::to_string)
    };
    // Names of the entries keyed by name.
    let plain: HashSet<String> = arr
        .iter()
        .filter(|item| item.get("id").is_none_or(Value::is_null))
        .filter_map(entry_name)
        .collect();
    let mut taken = HashSet::new();
    let mut ids = Vec::with_capacity(arr.len());
    for item in arr {
        let id = match item.get("id") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if !s.is_empty() && s.chars().count() <= MAX_INSTANCE_ID => {
                if !taken.insert(s.clone()) {
                    return Err(format!("Duplicate indicator id: {s}"));
                }
                if plain.contains(s) {
                    return Err(format!(
                        "Indicator id {s} is also the name of an entry without an id; give that entry an id too"
                    ));
                }
                Some(s.clone())
            }
            Some(_) => {
                return Err(format!(
                    "Invalid indicator id: must be a string of 1 to {MAX_INSTANCE_ID} characters"
                ))
            }
        };
        ids.push(id);
    }
    // Next number to try per name (1: the bare name). Keys are only ever
    // added, so it never has to go back: linear in the entry count.
    let mut next: HashMap<String, usize> = HashMap::new();
    Ok(arr
        .iter()
        .zip(ids)
        .map(|(item, id)| {
            let name = entry_name(item)?;
            if id.is_some() {
                return id;
            }
            let n = next.entry(name.clone()).or_insert(1);
            let key = loop {
                let key = match *n {
                    1 => name.clone(),
                    k => format!("{name}#{k}"),
                };
                *n += 1;
                if !taken.contains(&key) {
                    break key;
                }
            };
            taken.insert(key.clone());
            Some(key)
        })
        .collect())
}

fn parse_panels(panels_value: &Value, user: Option<&str>) -> ParsedPanels {
    let arr = match panels_value.as_array() {
        Some(a) => a,
        None => return (vec![], vec![], vec![], vec![], vec![]),
    };
    let instance = match instance_keys(arr) {
        Ok(k) => k,
        Err(e) => return (vec![], vec![], vec![], vec![e], vec![]),
    };
    let mut invalid = Vec::new();
    let mut per_key = Vec::new();

    let mut result: Vec<Box<dyn Indicator>> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    let mut errors = Vec::new();

    for (item, key) in arr.iter().zip(instance) {
        let Some(key) = key else { continue };
        if let Some(name) = item.as_str() {
            // Simple string: "rsi"
            match indicators::by_name_for(name, user) {
                Some(ind) => {
                    result.push(ind);
                    keys.push(key);
                }
                None if user.is_some() => per_key.push((key, unknown_key_error(name))),
                None => errors.push(name.to_string()),
            }
        } else if let Some(obj) = item.as_object() {
            // Object: {"name": "rsi", "length": 21, "id": "rsi#2"}; the id
            // is the response key, not a parameter.
            let name = match obj.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };
            let mut params = obj.clone();
            params.remove("id");
            match indicators::by_name_configured_for(name, &Value::Object(params), user) {
                Ok(Some(ind)) => {
                    result.push(ind);
                    keys.push(key);
                }
                Ok(None) if user.is_some() => per_key.push((key, unknown_key_error(name))),
                Ok(None) => errors.push(name.to_string()),
                Err(msg) if indicators::is_user_script(name, user) => {
                    per_key.push((key, key_error("params", msg)));
                }
                Err(msg) => invalid.push(msg),
            }
        }
    }

    (result, keys, errors, invalid, per_key)
}

/// Whether an entry of a chart's indicators resolves to `user`'s own script.
fn names_user_script(indicators: &Value, user: Option<&str>) -> bool {
    indicators.as_array().into_iter().flatten().any(|item| {
        item.as_str()
            .or_else(|| item.get("name").and_then(|n| n.as_str()))
            .is_some_and(|name| indicators::is_user_script(name, user))
    })
}

/// Upper bound for `bars`: Binance pagination costs one upstream request per
/// 1000 bars.
const MAX_BARS: usize = 5000;
/// Upper bound for `warmup` (ADR-0009).
const MAX_WARMUP: usize = 1000;
/// Newest pattern labels in a summary: the rest are only counted.
const MAX_SUMMARY_LABELS: usize = 20;

/// The `warmup` argument as the client sent it, capped.
fn asked_warmup(args: &Value) -> usize {
    (args.get("warmup").and_then(|v| v.as_u64()).unwrap_or(0) as usize).min(MAX_WARMUP)
}

/// The `warmup` argument, capped. A non-zero one is raised to what the
/// indicators need to converge, so values at a page seam or on a tail match
/// the history loaded before (ADR-0032).
fn warmup_arg(args: &Value, panels: &[Box<dyn Indicator>]) -> usize {
    let asked = asked_warmup(args);
    if asked == 0 {
        return 0;
    }
    let need = panels.iter().map(|i| i.warmup_bars()).max().unwrap_or(0);
    asked.max(need).min(MAX_WARMUP)
}

/// Oldest accepted `end_time`: 1900-01-01 (index histories predate 1970).
const MIN_END_TIME_MS: i64 = -2_208_988_800_000;
/// Newest accepted `end_time`, relative to now.
const MAX_END_TIME_AHEAD_MS: i64 = 366 * 86_400_000;

fn chart_style_arg(args: &Value) -> Result<&'static str, &'static str> {
    match args.get("chart_style") {
        None | Some(Value::Null) => Ok("candles"),
        Some(v) => match v.as_str() {
            Some("candles") => Ok("candles"),
            Some("heikin_ashi") => Ok("heikin_ashi"),
            _ => Err("chart_style must be one of 'candles' or 'heikin_ashi'."),
        },
    }
}

/// The canonical form of a requested timeframe, or why it is rejected
/// (ADR-0038). A chart is echoed and keyed by this form.
fn check_interval(interval: &str) -> Result<String, String> {
    chartgen::interval::Interval::parse(interval).map(|iv| iv.canonical())
}

/// Accept both "indicators" and "panels" parameter names
fn indicators_arg(args: &Value) -> Value {
    args.get("indicators")
        .or_else(|| args.get("panels"))
        .cloned()
        .unwrap_or_else(|| json!(["ema_stack", "cipher_b", "macd"]))
}

fn check_panels(unknown: &[String], invalid: &[String]) -> Result<(), String> {
    if !invalid.is_empty() {
        return Err(invalid.join("; "));
    }
    if !unknown.is_empty() {
        return Err(format!(
            "Unknown indicator(s): {}. Call list_indicators to see available indicators.",
            unknown.join(", ")
        ));
    }
    Ok(())
}

/// `user` resolves the caller's scripted indicators first (ADR-0025).
fn tool_generate_chart(id: Option<Value>, args: &Value, user: Option<&str>) -> Value {
    generate_chart(id, args, fetch::DataSource::from_env(), user)
}

pub(crate) fn generate_chart(
    id: Option<Value>,
    args: &Value,
    source: fetch::DataSource,
    user: Option<&str>,
) -> Value {
    let bars = (args.get("bars").and_then(|v| v.as_u64()).unwrap_or(200) as usize).min(MAX_BARS);
    // Exclusive upper bound (ms since epoch) on bar open time — history paging.
    // JSON null counts as absent; anything else must be an in-range integer.
    let end_time = match args.get("end_time") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let max = fetch::now_ms().saturating_add(MAX_END_TIME_AHEAD_MS);
            match v.as_i64().filter(|t| (MIN_END_TIME_MS..=max).contains(t)) {
                Some(t) => Some(t),
                None => {
                    return tool_err(
                        id,
                        "end_time must be an integer (ms since epoch) between 1900-01-01 and one year from now.",
                    )
                }
            }
        }
    };
    let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
    let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;
    let chart_style = match chart_style_arg(args) {
        Ok(s) => s,
        Err(e) => return tool_err(id, e),
    };

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
    let interval = match check_interval(interval) {
        Ok(canonical) => canonical,
        Err(e) => return tool_err(id, &e),
    };
    let interval = interval.as_str();

    let panels_value = indicators_arg(args);

    // Output format: "png" (default), "summary" (JSON only), "both",
    // or "series" (full per-bar JSON time-series).
    let format = args
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("png")
        .to_lowercase();

    let want_png = matches!(format.as_str(), "png" | "both");
    let want_summary = matches!(format.as_str(), "summary" | "both");
    let want_series = format.as_str() == "series";
    if !want_png && !want_summary && !want_series {
        return tool_err(
            id,
            "Unknown format: expected 'png', 'summary', 'both', or 'series'.",
        );
    }

    if panels_value
        .as_array()
        .is_some_and(|a| a.len() > MAX_CHART_INDICATORS)
    {
        return tool_err(
            id,
            &format!("At most {MAX_CHART_INDICATORS} indicators per chart."),
        );
    }
    let (panel_indicators, indicator_keys, unknown, invalid, per_key) =
        parse_panels(&panels_value, user);
    if let Err(e) = check_panels(&unknown, &invalid) {
        return tool_err(id, &e);
    }

    // Warmup bars feed indicator computation but are trimmed from the series
    // payload. png/summary ignore it and render exactly `bars` (ADR-0009).
    let warmup = if want_series {
        warmup_arg(args, &panel_indicators)
    } else {
        0
    };
    let requested = bars + warmup;

    // Without a ticker the live source serves the synthetic demo series
    // (no timestamps, so nothing to page). With one, a fetch failure is an
    // error in every format: sample bars would masquerade as market data.
    let (mut data, has_more) = match (symbol, source) {
        (None, fetch::DataSource::Live) if end_time.is_none() => {
            (data::sample_data(requested), false)
        }
        (None, fetch::DataSource::Live) => {
            return tool_err(
                id,
                "end_time requires a ticker (or CHARTGEN_DATA_SOURCE=sample).",
            )
        }
        (sym, _) => {
            match fetch::fetch_ohlcv(source, sym.unwrap_or(""), interval, requested, end_time) {
                // Warmup bars are trimmed from the payload. When history ran
                // out they are still older bars the client has not seen, so
                // it must page on to get them (ADR-0032).
                Ok(f) => {
                    let more = f.has_more || f.data.len() > bars;
                    (f.data, more)
                }
                Err(e) => return tool_err(id, &format!("Data fetch failed: {e}")),
            }
        }
    };
    data.symbol = symbol.map(|s| s.to_string());
    data.interval = Some(interval.to_string());
    // Heikin Ashi runs over bars+warmup, before indicators and trimming, so
    // indicators see the same values TradingView computes on an HA chart.
    if chart_style == "heikin_ashi" {
        data = data.heikin_ashi();
    }

    // Compute indicators up-front (once) so both the summary and the series
    // payloads can reuse the same PanelResult values — no recomputation,
    // values already aligned 1:1 with bars. The PNG renders from them too.
    // An indicator that fails (a user script, ADR-0025) is left out and
    // reported under `indicator_errors`; the rest of the chart still renders.
    let (computed, inds, errors_value) = compute_panels(&data, &indicator_keys, &panel_indicators);
    let errors_value = with_key_errors(errors_value, per_key);

    // Build summary payload keyed by request name (or instance id, ADR-0028)
    // so the LLM can correlate output with the entries it passed in.
    let summary_value = if want_summary {
        let mut indicators_json = serde_json::Map::new();
        for (key, pr) in &computed {
            indicators_json.insert(key.clone(), panel_result_to_json(pr));
        }
        let mut summary = json!({
            "symbol": data.symbol,
            "interval": data.interval,
            "price": ohlcv_summary(&data),
            "indicators": Value::Object(indicators_json),
        });
        if let Some(errors) = &errors_value {
            summary["indicator_errors"] = errors.clone();
        }
        Some(summary)
    } else {
        None
    };

    let series_value = want_series.then(|| {
        let mut series = series_json(&data, &computed, &inds, bars, chart_style, has_more);
        // Only a chart's full load asks. Pages and tails keep the load's profile
        // (ADR-0009), so reading finer bars for them would pay for nothing.
        if args.get("profile_intrabars").and_then(Value::as_bool) == Some(true) {
            apply_intrabar_profiles(&mut series, &data, &computed, &inds, bars, source);
        }
        if let Some(errors) = &errors_value {
            series["indicator_errors"] = errors.clone();
        }
        series
    });

    let mut content: Vec<Value> = Vec::new();

    if want_png {
        let tmp_path = format!(
            "/tmp/chartgen_{}_{}.png",
            std::process::id(),
            uuid::Uuid::new_v4()
        );

        let results: Vec<&chartgen::indicator::PanelResult> =
            computed.iter().map(|(_, pr)| pr).collect();
        if let Err(e) = renderer::render_results(&data, &results, &tmp_path, width, height) {
            return tool_err(id, &format!("Render error: {}", e));
        }

        let png_bytes = match std::fs::read(&tmp_path) {
            Ok(b) => b,
            Err(e) => return tool_err(id, &format!("Read error: {}", e)),
        };

        std::fs::remove_file(&tmp_path).ok();

        let b64 = BASE64.encode(&png_bytes);
        content.push(json!({ "type": "image", "data": b64, "mimeType": "image/png" }));
    }

    if let Some(summary) = summary_value {
        content.push(json!({
            "type": "text",
            "text": serde_json::to_string_pretty(&summary).unwrap(),
        }));
    }

    if let Some(series) = series_value {
        content.push(json!({
            "type": "text",
            "text": serde_json::to_string_pretty(&series).unwrap(),
        }));
    }

    // format=png has no JSON payload to carry the errors: add a text item.
    if let (Some(errors), "png") = (&errors_value, format.as_str()) {
        content.push(json!({
            "type": "text",
            "text": serde_json::to_string_pretty(&json!({ "indicator_errors": errors })).unwrap(),
        }));
    }

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "content": content }
    })
}

/// Replace the profiles that want it with their form from lower-timeframe bars
/// (ADR-0048). Without those bars, for whatever reason, they stay as computed: the
/// entry then has no `profile_interval`, which is how a client can tell.
fn apply_intrabar_profiles(
    series: &mut Value,
    data: &chartgen::data::OhlcvData,
    computed: &[(String, chartgen::indicator::PanelResult)],
    inds: &[&dyn Indicator],
    bars: usize,
    source: fetch::DataSource,
) {
    if !inds.iter().any(|i| i.wants_intrabars()) {
        return;
    }
    let visible = chartgen::data::OhlcvData {
        bars: data.bars[data.len().saturating_sub(bars)..].to_vec(),
        symbol: data.symbol.clone(),
        interval: data.interval.clone(),
        session: data.session,
    };
    let Some(ltf) = intrabars_for(&visible, source) else {
        return;
    };
    for ((key, _), ind) in computed.iter().zip(inds) {
        if let Some(pr) = ind.with_intrabars(&visible, &ltf.0) {
            let mut entry = panel_result_to_series_json(&pr, &visible);
            entry["profile_interval"] = json!(ltf.1);
            series["indicators"][key] = entry;
        }
    }
}

/// Lower-timeframe bars covering `visible`, and their interval, if one can be afforded.
fn intrabars_for(
    visible: &chartgen::data::OhlcvData,
    source: fetch::DataSource,
) -> Option<(chartgen::data::OhlcvData, &'static str)> {
    let symbol = visible.symbol.as_deref()?;
    let iv = chartgen::interval::Interval::parse(visible.interval.as_deref()?).ok()?;
    let first: i64 = visible.bars.first()?.date.parse().ok()?;
    let last: i64 = visible.bars.last()?.date.parse().ok()?;
    let age = fetch::now_ms() / 1000 - first;
    let (name, secs) =
        intrabars::plan(fetch::detect_source(symbol), iv.secs(), visible.len(), age)?;
    let list =
        intrabars::load(source, symbol, name, secs, &visible.bars, iv.grid_end(last)).ok()?;
    if list.is_empty() {
        return None;
    }
    let bars = list
        .iter()
        .map(|b| chartgen::data::Bar {
            open: b.open,
            high: b.high,
            low: b.low,
            close: b.close,
            volume: b.volume,
            date: b.t.to_string(),
        })
        .collect();
    Some((
        chartgen::data::OhlcvData {
            bars,
            symbol: visible.symbol.clone(),
            interval: Some(name.to_string()),
            session: None,
        },
        name,
    ))
}

/// Computed panels for a chart: `(key, result)` and the indicator of each
/// success, plus the `indicator_errors` map of the failures (`None` when all
/// succeeded).
type Computed<'a> = (
    Vec<(String, chartgen::indicator::PanelResult)>,
    Vec<&'a dyn Indicator>,
    Option<Value>,
);

/// `errors` (an `indicator_errors` map) plus the errors found before computing.
fn with_key_errors(errors: Option<Value>, per_key: Vec<(String, Value)>) -> Option<Value> {
    if per_key.is_empty() {
        return errors;
    }
    let mut map = match errors {
        Some(Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    map.extend(per_key);
    Some(Value::Object(map))
}

fn compute_panels<'a>(
    data: &chartgen::data::OhlcvData,
    keys: &[String],
    panels: &'a [Box<dyn Indicator>],
) -> Computed<'a> {
    let (mut computed, mut inds) = (Vec::new(), Vec::new());
    let mut errors = serde_json::Map::new();
    let compute = || {
        for (key, ind) in keys.iter().zip(panels) {
            match ind.try_compute(data) {
                Ok(pr) => {
                    computed.push((key.clone(), pr));
                    inds.push(ind.as_ref());
                }
                Err(e) => {
                    errors.insert(key.clone(), e.to_json());
                }
            }
        }
    };
    // One script budget for the whole chart (ADR-0025).
    #[cfg(feature = "scripting")]
    scripting::with_chart_budget(compute);
    #[cfg(not(feature = "scripting"))]
    ({ compute })();
    let errors = (!errors.is_empty()).then_some(Value::Object(errors));
    (computed, inds, errors)
}

// Build series payload: full OHLCV bars plus per-indicator per-bar values.
// Indicators ran over warmup + visible bars; only the last `bars` are
// emitted, with every index-based channel re-based to match.
fn series_json(
    data: &chartgen::data::OhlcvData,
    computed: &[(String, chartgen::indicator::PanelResult)],
    panel_indicators: &[&dyn Indicator],
    bars: usize,
    chart_style: &str,
    has_more: bool,
) -> Value {
    // Trimming must never leave an empty page that claims more history: the
    // client pages on bars[0].t, so with nothing to read it would be stuck.
    // Return the fetched bars (indicators null where unconverged) instead.
    let offset = if bars == 0 {
        0
    } else {
        data.len().saturating_sub(bars)
    };
    let visible = chartgen::data::OhlcvData {
        bars: data.bars[offset..].to_vec(),
        symbol: data.symbol.clone(),
        interval: data.interval.clone(),
        session: data.session,
    };
    let bars_arr: Vec<Value> = visible
        .bars
        .iter()
        .map(|b| {
            json!({
                "t": bar_t(b),
                "o": finite_or_null(b.open),
                "h": finite_or_null(b.high),
                "l": finite_or_null(b.low),
                "c": finite_or_null(b.close),
                "v": finite_or_null(b.volume),
            })
        })
        .collect();
    let bars_arr_empty = bars_arr.is_empty();
    let mut indicators_json = serde_json::Map::new();
    for ((key, pr), ind) in computed.iter().zip(panel_indicators.iter()) {
        let mut trimmed = trim_panel_result(pr, offset);
        // Volume/time profiles aggregate over the whole input window,
        // not per bar: recompute their profile-derived channels (hbars
        // and POC/VA/node hlines) over the visible window so warmup bars
        // don't leak into the profile.
        if offset > 0 && ind.windowed_profile() {
            let profile = ind.compute(&visible);
            trimmed.hbars = profile.hbars;
            trimmed.hlines = profile.hlines;
        }
        let mut entry = panel_result_to_series_json(&trimmed, &visible);
        add_marks_to_series_json(&mut entry, pr, data, offset);
        if let Some(p) = ind.projection(data) {
            // Every bar, not the visible ones: the times read closed days off
            // the loaded bars, and a two-bar tail shows too few.
            entry["future"] = projection_to_json(&p, data);
        }
        indicators_json.insert(key.clone(), entry);
    }
    json!({
        "symbol": data.symbol,
        "interval": data.interval,
        "chart_style": chart_style,
        "bars": bars_arr,
        "indicators": Value::Object(indicators_json),
        "has_more": has_more && bars > 0 && !bars_arr_empty,
    })
}

/// The `format=series` payload for the newest `bars` of `raw` (symbol and
/// interval set, `bars` + warmup long), computed exactly like
/// `generate_chart`: Heikin Ashi first, then indicators over every bar, then
/// trimmed. Chart push (ADR-0021) builds its tails with this.
pub(crate) fn series_tail(
    raw: &chartgen::data::OhlcvData,
    indicators: &Value,
    chart_style: &str,
    bars: usize,
) -> Result<Value, String> {
    let (panel_indicators, keys, unknown, invalid, _) = parse_panels(indicators, None);
    check_panels(&unknown, &invalid)?;
    let ha;
    let data = if chart_style == "heikin_ashi" {
        ha = raw.heikin_ashi();
        &ha
    } else {
        raw
    };
    let (computed, inds, _) = compute_panels(data, &keys, &panel_indicators);
    let has_more = data.len() > bars;
    Ok(series_json(
        data,
        &computed,
        &inds,
        bars,
        chart_style,
        has_more,
    ))
}

/// `subscribe_chart`: push `generate_chart(format=series, bars=2, warmup)`
/// tails of a chart over the caller's SSE stream (ADR-0021).
fn tool_subscribe_chart(
    id: Option<Value>,
    args: &Value,
    push: Option<&PushCtx>,
    token: Option<&str>,
    user: Option<&str>,
) -> Value {
    let (Some(push), Some(token)) = (push, token) else {
        return no_push_transport(id);
    };
    let Some(ticker) = args
        .get("ticker")
        .or_else(|| args.get("symbol"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    else {
        return tool_err(id, "ticker is required.");
    };
    let timeframe = args
        .get("timeframe")
        .or_else(|| args.get("interval"))
        .and_then(|v| v.as_str())
        .unwrap_or("4h");
    let timeframe = match check_interval(timeframe) {
        Ok(canonical) => canonical,
        Err(e) => return tool_err(id, &e),
    };
    let chart_style = match chart_style_arg(args) {
        Ok(s) => s,
        Err(e) => return tool_err(id, e),
    };
    let indicators = indicators_arg(args);
    // Every push recomputes every indicator.
    if indicators
        .as_array()
        .is_some_and(|a| a.len() > MAX_INDICATORS)
    {
        return tool_err(
            id,
            &format!("{PUSH_LIMIT}: at most {MAX_INDICATORS} indicators per subscription"),
        );
    }
    let (panels, _, unknown, invalid, per_key) = parse_panels(&indicators, user);
    if let Err(e) = check_panels(&unknown, &invalid) {
        return tool_err(id, &e);
    }
    // Pushed tails are computed without a user (`series_tail`): a script would
    // be missing, or replaced by the built-in it shadows. Such a chart polls.
    if !per_key.is_empty() || names_user_script(&indicators, user) {
        return tool_err(
            id,
            &format!("{NOT_PUSHABLE}: the chart uses your scripts; poll generate_chart instead"),
        );
    }
    let req = ChartRequest {
        ticker: ticker.to_string(),
        timeframe,
        indicators: indicators.to_string(),
        chart_style: chart_style.to_string(),
        warmup: warmup_arg(args, &panels),
        asked_warmup: asked_warmup(args),
    };
    match push.hub.subscribe(token, &push.client, &push.stream, req) {
        Ok(sub) => tool_ok(id, json!({ "subscription_id": sub })),
        Err(e) => tool_err(id, &e),
    }
}

fn tool_unsubscribe_chart(
    id: Option<Value>,
    args: &Value,
    push: Option<&PushCtx>,
    token: Option<&str>,
) -> Value {
    let (Some(push), Some(token)) = (push, token) else {
        return no_push_transport(id);
    };
    let Some(sub) = args.get("subscription_id").and_then(|v| v.as_str()) else {
        return tool_err(id, "subscription_id is required.");
    };
    tool_ok(
        id,
        json!({ "unsubscribed": push.hub.unsubscribe(token, sub) }),
    )
}

fn no_push_transport(id: Option<Value>) -> Value {
    tool_err(
        id,
        "Chart push needs the HTTP transport: an authenticated POST /mcp with an open GET /mcp SSE stream.",
    )
}

/// Built-ins plus the caller's scripts (tagged `source: "user"`); a script
/// replaces the built-in entry it shadows.
fn tool_list_indicators(id: Option<Value>, user: Option<&str>) -> Value {
    let reg = indicators::registry();
    #[cfg(feature = "scripting")]
    let mine: Vec<Value> = user
        .and_then(|u| scripting::persistence::global()?.list(u).ok())
        .unwrap_or_default()
        .iter()
        .map(user_indicator_entry)
        .collect();
    #[cfg(not(feature = "scripting"))]
    let mine: Vec<Value> = {
        let _ = user;
        Vec::new()
    };
    let mut indicator_list: Vec<Value> = reg
        .iter()
        .filter(|info| !mine.iter().any(|m| m["name"] == info.name))
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
    indicator_list.extend(mine);

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
// User indicators: Starlark scripts (ADR-0024, ADR-0025)
// ---------------------------------------------------------------------------

#[cfg(feature = "scripting")]
/// `list_indicators` entry for a stored script.
fn user_indicator_entry(s: &scripting::persistence::StoredScript) -> Value {
    let m = &s.meta;
    let overlay = m["overlay"].as_bool().unwrap_or(false);
    let description = match m["description"].as_str() {
        Some(d) if !d.is_empty() => d,
        _ => m["title"].as_str().unwrap_or(&s.name),
    };
    let mut entry = json!({
        "name": s.name,
        "description": description,
        "type": if overlay { "overlay" } else { "panel" },
        "overlay": overlay,
        "source": "user",
    });
    if m["params"].as_array().is_some_and(|p| !p.is_empty()) {
        entry["configurable_params"] = m["params"].clone();
    }
    entry
}

#[cfg(feature = "scripting")]
/// The store and user for a scripting tool, or the error response.
fn scripting_session<'u>(
    id: &Option<Value>,
    user: Option<&'u str>,
    tool: &str,
) -> Result<(&'static scripting::persistence::Store, &'u str), Value> {
    let Some(user) = user else {
        return Err(tool_err(
            id.clone(),
            &format!(
                "{tool} needs an authenticated HTTP session: user indicators belong to the OAuth client. The stdio server has no user."
            ),
        ));
    };
    match scripting::persistence::global() {
        Some(store) => Ok((store, user)),
        None => Err(tool_err(
            id.clone(),
            "The indicator store is unavailable (see the server log for CHARTGEN_DB).",
        )),
    }
}

#[cfg(feature = "scripting")]
fn tool_register_indicator(id: Option<Value>, args: &Value, user: Option<&str>) -> Value {
    let (store, user) = match scripting_session(&id, user, "register_indicator") {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let Some(source) = args.get("source").and_then(|v| v.as_str()) else {
        return tool_err(id, "source is required");
    };
    if !scripting::valid_name(name) {
        return tool_err(
            id,
            "name must be 1-48 lowercase letters, digits or _, starting with a letter",
        );
    }
    let is_new = matches!(store.get(user, name), Ok(None));
    if is_new && store.count(user).unwrap_or(0) >= scripting::MAX_SCRIPTS_PER_USER {
        return tool_err(
            id,
            &format!(
                "You have {} indicators, the limit; delete one first.",
                scripting::MAX_SCRIPTS_PER_USER
            ),
        );
    }
    let compiled = match scripting::validate_as(user, name, source) {
        Ok(c) => c,
        Err(e) => {
            let payload = json!({ "ok": false, "errors": [e.to_json()] });
            return tool_err(id, &serde_json::to_string_pretty(&payload).unwrap());
        }
    };
    let info = &compiled.info;
    let hash = scripting::source_hash(source);
    let create_only = args
        .get("create_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let stored = if create_only {
        store.insert_new(user, name, source, &info.to_json(), &hash)
    } else {
        store
            .upsert(user, name, source, &info.to_json(), &hash)
            .map(|()| true)
    };
    match stored {
        Ok(true) => {}
        Ok(false) => {
            let error = json!({
                "step": "exists",
                "line": null,
                "message": format!("You already have an indicator named {name}. Open it, or pick another name."),
            });
            let payload = json!({ "ok": false, "errors": [error] });
            return tool_err(id, &serde_json::to_string_pretty(&payload).unwrap());
        }
        Err(e) => return tool_err(id, &format!("Could not store the indicator: {e}")),
    }
    tool_ok(
        id,
        json!({
            "ok": true,
            "name": name,
            "title": info.title,
            "overlay": info.overlay,
            "configurable_params": info.params,
            "replaced": !is_new,
            "shadows_builtin": indicators::by_name(name).is_some(),
        }),
    )
}

#[cfg(feature = "scripting")]
fn tool_fork_indicator(id: Option<Value>, args: &Value) -> Value {
    let name = args
        .get("builtin_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let Some(source) = scripting::refimpl(name) else {
        let forkable: Vec<&str> = scripting::REFIMPLS.iter().map(|(n, _)| *n).collect();
        return tool_err(
            id,
            &format!(
                "No Starlark reference implementation for `{name}`. Forkable built-ins: {}. Others can be rebuilt from ta.* functions.",
                forkable.join(", ")
            ),
        );
    };
    let params = scripting::runtime::compile(name, source)
        .map(|c| json!(c.info.params))
        .unwrap_or(json!([]));
    tool_ok(
        id,
        json!({ "name": name, "source": source, "configurable_params": params }),
    )
}

#[cfg(feature = "scripting")]
fn tool_list_my_indicators(id: Option<Value>, user: Option<&str>) -> Value {
    let (store, user) = match scripting_session(&id, user, "list_my_indicators") {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let scripts = match store.list(user) {
        Ok(s) => s,
        Err(e) => return tool_err(id, &format!("Could not read your indicators: {e}")),
    };
    let list: Vec<Value> = scripts
        .iter()
        .map(|s| {
            let mut e = user_indicator_entry(s);
            e["title"] = s.meta["title"].clone();
            e["created_at"] = json!(s.created_at);
            e["updated_at"] = json!(s.updated_at);
            e
        })
        .collect();
    tool_ok(id, json!({ "total": list.len(), "indicators": list }))
}

#[cfg(feature = "scripting")]
fn tool_get_my_indicator(id: Option<Value>, args: &Value, user: Option<&str>) -> Value {
    let (store, user) = match scripting_session(&id, user, "get_my_indicator") {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
    match store.get(user, name) {
        Ok(Some(s)) => tool_ok(
            id,
            json!({ "name": s.name, "source": s.source, "updated_at": s.updated_at }),
        ),
        // Told apart from a failed read: a client may treat this one as "gone".
        Ok(None) => {
            let payload = json!({
                "ok": false,
                "error": "not_found",
                "message": format!("You have no indicator named `{name}`."),
            });
            tool_err(id, &serde_json::to_string_pretty(&payload).unwrap())
        }
        Err(e) => tool_err(id, &format!("Could not read the indicator: {e}")),
    }
}

#[cfg(feature = "scripting")]
fn tool_delete_indicator(id: Option<Value>, args: &Value, user: Option<&str>) -> Value {
    let (store, user) = match scripting_session(&id, user, "delete_indicator") {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
    match store.delete(user, name) {
        Ok(true) => tool_ok(id, json!({ "ok": true, "name": name })),
        Ok(false) => tool_err(id, &format!("You have no indicator named `{name}`.")),
        Err(e) => tool_err(id, &format!("Could not delete the indicator: {e}")),
    }
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

pub(crate) fn tool_ok(id: Option<Value>, payload: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&payload).unwrap() }]
        }
    })
}

pub(crate) fn tool_err(id: Option<Value>, msg: &str) -> Value {
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
// Extract meaningful values from a PanelResult for JSON output.
// Generic over all indicators: reads only PanelResult fields, so any new
// indicator that returns a PanelResult is covered automatically.
// ---------------------------------------------------------------------------

fn finite_or_null(v: f64) -> Value {
    if v.is_finite() {
        json!(v)
    } else {
        Value::Null
    }
}

// Bar.date stores unix-seconds as a string for real feeds (Binance/Yahoo)
// and a synthetic label (e.g. "D0") for sample_data. Parse to u64 when
// possible; fall back to JSON null so fixtures still produce valid output.
fn bar_t(bar: &chartgen::data::Bar) -> Value {
    bar.date
        .parse::<i64>()
        .ok()
        .map(|n| json!(n))
        .unwrap_or(Value::Null)
}

// Drop the first `offset` (warmup) bars from every index-based channel and
// re-base indices onto the remaining bars. Dots inside the warmup are
// dropped; divlines wholly inside it are dropped, and ones straddling the
// boundary are clipped to start at bar 0 with y interpolated (lines that
// only touch bar 0 would collapse to a point and are dropped too). hlines, hbars
// and y_range are price-based and pass through unchanged.
fn trim_panel_result(
    pr: &chartgen::indicator::PanelResult,
    offset: usize,
) -> chartgen::indicator::PanelResult {
    fn tail<T: Clone>(v: &[T], offset: usize) -> Vec<T> {
        v.get(offset..).unwrap_or(&[]).to_vec()
    }
    let mut out = pr.clone();
    for line in &mut out.lines {
        line.y = tail(&line.y, offset);
    }
    for fill in &mut out.fills {
        fill.y1 = tail(&fill.y1, offset);
        fill.y2 = tail(&fill.y2, offset);
    }
    for b in &mut out.bars {
        b.y = tail(&b.y, offset);
        b.colors = tail(&b.colors, offset);
    }
    out.dots.retain(|d| d.x >= offset);
    for d in &mut out.dots {
        d.x -= offset;
    }
    out.divlines.retain(|d| d.x1.max(d.x2) > offset);
    for d in &mut out.divlines {
        if d.x1 < offset {
            d.y1 += (d.y2 - d.y1) * (offset - d.x1) as f64 / (d.x2 - d.x1) as f64;
            d.x1 = offset;
        }
        if d.x2 < offset {
            d.y2 += (d.y1 - d.y2) * (offset - d.x2) as f64 / (d.x1 - d.x2) as f64;
            d.x2 = offset;
        }
        d.x1 -= offset;
        d.x2 -= offset;
    }
    out
}

// CSS color string for the wire; the PNG renderer's colors are the source of
// truth, so the web chart draws overlay geometry with exactly these.
fn css_rgba(c: &plotters::style::RGBAColor) -> String {
    format!("rgba({},{},{},{})", c.0, c.1, c.2, c.3)
}

// Legend label of histogram `i` (ADR-0026). `Bars` carry no label: the
// panel name labels it (Volume, AO) unless a line already has that name
// (MACD's "MACD" line), then "Histogram"; numbered when there are several.
fn histogram_label(pr: &chartgen::indicator::PanelResult, i: usize) -> String {
    let taken = pr.label.is_empty()
        || pr
            .lines
            .iter()
            .any(|l| l.label.as_deref() == Some(pr.label.as_str()));
    let base = if taken {
        "Histogram"
    } else {
        pr.label.as_str()
    };
    if pr.bars.len() > 1 {
        format!("{base} {}", i + 1)
    } else {
        base.to_string()
    }
}

// Series-shape serializer: full per-bar values for each line, signals with
// bar timestamps, and the same metadata (label/is_overlay/y_range/hlines)
// that summary exposes. Pulls directly from the PanelResult — no
// recomputation. NaN pre-warmup values serialize as JSON null.
fn panel_result_to_series_json(
    pr: &chartgen::indicator::PanelResult,
    data: &chartgen::data::OhlcvData,
) -> Value {
    let mut out = json!({ "label": pr.label, "is_overlay": pr.is_overlay });

    if pr.level_lines {
        out["level_lines"] = json!(true);
    }

    if let Some((lo, hi)) = pr.y_range {
        out["y_range"] = json!([finite_or_null(lo), finite_or_null(hi)]);
    }

    if !pr.lines.is_empty() {
        let lines: Vec<Value> = pr
            .lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let values: Vec<Value> = line.y.iter().map(|v| finite_or_null(*v)).collect();
                let mut j = json!({
                    "index": i,
                    "label": line.label,
                    "values": values,
                    "color": css_rgba(&line.color),
                    "width": line.width,
                });
                if line.breaks {
                    j["breaks"] = json!(true);
                }
                j
            })
            .collect();
        out["lines"] = json!(lines);
    }

    // Histogram (bar-style) series — e.g. MACD histogram, volume. Same
    // 1:1 alignment with bars as `lines`. Per-bar colors (ADR-0026) are
    // `color_idx` into `palette`, which lists each distinct color once;
    // null where the bar has no value or no color.
    if !pr.bars.is_empty() {
        let histogram: Vec<Value> = pr
            .bars
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let values: Vec<Value> = b.y.iter().map(|v| finite_or_null(*v)).collect();
                let mut palette: Vec<String> = Vec::new();
                let color_idx: Vec<Value> = b
                    .y
                    .iter()
                    .enumerate()
                    .map(|(k, v)| match b.colors.get(k) {
                        Some(c) if v.is_finite() => {
                            let css = css_rgba(c);
                            let idx = palette.iter().position(|p| *p == css).unwrap_or_else(|| {
                                palette.push(css);
                                palette.len() - 1
                            });
                            json!(idx)
                        }
                        _ => Value::Null,
                    })
                    .collect();
                json!({
                    "index": i,
                    "label": histogram_label(pr, i),
                    "values": values,
                    "palette": palette,
                    "color_idx": color_idx,
                })
            })
            .collect();
        out["histogram"] = json!(histogram);
    }

    // Labeled dots only — decorative (unlabeled) dots are dropped, matching
    // summary's signal/decorative split semantics.
    let mut signals: Vec<Value> = Vec::new();
    for d in pr.dots.iter().filter(|d| d.y.is_finite()) {
        if let Some(label) = &d.label {
            let t = if d.x < data.bars.len() {
                bar_t(&data.bars[d.x])
            } else {
                Value::Null
            };
            signals.push(json!({
                "x": d.x,
                "t": t,
                "y": d.y,
                "label": label,
                "color": css_rgba(&d.color),
            }));
        }
    }
    if !signals.is_empty() {
        out["signals"] = json!(signals);
    }

    if !pr.hlines.is_empty() {
        let hlines: Vec<Value> = pr
            .hlines
            .iter()
            .map(|h| json!({ "y": h.y, "color": css_rgba(&h.color) }))
            .collect();
        out["hlines"] = json!(hlines);
    }

    // Fills: shaded regions between two y-series (BBands, Keltner, Ichimoku
    // cloud, VWAP bands). y1/y2 are 1:1 aligned with bars[]; NaN → null.
    if !pr.fills.is_empty() {
        let fills: Vec<Value> = pr
            .fills
            .iter()
            .map(|f| {
                let y1: Vec<Value> = f.y1.iter().map(|v| finite_or_null(*v)).collect();
                let y2: Vec<Value> = f.y2.iter().map(|v| finite_or_null(*v)).collect();
                let mut fill = json!({ "y1": y1, "y2": y2, "color": css_rgba(&f.color) });
                if let Some(below) = &f.color_below {
                    fill["color_below"] = json!(css_rgba(below));
                }
                fill
            })
            .collect();
        out["fills"] = json!(fills);
    }

    // Horizontal bars (VPVR, Session VP, HVN/LVN, Naked POC, TPO). Axis-
    // relative numerics preserved from the struct; color is the renderer's
    // (ADR-0012).
    if !pr.hbars.is_empty() {
        let hbars: Vec<Value> = pr
            .hbars
            .iter()
            .map(|h| {
                json!({
                    "y": h.y,
                    "height": h.height,
                    "width": h.width,
                    "offset": h.offset,
                    "left": h.left,
                    "color": css_rgba(&h.color),
                })
            })
            .collect();
        out["hbars"] = json!(hbars);
    }

    // Divergence lines: two-point segments (WaveTrend, RSI divs in Cipher B).
    // Each endpoint carries both `x` (bar index) and `t` (unix seconds) to
    // mirror the signal pattern; color is the renderer's (ADR-0012).
    if !pr.divlines.is_empty() {
        let divlines: Vec<Value> = pr
            .divlines
            .iter()
            .map(|d| {
                let t1 = if d.x1 < data.bars.len() {
                    bar_t(&data.bars[d.x1])
                } else {
                    Value::Null
                };
                let t2 = if d.x2 < data.bars.len() {
                    bar_t(&data.bars[d.x2])
                } else {
                    Value::Null
                };
                json!({
                    "x1": d.x1,
                    "t1": t1,
                    "y1": d.y1,
                    "x2": d.x2,
                    "t2": t2,
                    "y2": d.y2,
                    "dashed": d.dashed,
                    "color": css_rgba(&d.color),
                })
            })
            .collect();
        out["divlines"] = json!(divlines);
    }

    out
}

// Pattern channels of a series indicator (ADR-0046), from the untrimmed result:
// `pr`'s indices are bars of `data`, of which the response starts at `offset`.
// `labels` belong to their bar and are cut like signals. `patterns`, `levels`
// and `events` are re-detected as a whole, so they stay complete on a tail: an
// index before the response's first bar is negative, and its `t` still says which
// bar it is. Absent when empty, except `events`, which name what an alert can wait for.
fn add_marks_to_series_json(
    out: &mut Value,
    pr: &chartgen::indicator::PanelResult,
    data: &chartgen::data::OhlcvData,
    offset: usize,
) {
    let t_of = |x: i64| {
        usize::try_from(x)
            .ok()
            .and_then(|i| data.bars.get(i))
            .map_or(Value::Null, bar_t)
    };
    let at = |x: i64| x - offset as i64;
    let labels: Vec<Value> = pr
        .labels
        .iter()
        .filter(|l| l.x >= offset)
        .map(|l| {
            let mut j = json!({
                "x": l.x - offset,
                "t": t_of(l.x as i64),
                "y": l.y,
                "above": l.above,
                "text": l.text,
                "key": l.key,
                "name": l.name,
                "bias": l.bias.as_str(),
                "color": css_rgba(&l.bias.color()),
                "tooltip": l.tooltip,
            });
            if l.tentative {
                j["tentative"] = json!(true);
            }
            j
        })
        .collect();
    if !labels.is_empty() {
        out["labels"] = json!(labels);
    }
    let patterns: Vec<Value> = pr
        .patterns
        .iter()
        .map(|p| {
            let strokes: Vec<Value> = p
                .strokes
                .iter()
                .map(|s| {
                    json!({
                        "x1": at(s.x1),
                        "t1": t_of(s.x1),
                        "y1": s.y1,
                        "x2": at(s.x2),
                        "t2": t_of(s.x2),
                        "y2": s.y2,
                        "dashed": s.dashed,
                    })
                })
                .collect();
            let mut j = json!({
                "key": p.key,
                "name": p.name,
                "bias": p.bias.as_str(),
                "status": p.status.as_str(),
                "color": css_rgba(&p.bias.color()),
                "strokes": strokes,
                "label": {
                    "x": at(p.label_x),
                    "t": t_of(p.label_x),
                    "y": p.label_y,
                    "above": p.label_above,
                },
                "tooltip": p.tooltip,
            });
            if let Some(v) = p.target {
                j["target"] = json!(v);
            }
            if let Some(v) = p.invalidation {
                j["invalidation"] = json!(v);
            }
            if let Some(x) = p.confirmed_x {
                j["confirmed_t"] = t_of(x);
            }
            j
        })
        .collect();
    if !patterns.is_empty() {
        out["patterns"] = json!(patterns);
    }
    if !pr.levels.is_empty() {
        let levels: Vec<Value> = pr
            .levels
            .iter()
            .map(|l| json!({ "line": l.line, "x": at(l.x), "t": t_of(l.x), "y": l.y }))
            .collect();
        out["levels"] = json!(levels);
    }
    if !pr.events.is_empty() {
        let events: Vec<Value> = pr
            .events
            .iter()
            .map(|e| json!({ "key": e.key, "name": e.name, "t": e.x.map_or(Value::Null, t_of) }))
            .collect();
        out["events"] = json!(events);
    }
}

// `future` block of a series indicator (ADR-0033): the values plotted on the
// `bars` bars after the last one. `t` are their open times
// (`data::future_bar_times`); absent when the bars have no time.
fn projection_to_json(
    p: &chartgen::indicator::Projection,
    data: &chartgen::data::OhlcvData,
) -> Value {
    let vals = |v: &[f64]| v.iter().map(|x| finite_or_null(*x)).collect::<Vec<_>>();
    let mut out = json!({
        "bars": p.bars,
        "lines": p.lines.iter().map(|(i, v)| json!({ "index": i, "values": vals(v) })).collect::<Vec<_>>(),
        "fills": p.fills.iter().map(|(i, y1, y2)| json!({ "index": i, "y1": vals(y1), "y2": vals(y2) })).collect::<Vec<_>>(),
    });
    if let Some(t) = data::future_bar_times(data, p.bars) {
        out["t"] = json!(t);
    }
    out
}

fn panel_result_to_json(pr: &chartgen::indicator::PanelResult) -> Value {
    let mut out = json!({ "label": pr.label, "is_overlay": pr.is_overlay });

    if let Some((lo, hi)) = pr.y_range {
        out["y_range"] = json!([finite_or_null(lo), finite_or_null(hi)]);
    }

    // Lines: the last finite value. Lines that break at trend flips read the
    // last bar instead: null where the line has none there (an inactive
    // Supertrend line), not a stale one from an earlier bar.
    if !pr.lines.is_empty() {
        let lines: Vec<Value> = pr
            .lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let last_val = finite_or_null(if line.breaks {
                    line.y.last().copied().unwrap_or(f64::NAN)
                } else {
                    line.y
                        .iter()
                        .rev()
                        .find(|v| v.is_finite())
                        .copied()
                        .unwrap_or(f64::NAN)
                });
                json!({
                    "index": i,
                    "label": line.label,
                    "last_value": last_val,
                })
            })
            .collect();
        out["lines"] = json!(lines);
    }

    // Dots: split labeled (semantic signals) from decorative
    if !pr.dots.is_empty() {
        let mut signals: Vec<Value> = Vec::new();
        let mut decorative = 0usize;
        for d in pr.dots.iter().filter(|d| d.y.is_finite()) {
            match &d.label {
                Some(label) => signals.push(json!({ "x": d.x, "y": d.y, "label": label })),
                None => decorative += 1,
            }
        }
        if !signals.is_empty() {
            out["signals"] = json!(signals);
        }
        if decorative > 0 {
            out["decorative_dots"] = json!(decorative);
        }
    }

    // Hlines: horizontal reference levels
    if !pr.hlines.is_empty() {
        let hlines: Vec<Value> = pr.hlines.iter().map(|h| json!({ "y": h.y })).collect();
        out["hlines"] = json!(hlines);
    }

    // Bars (histogram): last finite value
    if !pr.bars.is_empty() {
        let bars_info: Vec<Value> = pr
            .bars
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let last =
                    b.y.iter()
                        .rev()
                        .find(|v| v.is_finite())
                        .copied()
                        .map(|v| json!(v))
                        .unwrap_or(Value::Null);
                json!({ "index": i, "last_value": last })
            })
            .collect();
        out["histogram"] = json!(bars_info);
    }

    if !pr.fills.is_empty() {
        out["fills_count"] = json!(pr.fills.len());
    }

    // Horizontal bars (e.g. VPVR price-volume nodes): expose top levels by width
    if !pr.hbars.is_empty() {
        let mut hbars: Vec<&chartgen::indicator::HBar> = pr.hbars.iter().collect();
        hbars.sort_by(|a, b| {
            b.width
                .partial_cmp(&a.width)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let top: Vec<Value> = hbars
            .iter()
            .take(5)
            .map(|h| json!({ "y": h.y, "weight": h.width }))
            .collect();
        out["hbars_top"] = json!(top);
        out["hbars_count"] = json!(pr.hbars.len());
    }

    // Patterns (ADR-0046): the newest labels and every chart pattern, by bar index.
    if !pr.labels.is_empty() {
        let labels: Vec<Value> = pr
            .labels
            .iter()
            .rev()
            .take(MAX_SUMMARY_LABELS)
            .rev()
            .map(|l| {
                let mut j =
                    json!({ "x": l.x, "key": l.key, "name": l.name, "bias": l.bias.as_str() });
                if l.tentative {
                    j["tentative"] = json!(true);
                }
                j
            })
            .collect();
        out["labels"] = json!(labels);
        out["labels_count"] = json!(pr.labels.len());
    }
    if !pr.patterns.is_empty() {
        let patterns: Vec<Value> = pr
            .patterns
            .iter()
            .map(|p| {
                let mut j = json!({
                    "key": p.key,
                    "name": p.name,
                    "bias": p.bias.as_str(),
                    "status": p.status.as_str(),
                    "tooltip": p.tooltip,
                });
                if let Some(v) = p.target {
                    j["target"] = json!(v);
                }
                if let Some(v) = p.invalidation {
                    j["invalidation"] = json!(v);
                }
                if let Some(x) = p.confirmed_x {
                    j["confirmed_x"] = json!(x);
                }
                j
            })
            .collect();
        out["patterns"] = json!(patterns);
    }
    if !pr.levels.is_empty() {
        let levels: Vec<Value> = pr
            .levels
            .iter()
            .map(|l| json!({ "line": l.line, "x": l.x, "y": l.y }))
            .collect();
        out["levels"] = json!(levels);
    }
    if !pr.events.is_empty() {
        let events: Vec<Value> = pr
            .events
            .iter()
            .map(|e| json!({ "key": e.key, "name": e.name, "x": e.x }))
            .collect();
        out["events"] = json!(events);
    }

    // Divergence lines (momentum vs price divergence — important signals)
    if !pr.divlines.is_empty() {
        let divs: Vec<Value> = pr
            .divlines
            .iter()
            .map(|d| {
                json!({
                    "x1": d.x1, "y1": d.y1,
                    "x2": d.x2, "y2": d.y2,
                    "dashed": d.dashed,
                })
            })
            .collect();
        out["divergences"] = json!(divs);
    }

    out
}

// ---------------------------------------------------------------------------
// OHLCV summary — compact view of the price series for LLM consumption
// ---------------------------------------------------------------------------

fn ohlcv_summary(data: &chartgen::data::OhlcvData) -> Value {
    if data.bars.is_empty() {
        return json!({ "bars": 0 });
    }
    let first = &data.bars[0];
    let last = data.bars.last().unwrap();
    let (mut hi, mut lo) = (f64::NEG_INFINITY, f64::INFINITY);
    let mut vol = 0.0f64;
    for b in &data.bars {
        if b.high > hi {
            hi = b.high;
        }
        if b.low < lo {
            lo = b.low;
        }
        vol += b.volume;
    }
    let change_pct = if first.close != 0.0 {
        Some((last.close - first.close) / first.close * 100.0)
    } else {
        None
    };
    json!({
        "bars": data.bars.len(),
        "first": { "date": first.date, "open": first.open, "close": first.close },
        "last": {
            "date": last.date,
            "open": last.open, "high": last.high, "low": last.low,
            "close": last.close, "volume": last.volume,
        },
        "range": { "high": hi, "low": lo },
        "total_volume": vol,
        "change_pct": change_pct,
    })
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
    let interval = match check_interval(interval) {
        Ok(canonical) => canonical,
        Err(e) => return tool_err(id, &e),
    };
    let interval = interval.as_str();

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
    let fetch_result =
        fetch::fetch_ohlcv(fetch::DataSource::from_env(), symbol, interval, bars, None)
            .map(|f| f.data);
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

// get_footprint and get_tpo (ADR-0048)
// ---------------------------------------------------------------------------

/// Most bars of one `get_footprint`: a screen of the widest cells, and the padding.
const MAX_FOOTPRINT_BARS: usize = 200;
const DEFAULT_FOOTPRINT_BARS: usize = 60;
/// Blocks of one `get_tpo`: 3 Binance requests at most.
const MAX_TPO_BLOCKS: usize = 3_000;
const MAX_TPO_PERIODS: usize = 60;

/// An optional number between `min` and `max`, else `default`. Anything else fails.
fn number_arg(args: &Value, name: &str, default: f64, min: f64, max: f64) -> Result<f64, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_f64()
            .filter(|n| (min..=max).contains(n))
            .ok_or_else(|| format!("{name} must be a number from {min} to {max}.")),
    }
}

/// `end_time` as `generate_chart` takes it.
fn end_time_arg(args: &Value) -> Result<Option<i64>, String> {
    match args.get("end_time") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let max = fetch::now_ms().saturating_add(MAX_END_TIME_AHEAD_MS);
            v.as_i64()
                .filter(|t| (MIN_END_TIME_MS..=max).contains(t))
                .map(Some)
                .ok_or_else(|| "end_time must be an integer, ms since epoch.".to_string())
        }
    }
}

/// A payload as compact JSON: these run to thousands of numbers.
fn tool_ok_compact(id: Option<Value>, payload: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "content": [{ "type": "text", "text": payload.to_string() }] }
    })
}

/// Row size from `row_size` or `row_ticks`, or `auto`. Whether it was automatic.
fn row_arg(args: &Value, tick: f64, auto: impl FnOnce() -> f64) -> Result<(f64, bool), String> {
    let size = number_arg(args, "row_size", 0.0, 0.0, f64::MAX)?;
    if size > 0.0 {
        return Ok((size, false));
    }
    let ticks = number_arg(args, "row_ticks", 0.0, 0.0, 1_000_000.0)?;
    if ticks > 0.0 {
        return Ok((ticks.round().max(1.0) * tick, false));
    }
    Ok((auto(), true))
}

fn tool_get_footprint(id: Option<Value>, args: &Value) -> Value {
    match footprint_payload(args, fetch::DataSource::from_env()) {
        Ok(payload) => tool_ok_compact(id, &payload),
        Err(e) => tool_err(id, &e),
    }
}

fn footprint_payload(args: &Value, source: fetch::DataSource) -> Result<Value, String> {
    use chartgen::orderflow as of;
    let symbol = args
        .get("ticker")
        .and_then(|v| v.as_str())
        .ok_or("ticker is required.")?;
    let interval = check_interval(
        args.get("timeframe")
            .and_then(|v| v.as_str())
            .unwrap_or("1h"),
    )?;
    let iv = chartgen::interval::Interval::parse(&interval)?;
    let bars = number_arg(
        args,
        "bars",
        DEFAULT_FOOTPRINT_BARS as f64,
        1.0,
        MAX_FOOTPRINT_BARS as f64,
    )? as usize;
    let atr_length = number_arg(args, "atr_length", 14.0, 1.0, 500.0)? as usize;
    let end_time = end_time_arg(args)?;
    let value_area = number_arg(args, "value_area", 70.0, 1.0, 100.0)?;
    let imbalance = number_arg(args, "imbalance", 300.0, 100.0, 100_000.0)?;
    let stacked = number_arg(args, "stacked", 3.0, 1.0, 100.0)? as usize;

    let fetched = fetch::fetch_ohlcv(source, symbol, &interval, bars + atr_length + 1, end_time)
        .map_err(|e| format!("Data fetch failed: {e}"))?;
    let all = &fetched.data.bars;
    let offset = all.len().saturating_sub(bars);
    let visible = &all[offset..];
    let opens: Vec<i64> = visible.iter().filter_map(|b| b.date.parse().ok()).collect();
    let (Some(&first), Some(&last)) = (opens.first(), opens.last()) else {
        return Ok(
            json!({ "symbol": symbol, "interval": interval, "footprints": [], "has_more": false }),
        );
    };
    let last_end = iv.grid_end(last);

    let age = fetch::now_ms() / 1000 - first;
    // As for a load, whatever is asked now: a daily chart's few bars must not fetch days
    // of 1m bars its load could not afford. A poll says outright what its load read.
    let plan_bars = visible.len().max(DEFAULT_FOOTPRINT_BARS);
    let exchange = fetch::detect_source(symbol);
    let plan = match args.get("intrabar_interval").and_then(Value::as_str) {
        Some(load) => intrabars::plan_like_load(exchange, iv.secs(), plan_bars, age, load)?,
        None => intrabars::plan(exchange, iv.secs(), plan_bars, age),
    };
    let list = match plan {
        Some((name, secs)) => intrabars::load(source, symbol, name, secs, visible, last_end)
            .map_err(|e| format!("Intrabar fetch failed: {e}"))?,
        // Nothing finer: the bar is its own only intrabar.
        None => visible
            .iter()
            .filter_map(|b| of::Intrabar::from_bar(b, None))
            .collect(),
    };

    let prices: Vec<f64> = visible
        .iter()
        .flat_map(|b| [b.high, b.low])
        .chain(list.iter().flat_map(|b| [b.high, b.low]))
        .collect();
    let tick = of::infer_tick(&prices);
    let (asked, auto) = row_arg(args, tick, || {
        of::atr(all, atr_length).map_or_else(
            || {
                let range = visible.iter().map(|b| b.high - b.low).fold(0.0, f64::max);
                of::nice_row_size(range / 20.0, tick)
            },
            |atr| of::auto_row_size(atr, tick),
        )
    })?;
    let ranges = of::group_by_bar(&opens, last_end, &list);
    let row = of::footprint_row(asked, visible, &list, &ranges, tick);

    let options = of::FootprintOptions {
        row,
        value_area,
        imbalance,
        stacked,
    };
    let without_data = ranges.iter().filter(|r| r.is_empty()).count();
    let footprints: Vec<Value> = ranges
        .into_iter()
        .zip(&opens)
        .enumerate()
        .map(|(k, (range, &t))| {
            let prev = all.get((offset + k).wrapping_sub(1)).map(|b| b.close);
            of::build_footprint(t, &list[range], prev, &options).to_json()
        })
        .collect();
    let side = if list.iter().any(|b| b.taker_buy.is_some()) {
        of::Side::TakerVolume
    } else {
        of::Side::CloseVersusOpen
    };
    Ok(json!({
        "symbol": symbol,
        "interval": interval,
        "intrabar_interval": plan.map(|p| p.0),
        "side": side.name(),
        "row_size": row,
        "row_auto": auto,
        "row_widened": row != asked,
        "tick": tick,
        "bars_without_data": without_data,
        "footprints": footprints,
        "has_more": fetched.has_more || offset > 0,
    }))
}

fn tool_get_tpo(id: Option<Value>, args: &Value) -> Value {
    match tpo_payload(args, fetch::DataSource::from_env()) {
        Ok(payload) => tool_ok_compact(id, &payload),
        Err(e) => tool_err(id, &e),
    }
}

fn tpo_payload(args: &Value, source: fetch::DataSource) -> Result<Value, String> {
    use chartgen::orderflow as of;
    let symbol = args
        .get("ticker")
        .and_then(|v| v.as_str())
        .ok_or("ticker is required.")?;
    let period = of::Period::parse(args.get("period").and_then(|v| v.as_str()).unwrap_or("1D"))?;
    let asked_block = check_interval(args.get("block").and_then(|v| v.as_str()).unwrap_or("30m"))?;
    let asked_secs = chartgen::interval::Interval::parse(&asked_block)?.secs();
    if !(300..=14_400).contains(&asked_secs) {
        return Err("block must be from 5m to 4h.".into());
    }
    let (block, block_secs) =
        of::block_for_period((&asked_block, asked_secs), period, MAX_TPO_BLOCKS as i64)
            .ok_or("period is too long: even 4h blocks cannot cover a whole one.")?;
    let periods = number_arg(args, "periods", 10.0, 1.0, MAX_TPO_PERIODS as f64)? as usize;
    let value_area = number_arg(args, "value_area", 70.0, 1.0, 100.0)?;
    let ib_blocks = number_arg(args, "ib_blocks", 2.0, 1.0, 100.0)? as usize;
    let end_time = end_time_arg(args)?;
    let sessions: Vec<of::DaySession> = match args.get("sessions") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(list)) if list.len() <= 5 => list
            .iter()
            .map(|v| of::DaySession::parse(v.as_str().unwrap_or("")))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("sessions must be a list of at most 5 HH:MM-HH:MM.".into()),
    };

    let per_period = (period.max_secs() / block_secs).max(1) as usize;
    let wanted = periods.saturating_mul(per_period);
    let limit = wanted.min(MAX_TPO_BLOCKS);
    let fetched = intrabars::tpo_blocks(source, symbol, &block, block_secs, limit, end_time)
        .map_err(|e| format!("Data fetch failed: {e}"))?;
    let blocks: Vec<of::Intrabar> = fetched
        .data
        .bars
        .iter()
        .filter_map(|b| of::Intrabar::from_bar(b, None))
        .collect();
    let tz = fetched.data.session.map_or(chrono_tz::UTC, |s| s.tz);
    let prices: Vec<f64> = blocks.iter().flat_map(|b| [b.high, b.low]).collect();
    let tick = if prices.is_empty() {
        0.01
    } else {
        of::infer_tick(&prices)
    };
    let (asked, auto) = row_arg(args, tick, || {
        of::tpo_auto_row(&blocks, period, &sessions, &tz, tick)
    })?;
    let widest = of::tpo_widest_range(&blocks, period, &sessions, &tz);
    let top = prices.iter().copied().fold(0.0, f64::max);
    let row = of::usable_row(asked, widest, top, tick);
    let options = of::TpoOptions {
        block_secs,
        row,
        value_area,
        ib_blocks,
    };
    let mut profiles = of::tpo_profiles(&blocks, period, &sessions, &tz, &options);
    // The oldest period may be cut off by the block budget: only a full read keeps it.
    let cut = wanted > limit;
    if cut && profiles.len() > 1 {
        profiles.remove(0);
    }
    let keep = periods.saturating_mul(sessions.len().max(1));
    let excess = profiles.len().saturating_sub(keep);
    profiles.drain(..excess);
    Ok(json!({
        "symbol": symbol,
        "period": args.get("period").and_then(|v| v.as_str()).unwrap_or("1D"),
        "block_widened": block != asked_block,
        "block": block,
        "row_size": row,
        "row_auto": auto,
        "row_widened": row != asked,
        "tick": tick,
        "timezone": tz.to_string(),
        "periods_cut": cut,
        "profiles": profiles.iter().map(of::TpoProfile::to_json).collect::<Vec<_>>(),
        "has_more": fetched.has_more,
    }))
}

// ---------------------------------------------------------------------------
// get_quotes (watchlist, ADR-0011)
// ---------------------------------------------------------------------------

/// Most symbols one `get_quotes` call accepts (the schema's `maxItems`).
const MAX_QUOTE_SYMBOLS: usize = 50;

/// Trim + uppercase a ticker; `None` unless it is 1–32 characters of
/// `A-Z 0-9 . ^ = - _` (the web client's `normalizeSymbol` rules) with at
/// least one letter or digit, so it can never be a `.`/`..` path segment.
pub(crate) fn normalize_quote_symbol(raw: &str) -> Option<String> {
    let s = raw.trim().to_uppercase();
    let ok = (1..=32).contains(&s.len())
        && s.chars().any(|c| c.is_ascii_alphanumeric())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".^=-_".contains(c));
    ok.then_some(s)
}

fn tool_get_quotes(id: Option<Value>, args: &Value) -> Value {
    get_quotes(id, args, fetch::DataSource::from_env())
}

fn get_quotes(id: Option<Value>, args: &Value, source: fetch::DataSource) -> Value {
    let Some(raw) = args.get("symbols").and_then(|v| v.as_array()) else {
        return tool_err(id, "symbols is required and must be an array of strings");
    };
    if raw.len() > MAX_QUOTE_SYMBOLS {
        return tool_err(id, &format!("At most {MAX_QUOTE_SYMBOLS} symbols per call"));
    }
    let mut seen = HashSet::new();
    let mut valid = Vec::new();
    // Requested order after dedupe; `Err` holds the rejected input.
    let mut order: Vec<Result<String, String>> = Vec::new();
    for v in raw {
        let Some(text) = v.as_str() else {
            return tool_err(id, "symbols must be an array of strings");
        };
        match normalize_quote_symbol(text) {
            Some(s) if seen.insert(s.clone()) => {
                valid.push(s.clone());
                order.push(Ok(s));
            }
            Some(_) => {}
            None => order.push(Err(text.to_string())),
        }
    }

    // One result per valid symbol, in the same order as the `Ok` entries.
    let mut fetched = fetch::fetch_quotes(source, &valid).into_iter();
    let quotes: Vec<Value> = order
        .into_iter()
        .map(|entry| match entry {
            Err(text) => json!({ "symbol": text, "error": "invalid symbol" }),
            Ok(sym) => match fetched.next() {
                Some(Ok(q)) => {
                    let change = q.last - q.prev_close;
                    let change_pct = change / q.prev_close * 100.0;
                    json!({
                        "symbol": sym,
                        "last": finite_or_null(q.last),
                        "change": finite_or_null(change),
                        "change_pct": finite_or_null(change_pct),
                        "prev_close": finite_or_null(q.prev_close),
                        "high": q.high.map_or(Value::Null, finite_or_null),
                        "low": q.low.map_or(Value::Null, finite_or_null),
                        "volume": q.volume.map_or(Value::Null, finite_or_null),
                        "source": q.source,
                    })
                }
                Some(Err(e)) => json!({ "symbol": sym, "error": e }),
                None => json!({ "symbol": sym, "error": "quote missing" }),
            },
        })
        .collect();
    tool_ok(id, json!({ "quotes": quotes }))
}

// ---------------------------------------------------------------------------
// search_symbols (symbol search dialog)
// ---------------------------------------------------------------------------

fn tool_search_symbols(id: Option<Value>, args: &Value) -> Value {
    search_symbols(id, args, fetch::DataSource::from_env())
}

fn search_symbols(id: Option<Value>, args: &Value, source: fetch::DataSource) -> Value {
    let Some(query) = args.get("query").and_then(|v| v.as_str()) else {
        return tool_err(id, "query is required and must be a string");
    };
    let limit = match args.get("limit") {
        None | Some(Value::Null) => search::DEFAULT_LIMIT,
        Some(v) => match v.as_u64() {
            Some(n) if (1..=search::MAX_LIMIT as u64).contains(&n) => n as usize,
            _ => {
                return tool_err(
                    id,
                    &format!("limit must be an integer 1-{}", search::MAX_LIMIT),
                )
            }
        },
    };
    let types: Vec<String> = match args.get("types") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => {
            let strings: Option<Vec<String>> = v
                .as_array()
                .and_then(|a| a.iter().map(|t| t.as_str().map(str::to_string)).collect());
            match strings {
                Some(types) => types,
                None => return tool_err(id, "types must be an array of strings"),
            }
        }
    };
    match search::search_symbols(source, query, limit, &types) {
        Ok(out) => {
            let results: Vec<Value> = out.results.iter().map(|r| r.to_json()).collect();
            let mut payload = json!({ "results": results });
            if !out.warnings.is_empty() {
                payload["warnings"] = json!(out.warnings);
            }
            tool_ok(id, payload)
        }
        Err(e) => tool_err(id, &e),
    }
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
        .filter(|s| !s.is_empty());

    let alert_types: Option<HashSet<String>> = args
        .get("alert_types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<HashSet<_>>()
        })
        .filter(|s| !s.is_empty());

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
    fn get_indicators_takes_any_spelling_of_an_interval() {
        // Under --trade -i 1h, 60m is the same chart and reads the engine's state.
        let dir = temp_dir("get_ind_spelling");
        let engine = make_engine(dir.clone());
        {
            let mut e = engine.write().unwrap();
            for i in 0..20 {
                e.on_bar(make_bar(100.0 + i as f64));
            }
        }
        let args = json!({ "symbol": "BTCUSDT", "interval": "60m", "indicators": ["rsi"] });
        let resp = tool_get_indicators(Some(json!(1)), &args, Some(&engine));
        let parsed: Value = serde_json::from_str(&response_text(&resp)).unwrap();
        assert_eq!(parsed["source"], "engine_cache");
        assert_eq!(parsed["interval"], "1h");
        std::fs::remove_dir_all(&dir).ok();

        // What it cannot serve is the same tool error as for generate_chart.
        let args = json!({ "symbol": "BTCUSDT", "interval": "61m", "indicators": ["rsi"] });
        let resp = tool_get_indicators(Some(json!(1)), &args, None);
        assert!(is_error(&resp));
        assert!(response_text(&resp).contains("Unsupported timeframe"));
    }

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
        let resp = handle_mcp_request("tools/list", Some(json!(1)), None, None, None, None, None);
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

    // --- generate_chart format param ---

    fn chart_content(resp: &Value) -> &[Value] {
        resp["result"]["content"].as_array().unwrap().as_slice()
    }

    #[test]
    fn generate_chart_summary_returns_json_only() {
        // No symbol → uses sample data, so no network.
        let args = json!({
            "bars": 60,
            "indicators": ["rsi", "macd"],
            "format": "summary"
        });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        assert!(!is_error(&resp));

        let content = chart_content(&resp);
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");

        let text = content[0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["price"]["bars"], 60);
        assert!(parsed["price"]["last"]["close"].is_number());
        assert!(parsed["indicators"]["rsi"]["lines"].is_array());
        assert!(parsed["indicators"]["macd"].is_object());
    }

    #[test]
    fn generate_chart_summary_exposes_hlines_and_y_range() {
        // RSI has hlines at 30/70 and y_range (0, 100).
        let args = json!({
            "bars": 60,
            "indicators": ["rsi"],
            "format": "summary"
        });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        let content = chart_content(&resp);
        let text = content[0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        let rsi = &parsed["indicators"]["rsi"];
        let hlines = rsi["hlines"].as_array().unwrap();
        let ys: Vec<f64> = hlines.iter().map(|h| h["y"].as_f64().unwrap()).collect();
        assert!(ys.iter().any(|y| (*y - 30.0).abs() < 0.01));
        assert!(ys.iter().any(|y| (*y - 70.0).abs() < 0.01));

        let y_range = rsi["y_range"].as_array().unwrap();
        assert_eq!(y_range[0].as_f64().unwrap(), 0.0);
        assert_eq!(y_range[1].as_f64().unwrap(), 100.0);
    }

    #[test]
    fn generate_chart_both_returns_png_and_summary() {
        let args = json!({
            "bars": 60,
            "indicators": ["rsi"],
            "format": "both",
            "width": 400,
            "height": 300
        });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        assert!(!is_error(&resp));

        let content = chart_content(&resp);
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content[0]["mimeType"], "image/png");
        assert_eq!(content[1]["type"], "text");

        let text = content[1]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert!(parsed["indicators"]["rsi"].is_object());
    }

    #[test]
    fn generate_chart_unknown_format_errors() {
        let args = json!({ "bars": 20, "format": "svg" });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        assert!(is_error(&resp));
        assert!(response_text(&resp).contains("Unknown format"));
    }

    #[test]
    fn generate_chart_png_default_unchanged() {
        // Default (no format param) returns a single image content item.
        let args = json!({
            "bars": 60,
            "indicators": ["rsi"],
            "width": 400,
            "height": 300
        });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        assert!(!is_error(&resp));
        let content = chart_content(&resp);
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "image");
    }

    #[test]
    fn css_rgba_formats_color() {
        assert_eq!(
            css_rgba(&plotters::style::RGBAColor(1, 2, 3, 0.5)),
            "rgba(1,2,3,0.5)"
        );
        assert_eq!(
            css_rgba(&plotters::style::RGBAColor(255, 0, 0, 1.0)),
            "rgba(255,0,0,1)"
        );
    }

    #[test]
    fn series_json_carries_geometry_colors() {
        use chartgen::indicator::{DivLine, Fill, HBar, HLine, PanelResult};
        use plotters::style::RGBAColor;

        let c = RGBAColor(10, 20, 30, 0.25);
        let pr = PanelResult {
            lines: vec![],
            fills: vec![Fill {
                y1: vec![0.0],
                y2: vec![1.0],
                color: c,
                color_below: None,
            }],
            bars: vec![],
            dots: vec![],
            hlines: vec![HLine { y: 30.0, color: c }],
            hbars: vec![HBar {
                y: 1.0,
                height: 1.0,
                width: 0.5,
                offset: 0.0,
                color: c,
                left: false,
            }],
            divlines: vec![DivLine {
                x1: 0,
                y1: 1.0,
                x2: 1,
                y2: 2.0,
                color: c,
                dashed: true,
            }],
            y_range: None,
            label: "T".into(),
            is_overlay: false,
            ..Default::default()
        };
        let data = chartgen::data::OhlcvData {
            bars: vec![],
            symbol: None,
            interval: None,
            session: None,
        };
        let j = panel_result_to_series_json(&pr, &data);
        let want = "rgba(10,20,30,0.25)";
        assert_eq!(j["fills"][0]["color"], want);
        assert_eq!(j["hbars"][0]["color"], want);
        assert_eq!(j["divlines"][0]["color"], want);
        assert_eq!(j["divlines"][0]["dashed"], true);
        assert_eq!(j["hlines"][0]["color"], want);
        assert_eq!(j["hlines"][0]["y"], 30.0);
    }

    #[test]
    fn series_json_serializes_fill_color_below_only_when_set() {
        use chartgen::indicator::{Fill, PanelResult};
        use plotters::style::RGBAColor;

        let mk = |below| Fill {
            y1: vec![1.0],
            y2: vec![0.0],
            color: RGBAColor(76, 175, 80, 0.2),
            color_below: below,
        };
        let pr = PanelResult {
            fills: vec![mk(Some(RGBAColor(239, 83, 80, 0.2))), mk(None)],
            label: "T".into(),
            is_overlay: true,
            ..Default::default()
        };
        let data = chartgen::data::OhlcvData {
            bars: vec![],
            symbol: None,
            interval: None,
            session: None,
        };
        let j = panel_result_to_series_json(&pr, &data);
        assert_eq!(j["fills"][0]["color"], "rgba(76,175,80,0.2)");
        assert_eq!(j["fills"][0]["color_below"], "rgba(239,83,80,0.2)");
        assert!(j["fills"][1].get("color_below").is_none());
    }

    #[test]
    fn panel_result_to_json_covers_all_fields() {
        use chartgen::indicator::{DivLine, Dot, Fill, HBar, HLine, Line, PanelResult};
        use plotters::style::RGBAColor;

        let c = RGBAColor(255, 0, 0, 1.0);
        let pr = PanelResult {
            lines: vec![Line {
                breaks: false,
                y: vec![f64::NAN, 1.0, 2.0, 3.5],
                color: c,
                width: 1,
                label: Some("main".into()),
            }],
            fills: vec![Fill {
                y1: vec![0.0],
                y2: vec![1.0],
                color: c,
                color_below: None,
            }],
            bars: vec![],
            dots: vec![
                Dot {
                    x: 5,
                    y: 42.0,
                    color: c,
                    size: 4,
                    label: Some("buy".into()),
                },
                Dot {
                    x: 6,
                    y: 43.0,
                    color: c,
                    size: 4,
                    label: None,
                },
            ],
            hlines: vec![HLine { y: 30.0, color: c }, HLine { y: 70.0, color: c }],
            hbars: vec![HBar {
                y: 100.0,
                height: 1.0,
                width: 0.8,
                offset: 0.0,
                color: c,
                left: true,
            }],
            divlines: vec![DivLine {
                x1: 1,
                y1: 10.0,
                x2: 5,
                y2: 20.0,
                color: c,
                dashed: true,
            }],
            y_range: Some((0.0, 100.0)),
            label: "Test".into(),
            is_overlay: false,
            ..Default::default()
        };

        let j = panel_result_to_json(&pr);
        assert_eq!(j["label"], "Test");
        assert_eq!(j["lines"][0]["last_value"], 3.5);
        assert_eq!(j["y_range"][0], 0.0);
        assert_eq!(j["y_range"][1], 100.0);
        assert_eq!(j["signals"][0]["label"], "buy");
        assert_eq!(j["decorative_dots"], 1);
        assert_eq!(j["hlines"].as_array().unwrap().len(), 2);
        assert_eq!(j["fills_count"], 1);
        assert_eq!(j["hbars_count"], 1);
        assert_eq!(j["divergences"][0]["dashed"], true);
    }

    // --- series format ---

    /// Bars of a double top on unix-second dates: 40 of them reach its breakout.
    fn double_top_data(bars: usize) -> chartgen::data::OhlcvData {
        let knots: [(usize, f64); 7] = [
            (0, 100.0),
            (10, 90.0),
            (20, 110.0),
            (30, 100.0),
            (40, 110.0),
            (70, 80.0),
            (100, 80.0),
        ];
        let mut out = vec![];
        for w in knots.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            for x in x0..x1 {
                let c = y0 + (y1 - y0) * (x - x0) as f64 / (x1 - x0) as f64;
                out.push(chartgen::data::Bar {
                    open: c,
                    high: c + 0.1,
                    low: c - 0.1,
                    close: c,
                    volume: 1.0,
                    date: (1_000_000 + x as i64 * 3600).to_string(),
                });
            }
        }
        out.truncate(bars);
        chartgen::data::OhlcvData {
            bars: out,
            symbol: Some("X".into()),
            interval: Some("1h".into()),
            session: None,
        }
    }

    fn series_of(
        data: &chartgen::data::OhlcvData,
        name: &str,
        params: Value,
        bars: usize,
    ) -> Value {
        let ind = indicators::by_name_configured(name, &params)
            .unwrap()
            .unwrap();
        let pr = ind.compute(data);
        let j = series_json(
            data,
            &[("k".into(), pr)],
            &[ind.as_ref()],
            bars,
            "candles",
            false,
        );
        j["indicators"]["k"].clone()
    }

    #[test]
    fn series_keeps_whole_patterns_and_levels_on_a_tail() {
        let data = double_top_data(90);
        let params = json!({"depth": 3, "pattern": "double_top"});
        let full = series_of(&data, "chart_patterns", params.clone(), 90);
        let tail = series_of(&data, "chart_patterns", params, 2);
        let (f, t) = (&full["patterns"][0], &tail["patterns"][0]);
        assert_eq!(f["key"], "double_top");
        assert_eq!(f["status"], "confirmed");
        assert_eq!(f["bias"], "bearish");
        // The tail's indices are from its own first bar, 88 on, so negative.
        let (fx, tx) = (
            f["strokes"][0]["x1"].as_i64().unwrap(),
            t["strokes"][0]["x1"].as_i64().unwrap(),
        );
        assert_eq!(tx, fx - 88);
        assert!(tx < 0);
        // The same bar by time, whichever window it came in.
        assert_eq!(f["strokes"][0]["t1"], t["strokes"][0]["t1"]);
        assert_eq!(f["strokes"][0]["t1"], json!(1_000_000 + fx * 3600));
        assert_eq!(f["confirmed_t"], t["confirmed_t"]);
        // Events carry the bar's time, and only bearish happened.
        let bear = &tail["events"][1];
        assert_eq!(bear["key"], "bearish");
        assert_eq!(bear["t"], f["confirmed_t"]);
        assert_eq!(tail["events"][0]["t"], Value::Null);

        let params = json!({"depth": 3});
        let fib = series_of(&data, "auto_fib", params.clone(), 90);
        let tail = series_of(&data, "auto_fib", params, 2);
        let (a, b) = (&fib["levels"][4], &tail["levels"][4]);
        assert_eq!(a["line"], 4);
        assert_eq!(a["y"], b["y"]);
        assert_eq!(a["t"], b["t"]);
        assert_eq!(b["x"].as_i64().unwrap(), a["x"].as_i64().unwrap() - 88);
        // The plot's own values are cut like any other's.
        assert_eq!(tail["lines"][4]["values"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn series_cuts_candle_labels_like_signals() {
        let data = double_top_data(90);
        let params = json!({"pattern": "all", "trend": "none"});
        let full = series_of(&data, "candlestick_patterns", params.clone(), 90);
        let all = full["labels"].as_array().expect("labels").clone();
        let last_x = all.last().unwrap()["x"].as_u64().unwrap() as usize;
        let cut = 90 - last_x;
        let tail = series_of(&data, "candlestick_patterns", params, cut);
        let got = tail["labels"].as_array().expect("labels");
        assert!(got.iter().all(|l| l["x"].as_u64().unwrap() < cut as u64));
        assert_eq!(got.last().unwrap()["t"], all.last().unwrap()["t"]);
        assert_eq!(
            got.last().unwrap()["tooltip"],
            all.last().unwrap()["tooltip"]
        );
    }

    #[test]
    fn summary_reports_patterns_labels_and_events() {
        let data = double_top_data(90);
        let ind = indicators::by_name_configured("chart_patterns", &json!({"depth": 3}))
            .unwrap()
            .unwrap();
        let j = panel_result_to_json(&ind.compute(&data));
        assert_eq!(j["patterns"][0]["status"], "confirmed");
        assert!(j["patterns"][0]["tooltip"]
            .as_str()
            .unwrap()
            .contains("Double Top"));
        assert_eq!(j["events"][1]["key"], "bearish");
        // More labels than a summary lists: it keeps the newest, and counts them all.
        let ind = indicators::by_name_configured(
            "candlestick_patterns",
            &json!({"pattern": "long_legged_doji", "trend": "none"}),
        )
        .unwrap()
        .unwrap();
        let mut data = double_top_data(90);
        data.interval = None;
        for bar in &mut data.bars {
            bar.open = bar.close - 0.01;
            bar.high = bar.close + 1.0;
            bar.low = bar.close - 1.0;
        }
        let pr = ind.compute(&data);
        assert!(pr.labels.len() > MAX_SUMMARY_LABELS, "{}", pr.labels.len());
        let j = panel_result_to_json(&pr);
        assert_eq!(j["labels_count"], pr.labels.len());
        let listed = j["labels"].as_array().unwrap();
        assert_eq!(listed.len(), MAX_SUMMARY_LABELS);
        assert_eq!(listed.last().unwrap()["x"], pr.labels.last().unwrap().x);
        assert_eq!(listed[0]["key"], "long_legged_doji");
    }

    #[test]
    fn bar_t_parses_unix_seconds() {
        use chartgen::data::Bar;
        let numeric = Bar {
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
            date: "1713420000".into(),
        };
        assert_eq!(bar_t(&numeric), json!(1713420000u64));

        let synthetic = Bar {
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
            date: "D5".into(),
        };
        assert_eq!(bar_t(&synthetic), Value::Null);

        let pre_epoch = Bar {
            date: "-631152000".into(), // 1950-01-01
            ..numeric
        };
        assert_eq!(bar_t(&pre_epoch), json!(-631_152_000_i64));
    }

    // ---- order flow (ADR-0048) ----

    use fetch::DataSource::Sample;

    fn flow_series(args: Value) -> Value {
        let resp = generate_chart(Some(json!(1)), &args, Sample, None);
        assert!(!is_error(&resp), "{resp}");
        serde_json::from_str(chart_content(&resp)[0]["text"].as_str().unwrap()).unwrap()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-3 * a.abs().max(1.0)
    }

    #[test]
    fn a_footprint_is_one_item_per_bar_with_its_rows_adding_up_to_its_totals() {
        // Not to the chart bar's volume: sample intrabars are cut from it, so that would
        // hold by construction here, and a real source's minutes need not sum to its hour.
        let args = json!({ "ticker": "BTCUSDT", "timeframe": "1h", "bars": 5, "end_time": 1_699_920_000_000i64 });
        let fp = footprint_payload(&args, Sample).unwrap();
        assert_eq!(fp["intrabar_interval"], "1m");
        assert_eq!(fp["side"], "taker_volume");
        assert_eq!(fp["row_auto"], true);
        let list = fp["footprints"].as_array().unwrap();
        assert_eq!(list.len(), 5);
        let chart = flow_series(json!({ "ticker": "BTCUSDT", "timeframe": "1h", "bars": 5,
            "end_time": 1_699_920_000_000i64, "format": "series", "indicators": [] }));
        for (f, bar) in list.iter().zip(chart["bars"].as_array().unwrap()) {
            assert_eq!(f["t"], bar["t"]);
            let total = f["buy"].as_f64().unwrap() + f["sell"].as_f64().unwrap();
            let rows = f["rows"].as_array().unwrap();
            let in_rows: f64 = rows
                .iter()
                .map(|r| r[1].as_f64().unwrap() + r[2].as_f64().unwrap())
                .sum();
            assert!(close(in_rows, total), "{in_rows} {total}");
            // POC and value area are row prices, and the POC is inside it.
            let (val, poc, vah) = (
                f["val"].as_f64().unwrap(),
                f["poc"].as_f64().unwrap(),
                f["vah"].as_f64().unwrap(),
            );
            assert!(val <= poc && poc < vah, "{val} {poc} {vah}");
        }
    }

    #[test]
    fn a_tail_reads_the_same_interval_as_its_load() {
        let interval = |bars: u64| {
            let args = json!({ "ticker": "BTCUSDT", "timeframe": "1d", "bars": bars });
            footprint_payload(&args, Sample).unwrap()["intrabar_interval"].clone()
        };
        assert_eq!(interval(60), "30m");
        assert_eq!(interval(3), "30m");
    }

    #[test]
    fn a_stocks_poll_reads_what_its_load_read_though_its_few_bars_are_younger() {
        // A load reaching 12 days back: too old for Yahoo's 1m.
        let end = fetch::now_ms() - 10 * 86_400_000;
        let load = json!({ "ticker": "AAPL", "timeframe": "1h", "bars": 60, "end_time": end });
        let load = footprint_payload(&load, Sample).unwrap();
        assert_eq!(load["intrabar_interval"], "5m");
        let poll = |extra: Value| {
            let mut args = json!({ "ticker": "AAPL", "timeframe": "1h", "bars": 3 });
            args.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            footprint_payload(&args, Sample)
        };
        assert_eq!(poll(json!({})).unwrap()["intrabar_interval"], "1m");
        let kept = json!({ "intrabar_interval": load["intrabar_interval"] });
        assert_eq!(poll(kept).unwrap()["intrabar_interval"], "5m");
        let nothing = poll(json!({ "intrabar_interval": "none" })).unwrap();
        assert_eq!(nothing["intrabar_interval"], Value::Null);
        assert!(poll(json!({ "intrabar_interval": "nonsense" })).is_err());
    }

    #[test]
    fn a_stock_has_no_taker_volume_and_says_so() {
        let args = json!({ "ticker": "AAPL", "timeframe": "1h", "bars": 3 });
        let fp = footprint_payload(&args, Sample).unwrap();
        assert_eq!(fp["side"], "close_vs_open");
        assert_eq!(fp["intrabar_interval"], "1m");
    }

    #[test]
    fn a_chart_with_nothing_finer_reads_its_own_bars_and_says_so() {
        // Yahoo's finest is 1m, and 1m is the chart.
        let args = json!({ "ticker": "AAPL", "timeframe": "1m", "bars": 3 });
        let fp = footprint_payload(&args, Sample).unwrap();
        assert_eq!(fp["intrabar_interval"], Value::Null);
        assert_eq!(fp["footprints"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn row_size_settings_apply_and_the_row_size_is_reported() {
        let base = json!({ "ticker": "BTCUSDT", "timeframe": "1h", "bars": 2, "row_size": 2.5 });
        let fp = footprint_payload(&base, Sample).unwrap();
        assert_eq!(fp["row_size"], 2.5);
        assert_eq!(fp["row_auto"], false);
        for r in fp["footprints"][0]["rows"].as_array().unwrap() {
            let steps = r[0].as_f64().unwrap() / 2.5;
            assert!((steps - steps.round()).abs() < 1e-6, "{r}");
        }
        let ticks = json!({ "ticker": "BTCUSDT", "timeframe": "1h", "bars": 2, "row_ticks": 4 });
        let fp = footprint_payload(&ticks, Sample).unwrap();
        let (row, tick) = (
            fp["row_size"].as_f64().unwrap(),
            fp["tick"].as_f64().unwrap(),
        );
        assert!(close(row, 4.0 * tick), "{row} {tick}");
        // Too fine to fit: widened, and it says so.
        let fine = json!({ "ticker": "BTCUSDT", "timeframe": "1h", "bars": 2, "row_size": 1e-6 });
        let fp = footprint_payload(&fine, Sample).unwrap();
        assert_eq!(fp["row_widened"], true);
        for f in fp["footprints"].as_array().unwrap() {
            assert!(f["rows"].as_array().unwrap().len() <= 210);
        }
        // Larger than any price: no row of a whole chart at a POC of 0.
        let huge = json!({ "ticker": "BTCUSDT", "timeframe": "1h", "bars": 2, "row_size": 1e308 });
        let fp = footprint_payload(&huge, Sample).unwrap();
        assert_eq!(fp["row_widened"], true);
        assert!(fp["row_size"].as_f64().unwrap() < 1e9, "{}", fp["row_size"]);
        let tpo = tpo_payload(
            &json!({ "ticker": "BTCUSDT", "periods": 1, "row_size": 1e308 }),
            Sample,
        );
        assert!(tpo.unwrap()["row_size"].as_f64().unwrap() < 1e9);
    }

    #[test]
    fn the_imbalance_setting_changes_what_is_marked() {
        let count = |pct: f64| {
            let args = json!({ "ticker": "BTCUSDT", "timeframe": "1h", "bars": 40, "imbalance": pct,
                "end_time": 1_699_920_000_000i64 });
            let fp = footprint_payload(&args, Sample).unwrap();
            fp["footprints"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["buy_imbalances"].as_array().unwrap().len())
                .sum::<usize>()
        };
        assert!(
            count(100.0) > count(300.0),
            "{} {}",
            count(100.0),
            count(300.0)
        );
    }

    #[test]
    fn footprint_arguments_are_checked() {
        for (args, wanted) in [
            (json!({}), "ticker"),
            (json!({ "ticker": "BTCUSDT", "bars": 201 }), "bars"),
            (json!({ "ticker": "BTCUSDT", "imbalance": 50 }), "imbalance"),
            (json!({ "ticker": "BTCUSDT", "timeframe": "nope" }), ""),
        ] {
            let err = footprint_payload(&args, Sample).unwrap_err();
            assert!(err.contains(wanted), "{err}");
        }
    }

    #[test]
    fn a_tpo_profile_covers_its_period_and_its_blocks() {
        let args = json!({ "ticker": "BTCUSDT", "periods": 3, "end_time": 1_699_920_000_000i64 });
        let tpo = tpo_payload(&args, Sample).unwrap();
        let list = tpo["profiles"].as_array().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(tpo["block"], "30m");
        for p in list {
            let blocks = p["block_times"].as_array().unwrap().len();
            assert_eq!(blocks, 48);
            let high = p["high"].as_f64().unwrap();
            let low = p["low"].as_f64().unwrap();
            let (val, poc, vah) = (
                p["val"].as_f64().unwrap(),
                p["poc"].as_f64().unwrap(),
                p["vah"].as_f64().unwrap(),
            );
            assert!(val <= poc && poc < vah);
            assert!(low <= poc && poc <= high);
            let letters = p["rows"].as_array().unwrap();
            let prints: usize = letters
                .iter()
                .map(|r| r["blocks"].as_array().unwrap().len())
                .sum();
            assert_eq!(prints, p["total_blocks"].as_u64().unwrap() as usize);
            assert_eq!(
                letters
                    .iter()
                    .map(|r| r["letters"].as_str().unwrap().len())
                    .sum::<usize>(),
                prints
            );
        }
    }

    #[test]
    fn tpo_settings_apply() {
        let base = |extra: Value| {
            let mut a =
                json!({ "ticker": "BTCUSDT", "periods": 2, "end_time": 1_699_920_000_000i64 });
            a.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            tpo_payload(&a, Sample).unwrap()
        };
        let hourly = base(json!({ "block": "1h" }));
        assert_eq!(
            hourly["profiles"][0]["block_times"]
                .as_array()
                .unwrap()
                .len(),
            24
        );
        let weekly = base(json!({ "period": "1W", "block": "4h" }));
        assert_eq!(
            weekly["profiles"][0]["block_times"]
                .as_array()
                .unwrap()
                .len(),
            42
        );
        let split = base(json!({ "sessions": ["00:00-08:00", "08:00-16:00"] }));
        assert_eq!(split["profiles"].as_array().unwrap().len(), 4);
        assert_eq!(
            split["profiles"][0]["block_times"]
                .as_array()
                .unwrap()
                .len(),
            16
        );
        let rows = base(json!({ "row_size": 5.0 }));
        assert_eq!(rows["row_size"], 5.0);
        let narrow = base(json!({ "value_area": 20 }));
        let wide = base(json!({ "value_area": 95 }));
        let span = |p: &Value| p["vah"].as_f64().unwrap() - p["val"].as_f64().unwrap();
        assert!(span(&narrow["profiles"][0]) < span(&wide["profiles"][0]));
        for bad in [
            json!({ "ticker": "BTCUSDT", "block": "1m" }),
            json!({ "ticker": "BTCUSDT", "block": "1d" }),
            json!({ "ticker": "BTCUSDT", "period": "1h" }),
            json!({ "ticker": "BTCUSDT", "sessions": ["bad"] }),
        ] {
            assert!(tpo_payload(&bad, Sample).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_auto_tpo_row_is_not_widened_by_the_range_of_all_periods_together() {
        // Long enough for the price to wander far more than a day's range.
        let args = json!({ "ticker": "BTCUSDT", "periods": 30, "end_time": 1_699_920_000_000i64 });
        let tpo = tpo_payload(&args, Sample).unwrap();
        assert_eq!(tpo["row_widened"], false, "{}", tpo["row_size"]);
    }

    #[test]
    fn a_block_too_short_for_a_whole_period_is_widened_and_says_so() {
        let args = json!({ "ticker": "BTCUSDT", "period": "1M", "block": "5m", "periods": 1,
            "end_time": 1_699_920_000_000i64 });
        let tpo = tpo_payload(&args, Sample).unwrap();
        assert_eq!(tpo["block"], "15m");
        assert_eq!(tpo["block_widened"], true);
        assert_eq!(tpo["periods_cut"], false);
        // The month from its start: 13 days and 16 hours in, on 15m blocks.
        let p = &tpo["profiles"][0];
        assert_eq!(p["t"], 1_698_796_800, "2023-11-01");
        assert_eq!(p["block_times"][0], 1_698_796_800);
        // More periods than fit are cut, whole ones.
        let args = json!({ "ticker": "BTCUSDT", "period": "1M", "block": "5m", "periods": 12,
            "end_time": 1_699_920_000_000i64 });
        let tpo = tpo_payload(&args, Sample).unwrap();
        assert_eq!(tpo["periods_cut"], true);
        let blocks: usize = tpo["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["block_times"].as_array().unwrap().len())
            .sum();
        assert!(blocks <= MAX_TPO_BLOCKS, "{blocks}");
        let as_asked = json!({ "ticker": "BTCUSDT", "period": "1D", "block": "5m", "periods": 1,
            "end_time": 1_699_920_000_000i64 });
        assert_eq!(
            tpo_payload(&as_asked, Sample).unwrap()["block_widened"],
            false
        );
        // Longer than 4h blocks can cover: refused rather than cut.
        let too_long = json!({ "ticker": "BTCUSDT", "period": "30M" });
        assert!(tpo_payload(&too_long, Sample)
            .unwrap_err()
            .contains("too long"));
    }

    #[test]
    fn a_profile_keeps_its_key_as_the_window_moves() {
        let key = |period: &str, block: &str, end: i64| {
            let args = json!({ "ticker": "BTCUSDT", "period": period, "block": block,
                "periods": 2, "end_time": end });
            let tpo = tpo_payload(&args, Sample).unwrap();
            tpo["profiles"].as_array().unwrap().last().unwrap()["t"].clone()
        };
        // Noon, so that these hours stay in the same day, week and month.
        let end = 1_699_920_000_000i64 + 12 * 3_600_000;
        for (period, block) in [("1M", "5m"), ("1W", "30m"), ("1D", "30m")] {
            let at = key(period, block, end);
            for hours in [1, 2, 6] {
                assert_eq!(key(period, block, end + hours * 3_600_000), at, "{period}");
            }
        }
    }

    #[test]
    fn profiles_come_in_time_order_with_a_key_each_whatever_the_order_of_sessions() {
        let args = json!({ "ticker": "BTCUSDT", "periods": 2, "end_time": 1_699_920_000_000i64,
            "sessions": ["16:00-20:00", "08:00-12:00"] });
        let tpo = tpo_payload(&args, Sample).unwrap();
        let list = tpo["profiles"].as_array().unwrap();
        assert_eq!(list.len(), 4);
        let first_block = |p: &Value| p["block_times"][0].as_i64().unwrap();
        assert!(
            list.windows(2)
                .all(|w| first_block(&w[0]) < first_block(&w[1])),
            "{list:?}"
        );
        // Each key is where its session starts, as the blocks here start with it.
        for p in list {
            assert_eq!(p["t"].as_i64().unwrap(), first_block(p));
        }
    }

    #[test]
    fn each_bar_and_profile_says_the_row_it_was_built_with() {
        let args = json!({ "ticker": "BTCUSDT", "timeframe": "1h", "bars": 3, "row_size": 2.5 });
        let fp = footprint_payload(&args, Sample).unwrap();
        assert_eq!(fp["footprints"][0]["row"], 2.5);
        // Too fine for this reply: it is wider than asked, on the bar too.
        let args = json!({ "ticker": "BTCUSDT", "timeframe": "1h", "bars": 3, "row_size": 0.01 });
        let fp = footprint_payload(&args, Sample).unwrap();
        assert_eq!(fp["row_widened"], true);
        assert_eq!(fp["footprints"][0]["row"], fp["row_size"]);
        let args = json!({ "ticker": "BTCUSDT", "periods": 1, "row_size": 2.5 });
        assert_eq!(
            tpo_payload(&args, Sample).unwrap()["profiles"][0]["row"],
            2.5
        );
    }

    #[test]
    fn only_a_load_that_asks_reads_its_profiles_from_finer_bars() {
        let series = |bars: u64, ask: Option<bool>, end: i64| {
            let mut args = json!({ "ticker": "BTCUSDT", "timeframe": "1h", "bars": bars,
                "format": "series", "indicators": ["volume_profile", "rsi"], "end_time": end });
            if let Some(ask) = ask {
                args["profile_intrabars"] = json!(ask);
            }
            flow_series(args)
        };
        let end = 1_699_920_000_000i64;
        let load = series(100, Some(true), end);
        assert_eq!(
            load["indicators"]["volume_profile"]["profile_interval"],
            "3m"
        );
        assert_eq!(load["indicators"]["rsi"].get("profile_interval"), None);
        assert!(!load["indicators"]["volume_profile"]["hbars"]
            .as_array()
            .unwrap()
            .is_empty());
        // `profile_interval` is only set after reading: without it nothing was read. A
        // page and a catch-up tail are longer than a tail, and do not ask.
        for (what, bars, end) in [
            ("tail", 2, end),
            ("catch-up tail", 9, end),
            ("page", 500, end - 500 * 3_600_000),
        ] {
            let s = series(bars, None, end);
            assert_eq!(
                s["indicators"]["volume_profile"].get("profile_interval"),
                None,
                "{what}"
            );
        }
        let no = series(100, Some(false), end);
        assert_eq!(
            no["indicators"]["volume_profile"].get("profile_interval"),
            None
        );
    }

    #[test]
    fn a_finer_profile_differs_from_spreading_each_bar() {
        use chartgen::indicator::Indicator as _;
        // A bar sold off at its high, and traded nearly all its volume in the top row.
        let bar = |o: f64, h: f64, l: f64, c: f64, v: f64, t: i64| data::Bar {
            open: o,
            high: h,
            low: l,
            close: c,
            volume: v,
            date: t.to_string(),
        };
        let data = |bars| data::OhlcvData {
            bars,
            symbol: None,
            interval: None,
            session: None,
        };
        let chart = data(vec![bar(100.0, 110.0, 100.0, 105.0, 100.0, 0)]);
        let finer = data(vec![
            bar(100.0, 100.5, 100.0, 100.5, 1.0, 0),
            bar(109.0, 110.0, 109.0, 109.5, 99.0, 60),
        ]);
        let vp = chartgen::indicators::custom::VolumeProfile::default();
        let poc = |pr: &chartgen::indicator::PanelResult| pr.hlines[0].y;
        let spread = vp.compute(&chart);
        let real = vp.with_intrabars(&chart, &finer).unwrap();
        assert!(poc(&real) > 109.0, "{}", poc(&real));
        assert!(poc(&spread) != poc(&real));
    }

    #[test]
    fn a_session_profile_has_the_values_of_its_load_on_a_tail() {
        let lines = |bars: u64, ask: bool| {
            let s = flow_series(
                json!({ "ticker": "BTCUSDT", "timeframe": "1h", "bars": bars,
                "warmup": 300, "format": "series", "indicators": ["session_vp"],
                "profile_intrabars": ask, "end_time": 1_699_920_000_000i64 }),
            );
            let ind = &s["indicators"]["session_vp"];
            assert_eq!(ind.get("profile_interval"), None);
            assert_eq!(ind["level_lines"], true);
            ind["lines"].as_array().unwrap().clone()
        };
        let (load, tail) = (lines(100, true), lines(2, false));
        assert_eq!(load.len(), 3);
        assert_eq!(tail.len(), 3);
        for (l, t) in load.iter().zip(&tail) {
            let values = l["values"].as_array().unwrap();
            assert_eq!(
                &values[values.len() - 2..],
                t["values"].as_array().unwrap().as_slice()
            );
        }
    }

    #[test]
    fn a_session_profile_keeps_a_value_on_every_bar_of_every_session() {
        for (timeframe, bars, per_session) in [("1h", 96, 24), ("4h", 60, 6), ("12h", 30, 2)] {
            let s = flow_series(
                json!({ "ticker": "BTCUSDT", "timeframe": timeframe, "bars": bars,
                "format": "series", "indicators": ["session_vp"], "warmup": 300,
                "end_time": 1_699_920_000_000i64 }),
            );
            let bar_times: Vec<i64> = s["bars"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| b["t"].as_i64().unwrap())
                .collect();
            let poc: Vec<f64> = s["indicators"]["session_vp"]["lines"][0]["values"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| {
                    v.as_f64()
                        .unwrap_or_else(|| panic!("{timeframe}: a bar has no POC"))
                })
                .collect();
            // Sessions are UTC days here: one level through each, the last bar included.
            let mut by_day: std::collections::BTreeMap<i64, Vec<f64>> = Default::default();
            for (t, v) in bar_times.iter().zip(&poc) {
                by_day.entry(t.div_euclid(86_400)).or_default().push(*v);
            }
            let whole = by_day.values().filter(|d| d.len() == per_session);
            assert!(whole.clone().count() >= 2, "{timeframe}");
            for day in whole {
                assert!(day.iter().all(|v| *v == day[0]), "{timeframe}: {day:?}");
            }
            let levels: std::collections::BTreeSet<u64> =
                by_day.values().map(|d| d[0].to_bits()).collect();
            assert!(levels.len() >= 2, "{timeframe}: the sessions differ");
        }
    }

    #[test]
    fn range_profiles_only_count_their_range() {
        let series = |ind: Value| {
            flow_series(
                json!({ "ticker": "BTCUSDT", "timeframe": "1h", "bars": 48, "format": "series",
                "indicators": [ind], "end_time": 1_699_920_000_000i64 }),
            )
        };
        let bars = series(json!("rsi"))["bars"].as_array().unwrap().clone();
        let t = |i: usize| bars[i]["t"].as_i64().unwrap();
        let all = series(json!("fixed_range_vp"));
        let some = series(
            json!({ "name": "fixed_range_vp", "from": t(10).to_string(), "to": t(20).to_string() }),
        );
        let hbars =
            |s: &Value, key: &str| s["indicators"][key]["hbars"].as_array().unwrap().clone();
        assert_ne!(
            hbars(&all, "fixed_range_vp"),
            hbars(&some, "fixed_range_vp")
        );
        assert_eq!(some["indicators"]["fixed_range_vp"]["label"], "FRVP");
        // Only prices of that range.
        let top = bars[10..20]
            .iter()
            .map(|b| b["h"].as_f64().unwrap())
            .fold(f64::MIN, f64::max);
        for h in hbars(&some, "fixed_range_vp") {
            assert!(h["y"].as_f64().unwrap() <= top + 1.0, "{h} {top}");
        }
        let anchored = series(json!({ "name": "anchored_vp", "anchor": t(40).to_string() }));
        assert_eq!(anchored["indicators"]["anchored_vp"]["label"], "AVP");
        assert_ne!(
            hbars(&anchored, "anchored_vp"),
            hbars(&all, "fixed_range_vp")
        );
    }

    #[test]
    fn generate_chart_series_returns_series_json() {
        let args = json!({
            "bars": 60,
            "indicators": ["rsi"],
            "format": "series"
        });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        assert!(!is_error(&resp));

        let content = chart_content(&resp);
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");

        let text = content[0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        let bars = parsed["bars"].as_array().unwrap();
        assert_eq!(bars.len(), 60);
        assert!(bars[0].get("t").is_some());
        assert!(bars[0]["o"].is_number());

        let rsi = &parsed["indicators"]["rsi"];
        let values = rsi["lines"][0]["values"].as_array().unwrap();
        assert_eq!(values.len(), 60, "values must be 1:1 with bars");

        let hlines = rsi["hlines"].as_array().unwrap();
        let ys: Vec<f64> = hlines.iter().map(|h| h["y"].as_f64().unwrap()).collect();
        assert!(ys.iter().any(|y| (*y - 30.0).abs() < 0.01));
        assert!(ys.iter().any(|y| (*y - 70.0).abs() < 0.01));

        let y_range = rsi["y_range"].as_array().unwrap();
        assert_eq!(y_range[0].as_f64().unwrap(), 0.0);
        assert_eq!(y_range[1].as_f64().unwrap(), 100.0);
    }

    #[test]
    fn panel_result_to_series_json_nan_is_null() {
        // Direct test of the serializer: NaN pre-warmup values must become
        // JSON null (serde_json can't emit NaN as a Number).
        use chartgen::data::{Bar, OhlcvData};
        use chartgen::indicator::{Line, PanelResult};
        use plotters::style::RGBAColor;

        let data = OhlcvData {
            bars: (0..5)
                .map(|i| Bar {
                    open: 1.0,
                    high: 1.0,
                    low: 1.0,
                    close: 1.0,
                    volume: 1.0,
                    date: format!("{}", 1_700_000_000_u64 + i * 3600),
                })
                .collect(),
            symbol: None,
            interval: None,
            session: None,
        };
        let pr = PanelResult {
            lines: vec![Line {
                breaks: false,
                y: vec![f64::NAN, f64::NAN, 1.0, 2.0, 3.0],
                color: RGBAColor(0, 0, 0, 1.0),
                width: 1,
                label: Some("L".into()),
            }],
            ..Default::default()
        };

        let j = panel_result_to_series_json(&pr, &data);
        let values = j["lines"][0]["values"].as_array().unwrap();
        assert_eq!(values.len(), 5);
        assert_eq!(values[0], Value::Null);
        assert_eq!(values[1], Value::Null);
        assert_eq!(values[2], json!(1.0));
        assert_eq!(values[4], json!(3.0));
    }

    #[test]
    fn panel_result_to_series_json_carries_plot_colors() {
        // ADR-0026: lines carry color + width, histograms a label and
        // per-bar colors as palette indices, signals their dot color.
        use chartgen::data::{Bar, OhlcvData};
        use chartgen::indicator::{Bars, Dot, Line, PanelResult};
        use plotters::style::RGBAColor;

        let data = OhlcvData {
            bars: (0..4)
                .map(|i| Bar {
                    open: 1.0,
                    high: 1.0,
                    low: 1.0,
                    close: 1.0,
                    volume: 1.0,
                    date: format!("{}", 1_700_000_000_u64 + i * 3600),
                })
                .collect(),
            symbol: None,
            interval: None,
            session: None,
        };
        let green = RGBAColor(38, 166, 154, 0.5);
        let red = RGBAColor(239, 83, 80, 0.5);
        let pr = PanelResult {
            lines: vec![Line {
                breaks: false,
                y: vec![1.0; 4],
                color: RGBAColor(41, 98, 255, 1.0),
                width: 2,
                label: Some("Volume MA".into()),
            }],
            bars: vec![Bars {
                y: vec![f64::NAN, 1.0, 2.0, 3.0],
                colors: vec![green, green, red, green],
                bottom: 0.0,
            }],
            dots: vec![Dot {
                x: 2,
                y: 1.0,
                color: red,
                size: 3,
                label: Some("sell".into()),
            }],
            label: "Volume".into(),
            ..Default::default()
        };

        let j = panel_result_to_series_json(&pr, &data);
        assert_eq!(j["lines"][0]["color"], "rgba(41,98,255,1)");
        assert_eq!(j["lines"][0]["width"], 2);
        let h = &j["histogram"][0];
        assert_eq!(h["label"], "Volume");
        assert_eq!(
            h["palette"],
            json!(["rgba(38,166,154,0.5)", "rgba(239,83,80,0.5)"])
        );
        // The warmup bar has no value, so no color either.
        assert_eq!(h["color_idx"], json!([null, 0, 1, 0]));
        assert_eq!(j["signals"][0]["color"], "rgba(239,83,80,0.5)");
    }

    #[test]
    fn histogram_label_falls_back_when_a_line_has_the_panel_name() {
        use chartgen::indicator::{Bars, Line, PanelResult};
        use plotters::style::RGBAColor;

        let line = |label: &str| Line {
            breaks: false,
            y: vec![],
            color: RGBAColor(0, 0, 0, 1.0),
            width: 1,
            label: Some(label.into()),
        };
        let bars = || Bars {
            y: vec![],
            colors: vec![],
            bottom: 0.0,
        };
        let volume = PanelResult {
            lines: vec![line("Volume MA")],
            bars: vec![bars()],
            label: "Volume".into(),
            ..Default::default()
        };
        assert_eq!(histogram_label(&volume, 0), "Volume");
        let macd = PanelResult {
            lines: vec![line("MACD"), line("Signal")],
            bars: vec![bars()],
            label: "MACD".into(),
            ..Default::default()
        };
        assert_eq!(histogram_label(&macd, 0), "Histogram");
        let two = PanelResult {
            bars: vec![bars(), bars()],
            label: "KVF".into(),
            ..Default::default()
        };
        assert_eq!(histogram_label(&two, 1), "KVF 2");
    }

    #[test]
    fn generate_chart_series_volume_has_up_and_down_colors() {
        let args = json!({ "bars": 120, "indicators": ["volume"], "format": "series" });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        let parsed: Value =
            serde_json::from_str(chart_content(&resp)[0]["text"].as_str().unwrap()).unwrap();
        let h = &parsed["indicators"]["volume"]["histogram"][0];
        assert_eq!(h["label"], "Volume");
        let palette: Vec<&str> = h["palette"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap())
            .collect();
        assert!(palette.contains(&"rgba(38,166,154,0.5)"), "{palette:?}");
        assert!(palette.contains(&"rgba(239,83,80,0.5)"), "{palette:?}");
        let idx = h["color_idx"].as_array().unwrap();
        assert_eq!(idx.len(), h["values"].as_array().unwrap().len());
        assert!(idx.iter().all(|i| i.as_u64().is_some_and(|i| i < 2)));
    }

    #[test]
    fn generate_chart_series_signals_align_with_bars() {
        // cipher_b emits labeled dots; assert every signal.x is in range
        // and when signal.t is non-null it matches bars[x].t.
        let args = json!({
            "bars": 120,
            "indicators": ["cipher_b"],
            "format": "series"
        });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        let text = chart_content(&resp)[0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        let bars = parsed["bars"].as_array().unwrap();
        assert_eq!(bars.len(), 120);

        let cipher = &parsed["indicators"]["cipher_b"];
        if let Some(signals) = cipher["signals"].as_array() {
            for sig in signals {
                let x = sig["x"].as_u64().unwrap() as usize;
                assert!(x < bars.len(), "signal.x out of range");
                let sig_t = &sig["t"];
                let bar_t = &bars[x]["t"];
                assert_eq!(sig_t, bar_t, "signal.t must match bars[x].t");
            }
        }
    }

    #[test]
    fn generate_chart_series_macd_histogram_aligns_with_bars() {
        // MACD emits a histogram (pr.bars) in addition to lines. Series
        // must expose the full per-bar histogram values with 1:1 alignment.
        let args = json!({
            "bars": 80,
            "indicators": ["macd"],
            "format": "series"
        });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        let text = chart_content(&resp)[0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["bars"].as_array().unwrap().len(), 80);
        let macd = &parsed["indicators"]["macd"];
        let histogram = macd["histogram"]
            .as_array()
            .expect("macd series must include histogram");
        assert!(!histogram.is_empty());
        let values = histogram[0]["values"].as_array().unwrap();
        assert_eq!(values.len(), 80, "histogram values must be 1:1 with bars");
    }

    #[test]
    fn generate_chart_series_omits_png() {
        let args = json!({
            "bars": 40,
            "indicators": ["rsi"],
            "format": "series",
            "width": 400,
            "height": 300
        });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        let content = chart_content(&resp);
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
    }

    #[test]
    fn generate_chart_unknown_format_lists_series() {
        // Regression: ensure the error message now advertises 'series'.
        let args = json!({ "bars": 20, "format": "svg" });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        assert!(is_error(&resp));
        let text = response_text(&resp);
        assert!(text.contains("Unknown format"));
        assert!(text.contains("series"));
    }

    #[test]
    fn initialize_advertises_streamable_http_protocol_version() {
        let resp = handle_initialize(Some(json!(1)));
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(resp["result"]["serverInfo"]["name"], "chartgen");
    }

    #[test]
    fn series_includes_fills_when_indicator_emits_them() {
        // Bollinger Bands emits a Fill between upper/lower bands. Series must
        // expose it as fills[*].y1/y2 arrays aligned 1:1 with bars[].
        let args = json!({
            "bars": 60,
            "indicators": ["bbands"],
            "format": "series"
        });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        let text = chart_content(&resp)[0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["bars"].as_array().unwrap().len(), 60);
        let bb = &parsed["indicators"]["bbands"];
        let fills = bb["fills"]
            .as_array()
            .expect("bbands series must include fills");
        assert!(!fills.is_empty());
        let y1 = fills[0]["y1"].as_array().unwrap();
        let y2 = fills[0]["y2"].as_array().unwrap();
        assert_eq!(y1.len(), 60, "fills.y1 must be 1:1 with bars");
        assert_eq!(y2.len(), 60, "fills.y2 must be 1:1 with bars");
        // Last bar must carry finite upper/lower band values with upper >= lower.
        assert!(y1[59].is_number());
        assert!(y2[59].is_number());
        assert!(y1[59].as_f64().unwrap() >= y2[59].as_f64().unwrap());
    }

    #[test]
    fn series_includes_hbars_for_vpvr() {
        // VPVR emits horizontal bars at price levels. Series must expose them
        // as hbars[*] with numeric y/height/width/offset and a boolean left.
        let args = json!({
            "bars": 120,
            "indicators": ["vpvr"],
            "format": "series"
        });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        let text = chart_content(&resp)[0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        let vpvr = &parsed["indicators"]["vpvr"];
        let hbars = vpvr["hbars"]
            .as_array()
            .expect("vpvr series must include hbars");
        assert!(!hbars.is_empty());
        let first = &hbars[0];
        assert!(first["y"].is_number());
        assert!(first["height"].is_number());
        assert!(first["width"].is_number());
        assert!(first["offset"].is_number());
        assert!(first["left"].is_boolean());
    }

    #[test]
    fn series_includes_divlines_when_indicator_emits_them() {
        // cipher_b produces divergence lines under the right conditions. Call
        // the serializer directly on a synthetic PanelResult so the test is
        // deterministic and doesn't depend on sample_data hitting a divergence.
        use chartgen::data::{Bar, OhlcvData};
        use chartgen::indicator::{DivLine, PanelResult};
        use plotters::style::RGBAColor;

        let data = OhlcvData {
            bars: (0..5)
                .map(|i| Bar {
                    open: 1.0,
                    high: 1.0,
                    low: 1.0,
                    close: 1.0,
                    volume: 1.0,
                    date: format!("{}", 1_713_427_200_u64 + i * 3600),
                })
                .collect(),
            symbol: None,
            interval: None,
            session: None,
        };
        let pr = PanelResult {
            divlines: vec![DivLine {
                x1: 1,
                y1: 30.2,
                x2: 4,
                y2: 18.7,
                color: RGBAColor(0, 255, 0, 0.8),
                dashed: true,
            }],
            ..Default::default()
        };

        let j = panel_result_to_series_json(&pr, &data);
        let divs = j["divlines"]
            .as_array()
            .expect("panel with divlines must emit divlines[]");
        assert_eq!(divs.len(), 1);
        let d = &divs[0];
        assert_eq!(d["x1"], 1);
        assert_eq!(d["y1"], 30.2);
        assert_eq!(d["x2"], 4);
        assert_eq!(d["y2"], 18.7);
        assert_eq!(d["dashed"], true);
        // t1/t2 must mirror bar_t for the bar at x1/x2.
        assert_eq!(d["t1"], json!(1_713_427_200_u64 + 3600));
        assert_eq!(d["t2"], json!(1_713_427_200_u64 + 4 * 3600));
    }

    #[test]
    fn series_omits_channels_when_empty() {
        // An indicator that produces no fills/hbars/divlines must not include
        // those keys — matches the existing pattern for lines/histogram.
        let args = json!({
            "bars": 60,
            "indicators": ["rsi"],
            "format": "series"
        });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        let text = chart_content(&resp)[0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        let rsi = &parsed["indicators"]["rsi"];
        assert!(
            rsi.get("fills").is_none(),
            "fills must be omitted when empty"
        );
        assert!(
            rsi.get("hbars").is_none(),
            "hbars must be omitted when empty"
        );
        assert!(
            rsi.get("divlines").is_none(),
            "divlines must be omitted when empty"
        );
    }

    #[test]
    fn generate_chart_summary_byte_identity_golden() {
        // sample_data is seeded (Rng::new(42)), so the pretty-printed summary
        // for a fixed input is stable. This test guards against accidental
        // changes to the summary serializer while the series format is added.
        let args = json!({
            "bars": 30,
            "indicators": ["rsi"],
            "format": "summary"
        });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        let actual = chart_content(&resp)[0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        const EXPECTED: &str = include_str!("../tests/golden/generate_chart_summary_rsi_30.json");
        assert_eq!(
            actual, EXPECTED,
            "summary output must remain byte-identical"
        );
    }

    #[test]
    fn generate_chart_invalid_param_returns_tool_error() {
        for (ind, msg) in [
            (
                json!({"name": "rsi", "length": 0}),
                "Invalid parameter for rsi: length must be >= 1",
            ),
            (
                json!({"name": "macd", "fast": 0}),
                "Invalid parameter for macd: fast must be >= 1",
            ),
            (
                json!({"name": "rsi", "length": -3}),
                "Invalid parameter for rsi: length must be >= 1",
            ),
        ] {
            let args = json!({"indicators": [ind], "bars": 50, "format": "summary"});
            let resp = tool_generate_chart(Some(json!(1)), &args, None);
            assert_eq!(resp["result"]["isError"], true);
            assert_eq!(resp["result"]["content"][0]["text"], msg);
        }
    }

    // --- multiple instances of one indicator (ADR-0028) ---

    fn last_rsi(p: &Value, key: &str) -> f64 {
        let v = p["indicators"][key]["lines"][0]["values"]
            .as_array()
            .unwrap();
        v.last().unwrap().as_f64().unwrap()
    }

    #[test]
    fn two_instances_are_keyed_by_id_regardless_of_order() {
        let series = |indicators: Value| {
            series_payload(
                json!({ "bars": 60, "indicators": indicators, "format": "series" }),
                fetch::DataSource::Live,
            )
        };
        let p = series(json!([
            { "name": "rsi", "length": 14 },
            { "name": "rsi", "length": 21, "id": "rsi#2" },
        ]));
        let keys: Vec<&String> = p["indicators"].as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 2);
        let (r14, r21) = (last_rsi(&p, "rsi"), last_rsi(&p, "rsi#2"));
        assert_ne!(r14, r21, "each instance has its own params");
        // Each instance matches a single-instance request with its params.
        let solo = series(json!([{ "name": "rsi", "length": 21 }]));
        assert_eq!(r21, last_rsi(&solo, "rsi"));
        // Reordering the request keeps every key on its instance.
        let swapped = series(json!([
            { "name": "rsi", "length": 21, "id": "rsi#2" },
            { "name": "rsi", "length": 14 },
        ]));
        assert_eq!(last_rsi(&swapped, "rsi"), r14);
        assert_eq!(last_rsi(&swapped, "rsi#2"), r21);
    }

    #[test]
    fn repeats_without_an_id_get_numbered_keys() {
        let p = series_payload(
            json!({ "bars": 40, "format": "series", "indicators": [
                "rsi",
                { "name": "rsi", "length": 7 },
                { "name": "macd", "id": "rsi#2" },
            ] }),
            fetch::DataSource::Live,
        );
        let mut keys: Vec<&String> = p["indicators"].as_object().unwrap().keys().collect();
        keys.sort();
        // The explicit id is reserved first, so the repeat skips to #3.
        assert_eq!(keys, ["rsi", "rsi#2", "rsi#3"]);
        assert!(
            p["indicators"]["rsi#2"]["histogram"].is_array(),
            "macd under its id"
        );
        assert_ne!(last_rsi(&p, "rsi"), last_rsi(&p, "rsi#3"));
        // The summary format uses the same keys.
        let resp = tool_generate_chart(
            Some(json!(1)),
            &json!({ "bars": 40, "format": "summary", "indicators": ["rsi", "rsi"] }),
            None,
        );
        let s: Value = serde_json::from_str(&response_text(&resp)).unwrap();
        assert!(s["indicators"]["rsi"].is_object() && s["indicators"]["rsi#2"].is_object());
    }

    #[test]
    fn invalid_or_duplicate_ids_are_tool_errors() {
        for (indicators, msg) in [
            (
                json!([{ "name": "rsi", "id": "a" }, { "name": "macd", "id": "a" }]),
                "Duplicate indicator id: a",
            ),
            (
                json!([{ "name": "rsi", "id": "" }]),
                "Invalid indicator id: must be a string of 1 to 64 characters",
            ),
            (
                json!([{ "name": "rsi", "id": 2 }]),
                "Invalid indicator id: must be a string of 1 to 64 characters",
            ),
            (
                json!([{ "name": "rsi", "id": "x".repeat(65) }]),
                "Invalid indicator id: must be a string of 1 to 64 characters",
            ),
        ] {
            let args = json!({ "indicators": indicators, "bars": 30, "format": "series" });
            let resp = tool_generate_chart(Some(json!(1)), &args, None);
            assert_eq!(resp["result"]["isError"], true);
            assert_eq!(resp["result"]["content"][0]["text"], msg);
        }
    }

    #[test]
    fn an_id_may_not_take_the_name_of_an_entry_without_one() {
        let args = json!({ "bars": 30, "format": "series",
                           "indicators": [{ "name": "rsi", "id": "macd" }, "macd"] });
        let resp = tool_generate_chart(Some(json!(1)), &args, None);
        assert_eq!(resp["result"]["isError"], true);
        assert_eq!(
            resp["result"]["content"][0]["text"],
            "Indicator id macd is also the name of an entry without an id; give that entry an id too"
        );
        // Its own name is fine, and so is a name whose entries all have ids.
        let ok = json!([{ "name": "rsi", "id": "rsi" }, { "name": "macd", "id": "rsi#2" }]);
        assert!(instance_keys(ok.as_array().unwrap()).is_ok());
    }

    #[test]
    fn generate_chart_caps_the_entry_count() {
        let args = |n: usize| json!({ "bars": 30, "format": "summary", "indicators": vec![json!("rsi"); n] });
        let resp = tool_generate_chart(Some(json!(1)), &args(MAX_CHART_INDICATORS), None);
        assert!(!is_error(&resp), "{}", response_text(&resp));
        let s: Value = serde_json::from_str(&response_text(&resp)).unwrap();
        assert_eq!(s["indicators"].as_object().unwrap().len(), 50);
        assert!(s["indicators"]["rsi#50"].is_object());
        let resp = tool_generate_chart(Some(json!(1)), &args(MAX_CHART_INDICATORS + 1), None);
        assert_eq!(resp["result"]["isError"], true);
        assert_eq!(
            resp["result"]["content"][0]["text"],
            "At most 50 indicators per chart."
        );
    }

    #[test]
    fn instance_keys_are_linear_in_the_repeat_count() {
        // Far past the cap: key assignment alone must not be quadratic.
        let arr = vec![json!("rsi"); 100_000];
        let started = std::time::Instant::now();
        let keys = instance_keys(&arr).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert_eq!(keys[0].as_deref(), Some("rsi"));
        assert_eq!(keys[99_999].as_deref(), Some("rsi#100000"));
        // An explicit id is skipped by the counter.
        let arr = [
            json!("rsi"),
            json!({ "name": "rsi", "id": "rsi#2" }),
            json!("rsi"),
        ];
        let keys: Vec<_> = instance_keys(&arr).unwrap().into_iter().flatten().collect();
        assert_eq!(keys, ["rsi", "rsi#2", "rsi#3"]);
    }

    #[test]
    fn id_is_not_passed_as_a_parameter() {
        let with_id = series_payload(
            json!({ "bars": 40, "format": "series",
                    "indicators": [{ "name": "rsi", "length": 9, "id": "fast" }] }),
            fetch::DataSource::Live,
        );
        let plain = series_payload(
            json!({ "bars": 40, "format": "series",
                    "indicators": [{ "name": "rsi", "length": 9 }] }),
            fetch::DataSource::Live,
        );
        assert_eq!(with_id["indicators"]["fast"], plain["indicators"]["rsi"]);
        assert!(with_id["indicators"].get("rsi").is_none());
    }

    #[test]
    fn series_tail_keys_instances_like_generate_chart() {
        let raw = data::sample_ohlcv("BTCUSDT", "1h", 80, None, 1_704_067_200_000).unwrap();
        let tail = series_tail(
            &raw,
            &json!(["rsi", { "name": "rsi", "length": 21, "id": "rsi#2" }]),
            "candles",
            2,
        )
        .unwrap();
        assert_eq!(tail["bars"].as_array().unwrap().len(), 2);
        assert_ne!(last_rsi(&tail, "rsi"), last_rsi(&tail, "rsi#2"));
        let solo = series_tail(
            &raw,
            &json!([{ "name": "rsi", "length": 21 }]),
            "candles",
            2,
        )
        .unwrap();
        assert_eq!(last_rsi(&tail, "rsi#2"), last_rsi(&solo, "rsi"));
    }

    // --- warmup / end_time / paging (ADR-0009) ---

    const HOUR_MS: i64 = 3_600_000;
    const END_4H_MS: i64 = 1_700_006_400_000; // on the 4h grid

    fn series_payload(args: Value, source: fetch::DataSource) -> Value {
        let resp = generate_chart(Some(json!(1)), &args, source, None);
        assert!(
            !is_error(&resp),
            "unexpected error: {}",
            response_text(&resp)
        );
        serde_json::from_str(chart_content(&resp)[0]["text"].as_str().unwrap()).unwrap()
    }

    fn bar_ts(p: &Value) -> Vec<i64> {
        p["bars"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["t"].as_i64().unwrap())
            .collect()
    }

    #[test]
    fn trim_panel_result_rebases_every_channel() {
        use chartgen::data::{Bar, OhlcvData};
        use chartgen::indicator::{Bars, DivLine, Dot, Fill, HBar, HLine, Line, PanelResult};
        use plotters::style::RGBAColor;

        let c = RGBAColor(0, 0, 0, 1.0);
        let ys: Vec<f64> = (0..8).map(|i| i as f64).collect();
        let dot = |x: usize| Dot {
            x,
            y: x as f64,
            color: c,
            size: 1,
            label: Some(format!("d{x}")),
        };
        let div = |x1: usize, y1: f64, x2: usize, y2: f64| DivLine {
            x1,
            y1,
            x2,
            y2,
            color: c,
            dashed: false,
        };
        let pr = PanelResult {
            lines: vec![Line {
                breaks: false,
                y: ys.clone(),
                color: c,
                width: 1,
                label: None,
            }],
            fills: vec![Fill {
                y1: ys.clone(),
                y2: ys.iter().map(|v| -v).collect(),
                color: c,
                color_below: None,
            }],
            bars: vec![Bars {
                y: ys.clone(),
                colors: vec![c; 8],
                bottom: 0.0,
            }],
            dots: vec![dot(1), dot(3), dot(7)],
            hlines: vec![HLine { y: 70.0, color: c }],
            hbars: vec![HBar {
                y: 5.0,
                height: 1.0,
                width: 0.5,
                offset: 0.0,
                color: c,
                left: false,
            }],
            divlines: vec![
                div(0, 0.0, 2, 20.0),  // wholly inside warmup
                div(1, 10.0, 5, 50.0), // straddles the boundary
                div(4, 40.0, 7, 70.0), // fully visible
                div(6, 60.0, 2, 20.0), // reversed, straddling
                div(1, 10.0, 3, 30.0), // ends exactly on visible bar 0
            ],
            y_range: Some((0.0, 100.0)),
            label: "T".into(),
            is_overlay: false,
            ..Default::default()
        };

        let t = trim_panel_result(&pr, 3);
        assert_eq!(t.lines[0].y, vec![3.0, 4.0, 5.0, 6.0, 7.0]);
        assert_eq!(t.fills[0].y1, vec![3.0, 4.0, 5.0, 6.0, 7.0]);
        assert_eq!(t.fills[0].y2, vec![-3.0, -4.0, -5.0, -6.0, -7.0]);
        assert_eq!(t.bars[0].y, vec![3.0, 4.0, 5.0, 6.0, 7.0]);
        assert_eq!(t.bars[0].colors.len(), 5);
        let xs: Vec<(usize, f64)> = t.dots.iter().map(|d| (d.x, d.y)).collect();
        assert_eq!(xs, vec![(0, 3.0), (4, 7.0)]);
        let divs: Vec<(usize, f64, usize, f64)> = t
            .divlines
            .iter()
            .map(|d| (d.x1, d.y1, d.x2, d.y2))
            .collect();
        assert_eq!(
            divs,
            vec![(0, 30.0, 2, 50.0), (1, 40.0, 4, 70.0), (3, 60.0, 0, 30.0)]
        );
        assert_eq!(t.hlines.len(), 1);
        assert_eq!(t.hbars.len(), 1);
        assert_eq!(t.y_range, Some((0.0, 100.0)));

        // Serialized against the trimmed bars, t fields resolve to visible bars.
        let visible = OhlcvData {
            bars: (3..8)
                .map(|i| Bar {
                    open: 1.0,
                    high: 1.0,
                    low: 1.0,
                    close: 1.0,
                    volume: 1.0,
                    date: format!("{}", 1_000 + i),
                })
                .collect(),
            symbol: None,
            interval: None,
            session: None,
        };
        let j = panel_result_to_series_json(&t, &visible);
        assert_eq!(j["signals"][0]["t"], json!(1_003));
        assert_eq!(j["signals"][1]["t"], json!(1_007));
        assert_eq!(j["divlines"][0]["t1"], json!(1_003));
        assert_eq!(j["divlines"][0]["t2"], json!(1_005));
        assert_eq!(j["lines"][0]["values"].as_array().unwrap().len(), 5);

        // offset 0 is the identity.
        let same = trim_panel_result(&pr, 0);
        assert_eq!(same.lines[0].y, ys);
        assert_eq!(same.dots.len(), 3);
        assert_eq!(same.divlines.len(), 5);
    }

    #[test]
    fn series_warmup_keeps_histogram_colors_aligned() {
        // ADR-0026: warmup trimming cuts the per-bar colors with the values,
        // so a warmed volume histogram equals the tail of an unwarmed one.
        let call = |bars: u64, warmup: u64| -> Value {
            let args = json!({
                "ticker": "ETHUSDT", "timeframe": "4h", "bars": bars, "warmup": warmup,
                "end_time": END_4H_MS, "indicators": ["volume"], "format": "series"
            });
            serde_json::from_str(&response_text(&generate_chart(
                Some(json!(1)),
                &args,
                fetch::DataSource::Sample,
                None,
            )))
            .unwrap()
        };
        let colors = |p: &Value| -> Vec<Value> {
            let h = &p["indicators"]["volume"]["histogram"][0];
            h["color_idx"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| h["palette"][i.as_u64().unwrap() as usize].clone())
                .collect()
        };
        let warmed = call(20, 30);
        let full = call(50, 0);
        let all = colors(&full);
        assert_eq!(colors(&warmed), all[30..].to_vec());
        assert_eq!(
            warmed["indicators"]["volume"]["histogram"][0]["values"],
            json!(
                full["indicators"]["volume"]["histogram"][0]["values"]
                    .as_array()
                    .unwrap()[30..]
            )
        );
    }

    #[test]
    fn series_warmup_trims_to_bars_and_warms_indicators() {
        let args = |warmup: u64| {
            json!({
                "ticker": "BTCUSDT",
                "timeframe": "4h",
                "bars": 50,
                "warmup": warmup,
                "end_time": END_4H_MS,
                "indicators": ["rsi", "macd", "bbands", "donchian", "cipher_b"],
                "format": "series"
            })
        };
        let p = series_payload(args(100), fetch::DataSource::Sample);
        let ts = bar_ts(&p);
        assert_eq!(ts.len(), 50);
        assert!(ts.iter().all(|t| t % 14_400 == 0), "bars on the 4h grid");
        assert!(ts.windows(2).all(|w| w[1] - w[0] == 14_400));
        assert_eq!(*ts.last().unwrap() * 1000, END_4H_MS - 4 * HOUR_MS);
        assert_eq!(p["has_more"], true);

        let ind = &p["indicators"];
        let rsi = ind["rsi"]["lines"][0]["values"].as_array().unwrap();
        assert_eq!(rsi.len(), 50);
        assert!(rsi.iter().all(|v| v.is_number()));
        assert_eq!(
            ind["macd"]["histogram"][0]["values"]
                .as_array()
                .unwrap()
                .len(),
            50
        );
        let y1 = ind["bbands"]["fills"][0]["y1"].as_array().unwrap();
        assert_eq!(y1.len(), 50);
        assert!(
            y1[0].is_number(),
            "warmup must fill the first visible value"
        );
        for sig in ind["cipher_b"]["signals"].as_array().into_iter().flatten() {
            let x = sig["x"].as_u64().unwrap() as usize;
            assert!(x < 50);
            assert_eq!(sig["t"].as_i64().unwrap(), ts[x]);
        }

        // Without warmup the same window starts with pre-warmup nulls.
        let cold = series_payload(args(0), fetch::DataSource::Sample);
        assert_eq!(bar_ts(&cold), ts);
        assert!(cold["indicators"]["donchian"]["lines"][0]["values"][0].is_null());
        assert_ne!(
            cold["indicators"]["rsi"]["lines"][0]["values"][0],
            ind["rsi"]["lines"][0]["values"][0]
        );
    }

    fn ha_args(style: Value, warmup: u64, end: i64) -> Value {
        json!({
            "ticker": "ETHUSDT", "timeframe": "4h", "bars": 40, "warmup": warmup,
            "end_time": end, "indicators": ["rsi"], "format": "series",
            "chart_style": style
        })
    }

    #[test]
    fn chart_style_default_is_candles_and_unchanged() {
        let none = series_payload(
            json!({ "ticker": "ETHUSDT", "timeframe": "4h", "bars": 40,
                    "end_time": END_4H_MS, "indicators": ["rsi"], "format": "series" }),
            fetch::DataSource::Sample,
        );
        let candles = series_payload(
            ha_args(json!("candles"), 0, END_4H_MS),
            fetch::DataSource::Sample,
        );
        assert_eq!(none["chart_style"], "candles");
        assert_eq!(none, candles);
    }

    #[test]
    fn chart_style_heikin_ashi_transforms_bars_and_echoes() {
        let real = series_payload(
            ha_args(json!("candles"), 0, END_4H_MS),
            fetch::DataSource::Sample,
        );
        let ha = series_payload(
            ha_args(json!("heikin_ashi"), 0, END_4H_MS),
            fetch::DataSource::Sample,
        );
        assert_eq!(ha["chart_style"], "heikin_ashi");
        let (rb, hb) = (
            real["bars"].as_array().unwrap(),
            ha["bars"].as_array().unwrap(),
        );
        assert_eq!(rb.len(), hb.len());
        let f = |b: &Value, k: &str| b[k].as_f64().unwrap();
        // Seed bar and recurrence.
        assert!((f(&hb[0], "o") - (f(&rb[0], "o") + f(&rb[0], "c")) / 2.0).abs() < 1e-9);
        for i in 0..hb.len() {
            let r = &rb[i];
            let c = (f(r, "o") + f(r, "h") + f(r, "l") + f(r, "c")) / 4.0;
            assert!((f(&hb[i], "c") - c).abs() < 1e-9);
            if i > 0 {
                let o = (f(&hb[i - 1], "o") + f(&hb[i - 1], "c")) / 2.0;
                assert!((f(&hb[i], "o") - o).abs() < 1e-9);
            }
            assert_eq!(hb[i]["t"], r["t"]);
            assert_eq!(hb[i]["v"], r["v"]);
            assert!(f(&hb[i], "h") >= f(&hb[i], "o").max(f(&hb[i], "c")));
            assert!(f(&hb[i], "l") <= f(&hb[i], "o").min(f(&hb[i], "c")));
        }
        assert_ne!(real["bars"], ha["bars"]);
    }

    #[test]
    fn chart_style_heikin_ashi_pages_converge_after_warmup() {
        // Same window fetched as a tail (warmup 200) and as an older page
        // (shifted end_time): the HA seed difference decays by 1/2 per bar.
        let a = series_payload(
            ha_args(json!("heikin_ashi"), 200, END_4H_MS),
            fetch::DataSource::Sample,
        );
        let b = series_payload(
            ha_args(json!("heikin_ashi"), 60, END_4H_MS),
            fetch::DataSource::Sample,
        );
        assert_eq!(a["bars"].as_array().unwrap().len(), 40);
        for (x, y) in a["bars"]
            .as_array()
            .unwrap()
            .iter()
            .zip(b["bars"].as_array().unwrap())
        {
            for k in ["o", "h", "l", "c"] {
                let d = (x[k].as_f64().unwrap() - y[k].as_f64().unwrap()).abs();
                assert!(d < 1e-6, "{k} differs by {d}");
            }
        }
    }

    #[test]
    fn chart_style_rejects_unknown_value() {
        let resp = generate_chart(
            Some(json!(1)),
            &json!({ "chart_style": "renko", "format": "series" }),
            fetch::DataSource::Sample,
            None,
        );
        assert!(is_error(&resp));
        assert!(response_text(&resp).contains("chart_style"));
    }

    #[test]
    fn series_end_time_is_exclusive_and_pages_agree() {
        let page = |bars: u64, end: i64| {
            series_payload(
                json!({
                    "ticker": "ETHUSDT",
                    "timeframe": "4h",
                    "bars": bars,
                    "warmup": 30,
                    "end_time": end,
                    "indicators": ["rsi"],
                    "format": "series"
                }),
                fetch::DataSource::Sample,
            )
        };
        let a = page(60, END_4H_MS);
        let a_ts = bar_ts(&a);
        assert!(a_ts.iter().all(|t| t * 1000 < END_4H_MS));

        // Paging from the oldest loaded bar continues seamlessly.
        let b = page(60, a_ts[0] * 1000);
        let b_ts = bar_ts(&b);
        assert_eq!(*b_ts.last().unwrap() + 14_400, a_ts[0]);

        // Overlapping requests agree exactly on OHLCV.
        let c = page(60, END_4H_MS - 30 * 4 * HOUR_MS);
        let a_bars = a["bars"].as_array().unwrap();
        let c_bars = c["bars"].as_array().unwrap();
        let overlap: Vec<&Value> = c_bars
            .iter()
            .filter(|cb| a_bars.iter().any(|ab| ab == *cb))
            .collect();
        assert_eq!(overlap.len(), 30);
        assert_eq!(c_bars[30..], a_bars[..30]);
    }

    /// EMA 200 of ema_stack over `pages` history pages of 500 bars, fetched
    /// like the web client (initial load, then `end_time` = oldest bar, each
    /// with `warmup`), against one long, converged call. Returns the largest
    /// relative deviation from the converged values and the largest
    /// relative extra step at a page seam.
    fn ema200_paging_error(warmup: u64, pages: usize) -> (f64, f64) {
        const PAGE: usize = 500;
        let call = |bars: usize, warmup: u64, end: i64| {
            series_payload(
                json!({ "ticker": "BTCUSDT", "timeframe": "1m", "bars": bars,
                        "warmup": warmup, "end_time": end,
                        "indicators": ["ema_stack"], "format": "series" }),
                fetch::DataSource::Sample,
            )
        };
        let ema200 = |p: &Value| -> Vec<f64> {
            let line = &p["indicators"]["ema_stack"]["lines"][3];
            assert_eq!(line["label"], "EMA 200");
            line["values"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap_or(f64::NAN))
                .collect()
        };
        let mut merged = Vec::new();
        let mut seams = Vec::new();
        let mut end = END_4H_MS;
        for _ in 0..pages {
            let p = call(PAGE, warmup, end);
            end = bar_ts(&p)[0] * 1000;
            let mut v = ema200(&p);
            seams.push(v.len());
            v.extend(merged);
            merged = v;
        }
        // A seam follows each page but the newest; walk them oldest first.
        seams.remove(0);
        seams.reverse();
        // 4000 bars of history before the paged window: converged far
        // beyond any bound tested here.
        let reference = ema200(&call(PAGE * pages + 4000, 0, END_4H_MS));
        let reference = &reference[reference.len() - merged.len()..];
        let rel = |a: f64, b: f64| (a - b).abs() / b.abs();
        let dev = merged
            .iter()
            .zip(reference)
            .map(|(&a, &b)| rel(a, b))
            .fold(0.0, f64::max);
        let mut seam_step = 0.0f64;
        let mut s = 0;
        for len in seams {
            s += len;
            let step = (merged[s] - merged[s - 1]) - (reference[s] - reference[s - 1]);
            seam_step = seam_step.max(step.abs() / reference[s].abs());
        }
        (dev, seam_step)
    }

    #[test]
    fn ema_stack_pages_join_without_a_step() {
        // QA: on 1m BTCUSDT the EMA 200 stepped ~0.3% at history seams with
        // the web's warmup of 300 (1.9% on this sample data). The server now
        // raises it to ema_stack's 891 bars, after which the SMA seed weighs
        // < 0.1%: a seed a few % off leaves < 0.01%. Bound: 0.05%.
        let (dev, step) = ema200_paging_error(300, 4);
        eprintln!("EMA 200 paged with warmup 300: max dev {dev:e}, max seam step {step:e}");
        assert!(step < 5e-4, "seam step {step}");
        assert!(dev < 5e-4, "deviation {dev}");
    }

    #[test]
    fn per_bar_plots_reach_the_series_wire() {
        // Unlabeled dots are not serialized, so these plot lines instead.
        let p = series_payload(
            json!({ "ticker": "BTCUSDT", "timeframe": "1h", "bars": 100, "warmup": 300,
                    "end_time": END_4H_MS, "format": "series",
                    "indicators": ["sar", "kalman_volume", "heikin_ashi", "supertrend"] }),
            fetch::DataSource::Sample,
        );
        for (name, labels) in [
            ("sar", vec!["SAR Up", "SAR Down"]),
            ("kalman_volume", vec!["KVF", "Signal"]),
            ("heikin_ashi", vec!["HA Close"]),
            ("supertrend", vec!["Up Trend", "Down Trend"]),
        ] {
            let lines = p["indicators"][name]["lines"].as_array().unwrap();
            let got: Vec<&str> = lines.iter().map(|l| l["label"].as_str().unwrap()).collect();
            assert_eq!(got, labels, "{name}");
            for l in lines {
                let values = l["values"].as_array().unwrap();
                assert_eq!(values.len(), 100);
                assert!(values.iter().any(Value::is_number), "{name} {}", l["label"]);
            }
        }
    }

    #[test]
    fn summary_inactive_trend_line_has_no_last_value() {
        // Exactly one Supertrend line has a value at the last bar; the other
        // reports null, not the value it had when its trend ended.
        let pr = indicators::by_name("supertrend")
            .unwrap()
            .compute(&data::sample_data(200));
        let j = panel_result_to_json(&pr);
        let last = |i: usize| j["lines"][i]["last_value"].clone();
        assert_eq!([last(0), last(1)].iter().filter(|v| v.is_null()).count(), 1);
    }

    #[test]
    fn summary_ichimoku_lagging_span_keeps_last_finite_value() {
        // The lagging span is shifted back, so it ends in NaN by design; its
        // last_value is the last finite one. Only trend lines read the last bar.
        let pr = indicators::by_name("ichimoku")
            .unwrap()
            .compute(&data::sample_data(200));
        assert!(pr
            .lines
            .iter()
            .any(|l| l.y.last().is_some_and(|v| v.is_nan())));
        let j = panel_result_to_json(&pr);
        for (i, l) in pr.lines.iter().enumerate() {
            assert!(!l.breaks);
            if l.y.iter().any(|v| v.is_finite()) {
                assert!(j["lines"][i]["last_value"].is_number(), "line {i}");
            }
        }
    }

    #[test]
    fn warmup_bars_stay_reachable_when_history_runs_out() {
        // 10 bars of history, 5 asked for, warmup raised to 891: the 5
        // trimmed warmup bars are older history the client must page to.
        let page = |end: i64| {
            series_payload(
                json!({ "timeframe": "1h", "bars": 5, "warmup": 20, "end_time": end,
                        "indicators": ["ema_stack"], "format": "series" }),
                fetch::DataSource::Sample,
            )
        };
        let newest = page(10 * HOUR_MS);
        assert_eq!(
            bar_ts(&newest),
            (5..10).map(|h| h * 3_600).collect::<Vec<_>>()
        );
        assert_eq!(newest["has_more"], true);
        let oldest = page(bar_ts(&newest)[0] * 1000);
        assert_eq!(
            bar_ts(&oldest),
            (0..5).map(|h| h * 3_600).collect::<Vec<_>>()
        );
        assert_eq!(oldest["has_more"], false);
    }

    #[test]
    fn has_more_never_comes_with_empty_bars() {
        // bars=0 with the warmup raised: trimming would leave nothing. The
        // fetched bars come back instead, with no cursor to page on.
        let p = series_payload(
            json!({ "timeframe": "1h", "bars": 0, "warmup": 20, "end_time": 10 * HOUR_MS,
                    "indicators": ["ema_stack"], "format": "series" }),
            fetch::DataSource::Sample,
        );
        assert!(!bar_ts(&p).is_empty());
        assert_eq!(p["has_more"], false);
    }

    #[test]
    fn warmup_is_raised_to_the_indicators_need() {
        let panels = |names: Value| parse_panels(&names, None).0;
        let w =
            |warmup: u64, names: Value| warmup_arg(&json!({ "warmup": warmup }), &panels(names));
        let need = |name: &str| indicators::by_name(name).unwrap().warmup_bars();
        let declared: Vec<(&str, usize)> = indicators::registry()
            .iter()
            .map(|i| (i.name, need(i.name)))
            .filter(|&(_, n)| n > 0)
            .collect();
        // At their defaults only ema_stack, kalman_volume and the patterns' whole
        // window (ADR-0046) need more than the web's 300.
        assert_eq!(
            declared,
            [
                ("ema_stack", 891),
                ("keltner", 90),
                ("supertrend", 76),
                ("ichimoku", 77),
                ("macd", 156),
                ("rsi", 109),
                ("atr", 108),
                ("adx", 216),
                ("kalman_volume", 399),
                ("ma", 9),
                ("vwma", 9),
                ("hma", 11),
                ("ma_ribbon", 200),
                ("envelopes", 20),
                ("candlestick_patterns", 80),
                ("chart_patterns", 600),
                ("auto_fib", 600),
                ("stoch_rsi", 129),
                ("trix", 244),
                ("ppo", 94),
                ("elder_force_index", 59),
                ("tsi", 229),
                ("volume_oscillator", 45),
                ("smi_ergodic", 137),
                ("connors_rsi", 101),
            ]
        );
        // Parameters count: RSI 50 needs more than 300.
        let rsi50 = parse_panels(&json!([{ "name": "rsi", "length": 50 }]), None).0;
        assert_eq!(rsi50[0].warmup_bars(), 393);
        assert_eq!(w(300, json!(["ema_stack", "rsi"])), 891);
        assert_eq!(w(950, json!(["ema_stack"])), 950);
        assert_eq!(w(300, json!(["rsi"])), 300);
        // Capped, and warmup 0 (no warmup asked) stays 0.
        let long_ema = json!([{ "name": "ma", "type": "ema", "length": 1000 }]);
        assert_eq!(w(300, long_ema.clone()), MAX_WARMUP);
        assert_eq!(w(0, long_ema), 0);
        assert_eq!(w(5000, json!([])), MAX_WARMUP);
    }

    #[test]
    fn series_has_more_false_when_history_runs_out() {
        let p = series_payload(
            json!({
                "timeframe": "1h",
                "bars": 50,
                "warmup": 20,
                "end_time": 10 * HOUR_MS,
                "indicators": ["rsi"],
                "format": "series"
            }),
            fetch::DataSource::Sample,
        );
        assert_eq!(bar_ts(&p), (0..10).map(|h| h * 3_600).collect::<Vec<_>>());
        assert_eq!(p["has_more"], false);

        // Past the start of history: empty payload, not an error.
        let p = series_payload(
            json!({ "timeframe": "1h", "bars": 5, "end_time": 0,
                    "indicators": ["rsi", "vpvr"], "format": "series" }),
            fetch::DataSource::Sample,
        );
        assert!(p["bars"].as_array().unwrap().is_empty());
        assert_eq!(p["has_more"], false);

        // Exactly the requested count available → more may exist.
        let p = series_payload(
            json!({ "timeframe": "1h", "bars": 6, "warmup": 4, "end_time": 10 * HOUR_MS,
                    "indicators": ["rsi"], "format": "series" }),
            fetch::DataSource::Sample,
        );
        assert_eq!(p["bars"].as_array().unwrap().len(), 6);
        assert_eq!(p["has_more"], true);
    }

    #[test]
    fn series_tail_poll_returns_forming_bar() {
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let p = series_payload(
            json!({ "ticker": "BTCUSDT", "timeframe": "1h", "bars": 2, "warmup": 50,
                    "indicators": ["rsi"], "format": "series" }),
            fetch::DataSource::Sample,
        );
        let ts = bar_ts(&p);
        assert_eq!(ts.len(), 2);
        // Newest bar is the one forming now (allow an hour rollover mid-test).
        let current = before / HOUR_MS * HOUR_MS / 1000;
        assert!(ts[1] == current || ts[1] == current + 3_600);
        let rsi = p["indicators"]["rsi"]["lines"][0]["values"]
            .as_array()
            .unwrap();
        assert!(rsi.iter().all(|v| v.is_number()));
        assert_eq!(p["has_more"], true);
    }

    #[test]
    fn series_hbars_cover_visible_window_only() {
        let args = |warmup: u64| {
            json!({ "ticker": "BTCUSDT", "timeframe": "4h", "bars": 40, "warmup": warmup,
                    "end_time": END_4H_MS, "indicators": ["vpvr"], "format": "series" })
        };
        let warm = series_payload(args(200), fetch::DataSource::Sample);
        let cold = series_payload(args(0), fetch::DataSource::Sample);
        let hbars = &warm["indicators"]["vpvr"]["hbars"];
        assert!(!hbars.as_array().unwrap().is_empty());
        assert_eq!(hbars, &cold["indicators"]["vpvr"]["hbars"]);
        assert_eq!(
            warm["indicators"]["vpvr"]["hlines"],
            cold["indicators"]["vpvr"]["hlines"]
        );
    }

    #[test]
    fn series_profile_hlines_cover_visible_window_without_hbars() {
        // hvn_lvn derives node hlines from a volume profile but emits no hbars.
        let args = |warmup: u64| {
            json!({ "ticker": "BTCUSDT", "timeframe": "4h", "bars": 60, "warmup": warmup,
                    "end_time": END_4H_MS, "indicators": ["hvn_lvn", "rsi"], "format": "series" })
        };
        let warm = series_payload(args(300), fetch::DataSource::Sample);
        let cold = series_payload(args(0), fetch::DataSource::Sample);
        let nodes = &warm["indicators"]["hvn_lvn"]["hlines"];
        assert!(!nodes.as_array().unwrap().is_empty());
        assert_eq!(nodes, &cold["indicators"]["hvn_lvn"]["hlines"]);
        // Non-profile hlines (RSI 30/70) are untouched.
        assert_eq!(
            warm["indicators"]["rsi"]["hlines"],
            cold["indicators"]["rsi"]["hlines"]
        );
        let windowed: Vec<&str> = indicators::registry()
            .iter()
            .filter(|i| indicators::by_name(i.name).is_some_and(|ind| ind.windowed_profile()))
            .map(|i| i.name)
            .collect();
        assert_eq!(
            windowed,
            vec![
                "volume_profile",
                "fixed_range_vp",
                "anchored_vp",
                "hvn_lvn",
                "tpo"
            ]
        );
    }

    #[test]
    fn end_time_must_be_an_in_range_integer() {
        for bad in [
            json!("1700000000000"),
            json!(1.7e12),
            json!(1_700_000_000_000.5),
            json!(u64::MAX),
            json!(-2_208_988_800_001_i64),
            json!(fetch::now_ms() + 400 * 86_400_000),
        ] {
            let resp = generate_chart(
                Some(json!(1)),
                &json!({ "ticker": "BTCUSDT", "end_time": bad, "format": "series" }),
                fetch::DataSource::Sample,
                None,
            );
            assert!(is_error(&resp), "end_time {bad} must be rejected");
            assert!(response_text(&resp).contains("end_time"));
        }
        // Boundaries and null are accepted (pre-1970 just has no sample bars).
        for ok in [json!(null), json!(-2_208_988_800_000_i64), json!(END_4H_MS)] {
            series_payload(
                json!({ "ticker": "BTCUSDT", "end_time": ok, "bars": 5, "format": "series" }),
                fetch::DataSource::Sample,
            );
        }
    }

    #[test]
    fn unsupported_timeframe_is_a_tool_error() {
        for tf in ["4611686018427387904m", "0h", "61m", "53w", "1x"] {
            let resp = generate_chart(
                Some(json!(1)),
                &json!({ "ticker": "BTCUSDT", "timeframe": tf, "format": "series" }),
                fetch::DataSource::Sample,
                None,
            );
            assert!(is_error(&resp), "timeframe {tf} must be rejected");
            assert!(response_text(&resp).contains("Unsupported timeframe"));
        }
    }

    #[test]
    fn a_tails_future_times_match_the_full_load() {
        // A 24/7 daily chart on a Friday: the two-bar tail shows no weekend bar,
        // but its future times must step through weekends, as the full load's do.
        let friday_ms = 1_793_937_600_000; // 2026-11-06T04:00:00Z
        let mut raw = data::sample_ohlcv("BTCUSDT", "1d", 150, None, friday_ms).unwrap();
        raw.symbol = Some("BTCUSDT".into());
        raw.interval = Some("1d".into());
        let t = |bars: usize| -> Vec<i64> {
            let v = series_tail(&raw, &json!(["ichimoku"]), "candles", bars).unwrap();
            serde_json::from_value(v["indicators"]["ichimoku"]["future"]["t"].clone()).unwrap()
        };
        let (full, tail) = (t(100), t(2));
        assert_eq!(full.len(), 25);
        assert_eq!(tail, full);
        assert!(tail.windows(2).all(|w| w[1] - w[0] == 86_400), "{tail:?}");
    }

    #[test]
    fn custom_timeframes_are_echoed_canonically_on_their_grid() {
        for (asked, canonical, secs) in [
            ("7m", "7m", 420),
            ("45m", "45m", 2_700),
            ("90m", "90m", 5_400),
            ("120m", "2h", 7_200),
            ("8h", "8h", 28_800),
            ("2D", "2d", 172_800),
            ("2W", "2wk", 1_209_600),
        ] {
            let resp = generate_chart(
                Some(json!(1)),
                &json!({ "ticker": "BTCUSDT", "timeframe": asked, "format": "series", "bars": 5 }),
                fetch::DataSource::Sample,
                None,
            );
            assert!(!is_error(&resp), "{asked}: {}", response_text(&resp));
            let v: Value = serde_json::from_str(&response_text(&resp)).unwrap();
            assert_eq!(v["interval"], canonical, "{asked}");
            let iv = chartgen::interval::Interval::parse(asked).unwrap();
            let t: Vec<i64> = v["bars"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| b["t"].as_i64().unwrap())
                .collect();
            assert_eq!(t.len(), 5, "{asked}");
            assert!(t.windows(2).all(|w| w[1] - w[0] == secs), "{asked}: {t:?}");
            assert!(t.iter().all(|x| iv.grid_start(*x) == *x), "{asked}: {t:?}");
        }
    }

    #[test]
    fn fetch_failure_with_ticker_errors_in_every_format() {
        for format in ["png", "summary", "both", "series"] {
            for end_time in [Value::Null, json!(END_4H_MS)] {
                let resp = generate_chart(
                    Some(json!(1)),
                    &json!({ "ticker": "AAPL", "bars": 20, "format": format,
                             "end_time": end_time, "width": 200, "height": 100 }),
                    fetch::DataSource::Unavailable,
                    None,
                );
                assert!(
                    is_error(&resp),
                    "{format}: must not fall back to sample data"
                );
                assert!(response_text(&resp).contains("upstream unavailable"));
            }
        }
    }

    #[test]
    fn warmup_ignored_for_non_series_formats() {
        let text = |args: Value| {
            let resp = generate_chart(Some(json!(1)), &args, fetch::DataSource::Live, None);
            chart_content(&resp)[0]["text"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let base = json!({ "bars": 30, "indicators": ["rsi"], "format": "summary" });
        let mut warm = base.clone();
        warm["warmup"] = json!(500);
        assert_eq!(text(base), text(warm));
    }

    #[test]
    fn end_time_without_ticker_errors_on_live_source() {
        let resp = generate_chart(
            Some(json!(1)),
            &json!({ "end_time": 1_700_000_000_000_i64, "format": "series" }),
            fetch::DataSource::Live,
            None,
        );
        assert!(is_error(&resp));
        assert!(response_text(&resp).contains("end_time"));
    }

    #[test]
    fn legacy_sample_series_has_more_false_and_bars_clamped() {
        // No ticker, live source: synthetic sample_data has no timestamps to page from.
        let p = series_payload(
            json!({ "bars": 20, "warmup": 10, "indicators": ["rsi"], "format": "series" }),
            fetch::DataSource::Live,
        );
        assert_eq!(p["bars"].as_array().unwrap().len(), 20);
        assert_eq!(p["has_more"], false);

        let p = series_payload(
            json!({ "timeframe": "1m", "bars": 100_000, "indicators": [], "format": "series" }),
            fetch::DataSource::Sample,
        );
        assert_eq!(p["bars"].as_array().unwrap().len(), MAX_BARS);
    }

    // --- get_quotes ---

    fn quotes(args: Value, source: fetch::DataSource) -> Vec<Value> {
        let resp = get_quotes(Some(json!(1)), &args, source);
        assert!(!is_error(&resp), "{}", response_text(&resp));
        let p: Value = serde_json::from_str(&response_text(&resp)).unwrap();
        p["quotes"].as_array().unwrap().clone()
    }

    #[test]
    fn get_quotes_sample_normalizes_dedupes_and_computes_change() {
        let q = quotes(
            json!({ "symbols": [" btcusdt", "AAPL", "BTCUSDT"] }),
            fetch::DataSource::Sample,
        );
        assert_eq!(q.len(), 2);
        assert_eq!(q[0]["symbol"], "BTCUSDT");
        assert_eq!(q[1]["symbol"], "AAPL");
        for e in &q {
            assert_eq!(e["source"], "sample");
            let (last, prev) = (
                e["last"].as_f64().unwrap(),
                e["prev_close"].as_f64().unwrap(),
            );
            let change = e["change"].as_f64().unwrap();
            assert!((change - (last - prev)).abs() < 1e-9);
            assert!((e["change_pct"].as_f64().unwrap() - change / prev * 100.0).abs() < 1e-9);
            for k in ["high", "low", "volume"] {
                assert!(e[k].is_f64(), "{k}");
            }
        }
    }

    #[test]
    fn get_quotes_errors_per_symbol_without_failing_the_call() {
        let q = quotes(
            json!({ "symbols": ["BTCUSDT", "not a symbol!", ""] }),
            fetch::DataSource::Sample,
        );
        assert_eq!(q.len(), 3);
        assert!(q[0]["last"].is_f64());
        assert_eq!(q[1]["symbol"], "not a symbol!");
        assert_eq!(q[1]["error"], "invalid symbol");
        assert_eq!(q[2]["error"], "invalid symbol");

        // No path separators, and never a dot-only (`.`/`..`) segment.
        let q = quotes(
            json!({ "symbols": ["../../X", "A/B", "..", "BRK.B", "^GSPC", "EURUSD=X"] }),
            fetch::DataSource::Sample,
        );
        for bad in &q[..3] {
            assert_eq!(bad["error"], "invalid symbol", "{bad}");
        }
        for good in &q[3..] {
            assert!(good["last"].is_f64(), "{good}");
        }

        let q = quotes(
            json!({ "symbols": ["BTCUSDT", "AAPL"] }),
            fetch::DataSource::Unavailable,
        );
        assert_eq!(q.len(), 2);
        assert!(q
            .iter()
            .all(|e| e["error"] == "upstream unavailable" && e.get("last").is_none()));
    }

    #[test]
    fn get_quotes_rejects_bad_input_and_more_than_fifty_symbols() {
        for args in [
            json!({}),
            json!({ "symbols": "BTCUSDT" }),
            json!({ "symbols": [1] }),
        ] {
            let resp = get_quotes(Some(json!(1)), &args, fetch::DataSource::Sample);
            assert!(is_error(&resp), "{args}");
        }
        let many: Vec<String> = (0..51).map(|i| format!("S{i}")).collect();
        let resp = get_quotes(
            Some(json!(1)),
            &json!({ "symbols": many }),
            fetch::DataSource::Sample,
        );
        assert!(is_error(&resp));
        assert!(response_text(&resp).contains("At most 50"));
        // The cap applies to the raw array, before any entry is processed:
        // an oversized array is rejected even if it is all duplicates or
        // holds a non-string entry past the limit.
        let mut dupes: Vec<Value> = std::iter::repeat_n(json!("BTCUSDT"), 60).collect();
        dupes.push(json!(1));
        let resp = get_quotes(
            Some(json!(1)),
            &json!({ "symbols": dupes }),
            fetch::DataSource::Sample,
        );
        assert!(response_text(&resp).contains("At most 50"));
        let fifty: Vec<&str> = std::iter::repeat_n("BTCUSDT", 50).collect();
        assert_eq!(
            quotes(json!({ "symbols": fifty }), fetch::DataSource::Sample).len(),
            1
        );
        assert!(quotes(json!({ "symbols": [] }), fetch::DataSource::Sample).is_empty());
    }

    #[test]
    fn get_quotes_is_dispatched_and_listed() {
        let resp = handle_mcp_request("tools/list", Some(json!(1)), None, None, None, None, None);
        let tools = resp["result"]["tools"].as_array().unwrap();
        let tool = tools.iter().find(|t| t["name"] == "get_quotes").unwrap();
        assert_eq!(tool["inputSchema"]["required"], json!(["symbols"]));
        assert_eq!(tool["inputSchema"]["properties"]["symbols"]["maxItems"], 50);

        let resp = handle_mcp_request(
            "tools/call",
            Some(json!(2)),
            Some(&json!({ "name": "get_quotes", "arguments": {} })),
            None,
            None,
            None,
            None,
        );
        assert!(response_text(&resp).contains("symbols is required"));
    }

    // --- search_symbols ---

    fn search(args: Value) -> Value {
        search_symbols(Some(json!(1)), &args, fetch::DataSource::Sample)
    }

    #[test]
    fn search_symbols_returns_ranked_results_with_contract_fields() {
        let resp = search(json!({ "query": " spy " }));
        assert!(!is_error(&resp), "{}", response_text(&resp));
        let p: Value = serde_json::from_str(&response_text(&resp)).unwrap();
        let first = &p["results"][0];
        assert_eq!(first["symbol"], "SPY");
        assert_eq!(first["type"], "etf");
        assert_eq!(first["source"], "sample");
        assert!(first["name"].is_string() && first["exchange"].is_string());
        assert!(p.get("warnings").is_none());
    }

    #[test]
    fn search_symbols_honours_limit_and_types() {
        let resp = search(json!({ "query": "a", "limit": 2 }));
        let p: Value = serde_json::from_str(&response_text(&resp)).unwrap();
        assert_eq!(p["results"].as_array().unwrap().len(), 2);

        let resp = search(json!({ "query": "e", "types": ["forex"] }));
        let p: Value = serde_json::from_str(&response_text(&resp)).unwrap();
        let r = p["results"].as_array().unwrap();
        assert!(!r.is_empty() && r.iter().all(|x| x["type"] == "forex"));
    }

    #[test]
    fn search_symbols_rejects_bad_input() {
        for args in [
            json!({}),
            json!({ "query": 5 }),
            json!({ "query": "" }),
            json!({ "query": "a\u{7}" }),
            json!({ "query": "a", "limit": 0 }),
            json!({ "query": "a", "limit": 51 }),
            json!({ "query": "a", "limit": "x" }),
            json!({ "query": "a", "types": "etf" }),
            json!({ "query": "a", "types": [1] }),
            json!({ "query": "a", "types": ["bogus"] }),
        ] {
            assert!(is_error(&search(args.clone())), "{args}");
        }
    }

    #[test]
    fn search_symbols_is_dispatched_and_listed() {
        let resp = handle_mcp_request("tools/list", Some(json!(1)), None, None, None, None, None);
        let tools = resp["result"]["tools"].as_array().unwrap();
        let tool = tools
            .iter()
            .find(|t| t["name"] == "search_symbols")
            .unwrap();
        assert_eq!(tool["inputSchema"]["required"], json!(["query"]));

        let resp = handle_mcp_request(
            "tools/call",
            Some(json!(2)),
            Some(&json!({ "name": "search_symbols", "arguments": {} })),
            None,
            None,
            None,
            None,
        );
        assert!(response_text(&resp).contains("query is required"));
    }

    // --- subscribe_chart / unsubscribe_chart (ADR-0021) ---

    fn call(name: &str, args: Value, push: Option<&PushCtx>, token: Option<&str>) -> Value {
        handle_mcp_request(
            "tools/call",
            Some(json!(1)),
            Some(&json!({ "name": name, "arguments": args })),
            None,
            token,
            push,
            None,
        )
    }

    #[test]
    fn subscribe_chart_needs_the_http_transport() {
        let resp = call(
            "subscribe_chart",
            json!({ "ticker": "BTCUSDT" }),
            None,
            None,
        );
        assert!(is_error(&resp));
        assert!(response_text(&resp).contains("HTTP transport"));
        let resp = call(
            "unsubscribe_chart",
            json!({ "subscription_id": "x" }),
            None,
            None,
        );
        assert!(is_error(&resp));
    }

    #[tokio::test]
    async fn subscribe_chart_validates_and_round_trips() {
        let hub = crate::chart_push::ChartHub::new(
            Arc::new(crate::chart_push::SampleSource),
            crate::chart_push::Limits::default(),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(crate::chart_push::STREAM_BUFFER);
        hub.link_stream("s", tx);
        let push = PushCtx {
            hub: hub.clone(),
            stream: "s".into(),
            client: "127.0.0.1".into(),
        };
        let ok = |args: Value| {
            let resp = call("subscribe_chart", args, Some(&push), Some("tok"));
            (is_error(&resp), response_text(&resp))
        };
        for (args, needle) in [
            (json!({}), "ticker is required"),
            (
                json!({ "ticker": "BTCUSDT", "timeframe": "7x" }),
                "Unsupported timeframe",
            ),
            (
                json!({ "ticker": "BTCUSDT", "indicators": ["nope"] }),
                "Unknown indicator",
            ),
            (
                json!({ "ticker": "BTCUSDT", "chart_style": "renko" }),
                "chart_style",
            ),
            (json!({ "ticker": "AAPL" }), "not pushable"),
            (
                json!({ "ticker": "BTCUSDT", "indicators": vec!["rsi"; 11] }),
                "at most 10 indicators",
            ),
        ] {
            let (err, text) = ok(args.clone());
            assert!(err && text.contains(needle), "{args}: {text}");
        }

        let (err, text) = ok(json!({
            "ticker": "BTCUSDT", "timeframe": "1m", "indicators": ["rsi"], "warmup": 50,
            "chart_style": "heikin_ashi"
        }));
        assert!(!err, "{text}");
        let id = serde_json::from_str::<Value>(&text).unwrap()["subscription_id"]
            .as_str()
            .unwrap()
            .to_string();
        let note: Value = serde_json::from_str(
            &tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(note["params"]["subscription_id"], id.as_str());
        assert_eq!(note["params"]["series"]["chart_style"], "heikin_ashi");
        assert!(note["params"]["series"]["indicators"]["rsi"].is_object());

        let resp = call(
            "unsubscribe_chart",
            json!({ "subscription_id": id }),
            Some(&push),
            Some("tok"),
        );
        assert_eq!(
            serde_json::from_str::<Value>(&response_text(&resp)).unwrap()["unsubscribed"],
            true
        );
        assert_eq!(hub.counts().0, 0);
    }

    #[test]
    fn series_tail_matches_generate_chart() {
        // The pushed tail is the generate_chart tail for the same window.
        let args = json!({
            "ticker": "ETHUSDT", "timeframe": "4h", "bars": 2, "warmup": 40,
            "end_time": END_4H_MS, "indicators": ["rsi", "macd"], "format": "series",
            "chart_style": "heikin_ashi"
        });
        let polled: Value = serde_json::from_str(&response_text(&generate_chart(
            Some(json!(1)),
            &args,
            fetch::DataSource::Sample,
            None,
        )))
        .unwrap();
        // The group's window: the tail plus the raised warmup (ADR-0032).
        let indicators = json!(["rsi", "macd"]);
        let warmup = warmup_arg(&args, &parse_panels(&indicators, None).0);
        let mut raw = fetch::fetch_ohlcv(
            fetch::DataSource::Sample,
            "ETHUSDT",
            "4h",
            2 + warmup,
            Some(END_4H_MS),
        )
        .unwrap()
        .data;
        raw.symbol = Some("ETHUSDT".into());
        raw.interval = Some("4h".into());
        // The polled value went through text. Parse the pushed one the same
        // way: without starlark's `arbitrary_precision`, serde_json's float
        // parser can land 1 ulp off the computed f64.
        let pushed = series_tail(&raw, &indicators, "heikin_ashi", 2).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&pushed.to_string()).unwrap(),
            polled
        );
    }

    #[test]
    fn series_tail_carries_the_ichimoku_projection() {
        // ADR-0033: polled and pushed tails carry the cloud past the last
        // bar, and a forming-bar update moves it.
        let indicators = json!(["ichimoku"]);
        let args = json!({
            "ticker": "ETHUSDT", "timeframe": "4h", "bars": 2, "warmup": 40,
            "end_time": END_4H_MS, "indicators": indicators, "format": "series"
        });
        let polled: Value = serde_json::from_str(&response_text(&generate_chart(
            Some(json!(1)),
            &args,
            fetch::DataSource::Sample,
            None,
        )))
        .unwrap();
        let warmup = warmup_arg(&args, &parse_panels(&indicators, None).0);
        assert_eq!(warmup, 52 + 25, "ichimoku raises the warmup to its lead");
        let mut raw = fetch::fetch_ohlcv(
            fetch::DataSource::Sample,
            "ETHUSDT",
            "4h",
            2 + warmup,
            Some(END_4H_MS),
        )
        .unwrap()
        .data;
        raw.symbol = Some("ETHUSDT".into());
        raw.interval = Some("4h".into());
        let pushed = series_tail(&raw, &indicators, "candles", 2).unwrap();
        // Parse the pushed tail like the polled one (see the test above).
        assert_eq!(
            serde_json::from_str::<Value>(&pushed.to_string()).unwrap(),
            polled
        );
        let ichimoku = indicators::by_name("ichimoku").unwrap();
        let want = |data: &chartgen::data::OhlcvData| {
            let p = ichimoku.projection(data).unwrap();
            let vals = |v: &[f64]| v.iter().map(|x| finite_or_null(*x)).collect::<Vec<_>>();
            (vals(&p.lines[0].1), vals(&p.lines[1].1))
        };
        let fut = &pushed["indicators"]["ichimoku"]["future"];
        assert_eq!(fut["bars"], 25);
        let (a, b) = want(&raw);
        assert_eq!(fut["lines"][0]["index"], 2);
        assert_eq!(fut["lines"][0]["values"], json!(a));
        assert_eq!(fut["lines"][1]["values"], json!(b));
        assert_eq!(fut["fills"][0]["y1"], json!(a));
        assert_eq!(fut["fills"][0]["y2"], json!(b));
        // The whole window ends on the same bar: same projection.
        let full = series_tail(&raw, &indicators, "candles", raw.len()).unwrap();
        assert_eq!(&full["indicators"]["ichimoku"]["future"], fut);
        let last: i64 = raw.bars.last().unwrap().date.parse().unwrap();
        let t = fut["t"].as_array().unwrap();
        assert_eq!(t.len(), 25);
        assert_eq!(
            (t[0].as_i64(), t[24].as_i64()),
            (Some(last + 14_400), Some(last + 25 * 14_400))
        );
        // A forming bar that makes a new high moves the newest projected spans.
        raw.bars.last_mut().unwrap().high *= 1.5;
        let moved = series_tail(&raw, &indicators, "candles", 2).unwrap();
        let fut2 = &moved["indicators"]["ichimoku"]["future"];
        assert_ne!(
            fut2["lines"][0]["values"][24],
            fut["lines"][0]["values"][24]
        );
        assert_eq!(fut2["lines"][0]["values"], json!(want(&raw).0));
    }

    #[test]
    fn series_tail_carries_plot_colors_like_generate_chart() {
        // ADR-0026: pushed tails carry the same line colors and per-bar
        // histogram colors as polled ones.
        let indicators = json!(["volume", "macd", "ema"]);
        let args = json!({
            "ticker": "ETHUSDT", "timeframe": "4h", "bars": 2, "warmup": 40,
            "end_time": END_4H_MS, "indicators": indicators, "format": "series"
        });
        let polled: Value = serde_json::from_str(&response_text(&generate_chart(
            Some(json!(1)),
            &args,
            fetch::DataSource::Sample,
            None,
        )))
        .unwrap();
        // A group's window is the tail plus the effective warmup (ADR-0032):
        // ema needs more than the 40 asked.
        let warmup = warmup_arg(&args, &parse_panels(&indicators, None).0);
        assert!(warmup > 40);
        let mut raw = fetch::fetch_ohlcv(
            fetch::DataSource::Sample,
            "ETHUSDT",
            "4h",
            2 + warmup,
            Some(END_4H_MS),
        )
        .unwrap()
        .data;
        raw.symbol = Some("ETHUSDT".into());
        raw.interval = Some("4h".into());
        let pushed = series_tail(&raw, &indicators, "candles", 2).unwrap();
        assert_eq!(pushed, polled);
        let ind = &pushed["indicators"];
        for h in [&ind["volume"]["histogram"][0], &ind["macd"]["histogram"][0]] {
            assert_eq!(h["color_idx"].as_array().unwrap().len(), 2);
            assert!(!h["palette"].as_array().unwrap().is_empty());
        }
        assert_eq!(ind["macd"]["histogram"][0]["label"], "Histogram");
        assert!(ind["ema"]["lines"][0]["color"]
            .as_str()
            .unwrap()
            .starts_with("rgba("));
    }
}

/// Scripted indicators over MCP (ADR-0025): register → list → chart →
/// delete, per-user isolation, shadowing and per-indicator errors.
#[cfg(all(test, feature = "scripting"))]
mod scripting_tests {
    use super::*;

    /// One store per test process (the store is process-wide); tests use
    /// distinct users, so they don't see each other's scripts.
    fn user() -> String {
        let path = std::env::temp_dir().join(format!("chartgen-mcp-{}.db", std::process::id()));
        scripting::persistence::open_global(&path).expect("test store");
        format!("client-{}", uuid::Uuid::new_v4())
    }

    fn call(tool: &str, args: Value, user: Option<&str>) -> Value {
        call_tool(tool, Some(json!(1)), args, None, None, None, user, true)
    }

    fn text(resp: &Value, i: usize) -> &str {
        resp["result"]["content"][i]["text"].as_str().unwrap_or("")
    }

    fn payload(resp: &Value) -> Value {
        assert!(
            !resp["result"]["isError"].as_bool().unwrap_or(false),
            "unexpected error: {}",
            text(resp, 0)
        );
        serde_json::from_str(text(resp, 0)).unwrap()
    }

    fn chart(user: &str, indicators: Value, format: &str) -> Value {
        let args = json!({ "bars": 200, "indicators": indicators, "format": format });
        generate_chart(Some(json!(1)), &args, fetch::DataSource::Sample, Some(user))
    }

    const MOMENTUM: &str = r#"
PARAMS = [{"name": "length", "type": "int", "default": 10, "min": 1, "max": 500}]
META = {"title": "Momentum", "overlay": False}

def compute(ohlcv, params, chart):
    m = ohlcv.close - ohlcv.close[params["length"]]
    chart.line(m, title = "MOM", color = "teal")
    chart.dot(ta.crossover(m, 0), m, color = "green", label = "up")
    chart.hline(0)
"#;

    /// The error of `key` in a series response, which must not have failed.
    fn key_error_of(resp: &Value, key: &str) -> Value {
        payload(resp)["indicator_errors"][key].clone()
    }

    #[test]
    fn an_unknown_name_fails_only_its_key_for_a_user() {
        let u = user();
        let resp = chart(
            &u,
            json!(["gone", "rsi", { "name": "gone", "id": "gone#2" }]),
            "series",
        );
        assert_eq!(key_error_of(&resp, "gone")["step"], "unknown");
        assert_eq!(key_error_of(&resp, "gone#2")["step"], "unknown");
        assert!(payload(&resp)["indicators"]["rsi"].is_object());
        assert!(payload(&resp)["indicators"].get("gone").is_none());

        // png has no JSON payload: the errors come as a text item.
        let png = chart(&u, json!(["gone", "rsi"]), "png");
        assert_eq!(png["result"]["content"][0]["type"], "image");
        assert!(text(&png, 1).contains("Unknown indicator: gone"));

        // Without a user (stdio, scripting off) the request fails, as before.
        let resp = generate_chart(
            Some(json!(1)),
            &json!({"indicators": ["gone", "rsi"], "format": "series"}),
            fetch::DataSource::Sample,
            None,
        );
        assert_eq!(resp["result"]["isError"], true);
        assert!(text(&resp, 0).contains("Unknown indicator(s): gone"));
    }

    #[test]
    fn params_a_saved_script_no_longer_allows_fail_only_its_key() {
        let u = user();
        payload(&call(
            "register_indicator",
            json!({"name": "narrowed", "source": MOMENTUM}),
            Some(&u),
        ));
        let resp = chart(
            &u,
            json!([{ "name": "narrowed", "length": 9999 }, "rsi"]),
            "series",
        );
        let err = key_error_of(&resp, "narrowed");
        assert_eq!(err["step"], "params");
        assert!(err["message"].as_str().unwrap().contains("length"), "{err}");
        assert!(payload(&resp)["indicators"]["rsi"].is_object());

        // A built-in's bad params still fail the whole request.
        let resp = chart(&u, json!([{ "name": "rsi", "length": 0 }]), "series");
        assert_eq!(resp["result"]["isError"], true);
    }

    #[tokio::test]
    async fn a_chart_with_scripts_is_not_pushable() {
        let u = user();
        payload(&call(
            "register_indicator",
            json!({"name": "rsi", "source": MOMENTUM}),
            Some(&u),
        ));
        let hub = crate::chart_push::ChartHub::new(
            Arc::new(crate::chart_push::SampleSource),
            crate::chart_push::Limits::default(),
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(crate::chart_push::STREAM_BUFFER);
        hub.link_stream("s", tx);
        let push = PushCtx {
            hub,
            stream: "s".into(),
            client: "127.0.0.1".into(),
        };
        let subscribe = |indicators: Value, user: Option<&str>| {
            call_tool(
                "subscribe_chart",
                Some(json!(1)),
                json!({"ticker": "BTCUSDT", "indicators": indicators}),
                None,
                Some("tok"),
                Some(&push),
                user,
                true,
            )
        };
        // A script shadowing a built-in, under either entry form.
        for indicators in [json!(["rsi"]), json!([{ "name": "rsi", "length": 5 }])] {
            let resp = subscribe(indicators, Some(&u));
            assert!(text(&resp, 0).starts_with(NOT_PUSHABLE), "{resp}");
        }
        // A name the server no longer knows: polled, with its error per key.
        let resp = subscribe(json!(["gone"]), Some(&u));
        assert!(text(&resp, 0).starts_with(NOT_PUSHABLE), "{resp}");
        // Built-ins only: pushed, and another user's shadow does not count.
        let resp = subscribe(json!(["macd"]), Some(&u));
        assert!(!text(&resp, 0).contains(NOT_PUSHABLE), "{resp}");
        let resp = subscribe(json!(["rsi"]), Some(&user()));
        assert!(!text(&resp, 0).contains(NOT_PUSHABLE), "{resp}");
    }

    #[test]
    fn create_only_never_replaces_a_saved_script() {
        let u = user();
        let register = |source: &str, create_only: bool| {
            call(
                "register_indicator",
                json!({"name": "mine", "source": source, "create_only": create_only}),
                Some(&u),
            )
        };
        let other = MOMENTUM.replace("Momentum", "Other");

        assert_eq!(payload(&register(MOMENTUM, true))["replaced"], false);
        let refused = register(&other, true);
        assert_eq!(refused["result"]["isError"], true);
        let body: Value = serde_json::from_str(text(&refused, 0)).unwrap();
        assert_eq!(body["errors"][0]["step"], "exists");
        let got = payload(&call("get_my_indicator", json!({"name": "mine"}), Some(&u)));
        assert_eq!(got["source"], MOMENTUM, "the saved text must survive");

        // Without the flag, saving replaces as before.
        assert_eq!(payload(&register(&other, false))["replaced"], true);
    }

    #[test]
    fn tools_listed_only_when_enabled() {
        let names = |on: bool| -> Vec<String> {
            handle_tools_list(Some(json!(1)), on)["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect()
        };
        let scripting = [
            "register_indicator",
            "fork_indicator",
            "list_my_indicators",
            "get_my_indicator",
            "delete_indicator",
        ];
        assert!(scripting
            .iter()
            .all(|t| !names(false).contains(&t.to_string())));
        assert!(scripting
            .iter()
            .all(|t| names(true).contains(&t.to_string())));
        // codegen input never has them
        let base = chartgen::mcp_schema::tools_list_result().to_string();
        assert!(!base.contains("register_indicator"));
        // disabled: the tools are unknown
        let u = user();
        let resp = call_tool(
            "register_indicator",
            None,
            json!({"name": "x", "source": MOMENTUM}),
            None,
            None,
            None,
            Some(&u),
            false,
        );
        assert!(text(&resp, 0).contains("Unknown tool"));
    }

    #[test]
    fn register_list_chart_delete() {
        let u = user();
        let reg = payload(&call(
            "register_indicator",
            json!({"name": "my_momentum", "source": MOMENTUM}),
            Some(&u),
        ));
        assert_eq!(reg["ok"], true);
        assert_eq!(reg["replaced"], false);
        assert_eq!(reg["configurable_params"][0]["type"], "integer");

        let mine = payload(&call("list_my_indicators", json!({}), Some(&u)));
        assert_eq!(mine["total"], 1);
        assert_eq!(mine["indicators"][0]["name"], "my_momentum");
        assert_eq!(mine["indicators"][0]["source"], "user");

        // The stored source reads back for its owner, and only for its owner.
        let got = payload(&call(
            "get_my_indicator",
            json!({"name": "my_momentum"}),
            Some(&u),
        ));
        assert_eq!(got["source"], MOMENTUM);
        let other = call(
            "get_my_indicator",
            json!({"name": "my_momentum"}),
            Some("someone_else"),
        );
        assert_eq!(other["result"]["isError"], true);
        assert!(!other.to_string().contains("compute"), "{other}");
        let missing: Value = serde_json::from_str(text(&other, 0)).unwrap();
        assert_eq!(missing["error"], "not_found");

        let all = payload(&tool_list_indicators(Some(json!(1)), Some(&u)));
        let entry = all["indicators"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["name"] == "my_momentum")
            .expect("listed for its owner");
        assert_eq!(entry["source"], "user");
        assert_eq!(entry["type"], "panel");
        assert_eq!(
            all["total"].as_u64().unwrap() as usize,
            indicators::registry().len() + 1
        );

        // used like a built-in, with params, next to a built-in
        let resp = chart(
            &u,
            json!([{"name": "my_momentum", "length": 3}, "rsi"]),
            "series",
        );
        let p = payload(&resp);
        let mom = &p["indicators"]["my_momentum"];
        assert_eq!(mom["label"], "Momentum");
        let values = mom["lines"][0]["values"].as_array().unwrap();
        assert_eq!(values.len(), 200);
        assert!(values[..3].iter().all(|v| v.is_null()));
        let c: Vec<f64> = p["bars"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["c"].as_f64().unwrap())
            .collect();
        assert!((values[10].as_f64().unwrap() - (c[10] - c[7])).abs() < 1e-9);
        assert!(p["indicators"]["rsi"].is_object());
        assert!(p.get("indicator_errors").is_none());
        // invalid params are checked against PARAMS like a built-in
        let resp = chart(&u, json!([{"name": "my_momentum", "length": 0}]), "series");
        assert!(text(&resp, 0).contains("must be >= 1"));
        // png renders it too
        let resp = chart(&u, json!(["my_momentum"]), "png");
        assert_eq!(resp["result"]["content"][0]["type"], "image");

        let del = payload(&call(
            "delete_indicator",
            json!({"name": "my_momentum"}),
            Some(&u),
        ));
        assert_eq!(del["ok"], true);
        let resp = chart(&u, json!(["my_momentum"]), "series");
        assert_eq!(key_error_of(&resp, "my_momentum")["step"], "unknown");
        let resp = call("delete_indicator", json!({"name": "my_momentum"}), Some(&u));
        assert!(text(&resp, 0).contains("no indicator named"));
    }

    #[test]
    fn script_plot_colors_reach_the_series_wire() {
        // ADR-0026: a script's chart.line / chart.bars colors serialize like
        // a built-in's.
        let u = user();
        let source = r##"
PARAMS = []
META = {"title": "Colored", "overlay": False}

def compute(ohlcv, params, chart):
    m = ohlcv.close - ohlcv.close[3]
    chart.line(m, title = "MOM", color = "teal", width = 3)
    chart.bars(m, color = "#ff00ff", alpha = 0.5)
"##;
        payload(&call(
            "register_indicator",
            json!({"name": "colored", "source": source}),
            Some(&u),
        ));
        let p = payload(&chart(&u, json!(["colored"]), "series"));
        let ind = &p["indicators"]["colored"];
        assert_eq!(ind["lines"][0]["color"], "rgba(0,137,123,0.9)");
        assert_eq!(ind["lines"][0]["width"], 3);
        let h = &ind["histogram"][0];
        assert_eq!(h["label"], "Colored");
        assert_eq!(h["palette"], json!(["rgba(255,0,255,0.5)"]));
        let idx = h["color_idx"].as_array().unwrap();
        assert_eq!(idx.len(), 200);
        assert!(idx[..3].iter().all(|i| i.is_null()));
        assert!(idx[3..].iter().all(|i| *i == json!(0)));
    }

    #[test]
    fn scripts_are_per_user() {
        let (a, b) = (user(), user());
        payload(&call(
            "register_indicator",
            json!({"name": "secret_sauce", "source": MOMENTUM}),
            Some(&a),
        ));
        assert_eq!(
            payload(&call("list_my_indicators", json!({}), Some(&b)))["total"],
            0
        );
        let all_b = payload(&tool_list_indicators(Some(json!(1)), Some(&b)));
        assert!(!all_b.to_string().contains("secret_sauce"));
        let resp = chart(&b, json!(["secret_sauce"]), "series");
        assert_eq!(key_error_of(&resp, "secret_sauce")["step"], "unknown");
        assert!(payload(&resp)["indicators"].get("secret_sauce").is_none());
        let resp = call(
            "delete_indicator",
            json!({"name": "secret_sauce"}),
            Some(&b),
        );
        assert!(resp["result"]["isError"].as_bool().unwrap());
        // no user (stdio): built-ins only, scripting tools refuse
        let resp = generate_chart(
            Some(json!(1)),
            &json!({"indicators": ["secret_sauce"], "format": "series"}),
            fetch::DataSource::Sample,
            None,
        );
        assert!(text(&resp, 0).contains("Unknown indicator"));
        let resp = call("list_my_indicators", json!({}), None);
        assert!(text(&resp, 0).contains("authenticated"));
        assert!(payload(&call("list_my_indicators", json!({}), Some(&a)))["total"] == 1);
    }

    #[test]
    fn fork_shadows_the_builtin_for_its_owner_only() {
        let (a, b) = (user(), user());
        let fork = payload(&call(
            "fork_indicator",
            json!({"builtin_name": "wavetrend"}),
            None,
        ));
        let source = fork["source"].as_str().unwrap().replace(
            "chart.set_y_range(-105, 105)",
            "chart.set_y_range(-105, 105)\n    chart.set_label(\"My WaveTrend\")",
        );
        assert_eq!(fork["configurable_params"][0]["name"], "ch_len");
        let reg = payload(&call(
            "register_indicator",
            json!({"name": "wavetrend", "source": source}),
            Some(&a),
        ));
        assert_eq!(reg["shadows_builtin"], true);
        let label = |u: &str| {
            payload(&chart(u, json!(["wavetrend"]), "series"))["indicators"]["wavetrend"]["label"]
                .clone()
        };
        assert_eq!(label(&a), "My WaveTrend");
        assert_eq!(label(&b), "WaveTrend");
        let all = payload(&tool_list_indicators(Some(json!(1)), Some(&a)));
        let wt: Vec<&Value> = all["indicators"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["name"] == "wavetrend")
            .collect();
        assert_eq!(wt.len(), 1, "the fork replaces the built-in entry");
        assert_eq!(wt[0]["source"], "user");

        let resp = call("fork_indicator", json!({"builtin_name": "rsi"}), None);
        assert!(resp["result"]["isError"].as_bool().unwrap());
        assert!(text(&resp, 0).contains("Forkable built-ins: wavetrend"));
    }

    #[test]
    fn register_returns_structured_errors() {
        let u = user();
        let resp = call(
            "register_indicator",
            json!({"name": "bad", "source": "PARAMS = []\nMETA = {}\ndef compute(ohlcv, params, chart):\n    chart.line(ohlcv.close + \"x\")\n"}),
            Some(&u),
        );
        assert!(resp["result"]["isError"].as_bool().unwrap());
        let p: Value = serde_json::from_str(text(&resp, 0)).unwrap();
        assert_eq!(p["ok"], false);
        assert_eq!(p["errors"][0]["step"], "dry_run");
        assert_eq!(p["errors"][0]["line"], 4);
        assert!(p["errors"][0]["message"].is_string());
        assert_eq!(
            payload(&call("list_my_indicators", json!({}), Some(&u)))["total"],
            0
        );
        let resp = call(
            "register_indicator",
            json!({"name": "Bad-Name", "source": MOMENTUM}),
            Some(&u),
        );
        assert!(text(&resp, 0).contains("lowercase"));
    }

    #[test]
    fn a_failing_script_fails_only_its_indicator() {
        let u = user();
        // Passes the 100-bar dry run, fails on the 200-bar chart.
        let source = MOMENTUM.replace(
            "    m = ohlcv.close",
            "    if len(ohlcv.close) > 150:\n        fail(\"too many bars\")\n    m = ohlcv.close",
        );
        payload(&call(
            "register_indicator",
            json!({"name": "fragile", "source": source}),
            Some(&u),
        ));
        for format in ["series", "summary", "both"] {
            let resp = chart(&u, json!(["fragile", "rsi"]), format);
            let i = usize::from(format == "both");
            let p: Value = serde_json::from_str(text(&resp, i)).unwrap();
            assert!(p["indicators"]["rsi"].is_object(), "{format}");
            assert!(p["indicators"].get("fragile").is_none(), "{format}");
            let e = &p["indicator_errors"]["fragile"];
            assert_eq!(e["step"], "runtime");
            assert!(e["message"].as_str().unwrap().contains("too many bars"));
            assert_eq!(e["line"], 7);
        }
        let resp = chart(&u, json!(["fragile", "rsi"]), "png");
        assert_eq!(resp["result"]["content"][0]["type"], "image");
        let p: Value = serde_json::from_str(text(&resp, 1)).unwrap();
        assert_eq!(p["indicator_errors"]["fragile"]["step"], "runtime");
    }
}
