#[path = "../src/test_dir.rs"]
mod test_dir;

use chartgen::data::{sample_data, OhlcvData};
use chartgen::indicator::{ema, highest, lowest, sma, stoch, Indicator, PanelResult};
use chartgen::indicators;
use chartgen::renderer::render_chart;

// ---------------------------------------------------------------------------
// Validation helper
// ---------------------------------------------------------------------------

fn assert_panel_valid(result: &PanelResult, name: &str) {
    for (li, line) in result.lines.iter().enumerate() {
        for (vi, &v) in line.y.iter().enumerate() {
            assert!(
                !v.is_infinite(),
                "Indicator {}: infinite value in line {} at index {}",
                name,
                li,
                vi
            );
        }
    }
    for (di, dot) in result.dots.iter().enumerate() {
        assert!(
            !dot.y.is_infinite(),
            "Indicator {}: infinite value in dot {}",
            name,
            di
        );
    }
    if let Some((lo, hi)) = result.y_range {
        assert!(
            lo < hi,
            "Indicator {}: invalid y_range: {} >= {}",
            name,
            lo,
            hi
        );
        assert!(
            lo.is_finite() && hi.is_finite(),
            "Indicator {}: infinite y_range",
            name
        );
    }
}

// ===========================================================================
// 1. Helper function tests
// ===========================================================================

#[test]
fn test_ema_constant_series() {
    let data = vec![5.0; 20];
    let result = ema(&data, 10);
    // EMA of a constant should converge to that constant
    for &v in &result[10..] {
        assert!((v - 5.0).abs() < 0.01, "EMA of constant 5.0 gave {}", v);
    }
}

#[test]
fn test_ema_known_values() {
    let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
    let result = ema(&data, 3);
    assert_eq!(result.len(), 5);
    // Pine ta.ema: NaN warmup, seeded with SMA(1,2,3) = 2, then alpha = 0.5
    assert!(result[0].is_nan() && result[1].is_nan());
    assert!((result[2] - 2.0).abs() < 1e-10);
    assert!((result[3] - 3.0).abs() < 1e-10);
    assert!((result[4] - 4.0).abs() < 1e-10);
}

#[test]
fn test_ema_empty() {
    let result = ema(&[], 10);
    assert!(result.is_empty());
}

#[test]
fn test_ema_period_zero() {
    let result = ema(&[1.0, 2.0], 0);
    assert_eq!(result.len(), 2);
    assert!(result[0].is_nan());
    assert!(result[1].is_nan());
}

#[test]
fn test_sma_basic() {
    let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
    let result = sma(&data, 3);
    assert_eq!(result.len(), 5);
    // Pine ta.sma: NaN until a full window exists
    assert!(result[0].is_nan() && result[1].is_nan());
    assert!((result[2] - 2.0).abs() < 1e-10);
    assert!((result[3] - 3.0).abs() < 1e-10);
    assert!((result[4] - 4.0).abs() < 1e-10);
}

#[test]
fn test_sma_empty() {
    let result = sma(&[], 3);
    assert!(result.is_empty());
}

#[test]
fn test_sma_period_larger_than_data() {
    let data = vec![1.0, 2.0, 3.0];
    let result = sma(&data, 10);
    // Pine ta.sma: no full window, so every value is NaN
    assert_eq!(result.len(), 3);
    assert!(result.iter().all(|v| v.is_nan()));
}

#[test]
fn test_lowest_basic() {
    let data = vec![5.0, 3.0, 8.0, 1.0, 4.0];
    let result = lowest(&data, 3);
    assert_eq!(result.len(), 5);
    assert!(result[0].is_nan());
    assert!(result[1].is_nan());
    assert!((result[2] - 3.0).abs() < 1e-10);
    assert!((result[3] - 1.0).abs() < 1e-10);
    assert!((result[4] - 1.0).abs() < 1e-10);
}

#[test]
fn test_lowest_empty() {
    let result = lowest(&[], 3);
    assert!(result.is_empty());
}

#[test]
fn test_lowest_period_zero() {
    let result = lowest(&[1.0, 2.0], 0);
    assert_eq!(result.len(), 2);
    assert!(result[0].is_nan());
}

#[test]
fn test_highest_basic() {
    let data = vec![5.0, 3.0, 8.0, 1.0, 4.0];
    let result = highest(&data, 3);
    assert_eq!(result.len(), 5);
    assert!(result[0].is_nan());
    assert!(result[1].is_nan());
    assert!((result[2] - 8.0).abs() < 1e-10);
    assert!((result[3] - 8.0).abs() < 1e-10);
    assert!((result[4] - 8.0).abs() < 1e-10);
}

#[test]
fn test_highest_empty() {
    let result = highest(&[], 3);
    assert!(result.is_empty());
}

#[test]
fn test_stoch_basic() {
    let data = vec![10.0, 20.0, 30.0, 25.0, 15.0];
    let result = stoch(&data, &data, &data, 3);
    assert_eq!(result.len(), 5);
    // First two should be NaN (period-1 warmup for lowest/highest)
    assert!(result[0].is_nan());
    assert!(result[1].is_nan());
    // At index 2: src=30, lowest=10, highest=30 => 100*(30-10)/(30-10) = 100
    assert!((result[2] - 100.0).abs() < 1e-10);
}

#[test]
fn test_stoch_empty() {
    let result = stoch(&[], &[], &[], 3);
    assert!(result.is_empty());
}

// ===========================================================================
// 2. All indicators - 200 bars
// ===========================================================================

#[test]
fn test_all_indicators_200_bars() {
    let data = sample_data(200);
    for name in indicators::available() {
        let ind =
            indicators::by_name(name).unwrap_or_else(|| panic!("Unknown indicator: {}", name));
        let result = ind.compute(&data);
        assert_panel_valid(&result, name);
        assert!(
            !result.label.is_empty(),
            "Indicator {} has empty label",
            name
        );
    }
}

// ===========================================================================
// 3. All indicators - 5 bars
// ===========================================================================

#[test]
fn test_all_indicators_5_bars() {
    let data = sample_data(5);
    for name in indicators::available() {
        let ind = indicators::by_name(name).unwrap();
        let result = ind.compute(&data);
        assert_panel_valid(&result, name);
    }
}

// ===========================================================================
// 4. All indicators - 1 bar
// ===========================================================================

#[test]
fn test_all_indicators_1_bar() {
    let data = sample_data(1);
    for name in indicators::available() {
        let ind = indicators::by_name(name).unwrap();
        let result = ind.compute(&data); // should not panic
        assert_panel_valid(&result, name);
    }
}

// ===========================================================================
// 5. All indicators - 0 bars
// ===========================================================================

#[test]
fn test_all_indicators_0_bars() {
    let data = OhlcvData {
        bars: vec![],
        symbol: None,
        interval: None,
        session: None,
    };
    for name in indicators::available() {
        let ind = indicators::by_name(name).unwrap();
        let _result = ind.compute(&data); // should not panic
    }
}

// ===========================================================================
// 6. Overlay vs Panel classification
// ===========================================================================

#[test]
fn test_overlay_classification() {
    let data = sample_data(100);
    let overlays = [
        "ema_stack",
        "bbands",
        "keltner",
        "donchian",
        "vwap",
        "vwap_bands",
        "supertrend",
        "sar",
        "ichimoku",
        "heikin_ashi",
        "pivot",
        "volume_profile",
        "session_vp",
        "hvn_lvn",
        "naked_poc",
        "tpo",
    ];
    let panels = [
        "macd",
        "wavetrend",
        "rsi",
        "cipher_b",
        "stoch",
        "atr",
        "obv",
        "cci",
        "roc",
        "mfi",
        "williams_r",
        "cmf",
        "adx",
        "ad",
        "histvol",
        "cvd",
        "kalman_volume",
    ];

    for name in &overlays {
        let ind = indicators::by_name(name).unwrap();
        let result = ind.compute(&data);
        assert!(result.is_overlay, "{} should be overlay", name);
    }
    for name in &panels {
        let ind = indicators::by_name(name).unwrap();
        let result = ind.compute(&data);
        assert!(!result.is_overlay, "{} should be panel", name);
    }
}

// ===========================================================================
// 7. Indicator alias resolution
// ===========================================================================

#[test]
fn test_all_aliases_resolve() {
    let aliases = vec![
        ("ema", "ema_stack"),
        ("bollinger", "bbands"),
        ("stochastic", "stoch"),
        ("willr", "williams_r"),
        ("parabolic_sar", "sar"),
        ("ad_line", "ad"),
        ("hv", "histvol"),
        ("ha", "heikin_ashi"),
        ("pivot_points", "pivot"),
        ("vp", "volume_profile"),
        ("svp", "session_vp"),
        ("vp_nodes", "hvn_lvn"),
        ("npoc", "naked_poc"),
        ("market_profile", "tpo"),
        ("funding_rate", "funding"),
        ("open_interest", "oi"),
        ("ls_ratio", "long_short"),
        ("fng", "fear_greed"),
        ("kvf", "kalman_volume"),
    ];
    for (alias, _canonical) in &aliases {
        assert!(
            indicators::by_name(alias).is_some(),
            "Alias '{}' did not resolve",
            alias
        );
    }
}

// ===========================================================================
// 8. Rendering tests
// ===========================================================================

#[test]
fn test_render_no_panels() {
    let data = sample_data(50);
    let panels: Vec<Box<dyn Indicator>> = vec![];
    let result = render_chart(&data, &panels, "/tmp/test_no_panels.png", 800, 600);
    assert!(result.is_ok(), "render_chart failed: {:?}", result.err());
}

#[test]
fn test_render_single_overlay() {
    let data = sample_data(50);
    let panels: Vec<Box<dyn Indicator>> = vec![indicators::by_name("ema").unwrap()];
    let result = render_chart(&data, &panels, "/tmp/test_single_overlay.png", 800, 600);
    assert!(result.is_ok(), "render_chart failed: {:?}", result.err());
}

#[test]
fn test_render_single_panel() {
    let data = sample_data(50);
    let panels: Vec<Box<dyn Indicator>> = vec![indicators::by_name("rsi").unwrap()];
    let result = render_chart(&data, &panels, "/tmp/test_single_panel.png", 800, 600);
    assert!(result.is_ok(), "render_chart failed: {:?}", result.err());
}

#[test]
fn test_render_mixed_overlays_and_panels() {
    let data = sample_data(100);
    let panels: Vec<Box<dyn Indicator>> = vec![
        indicators::by_name("ema").unwrap(),
        indicators::by_name("bbands").unwrap(),
        indicators::by_name("cipher_b").unwrap(),
        indicators::by_name("macd").unwrap(),
        indicators::by_name("rsi").unwrap(),
    ];
    let result = render_chart(&data, &panels, "/tmp/test_mixed.png", 1920, 1080);
    assert!(result.is_ok(), "render_chart failed: {:?}", result.err());
}

#[test]
fn test_render_all_indicators() {
    let data = sample_data(200);
    // Skip external API indicators (funding, oi, long_short, fear_greed) as they need network
    let skip = ["funding", "oi", "long_short", "fear_greed"];
    let panels: Vec<Box<dyn Indicator>> = indicators::available()
        .iter()
        .filter(|name| !skip.contains(name))
        .filter_map(|name| indicators::by_name(name))
        .collect();
    let result = render_chart(&data, &panels, "/tmp/test_all.png", 3840, 2160);
    assert!(result.is_ok(), "render_chart failed: {:?}", result.err());
}

#[test]
fn test_render_empty_data() {
    let data = OhlcvData {
        bars: vec![],
        symbol: None,
        interval: None,
        session: None,
    };
    let panels: Vec<Box<dyn Indicator>> = vec![indicators::by_name("rsi").unwrap()];
    // Should not panic -- may error but should not crash
    let _ = render_chart(&data, &panels, "/tmp/test_empty.png", 800, 600);
}

#[test]
fn test_render_minimal_data() {
    let data = sample_data(3);
    let panels: Vec<Box<dyn Indicator>> = vec![
        indicators::by_name("cipher_b").unwrap(),
        indicators::by_name("macd").unwrap(),
    ];
    let result = render_chart(&data, &panels, "/tmp/test_minimal.png", 800, 600);
    assert!(result.is_ok(), "render_chart failed: {:?}", result.err());
}

// ===========================================================================
// 9. External indicators with no symbol
// ===========================================================================

#[test]
fn test_external_indicators_no_symbol() {
    let data = sample_data(100); // symbol = None
    let externals = ["funding", "oi", "long_short", "fear_greed"];
    for name in &externals {
        let ind = indicators::by_name(name).unwrap();
        let result = ind.compute(&data);
        // Should return gracefully with a label, not panic
        assert!(!result.label.is_empty());
    }
}

#[test]
fn test_external_indicators_stock_symbol() {
    let mut data = sample_data(100);
    data.symbol = Some("AAPL".to_string());
    data.interval = Some("1d".to_string());
    let crypto_only = ["funding", "oi", "long_short"];
    for name in &crypto_only {
        let ind = indicators::by_name(name).unwrap();
        let result = ind.compute(&data);
        // AAPL is not crypto: nothing is drawn, and the label says why.
        assert!(
            result.label.ends_with("(Binance futures only)"),
            "{}",
            result.label
        );
    }
}

// ===========================================================================
// 10. Data edge cases
// ===========================================================================

#[test]
fn test_sample_data_sizes() {
    for n in [0, 1, 2, 5, 10, 50, 100, 500] {
        let data = sample_data(n);
        assert_eq!(data.bars.len(), n);
        assert_eq!(data.len(), n);
    }
}

// ===========================================================================
// 11. Available indicators count
// ===========================================================================

#[test]
fn test_available_count() {
    let available = indicators::available();
    assert_eq!(available.len(), 77, "Expected 77 indicators");
}

#[test]
fn test_all_available_resolve() {
    for name in indicators::available() {
        assert!(
            indicators::by_name(name).is_some(),
            "available() lists '{}' but by_name returns None",
            name
        );
    }
}

#[test]
fn test_unknown_indicator_returns_none() {
    assert!(indicators::by_name("nonexistent_indicator").is_none());
    assert!(indicators::by_name("").is_none());
}

// ===========================================================================
// 12. Volume-profile family: session reset, naked-POC fill detection
// ===========================================================================

fn hourly_bars(n: usize) -> OhlcvData {
    // Craft 3 sessions of ~8 bars each using 86400s epoch boundaries.
    let start: i64 = 1_700_000_000; // starts at a day boundary-ish
    let mut base = sample_data(n);
    for (i, bar) in base.bars.iter_mut().enumerate() {
        let ts = start + (i as i64) * 3600;
        bar.date = format!("{}", ts);
    }
    base
}

#[test]
fn test_session_vp_resets_per_day() {
    let data = hourly_bars(72); // 3 days
    let ind = indicators::by_name_configured(
        "session_vp",
        &serde_json::json!({"bins": 12, "session": "daily"}),
    )
    .unwrap()
    .unwrap();
    let result = ind.compute(&data);
    // One POC line, whatever the number of sessions (ADR-0048): a live tail holds
    // fewer sessions than a load and must keep its shape. It holds one level through
    // each session, with a value on all of its bars.
    let poc_lines: Vec<_> = result
        .lines
        .iter()
        .filter(|l| l.label.as_deref() == Some("POC"))
        .collect();
    assert_eq!(poc_lines.len(), 1);
    assert!(result.level_lines);
    assert!(poc_lines[0].y.iter().all(|v| v.is_finite()));
    let sessions: Vec<&[f64]> = poc_lines[0].y.chunk_by(|a, b| a == b).collect();
    assert!(
        sessions.len() >= 2,
        "expected at least 2 sessions: {}",
        sessions.len()
    );
}

#[test]
fn test_naked_poc_excludes_filled_levels() {
    let data = sample_data(200);
    let ind = indicators::by_name_configured(
        "naked_poc",
        &serde_json::json!({"bars_per_session": 20, "include_current": false}),
    )
    .unwrap()
    .unwrap();
    let result = ind.compute(&data);
    // Every nPOC level must NOT be re-touched by any later bar's high/low range.
    for line in result
        .lines
        .iter()
        .filter(|l| l.label.as_deref() == Some("nPOC"))
    {
        let Some(start) = line.y.iter().position(|v| !v.is_nan()) else {
            continue;
        };
        let poc = line.y[start];
        // nPOC lines begin at the bar right after the source session closes,
        // so `start` is the first post-session bar.
        for i in start..data.bars.len() {
            let b = &data.bars[i];
            assert!(
                !(b.low <= poc && poc <= b.high),
                "naked POC {} was actually filled at bar {}",
                poc,
                i
            );
        }
    }
}

#[test]
fn test_hvn_lvn_produces_hlines() {
    let data = sample_data(300);
    let ind = indicators::by_name_configured(
        "hvn_lvn",
        &serde_json::json!({"bins": 40, "min_prominence": 0.05, "top_n": 5}),
    )
    .unwrap()
    .unwrap();
    let result = ind.compute(&data);
    assert!(result.is_overlay);
    assert!(
        !result.hlines.is_empty(),
        "expected at least one HVN or LVN line"
    );
    for h in &result.hlines {
        assert!(h.y.is_finite());
    }
}

#[test]
fn test_tpo_time_based_profile() {
    let data = sample_data(100);
    let ind = indicators::by_name("tpo").unwrap();
    let result = ind.compute(&data);
    assert!(result.is_overlay);
    assert_eq!(result.hlines.len(), 3, "TPO should emit POC/VAH/VAL hlines");
    assert!(!result.hbars.is_empty());
}

// ---------------------------------------------------------------------------
// Parameter validation: bad params must yield Err, never a panic
// ---------------------------------------------------------------------------

#[test]
fn test_invalid_params_rejected_for_every_indicator() {
    use serde_json::{json, Value};
    use std::panic::{catch_unwind, AssertUnwindSafe};

    let data = sample_data(60);
    let mut checked = 0;
    for info in indicators::registry() {
        for spec in info.params.as_array().unwrap() {
            let pname = spec["name"].as_str().unwrap();
            let ty = spec["type"].as_str().unwrap();
            let is_int = ty == "integer";
            let is_num = ty == "number";
            if !is_int && !is_num {
                continue;
            }
            let mut bad: Vec<Value> = vec![json!("x"), json!(true)];
            if is_int {
                bad.extend([json!(0), json!(-1), json!(-100), json!(1.5)]);
            }
            if let Some(max) = spec.get("max").and_then(|v| v.as_f64()) {
                bad.push(json!(max + 1.0));
            }
            if let Some(min) = spec.get("min").and_then(|v| v.as_f64()) {
                bad.push(json!(min - 1.0));
            }
            // Values inside a declared range (e.g. offset 0 or ma_length 0) are valid.
            let (lo, hi) = (
                spec.get("min").and_then(|v| v.as_f64()),
                spec.get("max").and_then(|v| v.as_f64()),
            );
            bad.retain(|v| {
                let in_range = v.as_f64().is_some_and(|f| {
                    lo.is_some_and(|l| f >= l)
                        && hi.is_some_and(|h| f <= h)
                        && (!is_int || f.fract() == 0.0)
                });
                !in_range
            });
            for v in bad {
                let params = json!({ "name": info.name, pname: v.clone() });
                let r = catch_unwind(AssertUnwindSafe(|| {
                    indicators::by_name_configured(info.name, &params)
                }));
                match r {
                    Ok(Err(msg)) => {
                        assert!(msg.contains(info.name) && msg.contains(pname), "{msg}")
                    }
                    Ok(Ok(_)) => panic!("{}.{pname}={v} was accepted", info.name),
                    Err(_) => panic!("{}.{pname}={v} panicked", info.name),
                }
                checked += 1;
            }
            // Boundary values must be accepted and computable without panicking.
            for bound in ["min", "max"] {
                let Some(b) = spec.get(bound) else { continue };
                let params = json!({ "name": info.name, pname: b });
                let r = catch_unwind(AssertUnwindSafe(|| {
                    let ind = indicators::by_name_configured(info.name, &params)
                        .unwrap()
                        .unwrap();
                    ind.compute(&data)
                }));
                assert!(r.is_ok(), "{}.{pname}={b} panicked in compute", info.name);
            }
        }
    }
    assert!(checked > 100, "table looked too small: {checked}");
}

#[test]
fn test_every_registered_indicator_has_schema_bounds_on_numeric_params() {
    for info in indicators::registry() {
        for spec in info.params.as_array().unwrap() {
            if spec["type"] == "integer" {
                assert!(
                    spec.get("min").is_some() && spec.get("max").is_some(),
                    "{}.{} lacks min/max",
                    info.name,
                    spec["name"]
                );
            }
        }
    }
}

#[test]
fn test_nan_inf_floats_are_not_representable_or_rejected() {
    use serde_json::{json, Number, Value};
    // JSON cannot carry NaN/inf: Value::from(NaN) degrades to null (= default),
    // which must neither panic nor change behaviour.
    assert!(Number::from_f64(f64::NAN).is_none());
    let nan = Value::from(f64::NAN);
    assert!(nan.is_null());
    for info in indicators::registry() {
        for spec in info.params.as_array().unwrap() {
            if spec["type"] == "number" {
                let params = json!({ "name": info.name, spec["name"].as_str().unwrap(): nan });
                assert!(indicators::by_name_configured(info.name, &params).is_ok());
            }
        }
    }
    // Non-finite values that do reach the validator are rejected.
    let ind = indicators::by_name("bbands").unwrap();
    let mut obj = serde_json::Map::new();
    obj.insert("std_dev".into(), json!(f64::MAX));
    assert!(
        indicators::validate_params("bbands", ind.as_ref(), &Value::Object(obj)).is_err(),
        "huge float must exceed max"
    );
}

// ===========================================================================
// 13. HTTP server: web app on the same origin, OAuth discovery (ADR-0027)
// ===========================================================================

/// A `chartgen --serve` child on a free port, killed on drop.
struct Server {
    child: std::process::Child,
    port: u16,
    /// Its throwaway `CHARTGEN_DB`, deleted on drop.
    temp_db: Option<std::path::PathBuf>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(db) = &self.temp_db {
            let _ = std::fs::remove_file(db);
        }
    }
}

/// A port nothing listens on, below the OS's ephemeral range (32768 and up on
/// Linux), each handed out once and spread by process id so parallel runs stay
/// apart. It must not open a listener to find one: a test forking at that moment
/// inherits the socket until its `exec`, so the wait in `spawn_server_on` connects
/// to that instead of the server, and the first request gets a reset or refusal.
/// The range also keeps the port clear of the many client sockets tests open.
fn free_port() -> u16 {
    static OFFSET: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);
    let base = 20_000 + (std::process::id() % 120) as u16 * 100;
    loop {
        let offset = OFFSET.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let port = base + offset % 12_000;
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            return port;
        }
    }
}

/// A server with an empty store of its own, never the user's real one.
fn spawn_server(web_dir: Option<&std::path::Path>) -> Server {
    let port = free_port();
    let db = crate::test_dir::new("chartgen-e2e").join("chartgen.db");
    let mut server = spawn_server_on(port, web_dir, &db, false);
    server.temp_db = Some(db);
    server
}

fn spawn_server_on(
    port: u16,
    web_dir: Option<&std::path::Path>,
    db: &std::path::Path,
    scripting: bool,
) -> Server {
    start_server(server_command(port, web_dir, db, scripting), port)
}

fn server_command(
    port: u16,
    web_dir: Option<&std::path::Path>,
    db: &std::path::Path,
    scripting: bool,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_chartgen"));
    cmd.args(["--serve", "--port", &port.to_string()])
        .env_remove("CHARTGEN_WEB_DIR")
        .env_remove("CHARTGEN_BASE_URL")
        .env_remove("CHARTGEN_SCRIPTING")
        .env_remove("CHARTGEN_GITHUB_CLIENT_ID")
        .env_remove("CHARTGEN_GITHUB_CLIENT_SECRET")
        .env_remove("CHARTGEN_GOOGLE_CLIENT_ID")
        .env_remove("CHARTGEN_GOOGLE_CLIENT_SECRET")
        .env_remove("CHARTGEN_CLAIM_OWNER")
        .env_remove("CHARTGEN_ADMIN_USERS")
        .env_remove("CHARTGEN_SERVER_ALERTS")
        .env_remove("CHARTGEN_WEBHOOK_TEST_ORIGIN")
        .env_remove("CHARTGEN_SERVER_ALERT_CLIENTS")
        .env("CHARTGEN_DB", db)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(dir) = web_dir {
        cmd.env("CHARTGEN_WEB_DIR", dir);
    }
    if scripting {
        cmd.env("CHARTGEN_SCRIPTING", "1");
    }
    cmd
}

fn start_server(mut cmd: std::process::Command, port: u16) -> Server {
    let server = Server {
        child: cmd.spawn().unwrap(),
        port,
        temp_db: None,
    };
    for _ in 0..200 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return server;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("chartgen --serve did not listen on {port}");
}

/// Raw HTTP/1.1 exchange, so paths like `/../x` reach the server verbatim.
/// Returns (status, lower-cased head, body).
fn http(
    port: u16,
    method: &str,
    path: &str,
    headers: &[&str],
    body: &str,
) -> (u16, String, String) {
    let (status, head, body) = http_exact(port, method, path, headers, body);
    (status, head.to_ascii_lowercase(), body)
}

/// `http` with the head as sent: cookies and `state` values are case-sensitive.
fn http_exact(
    port: u16,
    method: &str,
    path: &str,
    headers: &[&str],
    body: &str,
) -> (u16, String, String) {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for h in headers {
        req.push_str(&format!("{h}\r\n"));
    }
    req.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    s.write_all(req.as_bytes()).unwrap();
    let mut resp = String::new();
    s.read_to_string(&mut resp).unwrap();
    let (head, body) = resp.split_once("\r\n\r\n").unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    let chunked = head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked");
    let body = if chunked {
        unchunked(body)
    } else {
        body.to_string()
    };
    (status, head.to_string(), body)
}

fn unchunked(mut rest: &str) -> String {
    let mut body = String::new();
    while let Some((size, after)) = rest.split_once("\r\n") {
        let size = usize::from_str_radix(size, 16).unwrap();
        if size == 0 {
            break;
        }
        body.push_str(&after[..size]);
        rest = &after[size + 2..];
    }
    body
}

const HTML: &str = "Accept: text/html,application/xhtml+xml,*/*;q=0.8";
const JSON_POST: [&str; 2] = ["Content-Type: application/json", "Accept: application/json"];

fn initialize(port: u16, path: &str) -> (u16, String, String) {
    http(
        port,
        "POST",
        path,
        &JSON_POST,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
    )
}

/// Connections past the cap wait instead of failing, and are served when others close
/// (ADR-0051). The socket buffers and deadlines are tested on real sockets in the module.
#[test]
fn test_connections_past_the_cap_wait_their_turn() {
    use std::io::{Read, Write};
    let srv = spawn_server(None);
    let open: Vec<_> = (0..128)
        .map(|_| std::net::TcpStream::connect(("127.0.0.1", srv.port)).unwrap())
        .collect();
    std::thread::sleep(std::time::Duration::from_millis(300));

    let mut next = std::net::TcpStream::connect(("127.0.0.1", srv.port)).unwrap();
    next.write_all(b"GET /oauth/me HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .unwrap();
    next.set_read_timeout(Some(std::time::Duration::from_millis(500)))
        .unwrap();
    let mut answer = String::new();
    assert!(
        next.read_to_string(&mut answer).is_err(),
        "served past the cap: {answer}"
    );
    drop(open);
    next.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    next.read_to_string(&mut answer).unwrap();
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
}

/// Bodies are read whole, so they are limited for everyone.
#[test]
fn test_a_large_body_is_refused() {
    let srv = spawn_server(None);
    let large = "x".repeat(256 * 1024 + 1);
    assert_eq!(http(srv.port, "POST", "/mcp", &JSON_POST, &large).0, 413);
    assert_eq!(
        http(srv.port, "POST", "/register", &JSON_POST, &large).0,
        413
    );
    let fits = "x".repeat(256 * 1024);
    assert_ne!(http(srv.port, "POST", "/mcp", &JSON_POST, &fits).0, 413);
}

/// A client that reads little: its receive window stays small from the start.
fn small_window_client(port: u16) -> std::net::TcpStream {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let stream = runtime.block_on(async {
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.set_recv_buffer_size(4096).unwrap();
        let stream = socket
            .connect(std::net::SocketAddr::from(([127, 0, 0, 1], port)))
            .await
            .unwrap();
        stream.into_std().unwrap()
    });
    stream.set_nonblocking(false).unwrap();
    stream
}

/// A server on the sample source: these tests need charts of the size they ask for, with
/// or without a network, and never the real provider.
fn spawn_sample_server() -> Server {
    let port = free_port();
    let db = crate::test_dir::new("chartgen-e2e").join("chartgen.db");
    let mut cmd = server_command(port, None, &db, false);
    cmd.env("CHARTGEN_DATA_SOURCE", "sample");
    let mut server = start_server(cmd, port);
    server.temp_db = Some(db);
    server
}

/// The bars of a chart answer. An error answer has none, so it cannot pass for a chart.
fn bars_in(answer: &str) -> usize {
    let call: serde_json::Value = serde_json::from_str(answer)
        .unwrap_or_else(|e| panic!("{e}: {}", &answer[..200.min(answer.len())]));
    let text = call["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no chart: {}", &answer[..200.min(answer.len())]));
    let series: serde_json::Value =
        serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"));
    series["bars"].as_array().map_or(0, Vec::len)
}

/// The size of the answer an unread connection is holding, reading only its head: a real
/// chart and not a small error, or the test would hold nothing.
fn unread_answer_size(s: &mut std::net::TcpStream) -> usize {
    use std::io::Read;
    s.set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
    assert!(head.starts_with("http/1.1 200"), "{head}");
    head.lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .unwrap_or_else(|| panic!("no length: {head}"))
        .trim()
        .parse()
        .unwrap()
}

/// More than the socket's buffer and hyper's queue can take, so it really stays held.
const HELD: usize = 1024 * 1024;

/// A chart call as text.
fn chart_call(bars: u64, indicators: &[&str], extra: serde_json::Value) -> String {
    let mut arguments = serde_json::json!({
        "ticker": "BTCUSDT", "timeframe": "1h", "format": "series", "bars": bars,
        "indicators": indicators,
    });
    if let Some(extra) = extra.as_object() {
        for (k, v) in extra {
            arguments[k] = v.clone();
        }
    }
    serde_json::json!({
        "jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": { "name": "generate_chart", "arguments": arguments },
    })
    .to_string()
}

/// Sends a POST to `/mcp` from a client that reads little, and does not read the answer.
fn post_unread(port: u16, auth: &str, body: &str) -> std::net::TcpStream {
    use std::io::Write;
    let mut s = small_window_client(port);
    write!(
        s,
        "POST /mcp HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\n\
         Accept: application/json\r\n{auth}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    s
}

/// Five indicators: the largest chart at 5000 bars the budget can hold, 30,000 of 32,000.
const FIVE: [&str; 5] = ["rsi", "macd", "ema", "bbands", "vwap"];

/// One answer nobody reads holds its share, not the server: what is no tool call goes on.
#[test]
fn test_an_unread_answer_does_not_stop_the_others() {
    let srv = spawn_sample_server();
    let client = register_external(srv.port);
    let token = sign_in(srv.port, &client);
    let auth = format!("Authorization: Bearer {token}");
    let headers = [JSON_POST[0], JSON_POST[1], auth.as_str()];
    let mut unread = post_unread(
        srv.port,
        &auth,
        &chart_call(5000, &FIVE, serde_json::json!({})),
    );
    assert!(unread_answer_size(&mut unread) > HELD);

    let started = std::time::Instant::now();
    let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
    let (st, _, body) = http(srv.port, "POST", "/mcp", &headers, list);
    assert_eq!(st, 200);
    assert!(body.contains("generate_chart"), "{body}");
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
}

/// The app's real calls, `chart_style` and all, run side by side: a grid does not take
/// turns, with part of the budget held by an answer nobody reads. Eight at once, each a
/// chart of the 500 bars it asked for.
#[test]
fn test_the_apps_charts_run_in_parallel() {
    let srv = spawn_sample_server();
    let client = register_external(srv.port);
    let token = sign_in(srv.port, &client);
    let auth = format!("Authorization: Bearer {token}");
    let mut unread = post_unread(
        srv.port,
        &auth,
        &chart_call(5000, &["rsi"], serde_json::json!({})),
    );
    assert!(unread_answer_size(&mut unread) > HELD);

    let call = chart_call(
        500,
        &["rsi", "macd", "ema"],
        serde_json::json!({ "chart_style": "candles", "warmup": 300 }),
    );
    let started = std::time::Instant::now();
    std::thread::scope(|scope| {
        let charts: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    let headers = [JSON_POST[0], JSON_POST[1], auth.as_str()];
                    http(srv.port, "POST", "/mcp", &headers, &call)
                })
            })
            .collect();
        for chart in charts {
            let (st, _, body) = chart.join().unwrap();
            assert_eq!(st, 200, "{}", &body[..200.min(body.len())]);
            assert_eq!(bars_in(&body), 500);
        }
    });
    assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
}

/// What makes that test mean something: charts answer in milliseconds here, so taking turns
/// would look the same. These hold their share until read, and none is read. Each still gets
/// its answer, only because a chart with `candles` is priced at what it shows, 16,500 of the
/// 32,000 with cipher_b at 4, and keeps only its answer of about 1,900 once built
/// (ADR-0052). Priced at the source window, the second would wait for the first to be read.
#[test]
fn test_the_apps_charts_do_not_take_turns() {
    let srv = spawn_sample_server();
    let client = register_external(srv.port);
    let token = sign_in(srv.port, &client);
    let auth = format!("Authorization: Bearer {token}");
    let call = chart_call(
        500,
        &["macd", "bbands", "cipher_b", "ema", "vwap"].repeat(4),
        serde_json::json!({ "chart_style": "candles" }),
    );
    let mut unread: Vec<_> = (0..3)
        .map(|_| post_unread(srv.port, &auth, &call))
        .collect();
    for s in &mut unread {
        let size = unread_answer_size(s);
        assert!(size > HELD, "{size} bytes would not stay held");
    }
}

