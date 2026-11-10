use super::by_name;
use crate::data::sample_data;
use crate::indicator::PanelResult;

fn result(name: &str) -> PanelResult {
    by_name(name).unwrap().compute(&sample_data(300))
}

#[test]
fn colliding_overlays_have_distinct_palettes() {
    for (a, ai, b, bi) in [
        ("keltner", 0, "donchian", 0),
        ("keltner", 1, "donchian", 1),
        ("supertrend", 0, "sar", 0),
        ("supertrend", 1, "sar", 1),
        ("ma", 0, "vwma", 0),
        ("ma", 0, "hma", 0),
        ("vwma", 0, "hma", 0),
        ("ema_stack", 2, "bbands", 1),
        ("chande_kroll_stop", 1, "envelopes", 1),
        ("chande_kroll_stop", 1, "linreg_channel", 1),
        ("envelopes", 1, "linreg_channel", 1),
    ] {
        assert_ne!(
            result(a).lines[ai].color,
            result(b).lines[bi].color,
            "{a} vs {b}"
        );
    }
    assert_ne!(
        result("frvp").hbars[0].color,
        result("anchored_vp").hbars[0].color
    );
}

#[test]
fn oscillator_references_include_requested_and_existing_levels() {
    for (name, levels) in [
        ("uo", vec![30.0, 70.0]),
        ("cmo", vec![-50.0, 50.0]),
        ("aroon", vec![30.0, 70.0]),
        ("bb_percent_b", vec![0.5]),
        ("fisher", vec![-1.5, 1.5]),
    ] {
        let r = result(name);
        for level in levels {
            assert!(
                r.hlines.iter().any(|h| h.y == level),
                "{name} missing {level}"
            );
        }
    }
}

#[test]
fn legend_names_disambiguate_vwap_and_vigor() {
    let vwap = result("vwap");
    let bands = result("vwap_bands");
    assert_ne!(vwap.lines[0].label, bands.lines[1].label);
    let rvi = result("rvi");
    assert!(rvi.label.contains("RVI"));
    assert!(!rvi.label.contains("RVGI"));
}

#[test]
fn oscillator_secondary_lines_contrast_with_dark_background() {
    // Relative luminance contrast against the PNG background (#131722).
    let luminance = |v: u8| {
        let x = f64::from(v) / 255.0;
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    let bg = 0.2126 * luminance(19) + 0.7152 * luminance(23) + 0.0722 * luminance(34);
    for name in ["wavetrend", "cipher_b"] {
        for line in result(name).lines {
            if !matches!(line.label.as_deref(), Some("WT2" | "Stoch D")) {
                continue;
            }
            let c = line.color;
            let blended = |v: u8, b: f64| (f64::from(v) * c.3 + b * (1.0 - c.3)).round() as u8;
            let fg = 0.2126 * luminance(blended(c.0, 19.0))
                + 0.7152 * luminance(blended(c.1, 23.0))
                + 0.0722 * luminance(blended(c.2, 34.0));
            assert!((fg + 0.05) / (bg + 0.05) >= 3.0, "{name} {:?}", line.label);
        }
    }
}

#[test]
fn drawing_metadata_preserves_numeric_channels() {
    let data = sample_data(30);
    let sar = by_name("sar").unwrap().compute(&data);
    assert!(sar.point_lines);
    assert_eq!(sar.lines.len(), 2);
    assert!(sar.lines.iter().all(|l| l.y.len() == data.len()));
    for name in ["donchian", "pivot"] {
        assert!(result(name).steps);
        assert!(
            !result(name).level_lines,
            "AutoFib level semantics are separate"
        );
    }
    let ha = by_name("heikin_ashi").unwrap().compute(&data);
    let candles = data.heikin_ashi();
    assert_eq!(ha.lines[0].y, candles.closes());
    assert_eq!(
        ha.lines[1].y,
        candles.bars.iter().map(|b| b.open).collect::<Vec<_>>()
    );
    assert_eq!(ha.fills[0].y1, ha.lines[0].y);
    assert_eq!(ha.fills[0].y2, ha.lines[1].y);
    assert!(ha.fills[0].color_below.is_some());
    assert!(result("cmf").y_range.is_none());
}

#[test]
fn profile_semantics_cover_every_reference_line() {
    for name in ["vpvr", "frvp", "anchored_vp", "hvn_lvn", "tpo"] {
        let r = result(name);
        assert!(!r.hlines.is_empty(), "fixture has levels for {name}");
        assert_eq!(r.hlines.len(), r.hline_labels.len(), "{name}");
        assert!(r
            .hline_labels
            .iter()
            .all(|l| l.as_ref().is_some_and(|s| !s.is_empty())));
    }
}

#[test]
fn vwap_boundaries_do_not_erase_session_open_values() {
    let mut data = sample_data(18);
    for (i, bar) in data.bars.iter_mut().enumerate() {
        bar.date = (1793318400 + i as i64 * 4 * 3600).to_string();
    }
    let r = by_name("vwap").unwrap().compute(&data);
    let b = by_name("vwap_bands").unwrap().compute(&data);
    assert_eq!(r.line_breaks, [6, 12]);
    assert_eq!(b.line_breaks, r.line_breaks);
    for i in r.line_breaks {
        let bar = &data.bars[i];
        assert_eq!(r.lines[0].y[i], (bar.high + bar.low + bar.close) / 3.0);
        assert_eq!(b.lines[1].y[i], r.lines[0].y[i]);
    }
    let r = super::by_name_configured("vwap", &serde_json::json!({"anchor":"none"}))
        .unwrap()
        .unwrap()
        .compute(&data);
    assert!(r.line_breaks.is_empty());
}
