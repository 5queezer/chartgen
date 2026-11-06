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
        .env_remove("CHARTGEN_GITHUB_CLIENT_ID")
        .env_remove("CHARTGEN_GITHUB_CLIENT_SECRET")
        .env_remove("CHARTGEN_GOOGLE_CLIENT_ID")
        .env_remove("CHARTGEN_GOOGLE_CLIENT_SECRET")
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
    (status, head.to_string(), body.to_string())
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

    /// `GET /authorize` for a client, as it arrives from that client.
    fn authorize(&mut self, client_id: &str, redirect_uri: &str, state: &str) -> Page {
        let path = format!(
            "/authorize?response_type=code&client_id={client_id}&redirect_uri={}&code_challenge={PKCE_CHALLENGE}&state={state}",
            url_encode(redirect_uri)
        );
        self.get(&path)
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
    // One client: the web app gets a token per page load, and they do not pile up.
    let client = register_client(p, &redirect);
    let mut last = Vec::new();
    for _ in 0..21 {
        let answer = b.authorize(&client, &redirect, "s");
        last.push(token_from(p, &client, &redirect, &answer.query()["code"]));
    }
    assert_eq!(ping_status(p, &last[0]), 401);
    assert_eq!(ping_status(p, &last[20]), 200);
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
    for i in 0..300 {
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
    for i in 0..310 {
        let (st, _, body) = http(p, "POST", "/mcp", &headers, call);
        assert_eq!(st, 200, "call {i}: {body}");
    }
}

/// Sign-ins in progress are bounded on loopback too, where the per-address limit does
/// not apply, and hitting the bound leaves a signed-in user alone.
#[test]
fn test_signin_pending_logins_are_bounded() {
    let fake = FakeProvider::start();
    let srv = signin_server("signin-cap", &fake, &["github", "google"]);
    let p = srv.port;
    let mut holder = Browser::new(p);
    web_sign_in(&mut holder, &fake, "github", profile_of("github", 1, "a"));
    let client = register_client(p, &web_redirect(p));
    let mut limited = None;
    let mut b = Browser::new(p);
    for i in 0..2100 {
        let page = b.authorize(&client, &web_redirect(p), "s");
        if page.status == 429 {
            limited = Some((i, page));
            break;
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

/// ADR-0044: scripts from before sign-in go to the first user through their client,
/// the next user does not get them, and nothing is lost.
#[cfg(feature = "scripting")]
#[test]
fn test_signin_existing_scripts_move_to_the_first_user_of_their_client() {
    let fake = FakeProvider::start();
    let port = free_port();
    let db = store_path("signin-claim");
    let old = spawn_server_on(port, None, &db, true);
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
        serde_json::json!({"name": "from_before", "source": OWN_SCRIPT}),
    );
    assert!(answer.contains("from_before"), "{answer}");
    drop(old);

    let _srv = start_server(
        signin_command(port, &db, &fake, &["github"], None, true),
        port,
    );
    let mine = |token: &str| tool_text(port, token, "list_my_indicators", serde_json::json!({}));
    let mut first = Browser::new(port);
    let answer = first.authorize(&client, &redirect, "s");
    let back = provider_round_trip(
        &mut first,
        &fake,
        "github",
        profile_of("github", 1, "ann"),
        &answer,
    );
    let ann = token_from(port, &client, &redirect, &back.query()["code"]);
    assert!(mine(&ann).contains("from_before"), "{}", mine(&ann));

    let mut second = Browser::new(port);
    let answer = second.authorize(&client, &redirect, "s");
    let back = provider_round_trip(
        &mut second,
        &fake,
        "github",
        profile_of("github", 2, "bob"),
        &answer,
    );
    let bob = token_from(port, &client, &redirect, &back.query()["code"]);
    assert!(!mine(&bob).contains("from_before"), "{}", mine(&bob));
    assert!(mine(&ann).contains("from_before"), "{}", mine(&ann));
}