/// A call larger than the budget is told so, whichever name its indicators go by.
#[test]
fn test_a_call_too_large_is_refused_with_the_reason() {
    let srv = spawn_sample_server();
    let client = register_external(srv.port);
    let token = sign_in(srv.port, &client);
    let auth = format!("Authorization: Bearer {token}");
    let headers = [JSON_POST[0], JSON_POST[1], auth.as_str()];
    let many = vec!["rsi"; 50];
    let as_indicators = chart_call(5000, &many, serde_json::json!({}));
    let as_panels = as_indicators.replace("\"indicators\"", "\"panels\"");
    assert_ne!(as_indicators, as_panels);
    for call in [as_indicators, as_panels] {
        let (st, _, body) = http(srv.port, "POST", "/mcp", &headers, &call);
        assert_eq!(st, 200, "{body}");
        assert!(body.contains("too much memory"), "{body}");
        assert!(body.contains("\"id\":7"), "{body}");
    }
    let fits = chart_call(5000, &FIVE, serde_json::json!({}));
    let (_, _, body) = http(srv.port, "POST", "/mcp", &headers, &fits);
    assert_eq!(
        bars_in(&body),
        5000,
        "the largest that fits is a real chart"
    );
}

/// An image is a bitmap of its pixels, whatever is drawn on it. Too large is refused like
/// a chart too large, and what fits is a real image.
#[test]
fn test_an_image_too_large_is_refused_with_the_reason() {
    let srv = spawn_sample_server();
    let client = register_external(srv.port);
    let token = sign_in(srv.port, &client);
    let auth = format!("Authorization: Bearer {token}");
    let headers = [JSON_POST[0], JSON_POST[1], auth.as_str()];
    let image = |format: &str, width: u64, height: u64| {
        let extra = serde_json::json!({ "format": format, "width": width, "height": height });
        let call = chart_call(50, &[], extra);
        let (st, _, body) = http(srv.port, "POST", "/mcp", &headers, &call);
        assert_eq!(st, 200, "{body}");
        serde_json::from_str::<serde_json::Value>(&body).unwrap()
    };
    for (format, width, height) in [
        ("png", 16000, 16000),
        ("both", 4096, 4096),
        ("PNG", 4_294_967_396, 4_294_967_396),
    ] {
        let refused = image(format, width, height);
        assert_eq!(refused["error"]["code"], -32602, "{refused}");
        let message = refused["error"]["message"].as_str().unwrap();
        assert!(message.contains("a smaller image"), "{message}");
        assert_eq!(refused["id"], 7);
    }
    use base64::Engine as _;
    let fits = image("png", 4000, 4000);
    let drawn = fits["result"]["content"][0]["data"].as_str().unwrap();
    let drawn = base64::engine::general_purpose::STANDARD
        .decode(drawn)
        .unwrap();
    assert_eq!(
        &drawn[..4],
        b"\x89PNG",
        "the largest that fits is a real image"
    );
    // Nothing is drawn for these, so the size asks for nothing.
    let series = image("series", 16000, 16000);
    assert_eq!(bars_in(&series.to_string()), 50);
}

/// A price-based chart is charged for the bricks its indicators run on, up to 20,000 of
/// them, as 1200 bars more (ADR-0052): 17 indicators at 500 bars take their turn, 50 do not.
#[test]
fn test_a_price_based_chart_with_many_indicators_takes_its_turn() {
    let srv = spawn_sample_server();
    let client = register_external(srv.port);
    let token = sign_in(srv.port, &client);
    let auth = format!("Authorization: Bearer {token}");
    let headers = [JSON_POST[0], JSON_POST[1], auth.as_str()];
    let call = chart_call(
        500,
        &["rsi"; 17],
        serde_json::json!({ "chart_style": "renko" }),
    );
    let (st, _, body) = http(srv.port, "POST", "/mcp", &headers, &call);
    assert_eq!(st, 200, "{body}");
    assert_eq!(bars_in(&body), 500, "a real chart");
    let call = chart_call(
        500,
        &["rsi"; 50],
        serde_json::json!({ "chart_style": "renko" }),
    );
    let (_, _, body) = http(srv.port, "POST", "/mcp", &headers, &call);
    assert!(body.contains("too much memory"), "too large on bricks");
    let call = chart_call(
        500,
        &["rsi"; 50],
        serde_json::json!({ "chart_style": "candles" }),
    );
    let (_, _, body) = http(srv.port, "POST", "/mcp", &headers, &call);
    assert_eq!(bars_in(&body), 500, "and fits bar for bar");
}

/// Indicators that draw several times what a usual one does cost that much (ADR-0052). 50
/// cipher_b at 627 bars once fitted, at 31,977 of 32,000, and peaked at 137 MiB by itself,
/// so that two got the server killed. On a small box, 80 bars of them ran on 19,000 bricks.
#[test]
fn test_a_chart_of_heavy_indicators_costs_what_they_draw() {
    let srv = spawn_sample_server();
    let client = register_external(srv.port);
    let token = sign_in(srv.port, &client);
    let auth = format!("Authorization: Bearer {token}");
    let headers = [JSON_POST[0], JSON_POST[1], auth.as_str()];
    let small_box = serde_json::json!({
        "chart_style": "renko", "chart_params": { "box": "percent", "percent": 0.15 },
    });
    for call in [
        chart_call(
            627,
            &["cipher_b"; 50],
            serde_json::json!({ "chart_style": "candles" }),
        ),
        chart_call(627, &["candle_patterns"; 50], serde_json::json!({})),
        chart_call(80, &["cipher_b"; 50], small_box),
    ] {
        let (st, _, body) = http(srv.port, "POST", "/mcp", &headers, &call);
        assert_eq!(st, 200, "{body}");
        assert!(
            body.contains("too much memory"),
            "{}",
            &body[..300.min(body.len())]
        );
    }
    // On the default box, measured from how far the bars move: about a brick a source bar,
    // at any hour. A percent box is a share of the price, and the sample's price swings by
    // 40% while its bars move as far at any price, so at most hours 0.15% builds more
    // bricks than a chart may have. Every price-based chart costs the same.
    let fits = chart_call(
        80,
        &["cipher_b"; 5],
        serde_json::json!({ "chart_style": "renko" }),
    );
    let (_, _, body) = http(srv.port, "POST", "/mcp", &headers, &fits);
    assert_eq!(bars_in(&body), 80, "fewer of them is a real chart");
}

/// A client that hangs up its side with an answer unread is reset, in the middle of the
/// answer or at the end of one: a plain close would leave the rest in the kernel's hands,
/// charged to the server, with the place free for the next.
#[cfg(target_os = "linux")]
#[test]
fn test_a_client_hanging_up_its_side_leaves_no_orphan() {
    let srv = spawn_sample_server();
    let client = register_external(srv.port);
    let token = sign_in(srv.port, &client);
    let auth = format!("Authorization: Bearer {token}");
    let large = chart_call(5000, &FIVE, serde_json::json!({}));
    let mut unread = post_unread(srv.port, &auth, &large);
    assert!(unread_answer_size(&mut unread) > HELD);
    std::thread::sleep(std::time::Duration::from_millis(300));
    unread.shutdown(std::net::Shutdown::Write).unwrap();
    // The answer stays unread while waiting for the reset. A client that reads on empties
    // what the kernel holds, and a server that gets to the hang-up late, on a busy machine,
    // then has nothing unsent to drop: it closes plainly, rightly, and leaves no orphan
    // either. Unread, there is always something to drop, however late the server is.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let reset = loop {
        match unread.take_error().unwrap() {
            Some(e) => break Some(e.kind()),
            None if std::time::Instant::now() >= deadline => break None,
            None => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    };
    assert_eq!(
        reset,
        Some(std::io::ErrorKind::ConnectionReset),
        "no reset within 10 s of hanging up with the answer unread"
    );
    // And what it held is free again: the same call fits, and it is a real chart.
    let headers = [JSON_POST[0], JSON_POST[1], auth.as_str()];
    let (_, _, body) = http(srv.port, "POST", "/mcp", &headers, &large);
    assert_eq!(bars_in(&body), 5000);
}

/// A session's new stream ends the one it replaces, and the connection goes with it. Else
/// one session id, again and again, takes every connection there is, past the stream cap.
#[test]
fn test_a_replaced_stream_gives_back_its_connection() {
    use std::io::{Read, Write};
    let srv = spawn_sample_server();
    let client = register_external(srv.port);
    let token = sign_in(srv.port, &client);
    let open_stream = || {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", srv.port)).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        write!(
            s,
            "GET /mcp HTTP/1.1\r\nHost: x\r\nAccept: text/event-stream\r\n\
             Mcp-Session-Id: same\r\nAuthorization: Bearer {token}\r\n\r\n"
        )
        .unwrap();
        let mut first = [0u8; 15];
        s.read_exact(&mut first).unwrap();
        assert!(first.starts_with(b"HTTP/1.1 200"));
        s
    };
    let mut old = open_stream();
    let new = open_stream();
    let mut rest = Vec::new();
    old.read_to_end(&mut rest)
        .expect("the server should have closed the replaced stream's connection");
    // More times than there are connections, all kept open on this side.
    let mut held: Vec<_> = (0..140).map(|_| open_stream()).collect();
    held.push(new);
    let started = std::time::Instant::now();
    assert_eq!(http(srv.port, "GET", "/oauth/me", &[], "").0, 200);
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
    // The newest is live: no end, as it waits for its next keepalive.
    let mut newest = held.swap_remove(139);
    newest
        .set_read_timeout(Some(std::time::Duration::from_millis(300)))
        .unwrap();
    let mut sink = [0u8; 4096];
    while let Ok(n) = newest.read(&mut sink) {
        assert!(n > 0, "the newest stream should have stayed open");
    }
}

/// Answers a client does not read are memory until it does. Past the budget a tool call
/// waits, and then is told to come back. No account, no budget, and no wait.
#[test]
fn test_a_full_budget_makes_calls_wait_and_come_back() {
    let srv = spawn_sample_server();
    let client = register_external(srv.port);
    let token = sign_in(srv.port, &client);
    let auth = format!("Authorization: Bearer {token}");
    let large = chart_call(5000, &FIVE, serde_json::json!({}));
    let mut unread = post_unread(srv.port, &auth, &large);
    assert!(unread_answer_size(&mut unread) > HELD);

    // Nothing for those without a token: at once, however full it is.
    let started = std::time::Instant::now();
    let (st, _, _) = http(srv.port, "POST", "/mcp", &JSON_POST, &large);
    assert_eq!(st, 401);
    assert!(started.elapsed().as_secs() < 2, "{:?}", started.elapsed());

    let started = std::time::Instant::now();
    let headers = [JSON_POST[0], JSON_POST[1], auth.as_str()];
    let (st, head, _) = http(srv.port, "POST", "/mcp", &headers, &large);
    assert_eq!(st, 503);
    assert!(head.contains("retry-after: 5"), "{head}");
    let waited = started.elapsed().as_secs();
    assert!((18..25).contains(&waited), "waited {waited} s");
    drop(unread);
}

/// Long-lived streams cannot take every connection, and a place comes back when one ends.
#[test]
fn test_streams_leave_room_for_everything_else() {
    use std::io::{Read, Write};
    let srv = spawn_server(None);
    let client = register_external(srv.port);
    let token = sign_in(srv.port, &client);
    // A session of its own each: the same one again replaces the stream it has.
    let open_stream = |session: u32| {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", srv.port)).unwrap();
        write!(
            s,
            "GET /mcp HTTP/1.1\r\nHost: x\r\nAccept: text/event-stream\r\n\
             Mcp-Session-Id: s{session}\r\nAuthorization: Bearer {token}\r\n\r\n"
        )
        .unwrap();
        let mut first = [0u8; 15];
        s.read_exact(&mut first).unwrap();
        (s, String::from_utf8_lossy(&first).to_string())
    };
    let mut streams: Vec<_> = (0..64).map(open_stream).collect();
    assert!(streams
        .iter()
        .all(|(_, head)| head.starts_with("HTTP/1.1 200")));
    let (_refused, head) = open_stream(64);
    assert!(head.starts_with("HTTP/1.1 503"), "{head}");
    // And the app is there for everything else.
    assert_eq!(http(srv.port, "GET", "/oauth/me", &[], "").0, 200);
    streams.pop();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let (_again, head) = open_stream(65);
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    // Full again, and a session already holding a place comes back to it.
    let (_full, head) = open_stream(66);
    assert!(head.starts_with("HTTP/1.1 503"), "{head}");
    let (_back, head) = open_stream(0);
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
}

/// What a browser sends before a call from a page of another origin.
const PREFLIGHT: [&str; 3] = [
    "Origin: https://other.example",
    "Access-Control-Request-Method: POST",
    "Access-Control-Request-Headers: authorization, content-type, mcp-session-id",
];

