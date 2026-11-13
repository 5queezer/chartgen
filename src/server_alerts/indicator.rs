//! Server alerts on an indicator's line, at bar closes (ADR-0067).
//!
//! An alert names a built-in, its params and one of its lines. At each close of
//! its interval the server computes the indicator over the feed's window of closed
//! bars and compares the line with a level, or the bar's close with the line. The
//! previous reading is the same comparison one bar earlier, from the same window,
//! so a crossing is the change between two closes and needs nothing kept in memory.

use chartgen::data::{Bar, OhlcvData};
use chartgen::indicators;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Bars a feed keeps for its indicator alerts: one Binance page. The read leaves out
/// the forming bar, so an alert can count on one fewer.
pub(super) const WINDOW_BARS: usize = 1000;

/// Indicators that rescan a window of `period` bars at every bar, so they cost bars
/// times period; and the most of each of their params on a server alert (ADR-0067).
const RESCANNING: &[&str] = &["stoch", "cci", "mfi", "williams_r", "roc"];
const MAX_RESCAN_PARAM: i64 = 200;

/// The shortest interval an indicator alert reads: what it costs is per close.
pub(super) const MIN_INTERVAL_SECS: i64 = 3600;

/// Built-ins an alert may watch: each is computed from the bars alone, has lines and
/// costs about linearly in them. User scripts are not here: they are code a stranger
/// wrote, run at every close with nobody watching.
pub(super) const ALLOWED: &[&str] = &[
    "rsi",
    "ma",
    "macd",
    "stoch",
    "cci",
    "mfi",
    "williams_r",
    "roc",
    "atr",
    "adx",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Compare {
    /// The line against `level`.
    Level,
    /// The bar's close against the line.
    Close,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Spec {
    pub name: String,
    pub params: Value,
    /// The line's label, as the chart's legend shows it.
    pub line: String,
    pub compare: Compare,
    /// With `Compare::Level` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<f64>,
}

/// One close: the comparison now and one bar before, and the line's value now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Reading {
    pub d: f64,
    pub prev: Option<f64>,
    pub value: f64,
}

/// The indicator alert `args` ask for, or `None` for a price alert.
pub(super) fn parse(args: &Value) -> Result<Option<Spec>, String> {
    let name = match args.get("indicator") {
        None | Some(Value::Null) => {
            for field in ["params", "line", "compare"] {
                if args.get(field).is_some_and(|v| !v.is_null()) {
                    return Err(format!("{field} needs indicator"));
                }
            }
            return Ok(None);
        }
        Some(v) => v.as_str().ok_or("indicator must be a string")?.trim(),
    };
    if !ALLOWED.contains(&name) {
        return Err(format!(
            "Server alerts cannot watch the indicator {name}; they watch {}",
            ALLOWED.join(", ")
        ));
    }
    let given = match args.get("params") {
        None | Some(Value::Null) => json!({}),
        Some(v @ Value::Object(_)) => v.clone(),
        Some(_) => return Err("params must be an object".into()),
    };
    if given.get("offset").is_some_and(|o| o.as_i64() != Some(0)) {
        return Err(
            "A server alert reads the line at the bar that closed; offset is not taken".into(),
        );
    }
    let params = full_params(name, &given)?;
    if RESCANNING.contains(&name) {
        for (k, v) in params.as_object().into_iter().flatten() {
            if v.as_i64().is_some_and(|n| n > MAX_RESCAN_PARAM) {
                return Err(format!(
                    "{name} {k} is at most {MAX_RESCAN_PARAM} on a server alert"
                ));
            }
        }
    }
    let ind = indicators::by_name_configured(name, &params)?
        .ok_or_else(|| format!("unknown indicator {name}"))?;
    let need = ind.warmup_bars() + 2;
    if need > WINDOW_BARS - 1 {
        return Err(format!(
            "{name} with these params needs {need} bars of history; a server alert reads {}",
            WINDOW_BARS - 1
        ));
    }
    // Labels do not depend on the bars, so three of them name the lines.
    let flat = Bar {
        open: 1.0,
        high: 1.0,
        low: 1.0,
        close: 1.0,
        volume: 1.0,
        date: "0".into(),
    };
    let labels: Vec<String> = ind
        .compute(&OhlcvData {
            bars: vec![flat; 3],
            symbol: None,
            interval: None,
            session: None,
        })
        .lines
        .into_iter()
        .filter_map(|l| l.label)
        .collect();
    let line = match args.get("line") {
        None | Some(Value::Null) => labels
            .first()
            .cloned()
            .ok_or("this indicator has no line")?,
        Some(v) => {
            let want = v.as_str().ok_or("line must be a string")?.trim();
            labels
                .iter()
                .find(|l| l.eq_ignore_ascii_case(want))
                .cloned()
                .ok_or_else(|| format!("{name} has no line {want}; it has {}", labels.join(", ")))?
        }
    };
    let compare = match args.get("compare") {
        None | Some(Value::Null) => Compare::Level,
        Some(v) => serde_json::from_value(v.clone()).map_err(|_| "compare has an unknown value")?,
    };
    let level = match args.get("level") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_f64()
                .filter(|l| l.is_finite())
                .ok_or("level must be a number")?,
        ),
    };
    match (compare, level) {
        (Compare::Level, None) => return Err("level is required".into()),
        (Compare::Close, Some(_)) => {
            return Err("compare: close compares the close with the line; send no level".into())
        }
        _ => {}
    }
    Ok(Some(Spec {
        name: name.to_string(),
        params,
        line,
        compare,
        level,
    }))
}

