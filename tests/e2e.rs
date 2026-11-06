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
        // AAPL is not crypto, should return empty gracefully
        assert!(!result.label.is_empty());
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
    assert_eq!(available.len(), 72, "Expected 72 indicators");
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
    // Expect multiple POC lines — one per session — each with NaN gaps.
    let poc_lines: Vec<_> = result
        .lines
        .iter()
        .filter(|l| l.label.as_deref() == Some("POC"))
        .collect();
    assert!(poc_lines.len() >= 2, "expected at least 2 session POCs");
    for line in &poc_lines {
        let nans = line.y.iter().filter(|v| v.is_nan()).count();
        let finite = line.y.len() - nans;
        assert!(
            nans > 0,
            "session POC should have NaN gaps outside its range"
        );
        assert!(finite > 0, "session POC should have some finite values");
    }
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
    let db = std::env::temp_dir().join(format!("chartgen-e2e-{}-{port}.db", std::process::id()));
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
        .env_remove("CHARTGEN_SERVER_ALERTS")
        .env_remove("CHARTGEN_WEBHOOK_TEST_ORIGIN")
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
    (status, head.to_ascii_lowercase(), body.to_string())
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

#[test]
fn test_server_serves_web_app_same_origin() {
    let root = std::env::temp_dir().join(format!("chartgen-web-{}", std::process::id()));
    let web = root.join("dist");
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html><title>app</title>").unwrap();
    std::fs::write(web.join("assets/x.js"), "console.log(1)").unwrap();
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
    let web = std::env::temp_dir().join(format!("chartgen-web-authz-{}", std::process::id()));
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

const SCRIPT_TOOLS: [&str; 4] = [
    "register_indicator",
    "fork_indicator",
    "list_my_indicators",
    "delete_indicator",
];

/// A fresh path for a store that outlives one server.
fn store_path(test: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("chartgen-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("chartgen.db")
}

const ALERT_TOOLS: [&str; 4] = [
    "create_server_alert",
    "list_server_alerts",
    "update_server_alert",
    "delete_server_alert",
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
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    /// A webhook receiver answering 200; returns its port and what it received.
    fn receiver() -> (u16, Arc<Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = vec![0u8; 16384];
                let n = s.read(&mut buf).unwrap_or(0);
                log.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).to_string());
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                );
            }
        });
        (port, seen)
    }

    fn alert_server(db: &std::path::Path, hook_port: u16, sample: bool) -> Server {
        let port = free_port();
        let mut cmd = server_command(port, None, db, false);
        cmd.env("CHARTGEN_SERVER_ALERTS", "1").env(
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
        assert_eq!(seen.lock().unwrap().len(), 2, "fired again after a restart");
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
        // Tokens stay in memory: the old one is gone, the new one works.
        assert_eq!(ping_status(port, &token), 401);
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