/// The lower-cased head of an answer whose body may be anything.
fn head_of(port: u16, method: &str, path: &str, headers: &[&str]) -> String {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for h in headers {
        req.push_str(&format!("{h}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let mut answer = Vec::new();
    s.read_to_end(&mut answer).unwrap();
    let end = answer.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    String::from_utf8_lossy(&answer[..end]).to_ascii_lowercase()
}

/// The lines of a lower-cased head that tell a browser what another origin may do.
fn cors_lines(head: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = head
        .lines()
        .filter(|l| l.starts_with("access-control-") || l.starts_with("vary:"))
        .collect();
    lines.sort_unstable();
    lines
}

/// The routes an MCP client in a browser uses answer another origin exactly as they did
/// before the others stopped (ADR-0062): any origin, method and header, no credentials.
#[test]
fn test_routes_for_browser_clients_answer_another_origin_as_before() {
    let srv = spawn_server(None);
    let p = srv.port;
    for path in [
        "/mcp",
        "/",
        "/message",
        "/sse",
        "/register",
        "/token",
        "/revoke",
        "/.well-known/oauth-authorization-server",
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let (st, head, _) = http(p, "OPTIONS", path, &PREFLIGHT, "");
        assert_eq!(st, 200, "{path}: {head}");
        assert_eq!(
            cors_lines(&head),
            [
                "access-control-allow-headers: *",
                "access-control-allow-methods: *",
                "access-control-allow-origin: *",
                "vary: origin, access-control-request-method, access-control-request-headers",
            ],
            "{path}"
        );
    }
    // And the answer itself can be read there, all of its headers.
    let from_elsewhere = ["Origin: https://other.example", JSON_POST[0], JSON_POST[1]];
    let (st, head, _) = http(p, "POST", "/mcp", &from_elsewhere, "{}");
    assert_eq!(st, 401, "{head}");
    assert_eq!(
        cors_lines(&head),
        [
            "access-control-allow-origin: *",
            "access-control-expose-headers: *",
            "vary: origin, access-control-request-method, access-control-request-headers",
        ]
    );
}

/// Everything else says nothing to another origin (ADR-0062): the pages that read the
/// session cookie, what only the web app asks, the app's files and an unknown path.
/// With and without the web app, where an unknown path is answered by other code.
#[test]
fn test_routes_for_this_origin_say_nothing_to_another() {
    let root = crate::test_dir::new(format!("chartgen-web-cors-{}", std::process::id()));
    let web = root.join("dist");
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html><title>app</title>").unwrap();
    std::fs::write(web.join("assets/x.js"), "console.log(1)").unwrap();
    for srv in [spawn_server(Some(&web)), spawn_server(None)] {
        let p = srv.port;
        // The icon's body is not text, so its head is read from the bytes.
        for extra in [&PREFLIGHT[..], &["Origin: https://other.example"][..]] {
            let method = if extra.len() > 1 { "OPTIONS" } else { "GET" };
            let head = head_of(p, method, "/favicon.ico", extra);
            assert!(
                !head.contains("access-control-"),
                "{method} /favicon.ico: {head}"
            );
        }
        for (method, path) in [
            ("GET", "/authorize?response_type=code&client_id=x"),
            ("GET", "/oauth/signin"),
            ("GET", "/oauth/login/github"),
            ("GET", "/oauth/callback/github"),
            ("GET", "/oauth/cancel"),
            ("GET", "/oauth/consent"),
            ("POST", "/oauth/consent"),
            ("POST", "/oauth/logout"),
            ("GET", "/oauth/signed-out"),
            ("GET", "/oauth/me"),
            ("POST", "/workspace/sync"),
            ("GET", "/favicon.svg"),
            ("GET", "/index.html"),
            ("GET", "/assets/x.js"),
            ("GET", "/no-such-page"),
            ("GET", "/admin/overview"),
        ] {
            let (_, head, _) = http(p, "OPTIONS", path, &PREFLIGHT, "");
            assert!(
                !head.contains("access-control-"),
                "preflight {path}: {head}"
            );
            for accept in [HTML, "Accept: application/json"] {
                let headers = ["Origin: https://other.example", accept];
                let (_, head, _) = http(p, method, path, &headers, "");
                assert!(!head.contains("access-control-"), "{method} {path}: {head}");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Every answer forbids guessing its content type (ADR-0062): JSON, an error, a page.
#[test]
fn test_every_answer_forbids_content_sniffing() {
    let srv = spawn_server(None);
    let p = srv.port;
    for (method, path) in [
        ("GET", "/"),
        ("GET", "/oauth/me"),
        ("POST", "/mcp"),
        ("GET", "/.well-known/oauth-authorization-server"),
        ("GET", "/no-such-page"),
        ("OPTIONS", "/oauth/me"),
        ("POST", "/token"),
    ] {
        let (_, head, _) = http(p, method, path, &JSON_POST, "{}");
        assert_eq!(
            head.matches("x-content-type-options: nosniff").count(),
            1,
            "{method} {path}: {head}"
        );
    }
}

#[test]
fn test_server_serves_web_app_same_origin() {
    let root = crate::test_dir::new(format!("chartgen-web-{}", std::process::id()));
    let web = root.join("dist");
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html><title>app</title>").unwrap();
    std::fs::write(web.join("assets/x.js"), "console.log(1)").unwrap();
    std::fs::write(web.join("sw.js"), "// worker").unwrap();
    std::fs::write(web.join("manifest.webmanifest"), "{}").unwrap();
    std::fs::write(web.join(".env"), "DOTSECRET").unwrap();
    std::fs::write(root.join("secret.txt"), "TOPSECRET").unwrap();
    let srv = spawn_server(Some(&web));
    let p = srv.port;

    // `/` for browsers: the app, never cached.
    let (st, head, body) = http(p, "GET", "/", &[HTML], "");
    assert_eq!(st, 200);
    assert!(body.contains("<title>app</title>"), "{body}");
    assert!(head.contains("cache-control: no-cache"), "{head}");
    assert!(head.contains("x-content-type-options: nosniff"), "{head}");
    // No other site may show the app in a frame (ADR-0062), at any path that is the app.
    for path in ["/", "/index.html", "/some/page"] {
        let (st, head, _) = http(p, "GET", path, &[HTML], "");
        assert_eq!(st, 200, "{path}");
        assert!(head.contains("x-frame-options: deny"), "{path}: {head}");
        assert!(
            head.contains("content-security-policy: frame-ancestors 'none'"),
            "{path}: {head}"
        );
        assert_eq!(
            head.matches("x-content-type-options: nosniff").count(),
            1,
            "{path}: {head}"
        );
    }

    // `/` for everyone else (health checks, MCP probes): the JSON.
    let (st, _, body) = http(p, "GET", "/", &[], "");
    assert_eq!(st, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["status"], "ok");
    assert_eq!(v["protocol"], "MCP");

    // Hashed assets: long-lived cache.
    let (st, head, body) = http(p, "GET", "/assets/x.js", &[], "");
    assert_eq!(st, 200);
    assert_eq!(body, "console.log(1)");
    assert!(
        head.contains("cache-control: public, max-age=31536000, immutable"),
        "{head}"
    );
    assert!(head.contains("x-content-type-options: nosniff"), "{head}");

    // The push worker and manifest (ADR-0045): at the root, and never cached, or a
    // browser would keep an old worker for a year.
    for path in ["/sw.js", "/manifest.webmanifest"] {
        let (st, head, _) = http(p, "GET", path, &[], "");
        assert_eq!(st, 200, "{path}");
        assert!(head.contains("cache-control: no-cache"), "{path}: {head}");
        assert!(!head.contains("immutable"), "{path}: {head}");
    }

    // Revalidation keeps `immutable` (a 304 must not downgrade to no-cache).
    let (st, head, _) = http(
        p,
        "GET",
        "/assets/x.js",
        &["If-Modified-Since: Fri, 01 Jan 2100 00:00:00 GMT"],
        "",
    );
    assert_eq!(st, 304, "{head}");
    assert!(head.contains("immutable"), "{head}");

    // Dotfiles and protocol-relative paths never reach ServeDir.
    for path in ["/.env", "/%2eenv", "/assets/.env", "//assets", "//"] {
        for accept in [&[][..], &[HTML][..]] {
            let (st, head, body) = http(p, "GET", path, accept, "");
            assert_eq!(st, 404, "{path}: {head}");
            assert!(!body.contains("DOTSECRET"), "{path} leaked");
        }
    }

    // SPA fallback for browser navigations only.
    let (st, head, body) = http(p, "GET", "/some/client/route?x=1", &[HTML], "");
    assert_eq!(st, 200);
    assert!(body.contains("<title>app</title>"));
    assert!(head.contains("cache-control: no-cache"), "{head}");
    assert_eq!(http(p, "GET", "/nope.json", &[], "").0, 404);
    assert_eq!(http(p, "GET", "/assets/missing.js", &[HTML], "").0, 404);
    assert_eq!(http(p, "POST", "/nope", &[], "").0, 404);

    // No path traversal out of the web dir (browsers get the app instead).
    for path in [
        "/../secret.txt",
        "/assets/../../secret.txt",
        "/%2e%2e/secret.txt",
    ] {
        let (st, _, body) = http(p, "GET", path, &[], "");
        assert_eq!(st, 404, "{path}");
        assert!(!body.contains("TOPSECRET"), "{path} leaked");
        let (st, _, body) = http(p, "GET", path, &[HTML], "");
        assert!(!body.contains("TOPSECRET"), "{path} leaked ({st})");
    }

    // MCP keeps working next to the app, at /mcp and the POST / alias.
    for path in ["/mcp", "/"] {
        let (st, _, body) = initialize(p, path);
        assert_eq!(st, 200, "{path}: {body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["serverInfo"]["name"], "chartgen", "{body}");
    }
    let (st, _, body) = http(
        p,
        "GET",
        "/.well-known/oauth-authorization-server",
        &[HTML],
        "",
    );
    assert_eq!(st, 200);
    assert!(body.contains("\"token_endpoint\""), "{body}");

    drop(srv);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_server_without_web_dir_keeps_root_json() {
    let srv = spawn_server(None);
    let (st, head, body) = http(srv.port, "GET", "/", &[HTML], "");
    assert_eq!(st, 200);
    assert!(head.contains("content-type: application/json"), "{head}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["status"], "ok");
    assert_eq!(http(srv.port, "GET", "/index.html", &[HTML], "").0, 404);
}

/// The S256 pair from RFC 7636 appendix B.
const PKCE_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const PKCE_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

/// `GET /authorize` for `client_id`, returning (status, head, body).
fn authorize(port: u16, client_id: &str, redirect_uri: &str) -> (u16, String, String) {
    authorize_with_state(port, client_id, redirect_uri, "a%20b%26c")
}

/// `authorize` with `state` as it goes on the query string.
fn authorize_with_state(
    port: u16,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
) -> (u16, String, String) {
    let redirect_uri: String = redirect_uri.bytes().map(|b| format!("%{b:02X}")).collect();
    let path = format!(
        "/authorize?response_type=code&client_id={client_id}&redirect_uri={redirect_uri}&code_challenge={PKCE_CHALLENGE}&state={state}"
    );
    http(port, "GET", &path, &[HTML], "")
}

fn assert_unknown_client_json(resp: (u16, String, String), case: &str) {
    let (st, head, body) = resp;
    assert_eq!(st, 400, "{case}: {head}");
    assert!(!head.contains("location:"), "{case}: {head}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"], "invalid_client", "{case}");
    assert_eq!(v["error_description"], "Unknown client_id", "{case}");
}

/// A stale client id in the browser must not strand the web app on a JSON
/// page, and nothing but the server's own app may be redirected to (ADR-0036).
#[test]
fn test_authorize_unknown_client_returns_to_the_own_web_app_only() {
    let web = crate::test_dir::new(format!("chartgen-web-authz-{}", std::process::id()));
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html>").unwrap();
    let srv = spawn_server(Some(&web));
    let p = srv.port;
    let own = format!("http://localhost:{p}");

    let (st, head, _) = authorize(p, "stale", &format!("{own}/"));
    assert_eq!(st, 302, "{head}");
    assert!(
        head.contains(&format!(
            "location: {own}/?error=invalid_client&state=a+b%26c\r\n"
        )),
        "{head}"
    );
    let (st, head, _) = authorize(p, "stale", &format!("{own}/cb?keep=1"));
    assert_eq!(st, 302, "{head}");
    assert!(
        head.contains("/cb?keep=1&error=invalid_client&state=a+b%26c\r\n"),
        "{head}"
    );

    for uri in [
        format!("http://localhost.evil.com:{p}/"),
        format!("http://localhost:{p}@evil.com/"),
        format!("http://user@localhost:{p}/"),
        format!("https://localhost:{p}/"),
        format!("http://localhost:{}/", u32::from(p) + 1),
        format!("ftp://localhost:{p}/"),
        "javascript:alert(1)".to_string(),
        "https://evil.com/".to_string(),
    ] {
        assert_unknown_client_json(authorize(p, "stale", &uri), &uri);
    }

    // A known client is unaffected: code + state, and its URIs are exact.
    let (st, _, body) = http(
        p,
        "POST",
        "/register",
        &JSON_POST,
        &format!(r#"{{"redirect_uris":["{own}/"]}}"#),
    );
    assert_eq!(st, 201, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let id = v["client_id"].as_str().unwrap();
    let (st, head, _) = authorize(p, id, &format!("{own}/"));
    assert_eq!(st, 307, "{head}");
    assert!(head.contains(&format!("location: {own}/?code=")), "{head}");
    let (st, _, body) = authorize(p, id, &format!("{own}/other"));
    assert_eq!(st, 400);
    assert!(body.contains("redirect_uri mismatch"), "{body}");

    drop(srv);
    let _ = std::fs::remove_dir_all(&web);
}

/// An API-only server has no web app to return to.
#[test]
fn test_authorize_unknown_client_without_web_dir_stays_json() {
    let srv = spawn_server(None);
    let own = format!("http://localhost:{}/", srv.port);
    assert_unknown_client_json(authorize(srv.port, "stale", &own), "no web dir");
}

const EXTERNAL_REDIRECT: &str = "https://client.example/cb";
/// `MAX_CLIENTS` in `src/server.rs`.
const MAX_CLIENTS: usize = 1000;

fn register_external(port: u16) -> String {
    let body = format!(r#"{{"redirect_uris":["{EXTERNAL_REDIRECT}"]}}"#);
    let (st, _, body) = http(port, "POST", "/register", &JSON_POST, &body);
    assert_eq!(st, 201, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    v["client_id"].as_str().unwrap().to_string()
}

/// Run the whole code flow for `client_id`, returning the access token.
fn sign_in(port: u16, client_id: &str) -> String {
    let (st, head, _) = authorize(port, client_id, EXTERNAL_REDIRECT);
    assert_eq!(st, 307, "{head}");
    let code = head
        .split("code=")
        .nth(1)
        .and_then(|rest| rest.split(['&', '\r']).next())
        .unwrap();
    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": EXTERNAL_REDIRECT,
        "client_id": client_id,
        "code_verifier": PKCE_VERIFIER,
    });
    let (st, _, body) = http(port, "POST", "/token", &JSON_POST, &body.to_string());
    assert_eq!(st, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    v["access_token"].as_str().unwrap().to_string()
}

fn register(port: u16, redirect_uri: &str) -> (u16, String) {
    let body = format!(r#"{{"redirect_uris":["{redirect_uri}"]}}"#);
    let (st, _, resp) = http(port, "POST", "/register", &JSON_POST, &body);
    (st, resp)
}

/// Status of an authenticated MCP call with `token`.
fn ping_status(port: u16, token: &str) -> u16 {
    let auth = format!("Authorization: Bearer {token}");
    let headers = [JSON_POST[0], JSON_POST[1], auth.as_str()];
    let call = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
    http(port, "POST", "/mcp", &headers, call).0
}

fn tool_names(port: u16) -> Vec<String> {
    let (st, _, body) = http(
        port,
        "POST",
        "/mcp",
        &JSON_POST,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
    );
    assert_eq!(st, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    v["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

const SCRIPT_TOOLS: [&str; 5] = [
    "register_indicator",
    "fork_indicator",
    "list_my_indicators",
    "get_my_indicator",
    "delete_indicator",
];

/// A fresh path for a store that outlives one server.
fn store_path(test: &str) -> std::path::PathBuf {
    let dir = crate::test_dir::new(format!("chartgen-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("chartgen.db")
}

const ALERT_TOOLS: [&str; 6] = [
    "create_server_alert",
    "list_server_alerts",
    "update_server_alert",
    "delete_server_alert",
    "subscribe_push",
    "unsubscribe_push",
];

/// Call a tool; `Err` is an `isError` result's text.
fn call_tool(
    port: u16,
    token: &str,
    tool: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let auth = format!("Authorization: Bearer {token}");
    let call = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": tool, "arguments": args},
    });
    let headers = [JSON_POST[0], JSON_POST[1], auth.as_str()];
    let (st, _, resp) = http(port, "POST", "/mcp", &headers, &call.to_string());
    assert_eq!(st, 200, "{resp}");
    let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let text = v["result"]["content"][0]["text"].as_str().unwrap();
    if v["result"]["isError"] == true {
        Err(text.to_string())
    } else {
        Ok(serde_json::from_str(text).unwrap())
    }
}

/// Off unless asked for: not listed, and only the list answers, saying so.
#[test]
fn test_server_alerts_are_off_by_default() {
    let db = store_path("alerts-off");
    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    let tools = tool_names(port);
    for tool in ALERT_TOOLS {
        assert!(!tools.iter().any(|t| t == tool), "{tool} listed");
    }
    let client = register_external(port);
    let token = sign_in(port, &client);
    let list = call_tool(port, &token, "list_server_alerts", serde_json::json!({})).unwrap();
    assert_eq!(list, serde_json::json!({"enabled": false, "alerts": []}));
    let spec = serde_json::json!({
        "symbol": "BTCUSDT", "condition": "greater", "trigger": "once", "level": 1,
    });
    let e = call_tool(port, &token, "create_server_alert", spec).unwrap_err();
    assert!(e.contains("off on this server"), "{e}");
    drop(srv);
}

#[cfg(feature = "scripting")]
mod server_alerts {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use openssl::bn::BigNumContext;
    use openssl::derive::Deriver;
    use openssl::ec::{EcGroup, EcKey, EcPoint, PointConversionForm};
    use openssl::hash::MessageDigest;
    use openssl::nid::Nid;
    use openssl::pkey::{PKey, Private};
    use openssl::sign::Signer;
    use openssl::symm::{decrypt_aead, Cipher};
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// A webhook receiver answering 200; returns its port and the POSTs it received.
    pub(super) fn receiver() -> (u16, Arc<Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = vec![0u8; 16384];
                let n = s.read(&mut buf).unwrap_or(0);
                // Other programs on this host probe local ports: only a POST is a delivery.
                if buf.starts_with(b"POST ") {
                    log.lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&buf[..n]).to_string());
                }
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                );
            }
        });
        (port, seen)
    }

    fn alert_server(db: &std::path::Path, hook_port: u16, sample: bool) -> Server {
        alert_server_for(db, hook_port, sample, "*")
    }

    fn alert_server_for(
        db: &std::path::Path,
        hook_port: u16,
        sample: bool,
        clients: &str,
    ) -> Server {
        let port = free_port();
        let mut cmd = server_command(port, None, db, false);
        cmd.env("CHARTGEN_SERVER_ALERTS", "1")
            .env("CHARTGEN_SERVER_ALERT_CLIENTS", clients)
            .env("CHARTGEN_VAPID_SUBJECT", "mailto:ops@example.com")
            .env(
                "CHARTGEN_WEBHOOK_TEST_ORIGIN",
                format!("http://127.0.0.1:{hook_port}"),
            );
        if sample {
            cmd.env("CHARTGEN_DATA_SOURCE", "sample");
        } else {
            cmd.env_remove("CHARTGEN_DATA_SOURCE");
        }
        start_server(cmd, port)
    }

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        for _ in 0..200 {
            if done() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("timed out waiting for {what}");
    }

    // ---- ADR-0061: alerts that wake the owner's agent ----

    const AGENT_SECRET: &str = "a-secret-of-thirty-two-characters";
    /// Stands for what an owner's thesis and note hold: it may reach the owner's
    /// receiver and nothing else, least of all a log.
    const PRIVATE: &str = "PRIVATE-REASONING-MARKER";

    /// A server with alerts and setups on the sample feed, the agent's receiver set
    /// as `agent` gives it, and everything it prints kept in `log`.
    fn waking_server(
        db: &std::path::Path,
        owner: &str,
        agent: &[(&str, &str)],
        log: &std::path::Path,
    ) -> Server {
        let port = free_port();
        let mut cmd = server_command(port, None, db, false);
        let out = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap();
        cmd.env("CHARTGEN_SERVER_ALERTS", "1")
            .env("CHARTGEN_SETUPS", "1")
            .env("CHARTGEN_SERVER_ALERT_CLIENTS", owner)
            .env("CHARTGEN_DATA_SOURCE", "sample")
            .env_remove("CHARTGEN_AGENT_WEBHOOK_URL")
            .env_remove("CHARTGEN_AGENT_WEBHOOK_OWNER")
            .env_remove("CHARTGEN_AGENT_WEBHOOK_SECRET")
            .envs(agent.iter().copied())
            .stdout(out.try_clone().unwrap())
            .stderr(out);
        start_server(cmd, port)
    }

    /// A long far below the sample price, which nothing fills during a test, with a
    /// thesis that carries the marker.
    fn a_setup(port: u16, token: &str) -> String {
        let quotes = call_tool(
            port,
            token,
            "get_quotes",
            serde_json::json!({"symbols": ["BTCUSDT"]}),
        )
        .unwrap();
        let p = quotes["quotes"][0]["last"].as_f64().unwrap();
        let setup = call_tool(
            port,
            token,
            "create_setup",
            serde_json::json!({
                "symbol": "BTCUSDT", "interval": "1h", "direction": "long",
                "thesis": format!("{PRIVATE} thesis"),
                "entry": {"low": p * 0.5, "high": p * 0.51}, "invalidation": p * 0.4,
                "targets": [p * 0.9], "expires_at": chrono::Utc::now().timestamp_millis() + 86_400_000,
            }),
        )
        .unwrap();
        setup["id"].as_str().unwrap().to_string()
    }

    /// An alert that fires at the first tick and wakes the agent about `setup`.
    fn a_waking_alert(port: u16, token: &str, setup: &str) -> Result<serde_json::Value, String> {
        call_tool(
            port,
            token,
            "create_server_alert",
            serde_json::json!({
                "symbol": "BTCUSDT", "condition": "greater", "trigger": "once", "level": 0.01,
                "notify_agent": true, "setup_id": setup, "message": format!("{PRIVATE} note"),
            }),
        )
    }

    /// A receiver that answers every delivery with `status`, and keeps what it got.
    fn receiver_answering(status: &'static str) -> (u16, Arc<Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = vec![0u8; 16384];
                let n = s.read(&mut buf).unwrap_or(0);
                if buf.starts_with(b"POST ") {
                    log.lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&buf[..n]).to_string());
                }
                let _ = s.write_all(
                    format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                        .as_bytes(),
                );
            }
        });
        (port, seen)
    }

    /// The owner of a server with sign-in off is its OAuth client, and the operator
    /// names it before the server starts: so the client is registered on a first
    /// server, and the one under test opens the same store.
    fn a_registered_client(db: &std::path::Path) -> String {
        let port = free_port();
        let srv = spawn_server_on(port, None, db, false);
        let client = register_external(port);
        drop(srv);
        client
    }

    /// The real binary: an alert about a setup wakes the agent's receiver with a
    /// signed body that carries the setup, on a receiver that answers, one that
    /// keeps failing, and one that is not there. On none of these paths does the
    /// thesis or the note, or the receiver's path, reach what the server prints.
    #[test]
    fn test_a_wake_up_reaches_the_agents_receiver_and_no_log_holds_its_body() {
        for (name, status) in [
            ("ok", Some("200 OK")),
            ("failing", Some("503 Service Unavailable")),
            ("absent", None),
        ] {
            let db = store_path(&format!("wake-up-{name}"));
            let log = db.with_extension("log");
            let client = a_registered_client(&db);
            let (hook, seen) = match status {
                Some(status) => receiver_answering(status),
                // A port nothing listens on.
                None => (free_port(), Arc::new(Mutex::new(Vec::new()))),
            };
            let url = format!("http://127.0.0.1:{hook}/webhooks/route-TOPSECRET");
            let agent = [
                ("CHARTGEN_AGENT_WEBHOOK_URL", url.as_str()),
                ("CHARTGEN_AGENT_WEBHOOK_OWNER", client.as_str()),
                ("CHARTGEN_AGENT_WEBHOOK_SECRET", AGENT_SECRET),
            ];
            let srv = waking_server(&db, &client, &agent, &log);
            let port = srv.port;
            let token = sign_in(port, &client);
            let setup = a_setup(port, &token);
            let alert = a_waking_alert(port, &token, &setup).unwrap();
            let id = alert["id"].as_str().unwrap().to_string();
            let outcome = || {
                let list =
                    call_tool(port, &token, "list_server_alerts", serde_json::json!({})).unwrap();
                let alert = list["alerts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|a| a["id"] == id.as_str())
                    .cloned();
                alert.filter(|a| !a["last_delivery_at"].is_null())
            };
            wait_for("the wake-up's outcome", || outcome().is_some());
            let alert = outcome().unwrap();
            match status {
                Some("200 OK") => {
                    assert_eq!(alert["last_status"], 200, "{alert}");
                    let raw = seen.lock().unwrap()[0].clone();
                    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
                    assert!(head.starts_with("POST /webhooks/route-TOPSECRET"), "{head}");
                    let body: serde_json::Value = serde_json::from_str(body).unwrap();
                    assert_eq!(body["setup"]["id"], setup.as_str());
                    assert_eq!(body["setup"]["thesis"], format!("{PRIVATE} thesis"));
                    assert_eq!(body["note"], format!("{PRIVATE} note"));
                    let lower = head.to_lowercase();
                    assert!(
                        lower.contains("webhook-signature: v1,") && lower.contains("webhook-id: "),
                        "{head}"
                    );
                }
                Some(_) => {
                    assert_eq!(alert["last_status"], 503, "{alert}");
                    assert_eq!(
                        seen.lock().unwrap().len(),
                        3,
                        "three attempts, then given up"
                    );
                }
                None => assert_eq!(alert["last_error"], "could not connect", "{alert}"),
            }
            drop(srv);
            let printed = std::fs::read_to_string(&log).unwrap();
            assert!(
                printed.contains(&format!(
                    "Agent receiver at http://127.0.0.1:{hook} wakes the agent of owner {client}"
                )),
                "{printed}"
            );
            for private in [PRIVATE, "TOPSECRET", AGENT_SECRET, "route-"] {
                assert!(
                    !printed.contains(private),
                    "{name}: {private} is in what the server printed"
                );
            }
            let _ = std::fs::remove_dir_all(db.parent().unwrap());
        }
    }

    /// A wrong agent setting leaves server alerts off, and says so where it cannot be
    /// missed. Setups do not depend on it: their tools work and they are scored.
    #[test]
    fn test_a_wrong_agent_setting_turns_alerts_off_and_leaves_setups_working() {
        let db = store_path("wake-up-wrong");
        let log = db.with_extension("log");
        let client = a_registered_client(&db);
        // A name, the settings, and the line the server must print.
        type Case<'a> = (&'a str, Vec<(&'a str, &'a str)>, String);
        let cases: [Case; 3] = [
            ("no secret", vec![("CHARTGEN_AGENT_WEBHOOK_URL", "http://127.0.0.1:9/h-TOPSECRET"), ("CHARTGEN_AGENT_WEBHOOK_OWNER", client.as_str())],
             "ERROR [alerts] CHARTGEN_AGENT_WEBHOOK_SECRET is required".to_string()),
            ("public address", vec![("CHARTGEN_AGENT_WEBHOOK_URL", "http://93.184.216.34/h-TOPSECRET"), ("CHARTGEN_AGENT_WEBHOOK_OWNER", client.as_str()), ("CHARTGEN_AGENT_WEBHOOK_SECRET", AGENT_SECRET)],
             "ERROR [alerts] CHARTGEN_AGENT_WEBHOOK_URL must be on this host or its private network".to_string()),
            ("owner not allowed", vec![("CHARTGEN_AGENT_WEBHOOK_URL", "http://127.0.0.1:9/h-TOPSECRET"), ("CHARTGEN_AGENT_WEBHOOK_OWNER", "someone-not-on-the-list"), ("CHARTGEN_AGENT_WEBHOOK_SECRET", AGENT_SECRET)],
             "ERROR [alerts] CHARTGEN_AGENT_WEBHOOK_OWNER is not one of the ids in CHARTGEN_SERVER_ALERT_CLIENTS".to_string()),
        ];
        for (name, agent, line) in cases {
            let _ = std::fs::remove_file(&log);
            let srv = waking_server(&db, &client, &agent, &log);
            let port = srv.port;
            let token = sign_in(port, &client);
            let tools = tool_names(port);
            assert!(
                !tools.iter().any(|t| t == "create_server_alert"),
                "{name}: alerts are listed"
            );
            assert!(
                tools.iter().any(|t| t == "create_setup"),
                "{name}: setups are gone"
            );
            // Setups work: one is stored, read back, and watched on the feed.
            let setup = a_setup(port, &token);
            let read =
                call_tool(port, &token, "get_setup", serde_json::json!({"id": setup})).unwrap();
            assert_eq!(read["status"], "watching", "{name}: {read}");
            // Scored, with alerts off as with them on. A setup counts from the first 1m
            // bar that opens after it was made, so its first score comes one to two
            // minutes later: once is enough here.
            if name == "no secret" {
                let scored = || {
                    let read =
                        call_tool(port, &token, "get_setup", serde_json::json!({"id": setup}));
                    !read.unwrap()["scored_through"].is_null()
                };
                for _ in 0..135 {
                    if scored() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
                assert!(
                    scored(),
                    "the setup was not scored through a bar within 135 s"
                );
            }
            // And an alert gets the answer of a server without alerts.
            let refused = a_waking_alert(port, &token, &setup).unwrap_err();
            assert!(refused.contains("off on this server"), "{name}: {refused}");
            drop(srv);
            let printed = std::fs::read_to_string(&log).unwrap();
            assert!(printed.contains(&line), "{name}: {printed}");
            assert!(
                printed.contains("Server alerts stay OFF"),
                "{name}: {printed}"
            );
            assert!(printed.contains("Setups enabled"), "{name}: {printed}");
            assert!(
                !printed.contains("TOPSECRET") && !printed.contains(AGENT_SECRET),
                "{name}: a value is in the log"
            );
        }
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    /// The real binary, killed and started again on the same store: the alerts
    /// come back, and what fired before does not fire again.
    #[test]
    fn test_alerts_survive_a_restart_and_do_not_fire_again() {
        let db = store_path("alerts-restart");
        let (hook, seen) = receiver();
        let srv = alert_server(&db, hook, true);
        let port = srv.port;
        for tool in ALERT_TOOLS {
            assert!(tool_names(port).iter().any(|t| t == tool), "{tool}");
        }
        let client = register_external(port);
        let token = sign_in(port, &client);
        let url = format!("http://127.0.0.1:{hook}/hook");
        // A level far below the sample price is simply greater, at once.
        let create = |trigger: &str| {
            call_tool(
                port,
                &token,
                "create_server_alert",
                serde_json::json!({
                    "symbol": "BTCUSDT", "condition": "greater", "trigger": trigger,
                    "level": 0.01, "webhook_url": url,
                }),
            )
            .unwrap()
        };
        let once = create("once");
        let every = create("every");
        wait_for("both deliveries", || seen.lock().unwrap().len() == 2);
        drop(srv);

        let srv = alert_server(&db, hook, true);
        let port = srv.port;
        let token = sign_in(port, &client);
        // Several of the sample feed's ticks, each above the level.
        std::thread::sleep(std::time::Duration::from_secs(3));
        // Delivery is at least once: a kill just after the receiver got a request sends
        // that event again. A new fire would have a new id.
        let events: std::collections::HashSet<String> = seen
            .lock()
            .unwrap()
            .iter()
            .filter_map(|req| {
                req.lines().find_map(|l| {
                    let l = l.to_ascii_lowercase();
                    l.strip_prefix("x-chartgen-event: ").map(str::to_string)
                })
            })
            .collect();
        assert_eq!(events.len(), 2, "fired again after a restart");
        let list = call_tool(port, &token, "list_server_alerts", serde_json::json!({})).unwrap();
        let alerts = list["alerts"].as_array().unwrap();
        assert_eq!(alerts.len(), 2);
        let find = |id: &serde_json::Value| alerts.iter().find(|a| a["id"] == *id).unwrap();
        let (o, e) = (find(&once["id"]), find(&every["id"]));
        assert_eq!((&o["active"], &o["fire_count"]), (&false.into(), &1.into()));
        assert_eq!((&e["active"], &e["fire_count"]), (&true.into(), &1.into()));
        assert_eq!(e["disarmed"], true);
        assert_eq!(e["feed"], "live", "the feed came back");
        assert_eq!(e["last_status"], 200);

        // Another client's list stays empty on the same server.
        let other = sign_in(port, &register_external(port));
        let list = call_tool(port, &other, "list_server_alerts", serde_json::json!({})).unwrap();
        assert_eq!(list["alerts"], serde_json::json!([]));
    }

    pub(super) type Bodies = Arc<Mutex<Vec<Vec<u8>>>>;

    /// A fake push service: what it was sent, whole. It holds its answer until told,
    /// so a delivery can be left unfinished.
    pub(super) fn push_service() -> (u16, Bodies, Arc<AtomicBool>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let answer = Arc::new(AtomicBool::new(false));
        let (log, open) = (seen.clone(), answer.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let (log, open) = (log.clone(), open.clone());
                std::thread::spawn(move || {
                    let mut got = Vec::new();
                    let mut buf = [0u8; 8192];
                    let body = loop {
                        let n = s.read(&mut buf).unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        got.extend_from_slice(&buf[..n]);
                        let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") else {
                            continue;
                        };
                        let head = String::from_utf8_lossy(&got[..end]).to_lowercase();
                        if !head.starts_with("post /push/") {
                            return;
                        }
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .and_then(|l| l.trim().parse().ok())
                            .unwrap_or(0);
                        if got.len() >= end + 4 + len {
                            break got[end + 4..end + 4 + len].to_vec();
                        }
                    };
                    log.lock().unwrap().push(body);
                    while !open.load(Ordering::SeqCst) {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    let _ = s.write_all(
                        b"HTTP/1.1 201 Created\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                    );
                });
            }
        });
        (port, seen, answer)
    }

    /// The browser's half of RFC 8291, written here from the RFC and not with the
    /// server's code, so that a mistake there cannot agree with itself.
    pub(super) fn open_push(message: &[u8], ua: &EcKey<Private>, auth: &[u8]) -> serde_json::Value {
        let hkdf = |salt: &[u8], ikm: &[u8], info: &[u8], len: usize| -> Vec<u8> {
            let hmac = |key: &[u8], data: &[u8]| {
                let key = PKey::hmac(key).unwrap();
                let mut signer = Signer::new(MessageDigest::sha256(), &key).unwrap();
                signer.update(data).unwrap();
                signer.sign_to_vec().unwrap()
            };
            let prk = hmac(salt, ikm);
            hmac(&prk, &[info, &[1]].concat())[..len].to_vec()
        };
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        let mut ctx = BigNumContext::new().unwrap();
        let ua_public = ua
            .public_key()
            .to_bytes(&group, PointConversionForm::UNCOMPRESSED, &mut ctx)
            .unwrap();
        let (salt, rest) = message.split_at(16);
        let (server_public, sealed) = rest[5..].split_at(rest[4] as usize);
        let point = EcPoint::from_bytes(&group, server_public, &mut ctx).unwrap();
        let peer = PKey::from_ec_key(EcKey::from_public_key(&group, &point).unwrap()).unwrap();
        let own = PKey::from_ec_key(ua.clone()).unwrap();
        let mut deriver = Deriver::new(&own).unwrap();
        deriver.set_peer(&peer).unwrap();
        let secret = deriver.derive_to_vec().unwrap();
        let info = [b"WebPush: info\0".as_slice(), &ua_public, server_public].concat();
        let ikm = hkdf(auth, &secret, &info, 32);
        let cek = hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0", 16);
        let nonce = hkdf(salt, &ikm, b"Content-Encoding: nonce\0", 12);
        let (body, tag) = sealed.split_at(sealed.len() - 16);
        let mut plain =
            decrypt_aead(Cipher::aes_128_gcm(), &cek, Some(&nonce), &[], body, tag).unwrap();
        assert_eq!(plain.pop(), Some(2));
        serde_json::from_slice(&plain).unwrap()
    }

    /// A browser's keys: the pair, and `subscribe_push`'s arguments for them.
    pub(super) fn browser(port: u16) -> (EcKey<Private>, Vec<u8>, serde_json::Value) {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        let key = EcKey::generate(&group).unwrap();
        let mut ctx = BigNumContext::new().unwrap();
        let public = key
            .public_key()
            .to_bytes(&group, PointConversionForm::UNCOMPRESSED, &mut ctx)
            .unwrap();
        let auth = b"sixteen byte key".to_vec();
        let args = serde_json::json!({
            "endpoint": format!("http://127.0.0.1:{port}/push/phone"),
            "p256dh": URL_SAFE_NO_PAD.encode(public),
            "auth": URL_SAFE_NO_PAD.encode(&auth),
        });
        (key, auth, args)
    }

    /// The whole path on the real binary: a fired alert reaches the fake service
    /// encrypted for the browser, and a delivery the kill left unfinished is sent again
    /// after the restart, as the same event, with no second fire.
    #[test]
    fn test_a_push_decrypts_and_survives_a_restart_as_the_same_event() {
        let db = store_path("alerts-push");
        let (service, seen, answer) = push_service();
        let srv = alert_server(&db, service, true);
        let client = register_external(srv.port);
        let token = sign_in(srv.port, &client);
        let (key, auth, device) = browser(service);
        let list = call_tool(
            srv.port,
            &token,
            "list_server_alerts",
            serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(list["push"]["devices"], 0);
        let public_key = list["push"]["public_key"].as_str().unwrap().to_string();
        call_tool(srv.port, &token, "subscribe_push", device.clone()).unwrap();
        // A level far below the sample price is simply greater, at once.
        let alert = call_tool(
            srv.port,
            &token,
            "create_server_alert",
            serde_json::json!({
                "symbol": "BTCUSDT", "condition": "greater", "trigger": "once",
                "level": 0.01, "push": true, "message": "hello",
            }),
        )
        .unwrap();
        wait_for("the push", || seen.lock().unwrap().len() == 1);
        let first = open_push(&seen.lock().unwrap()[0], &key, &auth);
        assert_eq!(first["alert_id"], alert["id"]);
        assert_eq!(first["symbol"], "BTCUSDT");
        assert_eq!(first["condition"], "greater");
        assert_eq!(first["message"], "hello");
        assert_eq!(first["fire_count"], 1);
        assert!(first["price"].as_f64().unwrap() > 0.01);
        // Unanswered, so the row is still there when the server goes.
        drop(srv);

        answer.store(true, Ordering::SeqCst);
        let srv = alert_server(&db, service, true);
        let token = sign_in(srv.port, &client);
        wait_for("the push again", || seen.lock().unwrap().len() == 2);
        let again = open_push(&seen.lock().unwrap()[1], &key, &auth);
        assert_eq!(again["id"], first["id"], "the same event");
        {
            let seen = seen.lock().unwrap();
            assert_ne!(seen[0], seen[1], "a new key and salt");
        }
        let delivered = || {
            let list = call_tool(
                srv.port,
                &token,
                "list_server_alerts",
                serde_json::json!({}),
            )
            .unwrap();
            list["alerts"][0].clone()
        };
        wait_for("its record", || delivered()["last_push_status"] == 201);
        let view = delivered();
        assert_eq!(
            (&view["fire_count"], &view["active"]),
            (&1.into(), &false.into())
        );
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert_eq!(seen.lock().unwrap().len(), 2, "exactly once more");
        // The key is kept, or every browser's subscription would stop working.
        let list = call_tool(
            srv.port,
            &token,
            "list_server_alerts",
            serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(list["push"]["public_key"], public_key.as_str());
        assert_eq!(list["push"]["devices"], 1);
        let out = call_tool(srv.port, &token, "unsubscribe_push", device).unwrap();
        assert_eq!(out["removed"], true);
    }

    /// A request cannot set the test origin, and on a live source it does not count.
    #[test]
    fn test_the_test_origin_does_not_open_the_push_list_on_a_live_source() {
        let db = store_path("alerts-push-live");
        let (service, seen, _) = push_service();
        let srv = alert_server(&db, service, false);
        let token = sign_in(srv.port, &register_external(srv.port));
        let (_, _, device) = browser(service);
        let e = call_tool(srv.port, &token, "subscribe_push", device).unwrap_err();
        assert!(e.contains("not a known push service"), "{e}");
        assert!(seen.lock().unwrap().is_empty());
    }

    /// With only the flag on, as an operator might leave it: strangers get alerts,
    /// and the setting that allows webhooks is what is missing, not a URL check.
    #[test]
    fn test_without_the_client_setting_no_webhook_is_accepted() {
        let db = store_path("alerts-allowlist");
        let (hook, seen) = receiver();
        let srv = alert_server_for(&db, hook, true, "");
        let token = sign_in(srv.port, &register_external(srv.port));
        let spec = |extra: serde_json::Value| {
            let mut args = serde_json::json!({
                "symbol": "BTCUSDT", "condition": "greater", "trigger": "once", "level": 0.01,
            });
            args.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            args
        };
        let url = format!("http://127.0.0.1:{hook}/hook");
        let e = call_tool(
            srv.port,
            &token,
            "create_server_alert",
            spec(serde_json::json!({"webhook_url": url})),
        )
        .unwrap_err();
        assert!(e.contains("operator allowed"), "{e}");
        call_tool(
            srv.port,
            &token,
            "create_server_alert",
            spec(serde_json::json!({})),
        )
        .unwrap();
        let list = call_tool(
            srv.port,
            &token,
            "list_server_alerts",
            serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(list["webhooks"], false);
        assert_eq!(list["can_create"], true);
        assert!(seen.lock().unwrap().is_empty());
    }

    /// The connection is pinned to checked addresses, so a proxy in the environment
    /// must not get to resolve the name instead. Only the explicit setting uses one.
    #[test]
    fn test_webhooks_ignore_a_proxy_in_the_environment() {
        let db = store_path("alerts-env-proxy");
        let (hook, seen) = receiver();
        let (proxy, proxied) = receiver();
        let port = free_port();
        let mut cmd = server_command(port, None, &db, false);
        let proxy_url = format!("http://127.0.0.1:{proxy}");
        cmd.env("CHARTGEN_SERVER_ALERTS", "1")
            .env("CHARTGEN_SERVER_ALERT_CLIENTS", "*")
            .env("CHARTGEN_DATA_SOURCE", "sample")
            .env(
                "CHARTGEN_WEBHOOK_TEST_ORIGIN",
                format!("http://127.0.0.1:{hook}"),
            )
            .env_remove("NO_PROXY")
            .env_remove("no_proxy");
        for name in ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
            cmd.env(name, &proxy_url);
        }
        let srv = start_server(cmd, port);
        let token = sign_in(srv.port, &register_external(srv.port));
        let args = serde_json::json!({
            "symbol": "BTCUSDT", "condition": "greater", "trigger": "once", "level": 0.01,
            "webhook_url": format!("http://127.0.0.1:{hook}/hook"),
        });
        call_tool(srv.port, &token, "create_server_alert", args).unwrap();
        wait_for("the delivery", || !seen.lock().unwrap().is_empty());
        assert!(proxied.lock().unwrap().is_empty(), "went through the proxy");
    }

    /// A security setting that does not parse leaves the feature off, not open.
    #[test]
    fn test_a_bad_webhook_setting_keeps_alerts_off() {
        let db = store_path("alerts-bad-setting");
        let port = free_port();
        let mut cmd = server_command(port, None, &db, false);
        cmd.env("CHARTGEN_SERVER_ALERTS", "1")
            .env("CHARTGEN_DATA_SOURCE", "sample")
            .env("CHARTGEN_WEBHOOK_BLOCKED_PREFIXES", "not-a-prefix");
        let srv = start_server(cmd, port);
        assert!(!tool_names(srv.port)
            .iter()
            .any(|t| t == "list_server_alerts"));
    }

    /// "Bar close" on the real binary, where the sample feed sends the next candle's
    /// first tick before the close, as a busy pair does. Takes up to a minute.
    #[test]
    fn test_a_close_alert_fires_when_the_candle_rolls_first() {
        let db = store_path("alerts-close");
        let (hook, _) = receiver();
        let srv = alert_server(&db, hook, true);
        let token = sign_in(srv.port, &register_external(srv.port));
        let args = serde_json::json!({
            "symbol": "BTCUSDT", "condition": "greater", "trigger": "once", "level": 0.01,
            "interval": "1m", "evaluate_on": "close",
        });
        call_tool(srv.port, &token, "create_server_alert", args).unwrap();
        let fired = || {
            let list = call_tool(
                srv.port,
                &token,
                "list_server_alerts",
                serde_json::json!({}),
            );
            list.unwrap()["alerts"][0]["fire_count"] == 1
        };
        for _ in 0..700 {
            if fired() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("no close alert fired within 70 s");
    }

    /// The test receiver setting means nothing outside the sample source, so
    /// a live server can never be pointed at loopback, however it is configured.
    #[test]
    fn test_loopback_is_refused_on_a_live_source_even_with_the_test_setting() {
        let db = store_path("alerts-live");
        let (hook, seen) = receiver();
        let srv = alert_server(&db, hook, false);
        let port = srv.port;
        let token = sign_in(port, &register_external(port));
        let e = call_tool(
            port,
            &token,
            "create_server_alert",
            serde_json::json!({
                "symbol": "BTCUSDT", "condition": "greater", "trigger": "once", "level": 1,
                "webhook_url": format!("http://127.0.0.1:{hook}/hook"),
            }),
        )
        .unwrap_err();
        assert!(e.contains("https"), "{e}");
        assert!(seen.lock().unwrap().is_empty());
    }
}

/// External MCP clients keep their registration across a restart without
/// `CHARTGEN_SCRIPTING`, which stays off (ADR-0036). Without the `scripting`
/// feature there is no store, so the client is gone.
#[test]
fn test_registered_client_survives_restart_without_scripting() {
    let db = store_path("restart");
    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    let client = register_external(port);
    let token = sign_in(port, &client);
    let tools = tool_names(port);
    assert!(tools.iter().any(|t| t == "generate_chart"), "{tools:?}");
    for script_tool in SCRIPT_TOOLS {
        assert!(!tools.iter().any(|t| t == script_tool), "{script_tool}");
    }
    drop(srv);

    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    assert_eq!(tool_names(port), tools);
    // Not just unlisted: a call to one is refused as well.
    let live = register_external(port);
    let auth = format!("Authorization: Bearer {}", sign_in(port, &live));
    for script_tool in SCRIPT_TOOLS {
        let call = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": script_tool, "arguments": {}},
        });
        let headers = [JSON_POST[0], JSON_POST[1], auth.as_str()];
        let (st, _, resp) = http(port, "POST", "/mcp", &headers, &call.to_string());
        assert_eq!(st, 200, "{resp}");
        let expected = format!("Unknown tool: {script_tool}");
        assert!(resp.contains(&expected), "{script_tool}: {resp}");
    }
    if cfg!(feature = "scripting") {
        let new_token = sign_in(port, &client);
        assert_ne!(new_token, token);
        // The access token of a client that holds a refresh token is known again
        // after a restart (ADR-0058), and a new one works beside it.
        assert_eq!(ping_status(port, &token), 200);
        assert_eq!(ping_status(port, &new_token), 200);
    } else {
        assert_unknown_client_json(authorize(port, &client, EXTERNAL_REDIRECT), "no store");
    }
    drop(srv);

    // The same store with scripting on: the client is back, and the tools
    // asserted absent above do exist, so that check can fail.
    #[cfg(feature = "scripting")]
    {
        let port = free_port();
        let _srv = spawn_server_on(port, None, &db, true);
        sign_in(port, &client);
        let tools = tool_names(port);
        for script_tool in SCRIPT_TOOLS {
            assert!(tools.iter().any(|t| t == script_tool), "{script_tool}");
        }
    }
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// The store stays bounded without scripting: a client that never got a token
/// is collected after a day, one that did is kept, also from the in-memory
/// table that the next registration sweeps.
#[cfg(feature = "scripting")]
#[test]
fn test_restart_collects_unused_clients_and_keeps_tokened_ones() {
    let db = store_path("restart-gc");
    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    let (used, unused) = (register_external(port), register_external(port));
    sign_in(port, &used);
    drop(srv);

    let two_days = 2 * 24 * 3600;
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute(
            "UPDATE oauth_clients SET created_at = created_at - ?1",
            [two_days],
        )
        .unwrap();

    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    register_external(port);
    sign_in(port, &used);
    assert_unknown_client_json(authorize(port, &unused, EXTERNAL_REDIRECT), "unused");
    drop(srv);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// A failed GC write at startup must not cost the clients it could still read.
#[cfg(feature = "scripting")]
#[test]
fn test_failed_gc_at_startup_still_restores_clients() {
    let db = store_path("gc-fails");
    seed_tokened_clients(&db, 1, 0);
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "INSERT INTO oauth_clients (client_id, redirect_uris, created_at)
             VALUES ('unused', '[]', 0);
         CREATE TRIGGER no_delete BEFORE DELETE ON oauth_clients
             BEGIN SELECT RAISE(ABORT, 'read-only'); END;",
    )
    .unwrap();
    drop(conn);

    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    assert_eq!(authorize(port, "c0000", EXTERNAL_REDIRECT).0, 307);
    drop(srv);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// A failed GC write must not cost a new registration its row either.
#[cfg(feature = "scripting")]
#[test]
fn test_failed_gc_at_registration_still_saves_the_client() {
    let db = store_path("gc-fails-register");
    seed_tokened_clients(&db, 1, 0);
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "INSERT INTO oauth_clients (client_id, redirect_uris, created_at)
             VALUES ('unused', '[]', 0);
         CREATE TRIGGER no_delete BEFORE DELETE ON oauth_clients
             BEGIN SELECT RAISE(ABORT, 'read-only'); END;",
    )
    .unwrap();
    drop(conn);

    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    let client = register_external(port);
    drop(srv);

    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    assert_eq!(authorize(port, &client, EXTERNAL_REDIRECT).0, 307);
    drop(srv);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// `/register` refuses script schemes, but a store can hold older rows, and
/// `/authorize` must not emit one as a `Location` or issue a code for it.
#[cfg(feature = "scripting")]
#[test]
fn test_authorize_refuses_a_stored_script_redirect_uri() {
    let db = store_path("stored-script-uri");
    let uris = [
        "javascript:alert(1)",
        "JAVASCRIPT:alert(1)",
        "data:text/html,x",
        "vbscript:x",
    ];
    chartgen::scripting::persistence::Store::open(&db).unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    for (i, uri) in uris.iter().enumerate() {
        conn.execute(
            "INSERT INTO oauth_clients (client_id, redirect_uris, created_at, last_token_at)
             VALUES (?1, ?2, 0, 0)",
            rusqlite::params![format!("old{i}"), serde_json::json!([uri]).to_string()],
        )
        .unwrap();
    }
    drop(conn);

    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    for (i, uri) in uris.iter().enumerate() {
        let (st, head, body) = authorize(port, &format!("old{i}"), uri);
        assert_eq!(st, 400, "{uri}: {head}");
        assert!(!head.contains("location:"), "{uri}: {head}");
        assert!(body.contains("invalid_request"), "{uri}: {body}");
    }
    drop(srv);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// A client registered before `last_token_at` existed, or whose use could not
/// be recorded, looks unused. If it owns a script it must not be collected.
#[cfg(feature = "scripting")]
#[test]
fn test_startup_gc_spares_script_owners() {
    let db = store_path("gc-owner");
    seed_tokened_clients(&db, 2, 1);
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute(
            "UPDATE oauth_clients SET last_token_at = NULL, created_at = 0",
            [],
        )
        .unwrap();

    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    assert_eq!(authorize(port, "c0000", EXTERNAL_REDIRECT).0, 307);
    assert_unknown_client_json(authorize(port, "c0001", EXTERNAL_REDIRECT), "no scripts");
    drop(srv);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// Writing the token's use to a busy store must not stall other requests.
#[cfg(feature = "scripting")]
#[test]
fn test_busy_store_does_not_stall_authenticated_calls_during_token() {
    let db = store_path("busy");
    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    let client = register_external(port);
    let token = sign_in(port, &client);
    let (_, head, _) = authorize(port, &client, EXTERNAL_REDIRECT);
    let code = head
        .split("code=")
        .nth(1)
        .and_then(|rest| rest.split(['&', '\r']).next())
        .unwrap()
        .to_string();

    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": EXTERNAL_REDIRECT,
        "client_id": client,
        "code_verifier": PKCE_VERIFIER,
    })
    .to_string();
    let exchange = std::thread::spawn(move || http(port, "POST", "/token", &JSON_POST, &body).0);
    std::thread::sleep(std::time::Duration::from_millis(500));

    let started = std::time::Instant::now();
    assert_eq!(ping_status(port, &token), 200);
    let waited = started.elapsed();
    assert!(waited.as_secs() < 2, "ping waited {waited:?}");
    assert!(
        !exchange.is_finished(),
        "/token should still be on the store"
    );

    conn.execute_batch("COMMIT").unwrap();
    assert_eq!(exchange.join().unwrap(), 200);
    drop(srv);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// Reading the script owners at a full table must not stall other requests
/// while the store is busy.
#[cfg(feature = "scripting")]
#[test]
fn test_busy_store_does_not_stall_authenticated_calls_at_a_full_table() {
    let db = store_path("busy-full");
    seed_tokened_clients(&db, MAX_CLIENTS - 1, 0);
    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    let live = register_external(port);
    let token = sign_in(port, &live);

    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let registering = std::thread::spawn(move || register(port, EXTERNAL_REDIRECT).0);
    std::thread::sleep(std::time::Duration::from_millis(500));

    let started = std::time::Instant::now();
    assert_eq!(ping_status(port, &token), 200);
    let waited = started.elapsed();
    assert!(waited.as_secs() < 2, "ping waited {waited:?}");
    assert!(
        !registering.is_finished(),
        "/register should be on the store"
    );

    conn.execute_batch("COMMIT").unwrap();
    assert_eq!(registering.join().unwrap(), 201);
    assert_unknown_client_json(authorize(port, "c0000", EXTERNAL_REDIRECT), "evicted");
    assert_eq!(ping_status(port, &token), 200);
    drop(srv);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// A store that cannot be opened costs persistence, not the server.
#[test]
fn test_unopenable_store_is_not_fatal() {
    let db = store_path("no-store");
    std::fs::write(&db, "a file, not a directory").unwrap();
    let unopenable = db.join("chartgen.db");
    let dir = db.parent().unwrap();
    let (log, home) = (dir.join("stderr.log"), dir.join("home"));
    std::fs::create_dir(&home).unwrap();
    let port = free_port();
    let mut cmd = server_command(port, None, &unopenable, false);
    cmd.stderr(std::fs::File::create(&log).unwrap())
        .env("HOME", &home);
    let srv = start_server(cmd, port);
    let client = register_external(port);
    sign_in(port, &client);
    drop(srv);

    // It tried the configured path, and did not fall back to the default one.
    assert!(
        !home.join(".chartgen/chartgen.db").exists(),
        "the server ignored CHARTGEN_DB"
    );
    if cfg!(feature = "scripting") {
        let log = std::fs::read_to_string(&log).unwrap();
        let expected = format!("cannot open {}", unopenable.display());
        assert!(log.contains(&expected), "{log}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// `/register` refuses schemes a browser would run, and keeps the native ones
/// MCP clients use for their callback, which `/authorize` then redirects to.
#[test]
fn test_register_refuses_script_redirect_uris_and_keeps_native_ones() {
    let srv = spawn_server(None);
    let p = srv.port;
    for uri in [
        "javascript:alert(1)",
        "JAVASCRIPT:alert(1)",
        "data:text/html,x",
        "vbscript:x",
    ] {
        let (st, resp) = register(p, uri);
        assert_eq!(st, 400, "{uri}: {resp}");
        assert!(resp.contains("invalid_redirect_uri"), "{uri}: {resp}");
    }
    for uri in [
        "cursor://anysphere.cursor-retrieval/oauth/callback",
        "vscode://vscode.dev/redirect",
    ] {
        let (st, resp) = register(p, uri);
        assert_eq!(st, 201, "{uri}: {resp}");
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        let (st, head, _) = authorize(p, v["client_id"].as_str().unwrap(), uri);
        assert_eq!(st, 307, "{head}");
        assert!(head.contains(&format!("location: {uri}?code=")), "{head}");
    }
}

/// What `/register` accepts is what `/authorize` accepts, so a client with a bad
/// redirect URI finds out when it registers, not when it signs in.
#[test]
fn test_register_and_authorize_agree_on_redirect_uris() {
    let srv = spawn_server(None);
    let p = srv.port;
    for uri in [
        "my_app://cb",
        "com.example.my_app:/cb",
        "1password://cb",
        "127.0.0.1:1234/cb",
        "/relative",
        "http://localhost:99999/cb",
        "http://[fe80::1%25en0]:1234/cb",
    ] {
        let (st, resp) = register(p, uri);
        assert_eq!(st, 400, "{uri}: {resp}");
        assert!(resp.contains("invalid_redirect_uri"), "{uri}: {resp}");
    }
    for uri in [
        "http://localhost:6274/oauth/callback",
        "http://127.0.0.1:33418",
        "http://[::1]:8080/cb",
        "https://claude.ai/api/mcp/auth_callback",
        "https://vscode.dev/redirect",
        "cursor://anysphere.cursor-retrieval/oauth/user-chartgen/callback",
        "windsurf://codeium.windsurf/cb",
        "exp://192.168.1.2:8081/--/cb",
        "com.example.app:/cb",
        "com.example.app:cb",
        "myapp:///cb",
        "myapp://cb",
    ] {
        let (st, resp) = register(p, uri);
        assert_eq!(st, 201, "{uri}: {resp}");
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        let (st, head, _) = authorize(p, v["client_id"].as_str().unwrap(), uri);
        assert_eq!(st, 307, "{uri}: {head}");
    }
}

/// A known client's callback gets `state` encoded, so it can add no parameters
/// and no headers, the same as the redirect for an unknown client.
#[test]
fn test_known_client_redirect_encodes_state() {
    let srv = spawn_server(None);
    let p = srv.port;
    let client = register_external(p);
    let (st, head, _) = authorize_with_state(p, &client, EXTERNAL_REDIRECT, "a%26admin%3D1%23frag");
    assert_eq!(st, 307, "{head}");
    assert!(head.contains("&state=a%26admin%3d1%23frag\r\n"), "{head}");

    let (st, head, _) = authorize_with_state(
        p,
        &client,
        EXTERNAL_REDIRECT,
        "x%0D%0ASet-Cookie:%20pwned=1",
    );
    assert_eq!(st, 307, "{head}");
    assert!(!head.contains("\r\nset-cookie"), "{head}");
    assert!(
        head.contains("&state=x%0d%0aset-cookie%3a+pwned%3d1\r\n"),
        "{head}"
    );

    // An existing query stays, and the code goes in the query, not the fragment.
    let (st, resp) = register(p, "https://client.example/cb?keep=1#frag");
    assert_eq!(st, 201, "{resp}");
    let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let (st, head, _) = authorize(
        p,
        v["client_id"].as_str().unwrap(),
        "https://client.example/cb?keep=1#frag",
    );
    assert_eq!(st, 307, "{head}");
    assert!(
        head.contains("location: https://client.example/cb?keep=1&code="),
        "{head}"
    );
    assert!(head.contains("&state=a+b%26c#frag\r\n"), "{head}");
}

/// With the table full, `/register` makes room instead of refusing, in any
/// build, and a client that got a token outlasts one that never did.
#[test]
fn test_register_at_the_limit_evicts_never_tokened_clients_first() {
    let srv = spawn_server(None);
    let p = srv.port;
    let used = register_external(p);
    sign_in(p, &used);
    let oldest_unused = register_external(p);
    let next_unused = register_external(p);
    for _ in 3..MAX_CLIENTS {
        register_external(p);
    }

    register_external(p);

    assert_unknown_client_json(authorize(p, &oldest_unused, EXTERNAL_REDIRECT), "oldest");
    assert_eq!(authorize(p, &next_unused, EXTERNAL_REDIRECT).0, 307);
    assert_eq!(authorize(p, &used, EXTERNAL_REDIRECT).0, 307);
}

/// Seed `oauth_clients` before a server starts. `c0000` has the oldest token
/// but the newest registration and the last row, so the order can only come
/// from `last_token_at`. Clients with an index below `owners` own a script.
#[cfg(feature = "scripting")]
fn seed_tokened_clients(db: &std::path::Path, count: usize, owners: usize) {
    chartgen::scripting::persistence::Store::open(db).unwrap();
    let mut conn = rusqlite::Connection::open(db).unwrap();
    let tx = conn.transaction().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let uris = format!(r#"["{EXTERNAL_REDIRECT}"]"#);
    for i in (0..count as i64).rev() {
        let id = seeded_client(i as usize);
        let token_at = now - (count as i64 - i) * 3600;
        let created_at = token_at - i * 7200;
        tx.execute(
            "INSERT INTO oauth_clients (client_id, redirect_uris, created_at, last_token_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, uris, created_at, token_at],
        )
        .unwrap();
        if (i as usize) < owners {
            tx.execute(
                "INSERT INTO user_indicators
                 (user_id, name, source, meta_json, source_hash, created_at, updated_at)
                 VALUES (?1, 'x', '', '{}', '', 0, 0)",
                [&id],
            )
            .unwrap();
        }
    }
    tx.commit().unwrap();
}

#[cfg(feature = "scripting")]
fn seeded_client(i: usize) -> String {
    format!("c{i:04}")
}

#[cfg(feature = "scripting")]
fn stored_clients(db: &std::path::Path) -> i64 {
    rusqlite::Connection::open(db)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM oauth_clients", [], |r| r.get(0))
        .unwrap()
}

/// At the limit the client longest out of use that owns no scripts goes,
/// from memory and from the store, and that order holds after a restart.
#[cfg(feature = "scripting")]
#[test]
fn test_full_table_evicts_longest_unused_non_owner_across_restart() {
    let db = store_path("evict");
    seed_tokened_clients(&db, MAX_CLIENTS, 2);
    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);

    // c0000 and c0001 own scripts, so c0002 is the first to go.
    register_external(port);
    assert_unknown_client_json(authorize(port, "c0002", EXTERNAL_REDIRECT), "evicted");
    for kept in ["c0000", "c0001", "c0003"] {
        assert_eq!(authorize(port, kept, EXTERNAL_REDIRECT).0, 307, "{kept}");
    }
    assert_eq!(stored_clients(&db), MAX_CLIENTS as i64);
    drop(srv);

    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    assert_unknown_client_json(authorize(port, "c0002", EXTERNAL_REDIRECT), "still gone");
    // The new client above never got a token, so it goes before anyone else.
    register_external(port);
    assert_eq!(authorize(port, "c0003", EXTERNAL_REDIRECT).0, 307);
    let tokened = register_external(port);
    sign_in(port, &tokened);
    // That took the unused client from the line above. Now only tokened
    // clients are left, and the restored times decide.
    register_external(port);
    assert_unknown_client_json(authorize(port, "c0003", EXTERNAL_REDIRECT), "next oldest");
    assert_eq!(authorize(port, "c0004", EXTERNAL_REDIRECT).0, 307);
    assert_eq!(stored_clients(&db), MAX_CLIENTS as i64);
    drop(srv);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// Only a table of nothing but script owners still refuses.
#[cfg(feature = "scripting")]
#[test]
fn test_full_table_of_script_owners_still_refuses() {
    let db = store_path("evict-owners");
    seed_tokened_clients(&db, MAX_CLIENTS, MAX_CLIENTS);
    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);

    let body = format!(r#"{{"redirect_uris":["{EXTERNAL_REDIRECT}"]}}"#);
    let (st, _, resp) = http(port, "POST", "/register", &JSON_POST, &body);
    assert_eq!(st, 503, "{resp}");
    assert!(resp.contains("client registry is full"), "{resp}");
    assert_eq!(authorize(port, "c0000", EXTERNAL_REDIRECT).0, 307);
    assert_eq!(stored_clients(&db), MAX_CLIENTS as i64);
    drop(srv);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// An evicted client loses its pending code, and recovers by registering again.
#[cfg(feature = "scripting")]
#[test]
fn test_evicted_client_loses_its_pending_code() {
    let db = store_path("evict-code");
    seed_tokened_clients(&db, MAX_CLIENTS - 1, MAX_CLIENTS - 1);
    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    let victim = register_external(port);
    let (_, head, _) = authorize(port, &victim, EXTERNAL_REDIRECT);
    let code = head
        .split("code=")
        .nth(1)
        .and_then(|rest| rest.split(['&', '\r']).next())
        .unwrap()
        .to_string();

    // Everyone else owns scripts, so this has to take the victim.
    let successor = register_external(port);

    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": EXTERNAL_REDIRECT,
        "client_id": victim,
        "code_verifier": PKCE_VERIFIER,
    });
    let (st, _, resp) = http(port, "POST", "/token", &JSON_POST, &body.to_string());
    assert_eq!(st, 400, "{resp}");
    assert!(resp.contains("invalid_grant"), "{resp}");
    assert_unknown_client_json(authorize(port, &victim, EXTERNAL_REDIRECT), "victim");
    assert_eq!(ping_status(port, &sign_in(port, &successor)), 200);
    drop(srv);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// A stranger repeating the whole sign-in flow cannot end a session in use: once
/// everything is live, `/register` refuses and the session keeps working.
#[test]
fn test_stranger_cannot_revoke_a_live_session_at_the_limit() {
    let srv = spawn_server(None);
    let p = srv.port;
    let victim = register_external(p);
    let token = sign_in(p, &victim);
    for _ in 1..MAX_CLIENTS {
        let client = register_external(p);
        sign_in(p, &client);
    }

    let body = format!(r#"{{"redirect_uris":["{EXTERNAL_REDIRECT}"]}}"#);
    let (st, _, resp) = http(p, "POST", "/register", &JSON_POST, &body);
    assert_eq!(st, 503, "{resp}");
    assert!(resp.contains("client registry is full"), "{resp}");

    assert_eq!(ping_status(p, &token), 200);
    assert_eq!(authorize(p, &victim, EXTERNAL_REDIRECT).0, 307);
}

/// A connector added at `/mcp` finds its RFC 9728 metadata, and from it the
/// authorization server, starting from the 401 hint alone.
#[test]
fn test_oauth_discovery_from_mcp() {
    let srv = spawn_server(None);
    let p = srv.port;
    let base = format!("http://localhost:{p}");
    let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"x"}}"#;

    let hint = format!(
        "www-authenticate: bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/mcp\""
    );
    let (st, head, _) = http(p, "POST", "/mcp", &JSON_POST, call);
    assert_eq!(st, 401);
    assert!(head.contains(&hint), "{head}");
    let (st, head, _) = http(p, "GET", "/mcp", &[], "");
    assert_eq!(st, 401);
    assert!(head.contains(&hint), "{head}");

    let (st, _, body) = http(
        p,
        "GET",
        "/.well-known/oauth-protected-resource/mcp",
        &[],
        "",
    );
    assert_eq!(st, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["resource"], format!("{base}/mcp"));
    assert_eq!(v["authorization_servers"][0], base);

    // Root document: the deprecated `POST /` alias.
    let (st, _, body) = http(p, "GET", "/.well-known/oauth-protected-resource", &[], "");
    assert_eq!(st, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["resource"], base);
    let (st, head, _) = http(p, "POST", "/", &JSON_POST, call);
    assert_eq!(st, 401);
    let root_hint = format!("resource_metadata=\"{base}/.well-known/oauth-protected-resource\"");
    assert!(head.contains(&root_hint), "{head}");

    // The authorization server named there is discoverable.
    let (st, _, body) = http(p, "GET", "/.well-known/oauth-authorization-server", &[], "");
    assert_eq!(st, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["issuer"], base);
}

/// TradingView's volume profile page: on equal volumes the row closer to the POC, and
/// then the one above. Volumes `[0, 6, 10, 6, 0]`: the POC and the 6 above hold 16 of 22,
/// past 70%, so the value area is bins 2 and 3 of 0..5.
#[test]
fn test_volume_profile_value_area_ties_go_up() {
    let flat = |price: f64, volume: f64, t: i64| chartgen::data::Bar {
        open: price,
        high: price,
        low: price,
        close: price,
        volume,
        date: t.to_string(),
    };
    let bars = chartgen::data::OhlcvData {
        bars: vec![
            chartgen::data::Bar {
                open: 0.0,
                high: 5.0,
                low: 0.0,
                close: 5.0,
                volume: 0.0,
                date: "0".into(),
            },
            flat(1.5, 6.0, 1),
            flat(2.5, 10.0, 2),
            flat(3.5, 6.0, 3),
        ],
        symbol: None,
        interval: None,
        session: None,
    };
    let vp = indicators::by_name_configured("volume_profile", &serde_json::json!({"bins": 5}))
        .unwrap()
        .unwrap()
        .compute(&bars);
    assert_eq!((vp.hlines[1].y, vp.hlines[2].y), (4.0, 2.0), "VAH, VAL");
    // `session_vp`, `hvn_lvn`, `naked_poc` and `tpo` share another implementation.
    let session = indicators::by_name_configured(
        "session_vp",
        &serde_json::json!({"bins": 5, "bars_per_session": 4}),
    )
    .unwrap()
    .unwrap()
    .compute(&bars);
    assert_eq!((session.lines[1].y[3], session.lines[2].y[3]), (4.0, 2.0));
}

// ===========================================================================
// 14. Sign-in through GitHub and Google (ADR-0044)
// ===========================================================================

/// What the fake provider does with a token request.
#[derive(Clone, Copy, PartialEq)]
enum FakeMode {
    Normal,
    /// A bad code as GitHub answers it: a 200 with an `error` field.
    RefuseToken,
    /// Accept the connection and never answer.
    Hang,
}

#[derive(Default)]
struct FakeState {
    /// By code: (profile JSON, PKCE challenge of the flow, provider).
    codes: std::collections::HashMap<String, (serde_json::Value, String, String)>,
    /// By provider access token.
    tokens: std::collections::HashMap<String, serde_json::Value>,
    next: u64,
}

/// GitHub and Google on loopback: `/<provider>/token` and `/<provider>/userinfo`
/// as each answers them. It checks the secret and the PKCE verifier, so a wrong
/// exchange fails here as it would for real.
struct FakeProvider {
    port: u16,
    state: std::sync::Arc<std::sync::Mutex<FakeState>>,
    mode: std::sync::Arc<std::sync::Mutex<FakeMode>>,
}

const FAKE_SECRET: &str = "fake-provider-secret";

impl FakeProvider {
    fn start() -> Self {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = std::sync::Arc::new(std::sync::Mutex::new(FakeState::default()));
        let mode = std::sync::Arc::new(std::sync::Mutex::new(FakeMode::Normal));
        let (st, md) = (state.clone(), mode.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                let (st, md) = (st.clone(), md.clone());
                std::thread::spawn(move || fake_answer(conn, &st, &md));
            }
        });
        Self { port, state, mode }
    }

    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn set_mode(&self, mode: FakeMode) {
        *self.mode.lock().unwrap() = mode;
    }

    /// The user approves at the provider: a code for `profile`.
    fn approve(&self, provider: &str, profile: serde_json::Value, challenge: &str) -> String {
        let mut s = self.state.lock().unwrap();
        s.next += 1;
        let code = format!("code-{}", s.next);
        s.codes.insert(
            code.clone(),
            (profile, challenge.to_string(), provider.to_string()),
        );
        code
    }
}

fn fake_answer(
    mut conn: std::net::TcpStream,
    state: &std::sync::Mutex<FakeState>,
    mode: &std::sync::Mutex<FakeMode>,
) {
    use base64::Engine as _;
    use sha2::Digest as _;
    use std::io::{Read, Write};
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let (head, body) = loop {
        let n = conn.read(&mut chunk).unwrap_or(0);
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf).to_string();
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let len = head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_owned)
                })
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if body.len() >= len {
                break (head.to_string(), body.to_string());
            }
        }
        if n == 0 {
            return;
        }
    };
    let mut parts = head.split(' ');
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let provider = path.split('/').nth(1).unwrap_or("").to_string();
    let current = *mode.lock().unwrap();
    if current == FakeMode::Hang {
        std::thread::sleep(std::time::Duration::from_secs(30));
        return;
    }
    let answer = |status: &str, json: serde_json::Value, conn: &mut std::net::TcpStream| {
        let body = json.to_string();
        let _ = write!(
            conn,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    };
    if method == "POST" && path.ends_with("/token") {
        let form: std::collections::HashMap<String, String> = url_form(&body);
        let mut s = state.lock().unwrap();
        let bad = serde_json::json!({"error": "bad_verification_code"});
        let Some((profile, challenge, code_provider)) =
            form.get("code").and_then(|c| s.codes.remove(c))
        else {
            return answer("200 OK", bad, &mut conn);
        };
        let verifier = form.get("code_verifier").cloned().unwrap_or_default();
        let computed = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(sha2::Sha256::digest(verifier.as_bytes()));
        if current == FakeMode::RefuseToken
            || code_provider != provider
            || computed != challenge
            || form.get("client_secret").map(String::as_str) != Some(FAKE_SECRET)
        {
            return answer("200 OK", bad, &mut conn);
        }
        s.next += 1;
        let token = format!("provider-token-{}", s.next);
        s.tokens.insert(token.clone(), profile);
        return answer(
            "200 OK",
            serde_json::json!({"access_token": token}),
            &mut conn,
        );
    }
    if method == "GET" && path.ends_with("/userinfo") {
        let bearer = head.lines().find_map(|l| {
            l.strip_prefix("authorization: Bearer ")
                .or_else(|| l.strip_prefix("Authorization: Bearer "))
        });
        let s = state.lock().unwrap();
        return match bearer.and_then(|t| s.tokens.get(t.trim())) {
            Some(profile) => answer("200 OK", profile.clone(), &mut conn),
            None => answer("401 Unauthorized", serde_json::json!({}), &mut conn),
        };
    }
    answer("404 Not Found", serde_json::json!({}), &mut conn);
}

fn url_decode(v: &str) -> String {
    let (bytes, mut out, mut i) = (v.as_bytes(), Vec::new(), 0);
    while i < bytes.len() {
        match bytes[i] {
            b'%' if bytes.len() - i > 2 => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap()
}

fn url_form(raw: &str) -> std::collections::HashMap<String, String> {
    raw.split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (url_decode(k), url_decode(v)))
        .collect()
}

fn url_encode(v: &str) -> String {
    v.bytes().map(|b| format!("%{b:02X}")).collect()
}

/// A browser: keeps cookies, and follows nothing by itself.
#[derive(Clone)]
struct Browser {
    port: u16,
    cookies: std::collections::HashMap<String, String>,
}

/// A response in the browser: status, exact head, body.
struct Page {
    status: u16,
    head: String,
    body: String,
}

impl Page {
    fn location(&self) -> String {
        self.head
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("location")
                    .then(|| v.trim().to_string())
            })
            .unwrap_or_else(|| panic!("no Location in {}", self.head))
    }

    fn set_cookies(&self) -> Vec<String> {
        self.head
            .lines()
            .filter_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("set-cookie")
                    .then(|| v.trim().to_string())
            })
            .collect()
    }

    fn query(&self) -> std::collections::HashMap<String, String> {
        let location = self.location();
        url_form(location.split_once('?').map_or("", |q| q.1))
    }
}

impl Browser {
    fn new(port: u16) -> Self {
        Self {
            port,
            cookies: std::collections::HashMap::new(),
        }
    }

    fn request(&mut self, method: &str, path: &str, extra: &[&str], body: &str) -> Page {
        let cookie = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
        let cookie = format!("Cookie: {cookie}");
        let mut headers = vec![HTML];
        if !self.cookies.is_empty() {
            headers.push(&cookie);
        }
        headers.extend_from_slice(extra);
        let (status, head, body) = http_exact(self.port, method, path, &headers, body);
        let page = Page { status, head, body };
        for set in page.set_cookies() {
            let pair = set.split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            if value.is_empty() {
                self.cookies.remove(name);
            } else {
                self.cookies.insert(name.to_string(), value.to_string());
            }
        }
        page
    }

    fn get(&mut self, path: &str) -> Page {
        self.request("GET", path, &[], "")
    }

    /// `GET /authorize` for a client, as a browser sends it from the app's own page.
    fn authorize(&mut self, client_id: &str, redirect_uri: &str, state: &str) -> Page {
        self.authorize_from(
            client_id,
            redirect_uri,
            state,
            &["Sec-Fetch-Site: same-origin"],
        )
    }

    /// `GET /authorize` with these headers, such as where the browser says it came from.
    fn authorize_from(
        &mut self,
        client_id: &str,
        redirect_uri: &str,
        state: &str,
        headers: &[&str],
    ) -> Page {
        let path = format!(
            "/authorize?response_type=code&client_id={client_id}&redirect_uri={}&code_challenge={PKCE_CHALLENGE}&state={state}",
            url_encode(redirect_uri)
        );
        self.request("GET", &path, headers, "")
    }

    fn has_session(&self) -> bool {
        self.cookies.keys().any(|k| k.contains("chartgen_session"))
    }
}

/// Everything but the fake provider's own page: follow `/authorize`'s answer through
/// the choice of provider to the provider's redirect, then come back as the provider
/// would with an approved code. Returns the answer of the callback.
fn provider_round_trip(
    b: &mut Browser,
    fake: &FakeProvider,
    provider: &str,
    profile: serde_json::Value,
    authorize_answer: &Page,
) -> Page {
    let at_provider = go_to_provider(b, provider, authorize_answer);
    let query = at_provider.query();
    let code = fake.approve(provider, profile, &query["code_challenge"]);
    b.get(&format!(
        "/oauth/callback/{provider}?code={code}&state={}",
        url_encode(&query["state"])
    ))
}

/// From `/authorize`'s answer to the redirect that leaves for the provider.
fn go_to_provider(b: &mut Browser, provider: &str, authorize_answer: &Page) -> Page {
    let mut next = authorize_answer.location();
    if next.starts_with("/oauth/signin") {
        let chooser = b.get(&next);
        assert_eq!(chooser.status, 200, "{}", chooser.body);
        let request = next.split("request=").nth(1).unwrap();
        next = format!("/oauth/login/{provider}?request={request}");
        assert!(chooser.body.contains(&next), "{}", chooser.body);
    }
    let page = b.get(&next);
    assert_eq!(page.status, 303, "{}", page.body);
    page
}

fn profile_of(provider: &str, id: u64, name: &str) -> serde_json::Value {
    match provider {
        "github" => serde_json::json!({"id": id, "login": name, "email": "ignored@example.com"}),
        _ => serde_json::json!({"sub": format!("sub-{id}"), "name": name}),
    }
}

/// A server on the sample source, signing in against `fake` with `providers`.
fn signin_command(
    port: u16,
    db: &std::path::Path,
    fake: &FakeProvider,
    providers: &[&str],
    base: Option<&str>,
    scripting: bool,
) -> std::process::Command {
    let mut cmd = server_command(port, None, db, scripting);
    cmd.env("CHARTGEN_DATA_SOURCE", "sample")
        .env("CHARTGEN_SIGNIN_TEST_PROVIDER_ORIGIN", fake.origin())
        .env(
            "CHARTGEN_BASE_URL",
            base.map_or_else(|| format!("http://localhost:{port}"), str::to_string),
        );
    for provider in providers {
        let upper = provider.to_ascii_uppercase();
        cmd.env(
            format!("CHARTGEN_{upper}_CLIENT_ID"),
            format!("{provider}-id"),
        )
        .env(format!("CHARTGEN_{upper}_CLIENT_SECRET"), FAKE_SECRET);
    }
    cmd
}

fn signin_server(test: &str, fake: &FakeProvider, providers: &[&str]) -> Server {
    let port = free_port();
    let db = store_path(test);
    let mut srv = start_server(
        signin_command(port, &db, fake, providers, None, false),
        port,
    );
    srv.temp_db = Some(db);
    srv
}

fn web_redirect(port: u16) -> String {
    format!("http://localhost:{port}/")
}

fn register_client(port: u16, redirect_uri: &str) -> String {
    let (st, body) = register(port, redirect_uri);
    assert_eq!(st, 201, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    v["client_id"].as_str().unwrap().to_string()
}

fn exchange(port: u16, client_id: &str, redirect_uri: &str, code: &str) -> (u16, String) {
    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": redirect_uri,
        "client_id": client_id,
        "code_verifier": PKCE_VERIFIER,
    });
    let (st, _, body) = http(port, "POST", "/token", &JSON_POST, &body.to_string());
    (st, body)
}

fn token_from(port: u16, client_id: &str, redirect_uri: &str, code: &str) -> String {
    let (st, body) = exchange(port, client_id, redirect_uri, code);
    assert_eq!(st, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    v["access_token"].as_str().unwrap().to_string()
}

/// `GET /oauth/me` with a bearer token: (status, JSON).
fn me(port: u16, token: &str) -> (u16, serde_json::Value) {
    let auth = format!("Authorization: Bearer {token}");
    let (st, _, body) = http(port, "GET", "/oauth/me", &[auth.as_str()], "");
    (st, serde_json::from_str(&body).unwrap())
}

/// Sign `b` in as the web app would and return the token.
fn web_sign_in(
    b: &mut Browser,
    fake: &FakeProvider,
    provider: &str,
    profile: serde_json::Value,
) -> String {
    let port = b.port;
    let client = register_client(port, &web_redirect(port));
    let first = b.authorize(&client, &web_redirect(port), "s1");
    let back = provider_round_trip(b, fake, provider, profile, &first);
    assert_eq!(back.status, 303, "{}", back.body);
    token_from(port, &client, &web_redirect(port), &back.query()["code"])
}

#[test]
fn test_signin_whole_flow_with_both_providers() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-flow", &fake, &["github", "google"]);
    let p = srv.port;
    let cases = [
        ("github", profile_of("github", 7, "octo"), "octo"),
        ("google", profile_of("google", 7, "Ann"), "Ann"),
    ];
    for (provider, profile, name) in cases {
        let mut b = Browser::new(p);
        let client = register_client(p, &web_redirect(p));
        // Not signed in: a chooser offering both, and no code anywhere.
        let first = b.authorize(&client, &web_redirect(p), "app%20state");
        assert_eq!(first.status, 303, "{}", first.body);
        assert!(first.location().starts_with("/oauth/signin?request="));
        let chooser = b.get(&first.location());
        assert!(
            chooser.body.contains("Continue with GitHub"),
            "{}",
            chooser.body
        );
        assert!(
            chooser.body.contains("Continue with Google"),
            "{}",
            chooser.body
        );

        // The provider gets the exact callback, our id, PKCE and a minimal scope.
        let leaving = go_to_provider(&mut b, provider, &first);
        let target = leaving.location();
        assert!(
            target.starts_with(&format!("{}/{provider}/authorize?", fake.origin())),
            "{target}"
        );
        let q = leaving.query();
        assert_eq!(q["client_id"], format!("{provider}-id"));
        assert_eq!(
            q["redirect_uri"],
            format!("http://localhost:{p}/oauth/callback/{provider}")
        );
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["response_type"], "code");
        assert!(!q["state"].is_empty());
        assert!(!target.contains(FAKE_SECRET), "{target}");
        match provider {
            "github" => assert!(!q.contains_key("scope"), "{target}"),
            _ => assert_eq!(q["scope"], "openid profile email"),
        }

        // Back from the provider: a session, then the code on the client's redirect
        // with its state, without consent because this is the web app's own origin.
        let code = fake.approve(provider, profile, &q["code_challenge"]);
        let back = b.get(&format!(
            "/oauth/callback/{provider}?code={code}&state={}",
            url_encode(&q["state"])
        ));
        assert_eq!(back.status, 303, "{}", back.body);
        assert!(b.has_session());
        assert!(
            back.location().starts_with(&web_redirect(p)),
            "{}",
            back.location()
        );
        assert_eq!(back.query()["state"], "app state");

        let token = token_from(p, &client, &web_redirect(p), &back.query()["code"]);
        assert_eq!(ping_status(p, &token), 200);
        let (st, who) = me(p, &token);
        assert_eq!(st, 200);
        assert_eq!(who["sign_in"], true);
        assert_eq!(who["user"]["name"], name);
        assert_eq!(who["user"]["provider"], provider);
    }
}

#[test]
fn test_signin_with_one_provider_skips_the_chooser() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-one", &fake, &["google"]);
    let p = srv.port;
    let mut b = Browser::new(p);
    let client = register_client(p, &web_redirect(p));
    let first = b.authorize(&client, &web_redirect(p), "s");
    assert!(
        first.location().starts_with("/oauth/login/google?request="),
        "{}",
        first.location()
    );
    // The other provider is not on offer, not even by asking for it.
    let request = first
        .location()
        .split("request=")
        .nth(1)
        .unwrap()
        .to_string();
    let other = b.get(&format!("/oauth/login/github?request={request}"));
    assert_eq!(other.status, 404, "{}", other.body);
    let back = provider_round_trip(
        &mut b,
        &fake,
        "google",
        profile_of("google", 1, "Ann"),
        &first,
    );
    assert_eq!(back.status, 303, "{}", back.body);
    assert!(back.location().starts_with(&web_redirect(p)));
}

#[test]
fn test_signin_a_client_id_alone_gets_no_token() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-noauto", &fake, &["github"]);
    let p = srv.port;
    let client = register_external(p);
    // The whole of the old attack: read the code off the Location header.
    let (st, head, _) = authorize(p, &client, EXTERNAL_REDIRECT);
    assert_ne!(st, 307, "{head}");
    assert!(!head.contains("code="), "{head}");
    assert!(head.contains("location: /oauth/login/github"), "{head}");
    // And there is nothing to guess: a made-up code is refused.
    let (st, body) = exchange(
        p,
        &client,
        EXTERNAL_REDIRECT,
        "00000000-0000-4000-8000-000000000000",
    );
    assert_eq!(st, 400, "{body}");
    // Still a bearer-protected server with the same discovery.
    let (st, head, _) = http(p, "GET", "/mcp", &[], "");
    assert_eq!(st, 401);
    assert!(
        head.contains("www-authenticate: bearer resource_metadata="),
        "{head}"
    );
    let (_, _, body) = http(p, "GET", "/.well-known/oauth-authorization-server", &[], "");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v["authorization_endpoint"],
        format!("http://localhost:{p}/authorize")
    );
    assert_eq!(
        v["registration_endpoint"],
        format!("http://localhost:{p}/register")
    );
}