/// `given` as `name` runs it: every param its schema names, with the defaults filled
/// in, so a stored alert and its webhooks say what is computed. A key the schema
/// does not name is refused rather than ignored.
fn full_params(name: &str, given: &Value) -> Result<Value, String> {
    let schema = indicators::by_name(name)
        .ok_or_else(|| format!("unknown indicator {name}"))?
        .params();
    let specs = schema.as_array().cloned().unwrap_or_default();
    let names: Vec<&str> = specs.iter().filter_map(|p| p["name"].as_str()).collect();
    for k in given.as_object().into_iter().flatten().map(|(k, _)| k) {
        if !names.contains(&k.as_str()) {
            let takes = if names.is_empty() {
                "it takes none".to_string()
            } else {
                format!("it takes {}", names.join(", "))
            };
            return Err(format!("{name} has no parameter {k}; {takes}"));
        }
    }
    let mut full = serde_json::Map::new();
    for spec in &specs {
        let Some(k) = spec["name"].as_str() else {
            continue;
        };
        let value = given
            .get(k)
            .filter(|v| !v.is_null())
            .or_else(|| spec.get("default"))
            .cloned();
        if let Some(v) = value {
            full.insert(k.to_string(), v);
        }
    }
    Ok(Value::Object(full))
}

/// `spec` at the close of the last bar of `bars`. `None` while the window holds
/// too little history for a converged value, or the line has none there.
pub(super) fn read(spec: &Spec, bars: &[Bar]) -> Option<Reading> {
    let ind = indicators::by_name_configured(&spec.name, &spec.params).ok()??;
    if bars.len() < ind.warmup_bars() + 2 {
        return None;
    }
    let data = OhlcvData {
        bars: bars.to_vec(),
        symbol: None,
        interval: None,
        session: None,
    };
    let y = ind
        .compute(&data)
        .lines
        .into_iter()
        .find(|l| l.label.as_deref() == Some(&spec.line))?
        .y;
    let at = |i: usize| -> Option<f64> {
        let v = *y.get(i)?;
        let d = match spec.compare {
            Compare::Level => v - spec.level?,
            Compare::Close => bars[i].close - v,
        };
        d.is_finite().then_some(d)
    };
    let last = bars.len() - 1;
    let value = y.get(last).copied().filter(|v| v.is_finite())?;
    Some(Reading {
        d: at(last)?,
        prev: last.checked_sub(1).and_then(at),
        value,
    })
}

/// The alert's indicator as its owner and its deliveries show it.
pub(super) fn view(spec: &Spec, value: Option<f64>) -> Value {
    json!({
        "name": spec.name,
        "params": spec.params,
        "line": spec.line,
        "compare": spec.compare,
        "value": value,
    })
}

