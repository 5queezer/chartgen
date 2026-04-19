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
    // All values should be finite (ta-rs EMA outputs from bar 0)
    for &v in &result {
        assert!(v.is_finite());
    }
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
    // ta-rs SMA outputs values from bar 0 (running average), not waiting for full period
    for &v in &result {
        assert!(v.is_finite());
    }
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
    // ta-rs SMA still outputs values, but they are from partial windows
    assert_eq!(result.len(), 3);
    for &v in &result {
        assert!(v.is_finite());
    }
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
    assert_eq!(available.len(), 38, "Expected 38 indicators");
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
        let session_end = line
            .y
            .iter()
            .enumerate()
            .skip(start)
            .take_while(|(_, v)| !v.is_nan())
            .last()
            .map(|(i, _)| i + 1)
            .unwrap_or(start + 1);
        for i in session_end..data.bars.len() {
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