#[test]
fn test_signin_state_is_single_use_and_bound_to_the_browser() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-state", &fake, &["github", "google"]);
    let p = srv.port;
    let client = register_client(p, &web_redirect(p));
    let profile = || profile_of("github", 9, "mallory");
    let start = |b: &mut Browser| {
        let first = b.authorize(&client, &web_redirect(p), "s");
        go_to_provider(b, "github", &first).query()
    };
    let refused = |page: &Page, b: &Browser, case: &str| {
        assert_eq!(page.status, 400, "{case}: {}", page.body);
        assert!(!b.has_session(), "{case}: no session");
        assert!(page.body.contains("expired"), "{case}: {}", page.body);
    };

    // Login CSRF: the attacker's own finished callback, opened by the victim. The
    // victim's browser did not start this flow, so it must not end up logged in.
    let mut attacker = Browser::new(p);
    let q = start(&mut attacker);
    let code = fake.approve("github", profile(), &q["code_challenge"]);
    let url = format!(
        "/oauth/callback/github?code={code}&state={}",
        url_encode(&q["state"])
    );
    let mut victim = Browser::new(p);
    let page = victim.get(&url);
    refused(&page, &victim, "no browser cookie");
    // Nor with a browser cookie of its own.
    let mut victim = Browser::new(p);
    let _ = start(&mut victim);
    let q = start(&mut attacker);
    let code = fake.approve("github", profile(), &q["code_challenge"]);
    let page = victim.get(&format!(
        "/oauth/callback/github?code={code}&state={}",
        url_encode(&q["state"])
    ));
    refused(&page, &victim, "another browser's cookie");

    // A made-up state, and none at all.
    let mut b = Browser::new(p);
    let _ = start(&mut b);
    refused(
        &b.get("/oauth/callback/github?code=x&state=made-up"),
        &b,
        "unknown state",
    );
    refused(&b.get("/oauth/callback/github?code=x"), &b, "no state");

    // A state for GitHub on the Google callback is used up and refused.
    let q = start(&mut b);
    let code = fake.approve("github", profile(), &q["code_challenge"]);
    let state = url_encode(&q["state"]);
    refused(
        &b.get(&format!("/oauth/callback/google?code={code}&state={state}")),
        &b,
        "another provider",
    );
    refused(
        &b.get(&format!("/oauth/callback/github?code={code}&state={state}")),
        &b,
        "after the wrong provider",
    );

    // Replay: the first use works, the second does not, with the same browser.
    let q = start(&mut b);
    let code = fake.approve("github", profile(), &q["code_challenge"]);
    let url = format!(
        "/oauth/callback/github?code={code}&state={}",
        url_encode(&q["state"])
    );
    assert_eq!(b.get(&url).status, 303);
    let mut no_session = b.clone();
    no_session.cookies.retain(|k, _| !k.contains("session"));
    refused(&no_session.get(&url), &no_session, "replay");

    // A pending request belongs to its browser: a stranger's link leads nowhere.
    let mut owner = Browser::new(p);
    let first = owner.authorize(&client, &web_redirect(p), "s");
    let mut other = Browser::new(p);
    assert_eq!(other.get(&first.location()).status, 400);
    let request = first
        .location()
        .split("request=")
        .nth(1)
        .unwrap()
        .to_string();
    assert_eq!(
        other
            .get(&format!("/oauth/login/github?request={request}"))
            .status,
        400
    );
    assert_eq!(
        other
            .get(&format!("/oauth/cancel?request={request}"))
            .status,
        400
    );
}