/// `level` in an update of an indicator alert: any number, and none at all where
/// the close is compared with the line.
pub(super) fn level_arg(args: &Value, compare: Compare) -> Result<Option<f64>, String> {
    match args.get("level") {
        None | Some(Value::Null) => Ok(None),
        Some(_) if compare == Compare::Close => {
            Err("This alert compares the close with the line; it takes no level".into())
        }
        Some(v) => match v.as_f64().filter(|l| l.is_finite()) {
            Some(l) => Ok(Some(l)),
            None => Err("level must be a number".into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_allowed_indicator_has_a_line_and_fits_the_window() {
        for name in ALLOWED {
            let spec = parse(&json!({ "indicator": name, "level": 1.0 }))
                .unwrap_or_else(|e| panic!("{name}: {e}"))
                .unwrap();
            assert!(!spec.line.is_empty(), "{name}");
            let ind = indicators::by_name(name).unwrap();
            assert!(!ind.reads_beyond_bars(), "{name}");
        }
    }

    #[test]
    fn the_tool_schema_offers_the_allowed_indicators() {
        let tools = chartgen::mcp_schema::server_alert_tools();
        let create = tools
            .iter()
            .find(|t| t["name"] == "create_server_alert")
            .unwrap();
        let offered = &create["inputSchema"]["properties"]["indicator"]["enum"];
        assert_eq!(offered, &json!(ALLOWED));
    }

    /// 999 closed bars of a wobbling rise, with volume.
    fn window() -> Vec<Bar> {
        (0..WINDOW_BARS - 1)
            .map(|i| {
                let c = 100.0 + i as f64 * 0.05 + (i as f64 * 0.7).sin() * 3.0;
                Bar {
                    open: c - 0.5,
                    high: c + 1.0,
                    low: c - 1.0,
                    close: c,
                    volume: 10.0 + (i % 7) as f64,
                    date: (i as i64 * 3600).to_string(),
                }
            })
            .collect()
    }

    /// The largest value of integer param `k` that `parse` accepts, with `base` for
    /// the others, if any.
    fn largest(name: &str, base: &Value, k: &str, max: i64) -> Option<i64> {
        let accepts = |v: i64| {
            let mut params = base.clone();
            params[k] = json!(v);
            parse(&json!({ "indicator": name, "params": params, "level": 1.0 })).is_ok()
        };
        let (mut lo, mut hi) = (1, max);
        if !accepts(lo) {
            return None;
        }
        while lo < hi {
            let mid = lo + (hi - lo + 1) / 2;
            if accepts(mid) {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        Some(lo)
    }

    /// An indicator whose `warmup_bars` says less than it needs would pass `parse`
    /// with params it cannot read on the bars an alert keeps, and never fire.
    #[test]
    fn every_allowed_indicator_reads_a_value_at_its_largest_params() {
        let bars = window();
        for name in ALLOWED {
            let schema = indicators::by_name(name).unwrap().params();
            let specs = schema.as_array().cloned().unwrap_or_default();
            // Each choice of each enum param on its own, the others at their default.
            let mut bases = vec![json!({})];
            for spec in &specs {
                for choice in spec["enum"].as_array().into_iter().flatten() {
                    bases.push(json!({ spec["name"].as_str().unwrap(): choice }));
                }
            }
            for base in &bases {
                for spec in specs.iter().filter(|s| s["type"] == "integer") {
                    let k = spec["name"].as_str().unwrap();
                    if k == "offset" {
                        continue;
                    }
                    let max = spec["max"].as_i64().unwrap_or(10_000);
                    let Some(v) = largest(name, base, k, max) else {
                        continue;
                    };
                    let mut params = base.clone();
                    params[k] = json!(v);
                    let spec = parse(&json!({ "indicator": name, "params": params, "level": 1.0 }))
                        .unwrap()
                        .unwrap();
                    let r = read(&spec, &bars);
                    assert!(
                        r.is_some_and(|r| r.prev.is_some()),
                        "{name} {params} reads nothing on {} bars",
                        bars.len()
                    );
                }
            }
        }
    }

    #[test]
    fn rescanning_indicators_are_held_to_200() {
        for name in RESCANNING {
            let schema = indicators::by_name(name).unwrap().params();
            for spec in schema.as_array().unwrap() {
                let k = spec["name"].as_str().unwrap();
                let at =
                    |v: i64| parse(&json!({ "indicator": name, "params": { k: v }, "level": 1.0 }));
                assert!(at(200).is_ok(), "{name} {k} 200");
                let err = at(201).err().unwrap();
                assert!(err.contains("at most 200"), "{name} {k}: {err}");
            }
        }
    }

    #[test]
    fn params_are_checked_by_name_and_stored_whole() {
        let err = parse(&json!({ "indicator": "rsi", "params": { "lenght": 21 }, "level": 30.0 }))
            .err()
            .unwrap();
        assert_eq!(err, "rsi has no parameter lenght; it takes length");
        let err = parse(&json!({ "indicator": "stoch", "params": { "k": 5 }, "level": 30.0 }))
            .err()
            .unwrap();
        assert!(err.contains("it takes period, k_smooth, d_smooth"), "{err}");
        // What runs, defaults and all.
        let rsi = parse(&json!({ "indicator": "rsi", "level": 30.0 }))
            .unwrap()
            .unwrap();
        assert_eq!(rsi.params, json!({ "length": 14 }));
        let ema = parse(&json!({
            "indicator": "ma", "params": { "type": "ema", "length": 50 }, "compare": "close",
        }))
        .unwrap()
        .unwrap();
        assert_eq!(
            ema.params,
            json!({ "type": "ema", "length": 50, "source": "close", "offset": 0 })
        );
    }

    #[test]
    fn ema_200_fits_one_page_of_history() {
        let spec = parse(&json!({
            "indicator": "ma", "params": { "type": "ema", "length": 200 }, "compare": "close",
        }))
        .unwrap()
        .unwrap();
        assert_eq!(spec.line, "EMA 200");
    }
}