#[test]
fn test_signin_denied_login_can_be_retried_or_cancelled() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-denied", &fake, &["github", "google"]);
    let p = srv.port;
    let mut b = Browser::new(p);
    let client = register_client(p, &web_redirect(p));
    let first = b.authorize(&client, &web_redirect(p), "app-state");
    let q = go_to_provider(&mut b, "github", &first).query();
    let denied = b.get(&format!(
        "/oauth/callback/github?error=access_denied&error_description=%3Cscript%3Ealert(1)%3C/script%3E&state={}",
        url_encode(&q["state"])
    ));
    assert_eq!(denied.status, 200, "{}", denied.body);
    assert!(denied.body.contains("cancelled"), "{}", denied.body);
    assert!(!denied.body.contains("<script"), "{}", denied.body);
    assert!(
        denied
            .head
            .to_ascii_lowercase()
            .contains("content-security-policy"),
        "{}",
        denied.head
    );
    assert!(!b.has_session());

    // Try again goes back to the chooser with the same request, and works.
    let request = first
        .location()
        .split("request=")
        .nth(1)
        .unwrap()
        .to_string();
    let retry = format!("/oauth/signin?request={request}");
    assert!(denied.body.contains(&retry), "{}", denied.body);
    assert_eq!(b.get(&retry).status, 200);
    let back = provider_round_trip(
        &mut b,
        &fake,
        "github",
        profile_of("github", 1, "a"),
        &first,
    );
    assert_eq!(back.status, 303, "{}", back.body);
    assert!(b.has_session());

    // Cancel tells the client, with its state.
    let mut b = Browser::new(p);
    let first = b.authorize(&client, &web_redirect(p), "app-state");
    let q = go_to_provider(&mut b, "github", &first).query();
    let denied = b.get(&format!(
        "/oauth/callback/github?error=access_denied&state={}",
        url_encode(&q["state"])
    ));
    let request = first
        .location()
        .split("request=")
        .nth(1)
        .unwrap()
        .to_string();
    let cancel = format!("/oauth/cancel?request={request}");
    assert!(denied.body.contains(&cancel), "{}", denied.body);
    let cancelled = b.get(&cancel);
    assert_eq!(cancelled.status, 303);
    assert!(cancelled.location().starts_with(&web_redirect(p)));
    assert_eq!(cancelled.query()["error"], "access_denied");
    assert_eq!(cancelled.query()["state"], "app-state");
    assert!(!cancelled.query().contains_key("code"));
    // Used up: the same link does not redirect twice.
    assert_eq!(b.get(&cancel).status, 400);
}

#[test]
fn test_signin_provider_failures_leave_a_retry() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-down", &fake, &["github", "google"]);
    let p = srv.port;
    let client = register_client(p, &web_redirect(p));
    let attempt = |mode: FakeMode, expect: &str| {
        fake.set_mode(mode);
        let mut b = Browser::new(p);
        let first = b.authorize(&client, &web_redirect(p), "s");
        let page = provider_round_trip(
            &mut b,
            &fake,
            "github",
            profile_of("github", 1, "a"),
            &first,
        );
        assert_eq!(page.status, 502, "{}", page.body);
        assert!(page.body.contains(expect), "{}", page.body);
        assert!(page.body.contains("Try again"), "{}", page.body);
        assert!(!page.body.contains(FAKE_SECRET), "{}", page.body);
        assert!(!b.has_session());
        fake.set_mode(FakeMode::Normal);
        // The retry path works once the provider is back.
        let request = first
            .location()
            .split("request=")
            .nth(1)
            .unwrap()
            .to_string();
        assert!(page
            .body
            .contains(&format!("/oauth/signin?request={request}")));
        let again = provider_round_trip(
            &mut b,
            &fake,
            "github",
            profile_of("github", 1, "a"),
            &first,
        );
        assert_eq!(again.status, 303, "{}", again.body);
    };
    attempt(FakeMode::RefuseToken, "refused");
    attempt(FakeMode::Hang, "could not be reached");

    // A provider error other than a denial is a failure, not a cancelled sign-in.
    let mut b = Browser::new(p);
    let first = b.authorize(&client, &web_redirect(p), "s");
    let q = go_to_provider(&mut b, "google", &first).query();
    let page = b.get(&format!(
        "/oauth/callback/google?error=server_error&state={}",
        url_encode(&q["state"])
    ));
    assert_eq!(page.status, 502, "{}", page.body);
    assert!(page.body.contains("reported a problem"), "{}", page.body);

    // Nothing listens on the provider's port at all.
    let closed = free_port();
    let dead = FakeProvider {
        port: closed,
        state: Default::default(),
        mode: std::sync::Arc::new(std::sync::Mutex::new(FakeMode::Normal)),
    };
    let srv = signin_server("signin-dead", &dead, &["github"]);
    let p = srv.port;
    let client = register_client(p, &web_redirect(p));
    let mut b = Browser::new(p);
    let first = b.authorize(&client, &web_redirect(p), "s");
    let page = provider_round_trip(
        &mut b,
        &dead,
        "github",
        profile_of("github", 1, "a"),
        &first,
    );
    assert_eq!(page.status, 502, "{}", page.body);
    assert!(page.body.contains("could not be reached"), "{}", page.body);
}

#[test]
fn test_signin_session_is_reused_for_a_second_client() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-reuse", &fake, &["github", "google"]);
    let p = srv.port;
    let mut b = Browser::new(p);
    let first = web_sign_in(&mut b, &fake, "github", profile_of("github", 5, "octo"));
    assert_eq!(ping_status(p, &first), 200);
    // Another client of the web app in this browser: straight to the code, as in the
    // flow without sign-in, with no provider and no page.
    let client = register_client(p, &web_redirect(p));
    let again = b.authorize(&client, &web_redirect(p), "s2");
    assert_eq!(again.status, 307, "{}", again.body);
    assert!(again.location().starts_with(&web_redirect(p)));
    let token = token_from(p, &client, &web_redirect(p), &again.query()["code"]);
    assert_eq!(me(p, &token).1["user"]["name"], "octo");
    // The same account again is the same user with a new session, and a renamed one
    // too: the id decides.
    let mut other = Browser::new(p);
    let renamed = web_sign_in(
        &mut other,
        &fake,
        "github",
        profile_of("github", 5, "renamed"),
    );
    assert_eq!(me(p, &renamed).1["user"]["name"], "renamed");
    // The same person on the other provider is another user: no linking.
    let mut google = Browser::new(p);
    let token = web_sign_in(
        &mut google,
        &fake,
        "google",
        profile_of("google", 5, "octo"),
    );
    assert_eq!(me(p, &token).1["user"]["provider"], "google");
}

/// The form fields of the consent page.
fn consent_fields(body: &str) -> (String, String) {
    let value = |name: &str| {
        let marker = format!("name=\"{name}\" value=\"");
        let rest = body
            .split(&marker)
            .nth(1)
            .unwrap_or_else(|| panic!("no {name} in {body}"));
        rest.split('"').next().unwrap().to_string()
    };
    (value("request"), value("csrf"))
}

#[test]
fn test_signin_third_party_client_needs_consent() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-consent", &fake, &["github"]);
    let p = srv.port;
    let mut b = Browser::new(p);
    web_sign_in(
        &mut b,
        &fake,
        "github",
        profile_of("github", 5, "<b>octo</b>"),
    );

    let body = format!(
        r#"{{"redirect_uris":["{EXTERNAL_REDIRECT}"],"client_name":"<script>alert('x')</script>Claude"}}"#
    );
    let (st, _, body) = http(p, "POST", "/register", &JSON_POST, &body);
    assert_eq!(st, 201, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let client = v["client_id"].as_str().unwrap().to_string();

    let first = b.authorize(&client, EXTERNAL_REDIRECT, "st");
    assert_eq!(first.status, 303, "{}", first.body);
    assert!(!first.location().contains("code="), "{}", first.location());
    assert!(first.location().starts_with("/oauth/consent?request="));
    let page = b.get(&first.location());
    assert_eq!(page.status, 200, "{}", page.body);
    // The page that asks for an approval cannot be framed, cached or sniffed, and tells
    // no other site where the browser came from (ADR-0044, ADR-0062).
    let head = page.head.to_ascii_lowercase();
    for line in [
        "x-frame-options: deny",
        "content-security-policy: default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'",
        "x-content-type-options: nosniff",
        "referrer-policy: same-origin",
        "cache-control: no-store",
    ] {
        assert_eq!(head.matches(line).count(), 1, "{line}: {head}");
    }
    // The redirect host is what to trust, and what the client chose is escaped.
    let host = reqwest::Url::parse(EXTERNAL_REDIRECT).unwrap();
    assert!(
        page.body
            .contains(&format!(">{}<", host.host_str().unwrap())),
        "{}",
        page.body
    );
    assert!(!page.body.contains("<script"), "{}", page.body);
    assert!(
        page.body
            .contains("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;Claude"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("&lt;b&gt;octo&lt;/b&gt;"),
        "{}",
        page.body
    );
    let head = page.head.to_ascii_lowercase();
    assert!(head.contains("frame-ancestors 'none'"), "{head}");
    assert!(head.contains("cache-control: no-store"), "{head}");
    // `no-referrer` would make a browser send `Origin: null` with the form below.
    assert!(head.contains("referrer-policy: same-origin"), "{head}");

    let (request, csrf) = consent_fields(&page.body);
    let form = "Content-Type: application/x-www-form-urlencoded";
    let post = |b: &mut Browser, fields: String, extra: &[&str]| {
        let mut headers = vec![form];
        headers.extend_from_slice(extra);
        b.request("POST", "/oauth/consent", &headers, &fields)
    };
    let approve = format!("request={request}&csrf={csrf}&decision=approve");

    // Refused: no token, a wrong one, another browser, another site. Each leaves the
    // request as it was, so the genuine answer below still works.
    let refusals: [(String, Browser, &[&str]); 5] = [
        (
            format!("request={request}&csrf=&decision=approve"),
            b.clone(),
            &[],
        ),
        (
            format!("request={request}&csrf=wrong&decision=approve"),
            b.clone(),
            &[],
        ),
        (
            approve.clone(),
            {
                let mut other = b.clone();
                other.cookies.retain(|k, _| k.contains("session"));
                other
            },
            &[],
        ),
        (
            approve.clone(),
            b.clone(),
            &["Origin: https://evil.example"],
        ),
        (approve.clone(), b.clone(), &["Origin: null"]),
    ];
    for (fields, mut who, extra) in refusals {
        let refused = post(&mut who, fields.clone(), extra);
        assert_eq!(refused.status, 400, "{fields}: {}", refused.body);
        assert!(!refused.head.contains("code="), "{}", refused.head);
    }
    // A signed-out browser cannot answer either.
    let mut logged_out = b.clone();
    logged_out.cookies.retain(|k, _| !k.contains("session"));
    assert_eq!(post(&mut logged_out, approve.clone(), &[]).status, 400);

    // Deny: the client hears it and gets no code.
    let denied = post(
        &mut b,
        format!("request={request}&csrf={csrf}&decision=deny"),
        &[],
    );
    assert_eq!(denied.status, 303, "{}", denied.body);
    assert!(denied.location().starts_with(EXTERNAL_REDIRECT));
    assert_eq!(denied.query()["error"], "access_denied");
    assert_eq!(denied.query()["state"], "st");
    assert!(!denied.query().contains_key("code"));
    // The decision used the request up.
    assert_eq!(post(&mut b, approve, &[]).status, 400);

    // Still not approved, so asked again; approve this time.
    let first = b.authorize(&client, EXTERNAL_REDIRECT, "st2");
    let page = b.get(&first.location());
    let (request, csrf) = consent_fields(&page.body);
    let origin = format!("Origin: http://localhost:{p}");
    let approved = post(
        &mut b,
        format!("request={request}&csrf={csrf}&decision=approve"),
        &[origin.as_str()],
    );
    assert_eq!(approved.status, 303, "{}", approved.body);
    assert_eq!(approved.query()["state"], "st2");
    let token = token_from(p, &client, EXTERNAL_REDIRECT, &approved.query()["code"]);
    assert_eq!(me(p, &token).1["user"]["name"], "<b>octo</b>");

    // Once approved the client is not asked about again.
    let again = b.authorize(&client, EXTERNAL_REDIRECT, "st3");
    assert_eq!(again.status, 307, "{}", again.body);
    assert_eq!(again.query()["state"], "st3");

    // Approval is per user: someone else is asked.
    let mut other = Browser::new(p);
    web_sign_in(
        &mut other,
        &fake,
        "github",
        profile_of("github", 6, "other"),
    );
    let theirs = other.authorize(&client, EXTERNAL_REDIRECT, "st4");
    assert!(
        theirs.location().starts_with("/oauth/consent?"),
        "{}",
        theirs.location()
    );

    // Without a session it is the provider first, and consent after.
    let mut fresh = Browser::new(p);
    let first = fresh.authorize(&client, EXTERNAL_REDIRECT, "st5");
    let back = provider_round_trip(
        &mut fresh,
        &fake,
        "github",
        profile_of("github", 7, "new"),
        &first,
    );
    assert_eq!(back.status, 303, "{}", back.body);
    assert!(fresh.has_session());
    assert!(
        back.location().starts_with("/oauth/consent?"),
        "{}",
        back.location()
    );
}

#[test]
fn test_signin_logout_ends_the_session_and_only_the_web_apps_tokens() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-logout", &fake, &["github"]);
    let p = srv.port;
    let mut b = Browser::new(p);
    let web = web_sign_in(&mut b, &fake, "github", profile_of("github", 5, "octo"));

    // A connector approved on purpose from the same browser.
    let client = register_external(p);
    let first = b.authorize(&client, EXTERNAL_REDIRECT, "s");
    let page = b.get(&first.location());
    let (request, csrf) = consent_fields(&page.body);
    let approved = b.request(
        "POST",
        "/oauth/consent",
        &["Content-Type: application/x-www-form-urlencoded"],
        &format!("request={request}&csrf={csrf}&decision=approve"),
    );
    let connector = token_from(p, &client, EXTERNAL_REDIRECT, &approved.query()["code"]);
    assert_eq!(ping_status(p, &web), 200);
    assert_eq!(ping_status(p, &connector), 200);

    // Not from another site, even with the cookie.
    let cross = b.request(
        "POST",
        "/oauth/logout",
        &["Origin: https://evil.example"],
        "",
    );
    assert_eq!(cross.status, 403, "{}", cross.body);
    assert!(b.has_session());
    assert_eq!(ping_status(p, &web), 200);

    let session_cookie = b.cookies.clone();
    let out = b.request("POST", "/oauth/logout", &[], "");
    assert_eq!(out.status, 204, "{}", out.head);
    let cleared = out.set_cookies().join("\n");
    assert!(cleared.contains("chartgen_session=;"), "{cleared}");
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    assert!(!b.has_session());
    assert_eq!(
        ping_status(p, &web),
        401,
        "the web app's token ends with its session"
    );
    assert_eq!(
        ping_status(p, &connector),
        200,
        "a connector keeps its token"
    );

    // The old cookie value is dead on the server too, not just cleared.
    let mut stale = Browser {
        port: p,
        cookies: session_cookie,
    };
    let client = register_client(p, &web_redirect(p));
    let again = stale.authorize(&client, &web_redirect(p), "s");
    assert!(
        again.location().starts_with("/oauth/login/github"),
        "{}",
        again.location()
    );
    // Harmless without a session, and the page after it needs nothing.
    assert_eq!(
        Browser::new(p)
            .request("POST", "/oauth/logout", &[], "")
            .status,
        204
    );
    assert_eq!(Browser::new(p).get("/oauth/signed-out").status, 200);
}

#[test]
fn test_signin_codes_and_tokens_carry_the_user() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-bound", &fake, &["github"]);
    let p = srv.port;
    let client = register_client(p, &web_redirect(p));
    let mut a = Browser::new(p);
    let first = a.authorize(&client, &web_redirect(p), "s");
    let back = provider_round_trip(
        &mut a,
        &fake,
        "github",
        profile_of("github", 1, "ann"),
        &first,
    );
    let code = back.query()["code"].clone();
    // A code works once, also after a failed attempt.
    let (st, _) = exchange(p, &client, "https://other.example/", &code);
    assert_eq!(st, 400);
    let (st, _) = exchange(p, &client, &web_redirect(p), &code);
    assert_eq!(st, 400, "a failed exchange used the code up");

    let first = a.authorize(&client, &web_redirect(p), "s");
    let code = first.query()["code"].clone();
    let token = token_from(p, &client, &web_redirect(p), &code);
    let (st, _) = exchange(p, &client, &web_redirect(p), &code);
    assert_eq!(st, 400, "replayed code");

    // Two users through one client are two users.
    let mut b = Browser::new(p);
    let first = b.authorize(&client, &web_redirect(p), "s");
    let back = provider_round_trip(
        &mut b,
        &fake,
        "github",
        profile_of("github", 2, "bob"),
        &first,
    );
    let bob = token_from(p, &client, &web_redirect(p), &back.query()["code"]);
    assert_eq!(me(p, &token).1["user"]["name"], "ann");
    assert_eq!(me(p, &bob).1["user"]["name"], "bob");
    // A bad token is nobody.
    assert_eq!(me(p, "nonsense").0, 401);
}

#[test]
fn test_signin_cookies_on_https() {
    let fake = FakeProvider::start();
    let port = free_port();
    let db = store_path("signin-https");
    let cmd = signin_command(
        port,
        &db,
        &fake,
        &["github"],
        Some("https://chartgen.example"),
        false,
    );
    let mut srv = start_server(cmd, port);
    srv.temp_db = Some(db);
    let redirect = "https://chartgen.example/";
    let client = register_client(port, redirect);
    let mut b = Browser::new(port);
    let first = b.authorize(&client, redirect, "s");
    let browser_cookie = first.set_cookies().join("\n");
    let attributes = |set: &str| {
        assert!(set.contains("HttpOnly"), "{set}");
        assert!(set.contains("; Secure"), "{set}");
        assert!(set.contains("SameSite=Lax"), "{set}");
        assert!(set.contains("Path=/"), "{set}");
    };
    assert!(
        browser_cookie.starts_with("__Host-chartgen_browser="),
        "{browser_cookie}"
    );
    attributes(&browser_cookie);
    let back = provider_round_trip(
        &mut b,
        &fake,
        "github",
        profile_of("github", 1, "a"),
        &first,
    );
    assert_eq!(back.status, 303, "{}", back.body);
    let session = back.set_cookies().join("\n");
    assert!(session.starts_with("__Host-chartgen_session="), "{session}");
    attributes(&session);
    let days: i64 = session
        .split("Max-Age=")
        .nth(1)
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(days > 0 && days <= 31 * 86400, "{session}");
    // A plain-named cookie of the same value is not ours: a subdomain could plant it.
    let value = b.cookies["__Host-chartgen_session"].clone();
    let mut planted = Browser::new(port);
    planted.cookies.insert("chartgen_session".into(), value);
    let answer = planted.authorize(&client, redirect, "s");
    assert!(
        answer.location().starts_with("/oauth/login/"),
        "{}",
        answer.location()
    );
}

/// ADR-0036 with sign-in on: a browser with a forgotten client still gets back to the
/// app by itself, and anyone else still gets the 400 and no way off this origin. The
/// app is served from one origin as in production, callbacks and all.
#[test]
fn test_signin_keeps_the_stale_client_recovery_and_serves_the_app() {
    let fake = FakeProvider::start();
    let dir = crate::test_dir::new(format!("chartgen-signin-app-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<!doctype html>the app").unwrap();
    let port = free_port();
    let db = store_path("signin-web");
    let mut cmd = signin_command(port, &db, &fake, &["github"], None, false);
    cmd.env("CHARTGEN_WEB_DIR", &dir);
    let mut srv = start_server(cmd, port);
    srv.temp_db = Some(db);

    let (st, head, _) = authorize(port, "stale-client", &web_redirect(port));
    assert_eq!(st, 302, "{head}");
    assert!(head.contains("error=invalid_client"), "{head}");
    assert!(!head.contains("code="), "{head}");
    assert_unknown_client_json(
        authorize(port, "stale-client", EXTERNAL_REDIRECT),
        "other origin",
    );

    // Sign-in's paths are its own, and the app still answers everything else.
    let mut b = Browser::new(port);
    let root = b.get("/");
    assert!(root.body.contains("the app"));
    assert!(
        root.head
            .to_ascii_lowercase()
            .contains("referrer-policy: same-origin"),
        "the callback's code is in this page's address: {}",
        root.head
    );
    assert!(b.get("/some/route").body.contains("the app"));
    let page = b.get("/oauth/signed-out");
    assert!(page.body.contains("You are signed out"), "{}", page.body);
    // The callback with its code lands on the app, as a browser would load it.
    let client = register_client(port, &web_redirect(port));
    let first = b.authorize(&client, &web_redirect(port), "s");
    let back = provider_round_trip(
        &mut b,
        &fake,
        "github",
        profile_of("github", 1, "a"),
        &first,
    );
    let location = back.location();
    let path = location
        .strip_prefix(&format!("http://localhost:{port}"))
        .unwrap();
    assert!(b.get(path).body.contains("the app"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// With nothing configured none of the above exists: the code comes straight back
/// with no cookie.
#[test]
fn test_signin_nothing_configured_is_todays_behaviour() {
    let srv = spawn_server(None);
    let p = srv.port;
    let client = register_external(p);
    let (st, head, _) = authorize(p, &client, EXTERNAL_REDIRECT);
    assert_eq!(st, 307, "{head}");
    assert!(head.contains("code="), "{head}");
    assert!(!head.contains("set-cookie"), "{head}");
    let token = sign_in(p, &client);
    assert_eq!(ping_status(p, &token), 200);
    let (st, who) = me(p, &token);
    assert_eq!(st, 200);
    assert_eq!(who["sign_in"], false);
    assert_eq!(who["user"], serde_json::Value::Null);
    for path in [
        "/oauth/signin?request=x",
        "/oauth/login/github?request=x",
        "/oauth/callback/github?state=x",
        "/oauth/consent?request=x",
        "/oauth/cancel?request=x",
    ] {
        assert_eq!(http(p, "GET", path, &[], "").0, 404, "{path}");
    }
    assert_eq!(http(p, "POST", "/oauth/logout", &[], "").0, 404);
    assert_eq!(http(p, "GET", "/oauth/signed-out", &[], "").0, 404);
}

#[test]
fn test_signin_half_a_provider_refuses_to_start() {
    let port = free_port();
    let db = store_path("signin-half");
    let mut cmd = server_command(port, None, &db, false);
    cmd.env("CHARTGEN_GITHUB_CLIENT_ID", "only-an-id");
    // A server that started anyway never exits, so this must not wait for it alone.
    let mut child = cmd.spawn().unwrap();
    let mut status = None;
    for _ in 0..400 {
        status = child.try_wait().unwrap();
        if status.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(
        status.and_then(|s| s.code()),
        Some(1),
        "it must not start open"
    );
}

/// The test provider is the only way past the real ones, and it takes the sample
/// source: on live data the same setting leaves the real GitHub in the Location.
#[test]
fn test_signin_test_provider_is_ignored_on_live_data() {
    let fake = FakeProvider::start();
    let port = free_port();
    let db = store_path("signin-live");
    let mut cmd = signin_command(port, &db, &fake, &["github"], None, false);
    cmd.env_remove("CHARTGEN_DATA_SOURCE");
    let mut srv = start_server(cmd, port);
    srv.temp_db = Some(db);
    let client = register_client(port, &web_redirect(port));
    let mut b = Browser::new(port);
    let first = b.authorize(&client, &web_redirect(port), "s");
    let leaving = b.get(&first.location());
    assert!(
        leaving
            .location()
            .starts_with("https://github.com/login/oauth/authorize?"),
        "{}",
        leaving.location()
    );
}

/// Secrets, codes, states and session values stay out of the log, whatever happens.
#[test]
fn test_signin_logs_no_secrets() {
    let fake = FakeProvider::start();
    let port = free_port();
    let db = store_path("signin-logs");
    let log_path = db.with_extension("log");
    let log = std::fs::File::create(&log_path).unwrap();
    let mut cmd = signin_command(port, &db, &fake, &["github"], None, false);
    cmd.stderr(log.try_clone().unwrap()).stdout(log);
    let mut srv = start_server(cmd, port);
    srv.temp_db = Some(db);

    let mut b = Browser::new(port);
    let client = register_client(port, &web_redirect(port));
    let first = b.authorize(&client, &web_redirect(port), "app-state-value");
    let q = go_to_provider(&mut b, "github", &first).query();
    let code = fake.approve(
        "github",
        profile_of("github", 77, "octo-name"),
        &q["code_challenge"],
    );
    let back = b.get(&format!(
        "/oauth/callback/github?code={code}&state={}",
        url_encode(&q["state"])
    ));
    let our_code = back.query()["code"].clone();
    let token = token_from(port, &client, &web_redirect(port), &our_code);
    let _ = b.request("POST", "/oauth/logout", &[], "");
    fake.set_mode(FakeMode::RefuseToken);
    let first = b.authorize(&client, &web_redirect(port), "s");
    let _ = provider_round_trip(
        &mut b,
        &fake,
        "github",
        profile_of("github", 77, "octo-name"),
        &first,
    );
    drop(srv);

    let logged = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        logged.contains("sign-in succeeded provider=github"),
        "{logged}"
    );
    for secret in [
        FAKE_SECRET,
        code.as_str(),
        q["state"].as_str(),
        q["code_challenge"].as_str(),
        our_code.as_str(),
        token.as_str(),
        client.as_str(),
        "app-state-value",
        "octo-name",
        "github:77",
    ] {
        assert!(!logged.contains(secret), "{secret} in the log:\n{logged}");
    }
    let _ = std::fs::remove_file(&log_path);
}

/// One user cannot hold the shared tables: live clients and tokens are capped per
/// user, so a client dropped from the cap becomes evictable again (ADR-0036).
#[test]
fn test_signin_per_user_limits() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-limits", &fake, &["github"]);
    let p = srv.port;
    let mut b = Browser::new(p);
    let first_token = web_sign_in(&mut b, &fake, "github", profile_of("github", 1, "a"));
    let redirect = web_redirect(p);
    let mut tokens = Vec::new();
    for _ in 0..20 {
        let client = register_client(p, &redirect);
        let answer = b.authorize(&client, &redirect, "s");
        tokens.push(token_from(p, &client, &redirect, &answer.query()["code"]));
    }
    assert_eq!(
        ping_status(p, &first_token),
        401,
        "the client longest out of use lost its token"
    );
    for token in &tokens {
        assert_eq!(ping_status(p, token), 200);
    }
    // One client: the web app gets a token per page load, and they do not pile up. What
    // goes is what nobody uses, not the oldest: a tab open for days keeps working.
    let client = register_client(p, &redirect);
    let mut last = Vec::new();
    for i in 0..101 {
        let answer = b.authorize(&client, &redirect, "s");
        last.push(token_from(p, &client, &redirect, &answer.query()["code"]));
        if i == 50 {
            assert_eq!(ping_status(p, &last[0]), 200, "a tab in use");
        }
    }
    assert_eq!(ping_status(p, &last[0]), 200, "in use, so it stays");
    assert_eq!(ping_status(p, &last[1]), 401, "out of use the longest");
    assert_eq!(ping_status(p, &last[100]), 200);
    // Someone else's are not touched by any of it.
    let mut other = Browser::new(p);
    let theirs = web_sign_in(&mut other, &fake, "github", profile_of("github", 2, "b"));
    for _ in 0..3 {
        let client = register_client(p, &redirect);
        let answer = b.authorize(&client, &redirect, "s");
        token_from(p, &client, &redirect, &answer.query()["code"]);
    }
    assert_eq!(ping_status(p, &theirs), 200);
}

/// A server that takes the client's address from `X-Test-Client`, so these tests can
/// be many addresses and not one exempt loopback.
fn signin_server_behind_proxy(test: &str, fake: &FakeProvider) -> Server {
    let port = free_port();
    let db = store_path(test);
    let mut cmd = signin_command(port, &db, fake, &["github", "google"], None, false);
    cmd.env("CHARTGEN_TRUSTED_PROXY_HEADER", "X-Test-Client");
    let mut srv = start_server(cmd, port);
    srv.temp_db = Some(db);
    srv
}

/// Nobody can hold sign-in shut with requests that create nothing.
#[test]
fn test_signin_junk_costs_nothing_and_locks_nobody_out() {
    let fake = FakeProvider::start();
    let srv = signin_server_behind_proxy("signin-junk", &fake);
    let p = srv.port;
    let client = register_client(p, &web_redirect(p));
    let from = "X-Test-Client: 198.51.100.7";
    let mut attacker = Browser::new(p);
    for _ in 0..150 {
        let page = attacker.request("GET", "/oauth/login/github?request=junk", &[from], "");
        assert_eq!(page.status, 400, "{}", page.body);
        let page = attacker.request(
            "GET",
            "/oauth/callback/github?state=junk&code=x",
            &[from],
            "",
        );
        assert_eq!(page.status, 400, "{}", page.body);
    }
    // The same address, so everyone behind a shared one, can still start a sign-in.
    let mut victim = Browser::new(p);
    let path = format!(
        "/authorize?response_type=code&client_id={client}&redirect_uri={}&code_challenge={PKCE_CHALLENGE}&state=s",
        url_encode(&web_redirect(p))
    );
    let page = victim.request("GET", &path, &[from], "");
    assert_eq!(page.status, 303, "{}", page.body);
}

/// What creates state is limited per address, and one address is not everyone's.
#[test]
fn test_signin_a_flood_from_one_address_leaves_the_others_alone() {
    let fake = FakeProvider::start();
    let srv = signin_server_behind_proxy("signin-flood", &fake);
    let p = srv.port;
    let client = register_client(p, &web_redirect(p));
    let path = format!(
        "/authorize?response_type=code&client_id={client}&redirect_uri={}&code_challenge={PKCE_CHALLENGE}&state=s",
        url_encode(&web_redirect(p))
    );
    let flood = "X-Test-Client: 198.51.100.7";
    let mut limited = false;
    for _ in 0..125 {
        let page = Browser::new(p).request("GET", &path, &[flood], "");
        if page.status == 429 {
            limited = true;
            break;
        }
    }
    assert!(limited, "a flood from one address must be refused");
    let page = Browser::new(p).request("GET", &path, &["X-Test-Client: 203.0.113.5"], "");
    assert_eq!(page.status, 303, "another address: {}", page.body);
    // Asking for consent creates state too, so a session does not get around the limit.
    let mut signed_in = Browser::new(p);
    web_sign_in(
        &mut signed_in,
        &fake,
        "github",
        profile_of("github", 1, "a"),
    );
    let external = register_external(p);
    let from = "X-Test-Client: 198.51.100.8";
    let limited = (0..125).any(|_| {
        signed_in
            .authorize_from(&external, EXTERNAL_REDIRECT, "s", &[from])
            .status
            == 429
    });
    assert!(limited, "a flood of consent requests must be refused too");
    // With no such header at all it is the peer, here loopback and exempt.
    let page = Browser::new(p).request("GET", &path, &[], "");
    assert_eq!(page.status, 303, "{}", page.body);
    // /register counts per client address too.
    let body = format!(r#"{{"redirect_uris":["{EXTERNAL_REDIRECT}"]}}"#);
    let headers = [JSON_POST[0], JSON_POST[1], "X-Test-Client: 192.0.2.8"];
    let statuses: Vec<u16> = (0..21)
        .map(|_| http(p, "POST", "/register", &headers, &body).0)
        .collect();
    assert_eq!(statuses[19], 201, "{statuses:?}");
    assert_eq!(statuses[20], 429, "{statuses:?}");
}

/// Present but unusable, a value shares one budget. It never reaches the peer's
/// loopback exemption, which a same-host proxy has, however it is spelled.
#[test]
fn test_signin_an_unusable_address_header_cannot_reach_the_exemption() {
    let fake = FakeProvider::start();
    let srv = signin_server_behind_proxy("signin-bad-header", &fake);
    let p = srv.port;
    let client = register_client(p, &web_redirect(p));
    let path = format!(
        "/authorize?response_type=code&client_id={client}&redirect_uri={}&code_challenge={PKCE_CHALLENGE}&state=s",
        url_encode(&web_redirect(p))
    );
    let bad = [
        "X-Test-Client: junk",
        "X-Test-Client: 198.51.100.7, 10.0.0.1",
        "X-Test-Client: 127.0.0.1",
        "X-Test-Client: ::1",
        "X-Test-Client: 198.51.100.7:1234",
    ];
    let accepted = (0..125)
        .take_while(|i| {
            let page = Browser::new(p).request("GET", &path, &[bad[i % bad.len()]], "");
            page.status != 429
        })
        .count();
    assert_eq!(accepted, 120, "all of them share one budget");
    // And a good address is not affected.
    let page = Browser::new(p).request("GET", &path, &["X-Test-Client: 203.0.113.5"], "");
    assert_eq!(page.status, 303, "{}", page.body);
}

/// A /64 is one address for every limit, not 2^64 of them, and two addresses and a
/// /64 flooding away do not reach another's open request.
#[test]
fn test_signin_a_few_addresses_cannot_shut_out_a_stranger() {
    let fake = FakeProvider::start();
    let srv = signin_server_behind_proxy("signin-per-address", &fake);
    let p = srv.port;
    let client = register_client(p, &web_redirect(p));
    let path = format!(
        "/authorize?response_type=code&client_id={client}&redirect_uri={}&code_challenge={PKCE_CHALLENGE}&state=s",
        url_encode(&web_redirect(p))
    );
    let mut stranger = Browser::new(p);
    let open = stranger.request("GET", &path, &["X-Test-Client: 203.0.113.5"], "");
    assert_eq!(open.status, 303, "{}", open.body);

    // The address's own oldest request goes, so its first is dead after 50 more.
    let mut first = Browser::new(p);
    let flood = "X-Test-Client: 198.51.100.7";
    let oldest = first.request("GET", &path, &[flood], "");
    for address in ["198.51.100.7", "198.51.100.8"] {
        let from = format!("X-Test-Client: {address}");
        for _ in 0..60 {
            let page = Browser::new(p).request("GET", &path, &[from.as_str()], "");
            assert_eq!(page.status, 303, "{}", page.body);
        }
    }
    assert_eq!(
        first.get(&oldest.location()).status,
        400,
        "pushed out by its own"
    );

    // Hosts of one /64 share one budget: without that each would have its own 120.
    let accepted = (0..125)
        .take_while(|i| {
            let from = format!("X-Test-Client: 2001:db8:1:1::{i:x}");
            Browser::new(p)
                .request("GET", &path, &[from.as_str()], "")
                .status
                != 429
        })
        .count();
    assert_eq!(accepted, 120, "a /64 is one address");
    let other = Browser::new(p).request("GET", &path, &["X-Test-Client: 2001:db8:1:2::1"], "");
    assert_eq!(other.status, 303, "the next /64 is another: {}", other.body);

    assert_eq!(
        stranger.get(&open.location()).status,
        200,
        "the stranger's still open"
    );

    // /register counts it the same way.
    let body = format!(r#"{{"redirect_uris":["{EXTERNAL_REDIRECT}"]}}"#);
    let statuses: Vec<u16> = (0..21)
        .map(|i| {
            let from = format!("X-Test-Client: 2001:db8:9:9::{i:x}");
            let headers = [JSON_POST[0], JSON_POST[1], from.as_str()];
            http(p, "POST", "/register", &headers, &body).0
        })
        .collect();
    assert_eq!(statuses[19], 201, "{statuses:?}");
    assert_eq!(statuses[20], 429, "{statuses:?}");
}

/// Not named, the header is anyone's to write, so it must do nothing: here 21
/// registrations claiming one address would be refused if it counted.
#[test]
fn test_a_spoofed_address_is_ignored_unless_the_operator_names_the_header() {
    let srv = spawn_server(None);
    let body = format!(r#"{{"redirect_uris":["{EXTERNAL_REDIRECT}"]}}"#);
    let headers = [
        JSON_POST[0],
        JSON_POST[1],
        "CF-Connecting-IP: 192.0.2.8",
        "X-Forwarded-For: 192.0.2.8",
        "X-Real-IP: 192.0.2.8",
    ];
    for i in 0..25 {
        let (st, _, resp) = http(srv.port, "POST", "/register", &headers, &body);
        assert_eq!(st, 201, "registration {i}: {resp}");
    }
}

/// A stranger's client that looks like the web app gets no code for a victim: a link to
/// it from elsewhere leads to consent. Only the app's own origin skips it.
#[test]
fn test_signin_a_look_alike_of_the_web_app_needs_consent_from_elsewhere() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-look-alike", &fake, &["github"]);
    let p = srv.port;
    let redirect = web_redirect(p);
    let mut victim = Browser::new(p);
    web_sign_in(&mut victim, &fake, "github", profile_of("github", 1, "ann"));
    let body = format!(r#"{{"redirect_uris":["{redirect}"],"client_name":"chartgen-web"}}"#);
    let (st, _, body) = http(p, "POST", "/register", &JSON_POST, &body);
    assert_eq!(st, 201, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let look_alike = v["client_id"].as_str().unwrap().to_string();

    for origin in [
        vec!["Sec-Fetch-Site: cross-site"],
        vec!["Sec-Fetch-Site: same-site"],
        // What a link in a mail or a chat sends, also through a redirect.
        vec!["Sec-Fetch-Site: none"],
        vec!["Sec-Fetch-Site: SAME-ORIGIN"],
        vec![],
    ] {
        let page = victim.authorize_from(&look_alike, &redirect, "evil", &origin);
        assert_eq!(page.status, 303, "{origin:?}: {}", page.body);
        assert!(
            page.location().starts_with("/oauth/consent?"),
            "{origin:?}: {}",
            page.location()
        );
        let consent = victim.get(&page.location());
        assert!(
            consent.body.contains("Approve this application?"),
            "{}",
            consent.body
        );
    }
    // From our own origin, where the app asks from, there is no consent to ask.
    let page = victim.authorize_from(
        &look_alike,
        &redirect,
        "s",
        &["Sec-Fetch-Site: same-origin"],
    );
    assert_eq!(page.status, 307, "{}", page.body);
    assert!(page.query().contains_key("code"));

    // Signed out, the answer is the same after the provider, whose callback always
    // looks cross-site: it is where the request began that counts.
    let mut fresh = Browser::new(p);
    let first = fresh.authorize_from(
        &look_alike,
        &redirect,
        "evil",
        &["Sec-Fetch-Site: cross-site"],
    );
    let back = provider_round_trip(
        &mut fresh,
        &fake,
        "github",
        profile_of("github", 2, "b"),
        &first,
    );
    assert!(
        back.location().starts_with("/oauth/consent?"),
        "{}",
        back.location()
    );
    let mut own = Browser::new(p);
    let first = own.authorize(&look_alike, &redirect, "s");
    let back = provider_round_trip(
        &mut own,
        &fake,
        "github",
        profile_of("github", 3, "c"),
        &first,
    );
    assert!(
        back.location().starts_with(&redirect),
        "{}",
        back.location()
    );
}

/// More tabs than the old cap of 20, each with its own token: none ends another's.
#[test]
fn test_signin_many_tabs_do_not_revoke_each_other() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-tabs", &fake, &["github"]);
    let p = srv.port;
    let redirect = web_redirect(p);
    let client = register_client(p, &redirect);
    let mut b = Browser::new(p);
    let first = b.authorize(&client, &redirect, "s");
    let back = provider_round_trip(
        &mut b,
        &fake,
        "github",
        profile_of("github", 1, "a"),
        &first,
    );
    let mut tabs = vec![token_from(p, &client, &redirect, &back.query()["code"])];
    for _ in 0..24 {
        let answer = b.authorize(&client, &redirect, "s");
        tabs.push(token_from(p, &client, &redirect, &answer.query()["code"]));
    }
    for (i, tab) in tabs.iter().enumerate() {
        assert_eq!(ping_status(p, tab), 200, "tab {i} of {}", tabs.len());
    }
}

/// One request cannot fill the table of provider round trips: it holds one at a time.
#[test]
fn test_signin_one_request_cannot_fill_the_round_trips() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-flows", &fake, &["github", "google"]);
    let p = srv.port;
    let client = register_client(p, &web_redirect(p));
    let mut attacker = Browser::new(p);
    let first = attacker.authorize(&client, &web_redirect(p), "s");
    let request = first
        .location()
        .split("request=")
        .nth(1)
        .unwrap()
        .to_string();
    let login = format!("/oauth/login/github?request={request}");
    let early = attacker.get(&login).query()["state"].clone();
    for _ in 0..2100 {
        assert_eq!(attacker.get(&login).status, 303);
    }
    let mut victim = Browser::new(p);
    let first = victim.authorize(&client, &web_redirect(p), "s");
    let back = provider_round_trip(
        &mut victim,
        &fake,
        "github",
        profile_of("github", 1, "a"),
        &first,
    );
    assert_eq!(back.status, 303, "{}", back.body);
    // The attempt it replaced is dead.
    let page = attacker.get(&format!(
        "/oauth/callback/github?code=x&state={}",
        url_encode(&early)
    ));
    assert_eq!(page.status, 400, "{}", page.body);
}

/// A browser holds few requests, on both the sign-in and the consent path.
#[test]
fn test_signin_a_browser_holds_few_open_requests() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-per-browser", &fake, &["github", "google"]);
    let p = srv.port;
    let client = register_client(p, &web_redirect(p));
    let mut b = Browser::new(p);
    let requests: Vec<String> = (0..6)
        .map(|_| b.authorize(&client, &web_redirect(p), "s").location())
        .collect();
    assert_eq!(b.get(&requests[0]).status, 400, "the oldest went");
    assert_eq!(b.get(&requests[1]).status, 200, "the rest stay");
    assert_eq!(b.get(&requests[5]).status, 200);

    let mut signed_in = Browser::new(p);
    web_sign_in(
        &mut signed_in,
        &fake,
        "github",
        profile_of("github", 1, "a"),
    );
    let external = register_external(p);
    let requests: Vec<String> = (0..6)
        .map(|_| {
            signed_in
                .authorize(&external, EXTERNAL_REDIRECT, "s")
                .location()
        })
        .collect();
    assert_eq!(signed_in.get(&requests[0]).status, 400);
    assert_eq!(signed_in.get(&requests[5]).status, 200);
}

/// A free account does not buy unlimited rendering and upstream quota, and one
/// user's flood is not another's problem.
#[test]
fn test_signin_tool_calls_are_limited_per_user() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-calls", &fake, &["github"]);
    let p = srv.port;
    let mut b = Browser::new(p);
    let token = web_sign_in(&mut b, &fake, "github", profile_of("github", 1, "a"));
    let mut other = Browser::new(p);
    let theirs = web_sign_in(&mut other, &fake, "github", profile_of("github", 2, "b"));
    let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_indicators","arguments":{}}}"#;
    let tool_status = |token: &str| {
        let auth = format!("Authorization: Bearer {token}");
        let headers = [JSON_POST[0], JSON_POST[1], auth.as_str()];
        http(p, "POST", "/mcp", &headers, call)
    };
    for i in 0..600 {
        let (st, _, body) = tool_status(&token);
        assert_eq!(st, 200, "call {i}: {body}");
    }
    let (st, head, body) = tool_status(&token);
    assert_eq!(st, 429, "{body}");
    assert!(head.contains("retry-after: 60"), "{head}");
    assert!(body.contains("per user"), "{body}");
    // Not a tool call, so still answered: the client can tell it is signed in.
    assert_eq!(ping_status(p, &token), 200);
    assert_eq!(tool_status(&theirs).0, 200, "another user is not affected");
}

/// Without sign-in, nothing counts tool calls: today's behaviour.
#[test]
fn test_tool_calls_are_not_limited_without_signin() {
    let srv = spawn_server(None);
    let p = srv.port;
    let client = register_external(p);
    let token = sign_in(p, &client);
    let auth = format!("Authorization: Bearer {token}");
    let headers = [JSON_POST[0], JSON_POST[1], auth.as_str()];
    let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_indicators","arguments":{}}}"#;
    for i in 0..610 {
        let (st, _, body) = http(p, "POST", "/mcp", &headers, call);
        assert_eq!(st, 200, "call {i}: {body}");
    }
}

/// Sign-ins in progress are bounded on the server too, which takes 40 addresses to reach,
/// and hitting the bound leaves a signed-in user alone.
#[test]
fn test_signin_pending_logins_are_bounded() {
    let fake = FakeProvider::start();
    let srv = signin_server_behind_proxy("signin-cap", &fake);
    let p = srv.port;
    let mut holder = Browser::new(p);
    web_sign_in(&mut holder, &fake, "github", profile_of("github", 1, "a"));
    let client = register_client(p, &web_redirect(p));
    let path = format!(
        "/authorize?response_type=code&client_id={client}&redirect_uri={}&code_challenge={PKCE_CHALLENGE}&state=s",
        url_encode(&web_redirect(p))
    );
    let mut limited = None;
    'flood: for address in 0..45 {
        let from = format!("X-Test-Client: 198.51.100.{address}");
        for i in 0..50 {
            let page = Browser::new(p).request("GET", &path, &[from.as_str()], "");
            if page.status == 429 {
                limited = Some((address * 50 + i, page));
                break 'flood;
            }
        }
    }
    let (at, page) = limited.expect("a flood of sign-ins must be refused");
    assert!(at >= 1990, "refused too soon: {at}");
    assert!(
        page.head.to_ascii_lowercase().contains("retry-after: 60"),
        "{}",
        page.head
    );
    assert!(page.body.contains("Too many"), "{}", page.body);
    let answer = holder.authorize(&client, &web_redirect(p), "s");
    assert_eq!(answer.status, 307, "{}", answer.body);
}

#[cfg(feature = "scripting")]
const OWN_SCRIPT: &str =
    "PARAMS = []\nMETA = {}\ndef compute(ohlcv, params, chart):\n    chart.line(ohlcv.close)\n";

/// `tools/call` with a token; the text of the answer.
#[cfg(feature = "scripting")]
fn tool_text(port: u16, token: &str, tool: &str, args: serde_json::Value) -> String {
    let auth = format!("Authorization: Bearer {token}");
    let headers = [JSON_POST[0], JSON_POST[1], auth.as_str()];
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": tool, "arguments": args}
    });
    let (st, _, body) = http(port, "POST", "/mcp", &headers, &call.to_string());
    assert_eq!(st, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    v["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

/// ADR-0044: scripts belong to the user. Two users are apart, one user sees them from
/// any client, and they survive a restart along with the session and the user.
#[cfg(feature = "scripting")]
#[test]
fn test_signin_scripts_follow_the_user_and_sessions_survive_restart() {
    let fake = FakeProvider::start();
    let port = free_port();
    let db = store_path("signin-scripts");
    let start = |fake: &FakeProvider| {
        start_server(
            signin_command(port, &db, fake, &["github"], None, true),
            port,
        )
    };
    let srv = start(&fake);
    let redirect = web_redirect(port);
    let register_script = |token: &str, name: &str| {
        let answer = tool_text(
            port,
            token,
            "register_indicator",
            serde_json::json!({"name": name, "source": OWN_SCRIPT}),
        );
        assert!(answer.contains(name), "{answer}");
    };
    let mine = |token: &str| tool_text(port, token, "list_my_indicators", serde_json::json!({}));

    let mut ann = Browser::new(port);
    let ann_token = web_sign_in(&mut ann, &fake, "github", profile_of("github", 1, "ann"));
    let mut bob = Browser::new(port);
    let bob_token = web_sign_in(&mut bob, &fake, "github", profile_of("github", 2, "bob"));
    register_script(&ann_token, "anns_line");
    register_script(&bob_token, "bobs_line");
    assert!(mine(&ann_token).contains("anns_line"));
    assert!(
        !mine(&ann_token).contains("bobs_line"),
        "{}",
        mine(&ann_token)
    );
    assert!(
        !mine(&bob_token).contains("anns_line"),
        "{}",
        mine(&bob_token)
    );

    // Ann from another client of hers: the same scripts. With client ownership
    // this would be an empty list.
    let client = register_client(port, &redirect);
    let answer = ann.authorize(&client, &redirect, "s");
    let second = token_from(port, &client, &redirect, &answer.query()["code"]);
    assert!(mine(&second).contains("anns_line"), "{}", mine(&second));

    // The limit is the user's: more clients do not buy more scripts.
    for i in 1..50 {
        register_script(&second, &format!("more_{i}"));
    }
    let answer = tool_text(
        port,
        &ann_token,
        "register_indicator",
        serde_json::json!({"name": "one_too_many", "source": OWN_SCRIPT}),
    );
    assert!(answer.contains("50"), "{answer}");

    // The file holds a hash of the session, not what would open it.
    {
        use sha2::Digest as _;
        let cookie = ann.cookies["chartgen_session"].clone();
        let conn = rusqlite::Connection::open(&db).unwrap();
        let hashes: Vec<String> = conn
            .prepare("SELECT token_hash FROM sessions")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(!hashes.contains(&cookie), "the raw value is stored");
        let hashed = hex::encode(sha2::Sha256::digest(cookie.as_bytes()));
        assert!(hashes.contains(&hashed), "{hashes:?}");
        // Getting a token counts as activity, which is what keeps a user's alerts alive.
        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM users WHERE id = 'github:1' AND last_token_at IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(active, 1);
    }

    // A restart: the session and user are back, the token is not, and signing the
    // app in again takes no provider.
    drop(srv);
    let srv = start(&fake);
    assert_eq!(
        ping_status(port, &ann_token),
        401,
        "tokens stay memory-only"
    );
    let answer = ann.authorize(&client, &redirect, "s");
    assert_eq!(answer.status, 307, "the session came back: {}", answer.body);
    let token = token_from(port, &client, &redirect, &answer.query()["code"]);
    assert_eq!(me(port, &token).1["user"]["name"], "ann");
    assert!(mine(&token).contains("anns_line"), "{}", mine(&token));

    // Approval survives too.
    let external = register_external(port);
    let first = ann.authorize(&external, EXTERNAL_REDIRECT, "s");
    let page = ann.get(&first.location());
    let (request, csrf) = consent_fields(&page.body);
    let approved = ann.request(
        "POST",
        "/oauth/consent",
        &["Content-Type: application/x-www-form-urlencoded"],
        &format!("request={request}&csrf={csrf}&decision=approve"),
    );
    assert_eq!(approved.status, 303);
    drop(srv);
    let srv = start(&fake);
    let again = ann.authorize(&external, EXTERNAL_REDIRECT, "s");
    assert_eq!(again.status, 307, "the approval came back: {}", again.body);
    // A signed-out session stays signed out across a restart.
    assert_eq!(ann.request("POST", "/oauth/logout", &[], "").status, 204);
    let stale = ann.cookies.clone();
    drop(srv);
    let _srv = start(&fake);
    let mut ann = Browser {
        port,
        cookies: stale,
    };
    let answer = ann.authorize(&client, &redirect, "s");
    assert_ne!(answer.status, 307, "{}", answer.head);
}

/// A script made through `client` with sign-in off, in `db`.
#[cfg(feature = "scripting")]
fn seed_script_through(port: u16, db: &std::path::Path, name: &str) -> String {
    let old = spawn_server_on(port, None, db, true);
    let redirect = web_redirect(port);
    let client = register_client(port, &redirect);
    let (st, head, _) = authorize(port, &client, &redirect);
    assert_eq!(st, 307, "{head}");
    let code = head
        .split("code=")
        .nth(1)
        .unwrap()
        .split(['&', '\r'])
        .next()
        .unwrap()
        .to_string();
    let token = token_from(port, &client, &redirect, &code);
    let answer = tool_text(
        port,
        &token,
        "register_indicator",
        serde_json::json!({"name": name, "source": OWN_SCRIPT}),
    );
    assert!(answer.contains(name), "{answer}");
    drop(old);
    client
}

// --- Refresh tokens (ADR-0058) ---

/// `POST /token` with a refresh token: (status, JSON).
#[cfg(feature = "scripting")]
fn refresh(port: u16, client_id: &str, refresh_token: &str) -> (u16, serde_json::Value) {
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "client_id": client_id,
        "refresh_token": refresh_token,
    });
    let (st, _, body) = http(port, "POST", "/token", &JSON_POST, &body.to_string());
    (st, serde_json::from_str(&body).unwrap_or_default())
}

/// An authorization code works once, with sign-in off as with it on. A second exchange
/// used to succeed for the 5 minutes a code lives, and would now mint a second
/// refresh token as well.
#[test]
fn test_an_authorization_code_is_exchanged_once() {
    let db = store_path("code-once");
    let port = free_port();
    let _srv = spawn_server_on(port, None, &db, false);
    let client = register_external(port);
    let (st, head, _) = authorize(port, &client, EXTERNAL_REDIRECT);
    assert_eq!(st, 307, "{head}");
    let code = head
        .split("code=")
        .nth(1)
        .and_then(|rest| rest.split(['&', '\r']).next())
        .unwrap();
    let (st, first) = exchange(port, &client, EXTERNAL_REDIRECT, code);
    assert_eq!(st, 200, "{first}");
    let (st, second) = exchange(port, &client, EXTERNAL_REDIRECT, code);
    assert_eq!(st, 400, "{second}");
    assert!(second.contains("invalid_grant"), "{second}");
    assert!(!second.contains("access_token") && !second.contains("refresh_token"));
    #[cfg(feature = "scripting")]
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM refresh_tokens", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "one authorization, one refresh token");
    }

    // A failed attempt uses the code up too: it cannot be tried against.
    let (_, head, _) = authorize(port, &client, EXTERNAL_REDIRECT);
    let code = head
        .split("code=")
        .nth(1)
        .and_then(|rest| rest.split(['&', '\r']).next())
        .unwrap();
    let wrong = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": EXTERNAL_REDIRECT,
        "client_id": client,
        "code_verifier": "not-the-verifier",
    });
    let (st, _, body) = http(port, "POST", "/token", &JSON_POST, &wrong.to_string());
    assert_eq!(st, 400, "{body}");
    let (st, body) = exchange(port, &client, EXTERNAL_REDIRECT, code);
    assert_eq!(st, 400, "{body}");
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// `POST /revoke`, as the form RFC 7009 names.
#[cfg(feature = "scripting")]
fn revoke(port: u16, client_id: &str, token: &str) -> u16 {
    let body = format!("token={token}&client_id={client_id}");
    let form = ["Content-Type: application/x-www-form-urlencoded"];
    http(port, "POST", "/revoke", &form, &body).0
}

/// The code flow for an external client with sign-in off: the whole `/token` answer.
#[cfg(feature = "scripting")]
fn authorize_headless(port: u16, client_id: &str) -> serde_json::Value {
    let (st, head, _) = authorize(port, client_id, EXTERNAL_REDIRECT);
    assert_eq!(st, 307, "{head}");
    let code = head
        .split("code=")
        .nth(1)
        .and_then(|rest| rest.split(['&', '\r']).next())
        .unwrap();
    let (st, body) = exchange(port, client_id, EXTERNAL_REDIRECT, code);
    assert_eq!(st, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}

#[cfg(feature = "scripting")]
fn text(v: &serde_json::Value, field: &str) -> String {
    v[field].as_str().unwrap_or_default().to_string()
}

/// A headless client is authorized once and then left alone: restarts, which are
/// what a deploy is, cost it one refused call and one refresh, and no person.
/// Without the `scripting` feature there is no store and nothing survives.
#[cfg(feature = "scripting")]
#[test]
fn test_a_headless_client_keeps_working_across_restarts() {
    let db = store_path("refresh-restart");
    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    let (_, _, meta) = http(
        port,
        "GET",
        "/.well-known/oauth-authorization-server",
        &[],
        "",
    );
    let meta: serde_json::Value = serde_json::from_str(&meta).unwrap();
    assert_eq!(
        meta["grant_types_supported"],
        serde_json::json!(["authorization_code", "refresh_token"])
    );
    assert_eq!(
        meta["revocation_endpoint"],
        format!("http://localhost:{port}/revoke")
    );
    let client = register_external(port);
    let first = authorize_headless(port, &client);
    let (access, mut token) = (text(&first, "access_token"), text(&first, "refresh_token"));
    assert!(!token.is_empty(), "{first}");
    assert_eq!(ping_status(port, &access), 200);
    drop(srv);

    let mut access_tokens = vec![access];
    for _ in 0..2 {
        let port = free_port();
        let _srv = spawn_server_on(port, None, &db, false);
        // The same access token calls a tool: no 401, and no `/token` in between.
        let (current, earlier) = access_tokens.split_last().unwrap();
        let listed = tool_text(port, current, "list_indicators", serde_json::json!({}));
        assert!(listed.contains("rsi"), "{listed}");
        for old in earlier {
            assert_eq!(ping_status(port, old), 401, "one from before an exchange");
        }
        let (st, next) = refresh(port, &client, &token);
        assert_eq!(st, 200, "{next}");
        assert_eq!(next["token_type"], "Bearer");
        assert_eq!(next["expires_in"], 604800);
        assert_eq!(ping_status(port, &text(&next, "access_token")), 200);
        assert_ne!(text(&next, "refresh_token"), token, "rotated");
        access_tokens.push(text(&next, "access_token"));
        token = text(&next, "refresh_token");
    }

    // Revoked, it stays revoked through the next start.
    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    let (st, last) = refresh(port, &client, &token);
    assert_eq!(st, 200, "{last}");
    let (access, token) = (text(&last, "access_token"), text(&last, "refresh_token"));
    assert_eq!(revoke(port, &client, "not-a-token"), 200);
    assert_eq!(ping_status(port, &access), 200);
    assert_eq!(revoke(port, &client, &token), 200);
    assert_eq!(ping_status(port, &access), 401, "its access token goes too");
    let (st, body) = refresh(port, &client, &token);
    assert_eq!((st, text(&body, "error").as_str()), (400, "invalid_grant"));
    drop(srv);
    let port = free_port();
    let _srv = spawn_server_on(port, None, &db, false);
    assert_eq!(ping_status(port, &access), 401, "revoked for good");
    assert_eq!(refresh(port, &client, &token).0, 400);

    // The store holds hashes, never a value that was handed out.
    let conn = rusqlite::Connection::open(&db).unwrap();
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM refresh_tokens", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0, "a revoked family leaves nothing");
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// The operator's revocation in the OAuth reference, for a token nobody holds any
/// more: stop, delete the rows, start.
#[cfg(feature = "scripting")]
#[test]
fn test_refresh_tokens_deleted_from_the_store_are_gone_after_a_restart() {
    let db = store_path("refresh-operator");
    let port = free_port();
    let srv = spawn_server_on(port, None, &db, false);
    let (lost, kept) = (register_external(port), register_external(port));
    let (lost_answer, kept_answer) = (
        authorize_headless(port, &lost),
        authorize_headless(port, &kept),
    );
    let lost_token = text(&lost_answer, "refresh_token");
    let kept_token = text(&kept_answer, "refresh_token");
    drop(srv);

    let conn = rusqlite::Connection::open(&db).unwrap();
    let stored: Vec<String> = conn
        .prepare("SELECT token_hash FROM refresh_tokens")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(stored.len(), 2);
    assert!(!stored.contains(&lost_token) && !stored.contains(&kept_token));
    let on_disk = std::fs::read(&db).unwrap();
    for answer in [&lost_answer, &kept_answer] {
        for field in ["access_token", "refresh_token"] {
            let value = text(answer, field);
            let found = on_disk.windows(value.len()).any(|w| w == value.as_bytes());
            assert!(!found, "a {field} value is in the file");
        }
    }
    let deleted = conn
        .execute(
            "DELETE FROM refresh_tokens WHERE client_id = ?1",
            rusqlite::params![lost],
        )
        .unwrap();
    assert_eq!(deleted, 1);
    drop(conn);

    let port = free_port();
    let _srv = spawn_server_on(port, None, &db, false);
    // The access token goes with the rows: a restart alone would not have ended it.
    assert_eq!(ping_status(port, &text(&lost_answer, "access_token")), 401);
    assert_eq!(ping_status(port, &text(&kept_answer, "access_token")), 200);
    assert_eq!(refresh(port, &lost, &lost_token).0, 400);
    assert_eq!(refresh(port, &kept, &kept_token).0, 200);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// A user keeps 20 clients live. When the web app's code exchange makes a connector
/// the 21st, the connector's refresh tokens end, and they stay ended after a restart.
#[cfg(feature = "scripting")]
#[test]
fn test_signin_a_family_ended_by_the_live_client_limit_stays_ended_after_a_restart() {
    let fake = FakeProvider::start();
    let db = store_path("refresh-live-clients");
    let profile = || profile_of("github", 5, "octo");
    let p = free_port();
    let srv = start_server(signin_command(p, &db, &fake, &["github"], None, false), p);
    let mut b = Browser::new(p);
    web_sign_in(&mut b, &fake, "github", profile());
    let client = register_external(p);
    let first = b.authorize(&client, EXTERNAL_REDIRECT, "s");
    let page = b.get(&first.location());
    let (request, csrf) = consent_fields(&page.body);
    let approved = b.request(
        "POST",
        "/oauth/consent",
        &["Content-Type: application/x-www-form-urlencoded"],
        &format!("request={request}&csrf={csrf}&decision=approve"),
    );
    let (st, body) = exchange(p, &client, EXTERNAL_REDIRECT, &approved.query()["code"]);
    assert_eq!(st, 200, "{body}");
    let connector: serde_json::Value = serde_json::from_str(&body).unwrap();
    let (access, token) = (
        text(&connector, "access_token"),
        text(&connector, "refresh_token"),
    );
    drop(srv);

    // A new process, because one address registers 20 clients in 10 minutes. The
    // connector is the user's only live client now, and 20 browsers follow.
    let p = free_port();
    let srv = start_server(signin_command(p, &db, &fake, &["github"], None, false), p);
    assert_eq!(ping_status(p, &access), 200);
    // A family's time is in whole seconds: let its second pass, so it is the oldest.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let mut b = Browser::new(p);
    web_sign_in(&mut b, &fake, "github", profile());
    for _ in 1..20 {
        let web = register_client(p, &web_redirect(p));
        let code = b.authorize(&web, &web_redirect(p), "s").query()["code"].clone();
        token_from(p, &web, &web_redirect(p), &code);
    }
    assert_eq!(ping_status(p, &access), 401);
    assert_eq!(refresh(p, &client, &token).0, 400);
    drop(srv);

    let p = free_port();
    let _srv = start_server(signin_command(p, &db, &fake, &["github"], None, false), p);
    assert_eq!(ping_status(p, &access), 401, "back after a restart");
    assert_eq!(refresh(p, &client, &token).0, 400, "back after a restart");
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// With sign-in on: a connector's refresh token is its user's, survives the user
/// signing out of the web app and a restart, and never reaches what only the web
/// app's own token may (ADR-0050). The web app itself gets none.
#[cfg(feature = "scripting")]
#[test]
fn test_signin_a_connector_refreshes_as_its_user_and_is_never_the_web_app() {
    let fake = FakeProvider::start();
    let db = store_path("refresh-signin");
    let p = free_port();
    let srv = start_server(signin_command(p, &db, &fake, &["github"], None, false), p);
    let mut b = Browser::new(p);

    let web_client = register_client(p, &web_redirect(p));
    let first = b.authorize(&web_client, &web_redirect(p), "s1");
    let back = provider_round_trip(
        &mut b,
        &fake,
        "github",
        profile_of("github", 5, "octo"),
        &first,
    );
    let (st, body) = exchange(p, &web_client, &web_redirect(p), &back.query()["code"]);
    assert_eq!(st, 200, "{body}");
    let web: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(web.get("refresh_token").is_none(), "{web}");
    let web = text(&web, "access_token");
    assert_eq!(
        workspace(p, Some(&web), serde_json::json!({"since": 0, "writes": []})).0,
        200
    );

    let client = register_external(p);
    let first = b.authorize(&client, EXTERNAL_REDIRECT, "s");
    let page = b.get(&first.location());
    let (request, csrf) = consent_fields(&page.body);
    let approved = b.request(
        "POST",
        "/oauth/consent",
        &["Content-Type: application/x-www-form-urlencoded"],
        &format!("request={request}&csrf={csrf}&decision=approve"),
    );
    let (st, body) = exchange(p, &client, EXTERNAL_REDIRECT, &approved.query()["code"]);
    assert_eq!(st, 200, "{body}");
    let connector: serde_json::Value = serde_json::from_str(&body).unwrap();
    let token = text(&connector, "refresh_token");
    assert!(!token.is_empty(), "{connector}");

    // Another client cannot spend it.
    assert_eq!(refresh(p, &web_client, &token).0, 400);

    // Signing out of the web app leaves the connector what it was given.
    assert_eq!(b.request("POST", "/oauth/logout", &[], "").status, 204);
    assert_eq!(ping_status(p, &web), 401);
    let (st, next) = refresh(p, &client, &token);
    assert_eq!(st, 200, "{next}");
    let access = text(&next, "access_token");
    assert_eq!(me(p, &access).1["user"]["id"], "github:5");
    let sync = workspace(
        p,
        Some(&access),
        serde_json::json!({"since": 0, "writes": []}),
    );
    assert_eq!(sync.0, 403, "not the web app's token: {}", sync.1);
    drop(srv);

    // The same after a restart, with nobody at a browser: the connector's access
    // token is known again, as its user and not as the web app, whose token is gone.
    let p = free_port();
    let _srv = start_server(signin_command(p, &db, &fake, &["github"], None, false), p);
    assert_eq!(me(p, &access).1["user"]["id"], "github:5");
    assert_eq!(
        workspace(
            p,
            Some(&access),
            serde_json::json!({"since": 0, "writes": []})
        )
        .0,
        403
    );
    assert_eq!(ping_status(p, &web), 401);
    let (st, again) = refresh(p, &client, &text(&next, "refresh_token"));
    assert_eq!(st, 200, "{again}");
    let access = text(&again, "access_token");
    assert_eq!(me(p, &access).1["user"]["id"], "github:5");
    assert_eq!(
        workspace(
            p,
            Some(&access),
            serde_json::json!({"since": 0, "writes": []})
        )
        .0,
        403
    );

    // Sign-in turned off: a user's token yields nothing, so no token without a user.
    drop(_srv);
    let p = free_port();
    let _srv = spawn_server_on(p, None, &db, false);
    assert_eq!(ping_status(p, &access), 401);
    assert_eq!(refresh(p, &client, &text(&again, "refresh_token")).0, 400);
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

/// Refreshing counts as getting a token, and that hands nothing over: rows from
/// before sign-in go to the named owner only (ADR-0044), also through a refresh.
#[cfg(feature = "scripting")]
#[test]
fn test_signin_refreshing_claims_nothing_for_anyone_but_the_named_owner() {
    let fake = FakeProvider::start();
    let port = free_port();
    let db = store_path("refresh-claim");
    let _seeded = seed_script_through(port, &db, "from_before");
    let mut cmd = signin_command(port, &db, &fake, &["github"], None, true);
    cmd.env("CHARTGEN_CLAIM_OWNER", "github:1");
    let _srv = start_server(cmd, port);
    let client = register_external(port);
    let mut stranger = Browser::new(port);
    let first = stranger.authorize_from(&client, EXTERNAL_REDIRECT, "s", &[]);
    let back = provider_round_trip(
        &mut stranger,
        &fake,
        "github",
        profile_of("github", 666, "stranger"),
        &first,
    );
    let page = stranger.get(&back.location());
    let (request, csrf) = consent_fields(&page.body);
    let approved = stranger.request(
        "POST",
        "/oauth/consent",
        &["Content-Type: application/x-www-form-urlencoded"],
        &format!("request={request}&csrf={csrf}&decision=approve"),
    );
    let (st, body) = exchange(port, &client, EXTERNAL_REDIRECT, &approved.query()["code"]);
    assert_eq!(st, 200, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
    let (st, next) = refresh(port, &client, &text(&answer, "refresh_token"));
    assert_eq!(st, 200, "{next}");
    let mine = tool_text(
        port,
        &text(&next, "access_token"),
        "list_my_indicators",
        serde_json::json!({}),
    );
    assert!(!mine.contains("from_before"), "{mine}");
    let store = chartgen::scripting::persistence::Store::open(&db).unwrap();
    assert_eq!(store.waiting_rows(None).unwrap().scripts, 1);
}

/// A client id travels in URLs, so one must never be what hands a script over.
/// A stranger through the owner's own client gets nothing, whatever is set.
#[cfg(feature = "scripting")]
#[test]
fn test_signin_a_stranger_through_a_known_client_gets_nothing() {
    for named in [None, Some("github:1")] {
        let fake = FakeProvider::start();
        let port = free_port();
        let db = store_path(if named.is_some() {
            "claim-set"
        } else {
            "claim-unset"
        });
        let client = seed_script_through(port, &db, "from_before");
        let mut cmd = signin_command(port, &db, &fake, &["github"], None, true);
        if let Some(owner) = named {
            cmd.env("CHARTGEN_CLAIM_OWNER", owner);
        }
        let _srv = start_server(cmd, port);
        let redirect = web_redirect(port);
        let mut stranger = Browser::new(port);
        let first = stranger.authorize(&client, &redirect, "s");
        let back = provider_round_trip(
            &mut stranger,
            &fake,
            "github",
            profile_of("github", 666, "stranger"),
            &first,
        );
        let token = token_from(port, &client, &redirect, &back.query()["code"]);
        let mine = tool_text(port, &token, "list_my_indicators", serde_json::json!({}));
        assert!(!mine.contains("from_before"), "{named:?}: {mine}");

        // The row is still there and still a client's, for whoever is named later.
        let store = chartgen::scripting::persistence::Store::open(&db).unwrap();
        assert_eq!(store.waiting_rows(None).unwrap().scripts, 1, "{named:?}");
    }
}

/// The account named collects what every client owned, whichever client it signs in through.
#[cfg(feature = "scripting")]
#[test]
fn test_signin_named_owner_collects_everything_through_any_client_once() {
    let fake = FakeProvider::start();
    let port = free_port();
    let db = store_path("claim-owner");
    let log_path = db.with_extension("log");
    let owner_client = seed_script_through(port, &db, "from_before");
    let _other_client = seed_script_through(port, &db, "from_elsewhere");
    let log = std::fs::File::create(&log_path).unwrap();
    let mut cmd = signin_command(port, &db, &fake, &["github"], None, true);
    cmd.env("CHARTGEN_CLAIM_OWNER", "github:1")
        .stderr(log.try_clone().unwrap())
        .stdout(log);
    let srv = start_server(cmd, port);
    let mine = |token: &str| tool_text(port, token, "list_my_indicators", serde_json::json!({}));

    // Through a client that never owned anything: both clients' rows arrive.
    let mut ann = Browser::new(port);
    let token = web_sign_in(&mut ann, &fake, "github", profile_of("github", 1, "ann"));
    assert!(mine(&token).contains("from_before"), "{}", mine(&token));
    assert!(mine(&token).contains("from_elsewhere"), "{}", mine(&token));

    // Anyone after that, through the old client, finds nothing there.
    let redirect = web_redirect(port);
    let mut bob = Browser::new(port);
    let first = bob.authorize(&owner_client, &redirect, "s");
    let back = provider_round_trip(
        &mut bob,
        &fake,
        "github",
        profile_of("github", 2, "b"),
        &first,
    );
    let theirs = token_from(port, &owner_client, &redirect, &back.query()["code"]);
    assert!(!mine(&theirs).contains("from_before"), "{}", mine(&theirs));

    // Once: a second collection moves nothing, and a restart has nothing left waiting.
    drop(srv);
    let store = chartgen::scripting::persistence::Store::open(&db).unwrap();
    assert_eq!(
        store.waiting_rows(Some("github:1")).unwrap(),
        Default::default()
    );
    let logged = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        logged.contains("2 script(s), 0 server alert(s) and 0 push device(s)"),
        "{logged}"
    );
    assert!(
        !logged.contains(&owner_client),
        "no ids in the log: {logged}"
    );
    assert!(!logged.contains("github:1"), "no ids in the log: {logged}");
    let _ = std::fs::remove_file(&log_path);
}

/// A name the owner already has cannot move, and startup says how many are stuck that way.
#[cfg(feature = "scripting")]
#[test]
fn test_signin_startup_counts_name_collisions() {
    let fake = FakeProvider::start();
    let port = free_port();
    let db = store_path("claim-collision");
    let log_path = db.with_extension("log");
    seed_script_through(port, &db, "same_name");
    seed_script_through(port, &db, "same_name");
    let start = |fake: &FakeProvider, log: std::fs::File| {
        let mut cmd = signin_command(port, &db, fake, &["github"], None, true);
        cmd.env("CHARTGEN_CLAIM_OWNER", "github:1")
            .stderr(log.try_clone().unwrap())
            .stdout(log);
        start_server(cmd, port)
    };
    let srv = start(&fake, std::fs::File::create(&log_path).unwrap());
    let mut ann = Browser::new(port);
    web_sign_in(&mut ann, &fake, "github", profile_of("github", 1, "ann"));
    drop(srv);
    let srv = start(&fake, std::fs::File::create(&log_path).unwrap());
    drop(srv);
    let logged = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        logged.contains("1 script(s), 0 server alert(s) and 0 push device(s)"),
        "the second client's copy stays: {logged}"
    );
    assert!(
        logged.contains("1 of these scripts are name collisions"),
        "{logged}"
    );
    let _ = std::fs::remove_file(&log_path);
}

/// Unset, startup says how many rows wait and what to set, as a count.
#[cfg(feature = "scripting")]
#[test]
fn test_signin_unnamed_owner_is_told_how_many_rows_wait() {
    let fake = FakeProvider::start();
    let port = free_port();
    let db = store_path("claim-notice");
    let log_path = db.with_extension("log");
    let client = seed_script_through(port, &db, "from_before");
    let log = std::fs::File::create(&log_path).unwrap();
    let mut cmd = signin_command(port, &db, &fake, &["github"], None, true);
    cmd.stderr(log.try_clone().unwrap()).stdout(log);
    let srv = start_server(cmd, port);
    drop(srv);
    let logged = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        logged.contains("1 script(s), 0 server alert(s) and 0 push device(s)"),
        "{logged}"
    );
    assert!(
        logged.contains("CHARTGEN_CLAIM_OWNER=github:<id>"),
        "{logged}"
    );
    assert!(!logged.contains(&client), "no ids in the log: {logged}");
    let _ = std::fs::remove_file(&log_path);
}

/// ADR-0044 with server alerts (ADR-0042): an alert belongs to the user.
#[cfg(feature = "scripting")]
mod signin_server_alerts {
    use super::server_alerts::{browser, open_push, push_service, receiver};
    use super::*;
    use std::sync::atomic::Ordering;

    fn start(
        test: &str,
        fake: &FakeProvider,
        allowed: &str,
        hook: u16,
        pairs: Option<&str>,
    ) -> Server {
        let port = free_port();
        let db = store_path(test);
        let mut cmd = signin_command(port, &db, fake, &["github"], None, false);
        cmd.env("CHARTGEN_SERVER_ALERTS", "1")
            .env("CHARTGEN_SERVER_ALERT_CLIENTS", allowed)
            .env("CHARTGEN_VAPID_SUBJECT", "mailto:ops@example.com")
            .env(
                "CHARTGEN_WEBHOOK_TEST_ORIGIN",
                format!("http://127.0.0.1:{hook}"),
            );
        if let Some(pairs) = pairs {
            cmd.env("CHARTGEN_SERVER_ALERT_PAIRS_PER_CLIENT", pairs);
        }
        let mut srv = start_server(cmd, port);
        srv.temp_db = Some(db);
        srv
    }

    fn alert(symbol: &str) -> serde_json::Value {
        serde_json::json!({
            "symbol": symbol, "condition": "less", "trigger": "once", "level": 0.01,
        })
    }

    fn listed(port: u16, token: &str) -> Vec<serde_json::Value> {
        let list = call_tool(port, token, "list_server_alerts", serde_json::json!({})).unwrap();
        list["alerts"].as_array().unwrap().clone()
    }

    /// Another client of the signed-in `b`, with a token.
    fn second_client(b: &mut Browser) -> String {
        let (port, redirect) = (b.port, web_redirect(b.port));
        let client = register_client(port, &redirect);
        let answer = b.authorize(&client, &redirect, "s");
        token_from(port, &client, &redirect, &answer.query()["code"])
    }

    /// Ownership and quotas are the user's, not the client's: more clients buy no more.
    #[test]
    fn test_alerts_and_their_limits_follow_the_user_across_clients() {
        let fake = FakeProvider::start();
        let srv = start("signin-alerts-owner", &fake, "*", 1, Some("1"));
        let p = srv.port;
        let mut ann = Browser::new(p);
        let first = web_sign_in(&mut ann, &fake, "github", profile_of("github", 1, "ann"));
        let second = second_client(&mut ann);
        let mut bob = Browser::new(p);
        let theirs = web_sign_in(&mut bob, &fake, "github", profile_of("github", 2, "bob"));

        call_tool(p, &first, "create_server_alert", alert("BTCUSDT")).unwrap();
        assert_eq!(listed(p, &second).len(), 1, "from her other client too");
        assert_eq!(listed(p, &theirs).len(), 0, "and not for anyone else");

        // One pair per owner: her second client is still her.
        let refused = call_tool(p, &second, "create_server_alert", alert("ETHUSDT")).unwrap_err();
        assert!(refused.contains("pair"), "{refused}");
        call_tool(p, &theirs, "create_server_alert", alert("ETHUSDT")).unwrap();
    }

    /// The list names accounts once sign-in is on. A client id on it grants nothing,
    /// because the client is not who owns anything now.
    #[test]
    fn test_the_allowlist_names_accounts() {
        let fake = FakeProvider::start();
        let (hook, _) = receiver();
        let srv = start("signin-alerts-list", &fake, "github:1", hook, None);
        let p = srv.port;
        let mut ann = Browser::new(p);
        let allowed = web_sign_in(&mut ann, &fake, "github", profile_of("github", 1, "ann"));
        let mut bob = Browser::new(p);
        let other = web_sign_in(&mut bob, &fake, "github", profile_of("github", 2, "bob"));

        let mut with_hook = alert("BTCUSDT");
        with_hook["webhook_url"] = format!("http://127.0.0.1:{hook}/hook").into();
        call_tool(p, &allowed, "create_server_alert", with_hook.clone()).unwrap();
        let refused = call_tool(p, &other, "create_server_alert", with_hook).unwrap_err();
        assert!(refused.contains("operator allowed"), "{refused}");
        let refused = call_tool(p, &other, "create_server_alert", alert("BTCUSDT")).unwrap_err();
        assert!(refused.contains("operator allowed"), "{refused}");
    }

    #[test]
    fn test_a_client_id_on_the_allowlist_grants_a_signed_in_stranger_nothing() {
        let fake = FakeProvider::start();
        let port = free_port();
        let db = store_path("signin-alerts-client-id");
        let id = "11111111-1111-4111-8111-111111111111";
        // A client id an operator listed before turning sign-in on, in the store.
        chartgen::scripting::persistence::Store::open(&db)
            .unwrap()
            .save_oauth_client(id, &[web_redirect(port)])
            .unwrap();
        let mut cmd = signin_command(port, &db, &fake, &["github"], None, false);
        cmd.env("CHARTGEN_SERVER_ALERTS", "1")
            .env("CHARTGEN_SERVER_ALERT_CLIENTS", id);
        let mut srv = start_server(cmd, port);
        srv.temp_db = Some(db);
        let mut b = Browser::new(port);
        let first = b.authorize(id, &web_redirect(port), "s");
        let back = provider_round_trip(
            &mut b,
            &fake,
            "github",
            profile_of("github", 9, "x"),
            &first,
        );
        let token = token_from(port, id, &web_redirect(port), &back.query()["code"]);
        let refused = call_tool(port, &token, "create_server_alert", alert("BTCUSDT")).unwrap_err();
        assert!(refused.contains("operator allowed"), "{refused}");
    }

    /// Logging out of a browser ends its token, and the alert goes on running with no
    /// browser at all: the point of an alert on the server.
    #[test]
    fn test_logout_leaves_alerts_running() {
        let fake = FakeProvider::start();
        let (hook, seen) = receiver();
        let srv = start("signin-alerts-logout", &fake, "*", hook, None);
        let p = srv.port;
        let mut b = Browser::new(p);
        let token = web_sign_in(&mut b, &fake, "github", profile_of("github", 1, "ann"));
        // Evaluated when the 1m bar closes, so it fires at the next minute boundary:
        // after the logout below, with no token left and no browser connected. Too close
        // to a boundary it could fire first and prove nothing.
        let secs = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                % 60
        };
        while secs() > 50 {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let spec = serde_json::json!({
            "symbol": "BTCUSDT", "interval": "1m", "evaluate_on": "close",
            "condition": "greater", "trigger": "once", "level": 0.01,
            "webhook_url": format!("http://127.0.0.1:{hook}/hook"),
        });
        call_tool(p, &token, "create_server_alert", spec).unwrap();

        assert_eq!(b.request("POST", "/oauth/logout", &[], "").status, 204);
        assert_eq!(ping_status(p, &token), 401, "signed out");
        assert!(seen.lock().unwrap().is_empty(), "it has not fired yet");

        for _ in 0..800 {
            if !seen.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(
            !seen.lock().unwrap().is_empty(),
            "the alert must fire with its owner signed out"
        );

        // And it is still theirs when they come back.
        let mut again = Browser::new(p);
        let token = web_sign_in(&mut again, &fake, "github", profile_of("github", 1, "ann"));
        let list = listed(p, &token);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["fire_count"], 1, "and it is on record");
    }

    /// A stranger through the owner's client gets nothing, the alert staying the client's;
    /// the named owner then collects it through another client, and it stays theirs across
    /// a restart. Its webhook secret goes with it, and a stranger gets neither.
    #[test]
    fn test_existing_alerts_go_to_the_named_owner_and_nobody_else() {
        let fake = FakeProvider::start();
        let port = free_port();
        let db = store_path("signin-alerts-claim");
        let redirect = web_redirect(port);
        let alerts_on = |cmd: &mut std::process::Command| {
            cmd.env("CHARTGEN_SERVER_ALERTS", "1")
                .env("CHARTGEN_SERVER_ALERT_CLIENTS", "*")
                .env("CHARTGEN_DATA_SOURCE", "sample");
        };
        let mut cmd = server_command(port, None, &db, false);
        alerts_on(&mut cmd);
        let old = start_server(cmd, port);
        let client = register_client(port, &redirect);
        let (_, head, _) = authorize(port, &client, &redirect);
        let code = head
            .split("code=")
            .nth(1)
            .unwrap()
            .split(['&', '\r'])
            .next()
            .unwrap()
            .to_string();
        let token = token_from(port, &client, &redirect, &code);
        call_tool(port, &token, "create_server_alert", alert("BTCUSDT")).unwrap();
        drop(old);

        let with_signin = |fake: &FakeProvider| {
            let mut cmd = signin_command(port, &db, fake, &["github"], None, false);
            cmd.env("CHARTGEN_CLAIM_OWNER", "github:1");
            alerts_on(&mut cmd);
            start_server(cmd, port)
        };
        let srv = with_signin(&fake);

        // A stranger through the client that made it, first in line: nothing.
        let mut bob = Browser::new(port);
        let first = bob.authorize(&client, &redirect, "s");
        let back = provider_round_trip(
            &mut bob,
            &fake,
            "github",
            profile_of("github", 2, "bob"),
            &first,
        );
        let theirs = token_from(port, &client, &redirect, &back.query()["code"]);
        assert_eq!(listed(port, &theirs).len(), 0, "the stranger gets nothing");

        // The named owner through a client of their own: all of it.
        let mut ann = Browser::new(port);
        let token = web_sign_in(&mut ann, &fake, "github", profile_of("github", 1, "ann"));
        assert_eq!(listed(port, &token).len(), 1, "hers now");
        assert_eq!(listed(port, &theirs).len(), 0, "still nothing for him");

        drop(srv);
        let _srv = with_signin(&fake);
        // Through another client again: the move has to have reached the store.
        let token = second_client(&mut ann);
        assert_eq!(listed(port, &token).len(), 1, "and it stays hers");
    }

    /// Wait for what the fake service got, on the real clock.
    fn until_received(what: &str, mut done: impl FnMut() -> bool) {
        for _ in 0..200 {
            if done() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("timed out waiting for {what}");
    }

    fn fired_now() -> serde_json::Value {
        // A level far below the sample price is simply greater, at once.
        serde_json::json!({
            "symbol": "BTCUSDT", "condition": "greater", "trigger": "once",
            "level": 0.01, "push": true, "message": "from the desktop",
        })
    }

    /// A phone gets an alert made on the desktop: two browsers, so two clients, one user.
    /// Decrypted with the phone's own keys, as the phone would. Someone else's gets nothing.
    #[test]
    fn test_a_phone_gets_an_alert_made_on_the_desktop() {
        let fake = FakeProvider::start();
        let (service, bodies, answer) = push_service();
        answer.store(true, Ordering::SeqCst);
        let srv = start("signin-push-phone", &fake, "*", service, None);
        let p = srv.port;
        let mut desktop = Browser::new(p);
        let at_desk = web_sign_in(
            &mut desktop,
            &fake,
            "github",
            profile_of("github", 1, "ann"),
        );
        let mut phone = Browser::new(p);
        let on_phone = web_sign_in(&mut phone, &fake, "github", profile_of("github", 1, "ann"));
        assert_ne!(at_desk, on_phone);

        let (key, auth, device) = browser(service);
        let answered = call_tool(p, &on_phone, "subscribe_push", device).unwrap();
        assert_eq!(answered["devices"], 1);
        let id = answered["device"].as_str().unwrap().to_string();
        assert_eq!(
            call_tool(p, &at_desk, "list_server_alerts", serde_json::json!({})).unwrap()["push"]
                ["devices"],
            1,
            "the desktop sees the phone: one user"
        );

        let alert = call_tool(p, &at_desk, "create_server_alert", fired_now()).unwrap();
        until_received("the push", || bodies.lock().unwrap().len() == 1);
        let got = open_push(&bodies.lock().unwrap()[0], &key, &auth);
        assert_eq!(got["alert_id"], alert["id"]);
        assert_eq!(got["message"], "from the desktop");

        // The outcome is recorded against that device.
        until_received("its outcome", || {
            let list = call_tool(p, &at_desk, "list_server_alerts", serde_json::json!({})).unwrap();
            list["alerts"][0]["last_push_devices"][&id]["status"].is_number()
        });

        // Someone else's phone is another user's device.
        let mut stranger = Browser::new(p);
        let theirs = web_sign_in(
            &mut stranger,
            &fake,
            "github",
            profile_of("github", 2, "bob"),
        );
        let (_, _, mut device) = browser(service);
        device["endpoint"] = format!("http://127.0.0.1:{service}/push/bobs").into();
        call_tool(p, &theirs, "subscribe_push", device).unwrap();
        let again = call_tool(p, &at_desk, "create_server_alert", fired_now()).unwrap();
        until_received("the second push", || bodies.lock().unwrap().len() == 2);
        let got = open_push(&bodies.lock().unwrap()[1], &key, &auth);
        assert_eq!(got["alert_id"], again["id"], "it went to her phone");
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert_eq!(bodies.lock().unwrap().len(), 2, "and to nobody else's");
    }

    /// Devices from before sign-in follow `CHARTGEN_CLAIM_OWNER`: not whoever holds their
    /// client id, and the named owner's alerts reach them after collecting.
    #[test]
    fn test_existing_devices_go_to_the_named_owner_and_nobody_else() {
        let fake = FakeProvider::start();
        let (service, bodies, answer) = push_service();
        answer.store(true, Ordering::SeqCst);
        let port = free_port();
        let db = store_path("signin-push-claim");
        let redirect = web_redirect(port);
        let alerts_on = |cmd: &mut std::process::Command| {
            cmd.env("CHARTGEN_SERVER_ALERTS", "1")
                .env("CHARTGEN_SERVER_ALERT_CLIENTS", "*")
                .env("CHARTGEN_VAPID_SUBJECT", "mailto:ops@example.com")
                .env("CHARTGEN_DATA_SOURCE", "sample")
                .env(
                    "CHARTGEN_WEBHOOK_TEST_ORIGIN",
                    format!("http://127.0.0.1:{service}"),
                );
        };
        let mut cmd = server_command(port, None, &db, false);
        alerts_on(&mut cmd);
        let old = start_server(cmd, port);
        let client = register_client(port, &redirect);
        let (_, head, _) = authorize(port, &client, &redirect);
        let code = head
            .split("code=")
            .nth(1)
            .unwrap()
            .split(['&', '\r'])
            .next()
            .unwrap()
            .to_string();
        let token = token_from(port, &client, &redirect, &code);
        let (key, auth, device) = browser(service);
        call_tool(port, &token, "subscribe_push", device).unwrap();
        drop(old);

        let devices = |token: &str| {
            call_tool(port, token, "list_server_alerts", serde_json::json!({})).unwrap()["push"]
                ["devices"]
                .clone()
        };
        let with_signin = |owner: Option<&str>| {
            let mut cmd = signin_command(port, &db, &fake, &["github"], None, false);
            if let Some(owner) = owner {
                cmd.env("CHARTGEN_CLAIM_OWNER", owner);
            }
            alerts_on(&mut cmd);
            start_server(cmd, port)
        };
        let through_client = |who: &mut Browser, id: u64| {
            let first = who.authorize(&client, &redirect, "s");
            let back =
                provider_round_trip(who, &fake, "github", profile_of("github", id, "x"), &first);
            token_from(port, &client, &redirect, &back.query()["code"])
        };

        // Nobody named, and a stranger through the very client: nothing, and no pushes.
        let srv = with_signin(None);
        let mut bob = Browser::new(port);
        let theirs = through_client(&mut bob, 2);
        assert_eq!(devices(&theirs), 0);
        call_tool(port, &theirs, "create_server_alert", fired_now()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert_eq!(
            bodies.lock().unwrap().len(),
            0,
            "their alert does not reach it"
        );
        drop(srv);
        let store = chartgen::scripting::persistence::Store::open(&db).unwrap();
        assert_eq!(
            store.waiting_rows(None).unwrap().devices,
            1,
            "still waiting"
        );
        drop(store);

        // Named: a stranger first still gets nothing, and the owner collects it elsewhere.
        let _srv = with_signin(Some("github:1"));
        let mut carol = Browser::new(port);
        let hers = through_client(&mut carol, 3);
        assert_eq!(devices(&hers), 0);
        let mut ann = Browser::new(port);
        let owner = web_sign_in(&mut ann, &fake, "github", profile_of("github", 1, "ann"));
        assert_eq!(devices(&owner), 1, "collected, through another client");
        assert_eq!(devices(&hers), 0);
        let alert = call_tool(port, &owner, "create_server_alert", fired_now()).unwrap();
        until_received("the push", || bodies.lock().unwrap().len() == 1);
        let got = open_push(&bodies.lock().unwrap()[0], &key, &auth);
        assert_eq!(got["alert_id"], alert["id"]);
    }

    /// On the real binary, two accounts on one browser: the device goes with the last to
    /// subscribe, and only their alerts reach it. A stranger who knows the endpoint and not
    /// the keys moves nothing.
    #[test]
    fn test_one_endpoint_has_one_owner() {
        let fake = FakeProvider::start();
        let (service, bodies, answer) = push_service();
        answer.store(true, Ordering::SeqCst);
        let srv = start("signin-push-owner", &fake, "*", service, None);
        let p = srv.port;
        let mut ann = Browser::new(p);
        let hers = web_sign_in(&mut ann, &fake, "github", profile_of("github", 1, "ann"));
        let mut bob = Browser::new(p);
        let his = web_sign_in(&mut bob, &fake, "github", profile_of("github", 2, "bob"));
        let devices = |token: &str| {
            call_tool(p, token, "list_server_alerts", serde_json::json!({})).unwrap()["push"]
                ["devices"]
                .clone()
        };

        let (key, auth, device) = browser(service);
        call_tool(p, &hers, "subscribe_push", device.clone()).unwrap();
        assert_eq!(devices(&hers), 1);

        // Bob knows the endpoint and makes up the keys: refused, and hers still works.
        let (_, _, mut forged) = browser(service);
        forged["endpoint"] = device["endpoint"].clone();
        assert_ne!(forged["p256dh"], device["p256dh"]);
        let refused = call_tool(p, &his, "subscribe_push", forged).unwrap_err();
        assert!(refused.contains("other keys"), "{refused}");
        assert_eq!(devices(&hers), 1);
        assert_eq!(devices(&his), 0);
        let alert = call_tool(p, &hers, "create_server_alert", fired_now()).unwrap();
        until_received("her push", || bodies.lock().unwrap().len() == 1);
        let got = open_push(&bodies.lock().unwrap()[0], &key, &auth);
        assert_eq!(got["alert_id"], alert["id"]);

        // The browser itself, signed in as Bob now: its subscription has both halves.
        call_tool(p, &his, "subscribe_push", device).unwrap();
        assert_eq!(devices(&his), 1);
        assert_eq!(devices(&hers), 0, "she lost that device");

        // Her alerts go on, reaching no device, and his reach it.
        let made = call_tool(p, &hers, "create_server_alert", fired_now()).unwrap();
        assert!(made["id"].is_string());
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert_eq!(bodies.lock().unwrap().len(), 1, "hers no longer reach it");
        let list = call_tool(p, &hers, "list_server_alerts", serde_json::json!({})).unwrap();
        assert_eq!(list["alerts"][1]["fire_count"], 1, "and still run: {list}");
        let alert = call_tool(p, &his, "create_server_alert", fired_now()).unwrap();
        until_received("his push", || bodies.lock().unwrap().len() == 2);
        let got = open_push(&bodies.lock().unwrap()[1], &key, &auth);
        assert_eq!(got["alert_id"], alert["id"]);
    }
}

// ===========================================================================
// 15. Per-user workspace documents (ADR-0050)
// ===========================================================================

/// `POST /workspace/sync` with this token: (status, JSON or null).
fn workspace(port: u16, token: Option<&str>, body: serde_json::Value) -> (u16, serde_json::Value) {
    let auth = token.map(|t| format!("Authorization: Bearer {t}"));
    let mut headers = vec![JSON_POST[0], JSON_POST[1]];
    if let Some(auth) = &auth {
        headers.push(auth);
    }
    let (st, _, resp) = http(port, "POST", "/workspace/sync", &headers, &body.to_string());
    (st, serde_json::from_str(&resp).unwrap_or_default())
}

#[cfg(feature = "scripting")]
fn write_doc(key: &str, value: Option<&str>, base: i64) -> serde_json::Value {
    serde_json::json!({ "key": key, "value": value, "base_version": base })
}

/// What an account holds, as a device with nothing yet would be told.
#[cfg(feature = "scripting")]
fn docs_of(port: u16, token: &str) -> std::collections::HashMap<String, serde_json::Value> {
    let (st, body) = workspace(port, Some(token), serde_json::json!({ "since": 0 }));
    assert_eq!(st, 200, "{body}");
    body["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["key"].as_str().unwrap().to_string(), d["value"].clone()))
        .collect()
}

/// Several clients of one user share one workspace. Two users never see or touch each
/// other's, under the same key, and there is nothing in the request to name another by.
#[cfg(feature = "scripting")]
#[test]
fn test_workspace_is_per_user_across_clients() {
    let fake = FakeProvider::start();
    let srv = signin_server("workspace-users", &fake, &["github"]);
    let p = srv.port;
    let mut ann = Browser::new(p);
    let phone = web_sign_in(&mut ann, &fake, "github", profile_of("github", 1, "ann"));
    let mut laptop_browser = Browser::new(p);
    let laptop = web_sign_in(
        &mut laptop_browser,
        &fake,
        "github",
        profile_of("github", 1, "ann"),
    );
    let mut bob = Browser::new(p);
    let his = web_sign_in(&mut bob, &fake, "github", profile_of("github", 2, "bob"));
    assert_ne!(phone, laptop, "two clients, two tokens");

    let key = "chartgen.chartSettings";
    let (st, body) = workspace(
        p,
        Some(&phone),
        serde_json::json!({ "since": 0, "writes": [write_doc(key, Some("anns"), 0)] }),
    );
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["results"][0]["status"], "ok");

    assert_eq!(docs_of(p, &laptop)[key], "anns", "her other client");
    assert!(docs_of(p, &his).is_empty(), "nothing of hers for him");

    // Bob writes the same key: his own document, at his own version.
    let (_, body) = workspace(
        p,
        Some(&his),
        serde_json::json!({ "since": 0, "writes": [write_doc(key, Some("bobs"), 0)] }),
    );
    assert_eq!(body["results"][0]["status"], "ok", "{body}");
    assert_eq!(docs_of(p, &his)[key], "bobs");
    assert_eq!(docs_of(p, &phone)[key], "anns");
    // Fields naming someone else are not a thing the server reads.
    let (_, body) = workspace(
        p,
        Some(&his),
        serde_json::json!({ "user": "github:1", "user_id": "github:1", "since": 0 }),
    );
    assert_eq!(body["changes"][0]["value"], "bobs", "{body}");
    assert_eq!(body["changes"].as_array().unwrap().len(), 1);
}

/// Two devices on one version: the second is told what the first wrote, and nothing of
/// the first's is replaced until it has seen it.
#[cfg(feature = "scripting")]
#[test]
fn test_workspace_conflict_answers_with_the_current_document() {
    let fake = FakeProvider::start();
    let srv = signin_server("workspace-conflict", &fake, &["github"]);
    let p = srv.port;
    let mut a = Browser::new(p);
    let first = web_sign_in(&mut a, &fake, "github", profile_of("github", 1, "ann"));
    let mut b = Browser::new(p);
    let second = web_sign_in(&mut b, &fake, "github", profile_of("github", 1, "ann"));
    let key = "chartgen.drawings.BTCUSDT";
    let (_, body) = workspace(
        p,
        Some(&first),
        serde_json::json!({ "since": 0, "writes": [write_doc(key, Some("v1"), 0)] }),
    );
    let v1 = body["results"][0]["version"].as_i64().unwrap();
    let (_, body) = workspace(
        p,
        Some(&first),
        serde_json::json!({ "since": v1, "writes": [write_doc(key, Some("v2"), v1)] }),
    );
    let v2 = body["results"][0]["version"].as_i64().unwrap();

    let (st, body) = workspace(
        p,
        Some(&second),
        serde_json::json!({ "since": v1, "writes": [write_doc(key, Some("stale"), v1)] }),
    );
    assert_eq!(st, 200);
    assert_eq!(body["results"][0]["status"], "conflict", "{body}");
    assert_eq!(body["results"][0]["value"], "v2");
    assert_eq!(body["results"][0]["version"], v2);
    assert_eq!(docs_of(p, &first)[key], "v2", "nothing replaced");
}

/// With the server restarted and no browser connected, everything is there for the next
/// device, and for a new client of that account. Tokens do not survive; the data does.
#[cfg(feature = "scripting")]
#[test]
fn test_workspace_survives_a_restart_with_no_browser_connected() {
    let fake = FakeProvider::start();
    let port = free_port();
    let db = store_path("workspace-restart");
    let start = |fake: &FakeProvider| {
        start_server(
            signin_command(port, &db, fake, &["github"], None, false),
            port,
        )
    };
    let srv = start(&fake);
    let mut ann = Browser::new(port);
    let token = web_sign_in(&mut ann, &fake, "github", profile_of("github", 1, "ann"));
    let (_, body) = workspace(
        port,
        Some(&token),
        serde_json::json!({ "since": 0, "writes": [
            write_doc("chartgen.layouts", Some("{\"items\":[]}"), 0),
            write_doc("chartgen.alerts.list", Some("[]"), 0),
        ]}),
    );
    let cursor = body["cursor"].as_i64().unwrap();
    assert_eq!(cursor, 2, "{body}");
    drop(srv);
    let _srv = start(&fake);
    assert_eq!(
        workspace(port, Some(&token), serde_json::json!({ "since": 0 })).0,
        401
    );

    let mut phone = Browser::new(port);
    let on_phone = web_sign_in(&mut phone, &fake, "github", profile_of("github", 1, "ann"));
    let docs = docs_of(port, &on_phone);
    assert_eq!(docs["chartgen.alerts.list"], "[]");
    assert_eq!(docs["chartgen.layouts"], "{\"items\":[]}");
    // Versions carry on from where they were, so a device's cursor still means something.
    let (_, body) = workspace(
        port,
        Some(&on_phone),
        serde_json::json!({ "since": cursor, "writes": [write_doc("chartgen.alerts.list", Some("[1]"), 2)] }),
    );
    assert_eq!(body["results"][0]["version"], 3, "{body}");
}

/// No account, no workspace: no token, a bad one, a signed-out one, and a server with
/// nothing configured, where it does not exist at all.
#[test]
fn test_workspace_needs_a_signed_in_user() {
    let fake = FakeProvider::start();
    let srv = signin_server("workspace-auth", &fake, &["github"]);
    let p = srv.port;
    let mut b = Browser::new(p);
    let token = web_sign_in(&mut b, &fake, "github", profile_of("github", 1, "a"));
    let empty = serde_json::json!({ "since": 0 });
    assert_eq!(workspace(p, None, empty.clone()).0, 401);
    assert_eq!(workspace(p, Some("nonsense"), empty.clone()).0, 401);
    // A build with no store has nowhere to keep it: the app stays local, and says nothing.
    let signed_in = if cfg!(feature = "scripting") {
        200
    } else {
        404
    };
    assert_eq!(workspace(p, Some(&token), empty.clone()).0, signed_in);
    assert_eq!(b.request("POST", "/oauth/logout", &[], "").status, 204);
    assert_eq!(
        workspace(p, Some(&token), empty.clone()).0,
        401,
        "signed out"
    );

    let open = spawn_server(None);
    let client = register_external(open.port);
    let token = sign_in(open.port, &client);
    assert_eq!(workspace(open.port, Some(&token), empty.clone()).0, 404);
    assert_eq!(workspace(open.port, None, empty).0, 404);
}

/// A request whose headers are in and whose body has only begun, so it stays running.
#[cfg(feature = "scripting")]
fn start_workspace_upload(port: u16, token: &str) -> std::net::TcpStream {
    use std::io::Write;
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let body = serde_json::json!({ "since": 0 }).to_string();
    write!(
        s,
        "POST /workspace/sync HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\
         Authorization: Bearer {token}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n\r\n{}",
        body.len(),
        &body[..1]
    )
    .unwrap();
    s.flush().unwrap();
    // The server must have let it in before the test goes on.
    std::thread::sleep(std::time::Duration::from_millis(150));
    s
}

#[cfg(feature = "scripting")]
fn finish_workspace_upload(mut s: std::net::TcpStream) -> String {
    use std::io::{Read, Write};
    let body = serde_json::json!({ "since": 0 }).to_string();
    s.write_all(&body.as_bytes()[1..]).unwrap();
    let mut answer = String::new();
    s.read_to_string(&mut answer).unwrap();
    answer
}

/// One sync per user at a time, and a caller past either limit costs no body read.
#[cfg(feature = "scripting")]
#[test]
fn test_workspace_runs_one_sync_per_user() {
    let fake = FakeProvider::start();
    let srv = signin_server("workspace-one", &fake, &["github"]);
    let p = srv.port;
    let mut a = Browser::new(p);
    let hers = web_sign_in(&mut a, &fake, "github", profile_of("github", 1, "a"));
    let mut b = Browser::new(p);
    let his = web_sign_in(&mut b, &fake, "github", profile_of("github", 2, "b"));
    let empty = serde_json::json!({ "since": 0 });

    let running = start_workspace_upload(p, &hers);
    let (st, _, _) = http(
        p,
        "POST",
        "/workspace/sync",
        &[
            JSON_POST[0],
            JSON_POST[1],
            &format!("Authorization: Bearer {hers}"),
        ],
        &empty.to_string(),
    );
    assert_eq!(st, 429, "her second one");
    assert_eq!(workspace(p, Some(&his), empty.clone()).0, 200, "not his");
    assert!(finish_workspace_upload(running).contains("\"cursor\""));
    assert_eq!(
        workspace(p, Some(&hers), empty.clone()).0,
        200,
        "free again"
    );

    // A client that hangs up frees it too.
    drop(start_workspace_upload(p, &hers));
    std::thread::sleep(std::time::Duration::from_millis(150));
    assert_eq!(workspace(p, Some(&hers), empty).0, 200);
}

/// A client that starts a body and stops does not keep its place.
#[cfg(feature = "scripting")]
#[test]
fn test_workspace_gives_up_on_a_body_that_does_not_come() {
    use std::io::Read;
    let fake = FakeProvider::start();
    let srv = signin_server("workspace-slow", &fake, &["github"]);
    let p = srv.port;
    let mut b = Browser::new(p);
    let token = web_sign_in(&mut b, &fake, "github", profile_of("github", 1, "a"));
    let mut stuck = start_workspace_upload(p, &token);
    std::thread::sleep(std::time::Duration::from_secs(11));
    stuck
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut answer = String::new();
    stuck.read_to_string(&mut answer).unwrap();
    assert!(answer.starts_with("HTTP/1.1 408"), "{answer}");
    let empty = serde_json::json!({ "since": 0 });
    assert_eq!(
        workspace(p, Some(&token), empty).0,
        429,
        "holding a place like that costs the rest of the minute"
    );
}

/// A client that does not read keeps its account's sync running while its answer is on its
/// way, and all eight of those do not keep anyone else out for long.
#[cfg(feature = "scripting")]
#[test]
fn test_workspace_unread_answers_hold_their_place_for_a_time() {
    use std::io::Write;
    let fake = FakeProvider::start();
    let srv = signin_server("workspace-unread", &fake, &["github"]);
    let p = srv.port;
    let doc = "x".repeat(500 * 1024);
    let mut tokens = Vec::new();
    for id in 1..10 {
        let mut b = Browser::new(p);
        tokens.push(web_sign_in(
            &mut b,
            &fake,
            "github",
            profile_of("github", id, "u"),
        ));
    }
    let empty = serde_json::json!({ "since": 0 });
    for token in &tokens[..8] {
        for i in 0..4 {
            let key = format!("chartgen.pane.{i}.big");
            let body =
                serde_json::json!({ "since": 0, "writes": [write_doc(&key, Some(&doc), 0)] });
            assert_eq!(
                workspace(p, Some(token), body).1["results"][0]["status"],
                "ok"
            );
        }
    }
    let unread: Vec<_> = tokens[..8]
        .iter()
        .map(|token| {
            let mut s = small_window_client(p);
            write!(
                s,
                "POST /workspace/sync HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\n\
                 Authorization: Bearer {token}\r\nContent-Length: 11\r\n\r\n{{\"since\":0}}"
            )
            .unwrap();
            s
        })
        .collect();
    std::thread::sleep(std::time::Duration::from_millis(500));

    assert_eq!(
        workspace(p, Some(&tokens[0]), empty.clone()).0,
        429,
        "its own next one"
    );
    assert_eq!(
        workspace(p, Some(&tokens[8]), empty.clone()).0,
        429,
        "all eight taken"
    );
    std::thread::sleep(std::time::Duration::from_secs(11));
    assert_eq!(
        workspace(p, Some(&tokens[8]), empty.clone()).0,
        200,
        "not for long"
    );
    drop(unread);
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        workspace(p, Some(&tokens[0]), empty).0,
        429,
        "and it cost those accounts the rest of the minute"
    );
}

/// A first sync on a slow line takes longer than a request's share of the syncs. It loses
/// that, and pays nothing for it, as its answer is moving.
#[cfg(feature = "scripting")]
#[test]
fn test_workspace_a_slow_download_that_keeps_moving_costs_nothing() {
    use std::io::{Read, Write};
    let fake = FakeProvider::start();
    let srv = signin_server("workspace-slow-read", &fake, &["github"]);
    let p = srv.port;
    let mut b = Browser::new(p);
    let token = web_sign_in(&mut b, &fake, "github", profile_of("github", 1, "a"));
    let doc = "x".repeat(500 * 1024);
    for i in 0..4 {
        let key = format!("chartgen.pane.{i}.big");
        let body = serde_json::json!({ "since": 0, "writes": [write_doc(&key, Some(&doc), 0)] });
        assert_eq!(
            workspace(p, Some(&token), body).1["results"][0]["status"],
            "ok"
        );
    }
    let mut s = small_window_client(p);
    write!(
        s,
        "POST /workspace/sync HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\
         Content-Type: application/json\r\nAuthorization: Bearer {token}\r\n\
         Content-Length: 11\r\n\r\n{{\"since\":0}}"
    )
    .unwrap();
    // About 2 MiB in about 14 s.
    let started = std::time::Instant::now();
    let mut total = 0;
    let mut buf = [0u8; 16 * 1024];
    loop {
        match s.read(&mut buf).unwrap() {
            0 => break,
            n => total += n,
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(started.elapsed().as_secs() > 11, "{:?}", started.elapsed());
    assert!(total > 2_000_000, "{total}");
    let empty = serde_json::json!({ "since": 0 });
    assert_eq!(workspace(p, Some(&token), empty).0, 200);
}

/// So many at once in all, however many accounts there are.
#[cfg(feature = "scripting")]
#[test]
fn test_workspace_runs_so_many_syncs_in_all() {
    let fake = FakeProvider::start();
    let srv = signin_server("workspace-cap", &fake, &["github"]);
    let p = srv.port;
    let mut tokens = Vec::new();
    for id in 1..10 {
        let mut b = Browser::new(p);
        tokens.push(web_sign_in(
            &mut b,
            &fake,
            "github",
            profile_of("github", id, "u"),
        ));
    }
    let running: Vec<_> = tokens[..8]
        .iter()
        .map(|t| start_workspace_upload(p, t))
        .collect();
    let empty = serde_json::json!({ "since": 0 });
    assert_eq!(workspace(p, Some(&tokens[8]), empty.clone()).0, 429);
    drop(running);
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_eq!(workspace(p, Some(&tokens[8]), empty).0, 200);
}

/// A body past the limit is refused, and nobody without a valid token has one read at all.
#[cfg(feature = "scripting")]
#[test]
fn test_workspace_body_limit_and_order_of_checks() {
    let fake = FakeProvider::start();
    let srv = signin_server("workspace-body", &fake, &["github"]);
    let p = srv.port;
    let mut b = Browser::new(p);
    let token = web_sign_in(&mut b, &fake, "github", profile_of("github", 1, "a"));
    let auth = format!("Authorization: Bearer {token}");
    let post = |auth: Option<&str>, body: &str| {
        let mut headers = vec![JSON_POST[0], JSON_POST[1]];
        headers.extend(auth);
        http(p, "POST", "/workspace/sync", &headers, body).0
    };
    let huge = "x".repeat(1024 * 1024 + 1);
    assert_eq!(post(Some(&auth), &huge), 413, "over 1 MiB");
    assert_eq!(
        post(None, &huge),
        401,
        "junk with no token is no parse error"
    );
    assert_eq!(post(Some("Authorization: Bearer no"), "junk"), 401);
    assert_eq!(post(Some(&auth), "junk"), 400);
    assert_eq!(post(Some(&auth), r#"{"since":"x"}"#), 422);
    // Exactly the limit is read and parsed.
    let mut exact = format!(r#"{{"since":0,"pad":"{}"#, "x".repeat(1024 * 1024));
    exact.truncate(1024 * 1024 - 2);
    exact.push_str("\"}");
    assert_eq!(post(Some(&auth), &exact), 200);
}

/// What it writes is bounded, not only what it keeps: a pause, with nothing lost.
#[cfg(feature = "scripting")]
#[test]
fn test_workspace_bytes_per_minute_are_bounded() {
    let fake = FakeProvider::start();
    let srv = signin_server("workspace-bytes", &fake, &["github"]);
    let p = srv.port;
    let mut a = Browser::new(p);
    let hers = web_sign_in(&mut a, &fake, "github", profile_of("github", 1, "a"));
    let mut b = Browser::new(p);
    let his = web_sign_in(&mut b, &fake, "github", profile_of("github", 2, "b"));
    let key = "chartgen.pane.1.big";
    let auth = format!("Authorization: Bearer {hers}");
    let mut refused_at = None;
    for i in 0..12 {
        let value = format!("{i}{}", "x".repeat(900 * 1024));
        let body = serde_json::json!({ "since": 0, "writes": [write_doc(key, Some(&value), 0)] });
        let (st, head, resp) = http(
            p,
            "POST",
            "/workspace/sync",
            &[JSON_POST[0], JSON_POST[1], &auth],
            &body.to_string(),
        );
        if st == 429 {
            assert!(
                head.to_ascii_lowercase().contains("retry-after: 60"),
                "{head}"
            );
            assert!(resp.contains("bytes"), "{resp}");
            refused_at = Some(i);
            break;
        }
        assert_eq!(st, 200, "{resp}");
        let answer: serde_json::Value = serde_json::from_str(&resp).unwrap();
        let result = &answer["results"][0];
        assert_eq!(
            result["status"], "too_large",
            "over the document limit, and still counted"
        );
    }
    assert_eq!(
        refused_at,
        Some(10),
        "ten of 900 KiB pass 8 MiB, nine do not"
    );
    let empty = serde_json::json!({ "since": 0 });
    assert_eq!(
        workspace(p, Some(&his), empty).0,
        200,
        "not another account"
    );
}

/// A cursor past the account's versions is said to be, for a device to send back what the
/// file does not have.
#[cfg(feature = "scripting")]
#[test]
fn test_workspace_says_when_the_cursor_is_past_it() {
    let fake = FakeProvider::start();
    let srv = signin_server("workspace-restored", &fake, &["github"]);
    let p = srv.port;
    let mut b = Browser::new(p);
    let token = web_sign_in(&mut b, &fake, "github", profile_of("github", 1, "a"));
    let (_, body) = workspace(p, Some(&token), serde_json::json!({ "since": 0 }));
    assert_eq!(body["restored"], false);
    let (_, body) = workspace(
        p,
        Some(&token),
        serde_json::json!({ "since": 1, "writes": [write_doc("chartgen.pane.1.a", Some("v"), 0)] }),
    );
    assert_eq!(body["restored"], true, "{body}");
    assert_eq!(body["cursor"], 1, "the write alone would have hidden it");
}

/// An application the user approved gets the tools and not the browser's data, though
/// the token is the same user's.
#[cfg(feature = "scripting")]
#[test]
fn test_workspace_is_the_web_apps_alone() {
    let fake = FakeProvider::start();
    let srv = signin_server("workspace-third-party", &fake, &["github"]);
    let p = srv.port;
    let mut b = Browser::new(p);
    let own = web_sign_in(&mut b, &fake, "github", profile_of("github", 5, "octo"));
    let key = "chartgen.alerts.list";
    let (st, _) = workspace(
        p,
        Some(&own),
        serde_json::json!({ "since": 0, "writes": [write_doc(key, Some("private"), 0)] }),
    );
    assert_eq!(st, 200);

    let client = register_client(p, EXTERNAL_REDIRECT);
    let first = b.authorize(&client, EXTERNAL_REDIRECT, "st");
    assert!(first.location().starts_with("/oauth/consent?request="));
    let page = b.get(&first.location());
    let (request, csrf) = consent_fields(&page.body);
    let approved = b.request(
        "POST",
        "/oauth/consent",
        &["Content-Type: application/x-www-form-urlencoded"],
        &format!("request={request}&csrf={csrf}&decision=approve"),
    );
    let theirs = token_from(p, &client, EXTERNAL_REDIRECT, &approved.query()["code"]);
    assert_eq!(me(p, &theirs).1["user"]["id"], "github:5", "the same user");

    let (st, body) = workspace(p, Some(&theirs), serde_json::json!({ "since": 0 }));
    assert_eq!(
        (st, &body["changes"]),
        (403, &serde_json::Value::Null),
        "{body}"
    );
    let (st, _) = workspace(
        p,
        Some(&theirs),
        serde_json::json!({ "since": 0, "writes": [write_doc(key, Some("evil"), 1)] }),
    );
    assert_eq!(st, 403);
    assert_eq!(docs_of(p, &own)[key], "private");
}

/// Anyone can sign in, so these are what a stranger meets. One document, the user's
/// total, requests, batches, and keys that are not the app's.
#[cfg(feature = "scripting")]
#[test]
fn test_workspace_limits() {
    let fake = FakeProvider::start();
    let srv = signin_server("workspace-limits", &fake, &["github"]);
    let p = srv.port;
    let mut b = Browser::new(p);
    let token = web_sign_in(&mut b, &fake, "github", profile_of("github", 1, "a"));
    let send = |writes: Vec<serde_json::Value>| {
        workspace(
            p,
            Some(&token),
            serde_json::json!({ "since": 0, "writes": writes }),
        )
    };

    let too_big = "x".repeat(512 * 1024 + 1);
    let (_, body) = send(vec![write_doc("chartgen.pane.1.a", Some(&too_big), 0)]);
    assert_eq!(
        body["results"][0]["status"], "too_large",
        "{}",
        body["results"]
    );

    let (_, body) = send(vec![
        write_doc("other.key", Some("v"), 0),
        write_doc("chartgen.sidebar.tab", Some("v"), 0),
        write_doc("chartgen.pane.1.good", Some("v"), 0),
    ]);
    let statuses: Vec<_> = body["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["status"].clone())
        .collect();
    assert_eq!(
        statuses,
        ["ok", "bad_key", "bad_key"],
        "the good one still goes through: {body}"
    );

    let many: Vec<_> = (0..51)
        .map(|i| write_doc(&format!("chartgen.pane.{i}.k"), Some("v"), 0))
        .collect();
    assert_eq!(send(many).0, 400, "too many in one request");

    // 2 MiB per user: four of the largest fit, and the fifth does not.
    let large = "x".repeat(512 * 1024);
    let mut refused = None;
    for i in 0..5 {
        let (_, body) = send(vec![write_doc(
            &format!("chartgen.pane.{i}.big"),
            Some(&large),
            0,
        )]);
        if body["results"][0]["status"] == "quota" {
            refused = Some(i);
            break;
        }
    }
    assert_eq!(
        refused,
        Some(3),
        "with the little ones above, the fourth is too many"
    );

    // Requests per user: the next is refused, and another account is not.
    let mut other = Browser::new(p);
    let theirs = web_sign_in(&mut other, &fake, "github", profile_of("github", 2, "b"));
    let empty = serde_json::json!({ "since": 0 });
    let mut limited = None;
    for i in 0..130 {
        let (st, _) = workspace(p, Some(&token), empty.clone());
        if st == 429 {
            limited = Some(i);
            break;
        }
    }
    assert!(limited.is_some(), "a flood of syncs must be refused");
    assert_eq!(
        workspace(p, Some(&theirs), empty).0,
        200,
        "not another account"
    );
}

/// The operator page's read, `GET /admin/overview` (ADR-0063).
#[cfg(feature = "scripting")]
mod operator_page {
    use super::*;

    const OVERVIEW: &str = "/admin/overview";
    const OWN_PAGE: &str = "Sec-Fetch-Site: same-origin";

    fn command(
        port: u16,
        db: &std::path::Path,
        fake: &FakeProvider,
        operators: &str,
        web: Option<&std::path::Path>,
    ) -> std::process::Command {
        let mut cmd = signin_command(port, db, fake, &["github"], None, false);
        cmd.env("CHARTGEN_ADMIN_USERS", operators);
        if let Some(web) = web {
            cmd.env("CHARTGEN_WEB_DIR", web);
        }
        cmd
    }

    fn server(
        test: &str,
        fake: &FakeProvider,
        operators: &str,
        web: Option<&std::path::Path>,
    ) -> Server {
        let port = free_port();
        let db = store_path(test);
        let mut srv = start_server(command(port, &db, fake, operators, web), port);
        srv.temp_db = Some(db);
        srv
    }

    fn web_dir(test: &str) -> std::path::PathBuf {
        let web = crate::test_dir::new(format!("chartgen-{test}"));
        std::fs::write(web.join("index.html"), "<!doctype html><title>app</title>").unwrap();
        web
    }

    /// An answer without what differs between any two: the date.
    fn answer(port: u16, method: &str, path: &str, headers: &[&str]) -> (u16, Vec<String>, String) {
        let (status, head, body) = http(port, method, path, headers, "");
        let mut lines: Vec<String> = head
            .lines()
            .skip(1)
            .filter(|l| !l.starts_with("date:"))
            .map(str::to_string)
            .collect();
        lines.sort();
        (status, lines, body)
    }

    fn as_operator(token: &str) -> Vec<String> {
        vec![
            format!("Authorization: Bearer {token}"),
            OWN_PAGE.to_string(),
        ]
    }

    fn overview(port: u16, token: &str) -> (u16, String, String) {
        let headers = as_operator(token);
        let headers: Vec<&str> = headers.iter().map(String::as_str).collect();
        http(port, "GET", OVERVIEW, &headers, "")
    }

    /// A token of `b`'s user for an application: its code goes to another site, after
    /// consent. With a refresh token.
    fn application_tokens(b: &mut Browser) -> (String, String, String) {
        let port = b.port;
        let client = register_external(port);
        let first = b.authorize(&client, EXTERNAL_REDIRECT, "s");
        let page = b.get(&first.location());
        let (request, csrf) = consent_fields(&page.body);
        let approved = b.request(
            "POST",
            "/oauth/consent",
            &["Content-Type: application/x-www-form-urlencoded"],
            &format!("request={request}&csrf={csrf}&decision=approve"),
        );
        assert_eq!(approved.status, 303, "{}", approved.body);
        let (st, body) = exchange(port, &client, EXTERNAL_REDIRECT, &approved.query()["code"]);
        assert_eq!(st, 200, "{body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        (
            client,
            v["access_token"].as_str().unwrap().to_string(),
            v["refresh_token"].as_str().unwrap().to_string(),
        )
    }

    #[test]
    fn test_the_operator_sees_accounts_clients_and_the_server_and_no_secret() {
        let fake = FakeProvider::start();
        let srv = server("admin-sees", &fake, "github:1", None);
        let p = srv.port;
        let mut ann = Browser::new(p);
        let token = web_sign_in(&mut ann, &fake, "github", profile_of("github", 1, "ann"));
        let mut bob = Browser::new(p);
        let bobs = web_sign_in(
            &mut bob,
            &fake,
            "github",
            profile_of("github", 2, "b\u{202e}ob"),
        );
        let (client, application, refresh) = application_tokens(&mut ann);

        let (st, head, body) = overview(p, &token);
        assert_eq!(st, 200, "{body}");
        for line in [
            "content-type: application/json",
            "cache-control: no-store",
            "vary: authorization",
            "x-frame-options: deny",
        ] {
            assert!(head.contains(line), "{line}: {head}");
        }
        assert_eq!(
            head.matches("x-content-type-options: nosniff").count(),
            1,
            "{head}"
        );
        assert!(!head.contains("access-control-"), "{head}");

        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["users"]["total"], 2);
        let users = v["users"]["shown"].as_array().unwrap();
        let of = |id: &str| users.iter().find(|u| u["id"] == id).unwrap().clone();
        let (a, b) = (of("github:1"), of("github:2"));
        assert_eq!(a["name"], "ann");
        assert_eq!(a["provider"], "github");
        assert_eq!(a["authorisations"], 1);
        assert!(a["last_login_at"].as_i64().unwrap() > 0);
        assert!(a["last_token_at"].as_i64().unwrap() > 0);
        // The character that turns text around is not passed on.
        assert_eq!(b["name"], "bob");
        assert_eq!(b["authorisations"], 0);
        assert_eq!(v["clients"]["limit"], 1000);
        assert!(v["clients"]["total"].as_i64().unwrap() >= 3);
        let short = &client[..8];
        let row = v["clients"]["shown"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == short)
            .unwrap();
        assert_eq!(row["has_authorisation"], true);
        assert_eq!(v["server"]["features"]["sign_in"], true);
        assert_eq!(v["server"]["features"]["replay"], false);
        assert!(v["server"]["schema_version"].as_i64().unwrap() >= 9);
        assert!(v["server"]["started_at"].as_i64().unwrap() > 0);
        assert!(v["server"]["store_bytes"].as_i64().unwrap() > 0);
        // Nothing that opens anything: no token, no whole client id, no cookie.
        for secret in [
            token.as_str(),
            bobs.as_str(),
            application.as_str(),
            refresh.as_str(),
            client.as_str(),
        ] {
            assert!(!body.contains(secret), "{secret} in {body}");
        }
        for value in ann.cookies.values().chain(bob.cookies.values()) {
            assert!(!body.contains(value.as_str()), "a cookie in {body}");
        }
        // The path alone decides: a query is still the endpoint.
        let headers = as_operator(&token);
        let headers: Vec<&str> = headers.iter().map(String::as_str).collect();
        assert_eq!(http(p, "GET", "/admin/overview?x=1", &headers, "").0, 200);
    }

    /// Everyone but an operator in the web app gets, to the byte, what an unknown path
    /// gets: for every kind of caller, and for everything the answer to an unknown path
    /// itself depends on. With the web app and without it.
    #[test]
    fn test_anyone_else_gets_what_an_unknown_path_gets() {
        let fake = FakeProvider::start();
        let web = web_dir("admin-else");
        for web in [Some(web.as_path()), None] {
            let srv = server("admin-else", &fake, "github:1", web);
            let p = srv.port;
            let mut ann = Browser::new(p);
            let anns = web_sign_in(&mut ann, &fake, "github", profile_of("github", 1, "ann"));
            let mut bob = Browser::new(p);
            let bobs = web_sign_in(&mut bob, &fake, "github", profile_of("github", 2, "bob"));
            let (client, application, refresh) = application_tokens(&mut ann);
            let (st, body) = refreshed(p, &client, &refresh);
            assert_eq!(st, 200, "{body}");
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            let refreshed = v["access_token"].as_str().unwrap().to_string();
            assert_eq!(overview(p, &anns).0, 200, "the operator, to compare with");

            let bearer = |t: &str| format!("Authorization: Bearer {t}");
            let callers: Vec<(&str, Vec<String>)> = vec![
                ("nobody", vec![]),
                ("nobody, from our page", vec![OWN_PAGE.into()]),
                (
                    "a token that is none",
                    vec![bearer("nope"), OWN_PAGE.into()],
                ),
                (
                    "another user in the web app",
                    vec![bearer(&bobs), OWN_PAGE.into()],
                ),
                (
                    "the operator's application",
                    vec![bearer(&application), OWN_PAGE.into()],
                ),
                (
                    "the operator's refreshed token",
                    vec![bearer(&refreshed), OWN_PAGE.into()],
                ),
                ("the operator, not from our page", vec![bearer(&anns)]),
                (
                    "the operator, from another site",
                    vec![bearer(&anns), "Sec-Fetch-Site: cross-site".into()],
                ),
            ];
            for (who, headers) in &callers {
                for accept in [Some(HTML), Some("Accept: application/json"), None] {
                    for method in ["GET", "HEAD", "POST"] {
                        let mut headers: Vec<&str> = headers.iter().map(String::as_str).collect();
                        headers.extend(accept);
                        assert_eq!(
                            answer(p, method, OVERVIEW, &headers),
                            answer(p, method, "/admin/zzzzzzzz", &headers),
                            "{who}, {method}, {accept:?}, web app: {}",
                            web.is_some()
                        );
                    }
                }
            }
            // And for the operator, any path but the one, and any method but GET.
            let operator = as_operator(&anns);
            let operator: Vec<&str> = operator.iter().map(String::as_str).collect();
            for (method, path, unknown) in [
                ("GET", "/admin/overview/", "/admin/zzzzzzzz/"),
                ("GET", "/Admin/Overview", "/Admin/Zzzzzzzz"),
                ("GET", "/admin/%6fverview", "/admin/%7azzzzzzz"),
                ("GET", "/admin", "/zzzzz"),
                ("HEAD", OVERVIEW, "/admin/zzzzzzzz"),
                ("POST", OVERVIEW, "/admin/zzzzzzzz"),
            ] {
                assert_eq!(
                    answer(p, method, path, &operator),
                    answer(p, method, unknown, &operator),
                    "{method} {path}, web app: {}",
                    web.is_some()
                );
            }
        }
    }

    fn refreshed(port: u16, client: &str, refresh: &str) -> (u16, String) {
        let body = format!("grant_type=refresh_token&client_id={client}&refresh_token={refresh}");
        let (st, _, body) = http(
            port,
            "POST",
            "/token",
            &["Content-Type: application/x-www-form-urlencoded"],
            &body,
        );
        (st, body)
    }

    /// Signing out ends it at once; a restart ends it too, as the web app's token is not
    /// kept; and who was taken off the list is refused after the restart.
    #[test]
    fn test_the_page_ends_with_the_session_a_restart_and_the_list() {
        let fake = FakeProvider::start();
        let port = free_port();
        let db = store_path("admin-ends");
        let unknown = |token: &str| {
            let headers = as_operator(token);
            let headers: Vec<&str> = headers.iter().map(String::as_str).collect();
            assert_eq!(
                answer(port, "GET", OVERVIEW, &headers),
                answer(port, "GET", "/admin/zzzzzzzz", &headers)
            );
        };
        let sign_in = || {
            let mut ann = Browser::new(port);
            let token = web_sign_in(&mut ann, &fake, "github", profile_of("github", 1, "ann"));
            (ann, token)
        };
        let srv = start_server(command(port, &db, &fake, "github:1", None), port);
        let (mut ann, token) = sign_in();
        assert_eq!(overview(port, &token).0, 200);
        assert_eq!(ann.request("POST", "/oauth/logout", &[], "").status, 204);
        unknown(&token);

        let (_ann, token) = sign_in();
        assert_eq!(overview(port, &token).0, 200);
        drop(srv);
        let srv = start_server(command(port, &db, &fake, "github:1", None), port);
        unknown(&token);
        // Signed in again, the page is back; and gone when the list names another.
        let (_ann, token) = sign_in();
        assert_eq!(overview(port, &token).0, 200);
        drop(srv);
        let mut srv = start_server(command(port, &db, &fake, "github:11", None), port);
        let (_ann, token) = sign_in();
        unknown(&token);
        srv.temp_db = Some(db);
    }

    /// A list the server cannot read as written, or one that can never take effect,
    /// stops the start: it is not read as "nobody".
    #[test]
    fn test_a_bad_operator_list_stops_the_start() {
        let fake = FakeProvider::start();
        for (test, operators, sign_in) in [
            ("admin-bad-star", "*", true),
            ("admin-bad-name", "github:1,someone", true),
            ("admin-bad-off", "github:1", false),
        ] {
            let port = free_port();
            let db = store_path(test);
            let mut cmd = if sign_in {
                command(port, &db, &fake, operators, None)
            } else {
                let mut cmd = server_command(port, None, &db, false);
                cmd.env("CHARTGEN_ADMIN_USERS", operators);
                cmd
            };
            let mut child = cmd
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let give_up = std::time::Instant::now() + std::time::Duration::from_secs(20);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if std::time::Instant::now() > give_up {
                    child.kill().unwrap();
                    panic!("{test}: the server started");
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            };
            assert!(!status.success(), "{test}");
            let mut said = String::new();
            use std::io::Read;
            child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut said)
                .unwrap();
            assert!(said.contains("CHARTGEN_ADMIN_USERS"), "{test}: {said}");
            assert!(!said.contains("someone"), "{test}: {said}");
            let _ = std::fs::remove_file(&db);
        }
    }

    /// Without the setting, and with sign-in off, there is no such page.
    #[test]
    fn test_without_operators_or_sign_in_there_is_no_page() {
        let fake = FakeProvider::start();
        let srv = signin_server("admin-none", &fake, &["github"]);
        let mut ann = Browser::new(srv.port);
        let token = web_sign_in(&mut ann, &fake, "github", profile_of("github", 1, "ann"));
        let off = spawn_server(None);
        for (port, token) in [(srv.port, token.as_str()), (off.port, "anything")] {
            let headers = as_operator(token);
            let headers: Vec<&str> = headers.iter().map(String::as_str).collect();
            assert_eq!(
                answer(port, "GET", OVERVIEW, &headers),
                answer(port, "GET", "/admin/zzzzzzzz", &headers)
            );
        }
    }
}
