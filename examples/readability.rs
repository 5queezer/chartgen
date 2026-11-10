//! Offline PNG fixtures for the readability review. Run with an output directory.
use chartgen::{data, indicators, renderer};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args().nth(1).expect("output directory");
    std::fs::create_dir_all(&directory)?;
    let mut data = data::sample_data(200);
    data.symbol = Some("BTCUSDT (deterministic fixture)".into());
    data.interval = Some("4h".into());
    for (i, bar) in data.bars.iter_mut().enumerate() {
        for price in [&mut bar.open, &mut bar.high, &mut bar.low, &mut bar.close] {
            *price = 80_000.0 + (*price - 30.0) * 100.0;
        }
        bar.date = (1_793_318_400_i64 + i as i64 * 14_400).to_string();
    }
    let groups: &[(&str, &[&str])] = &[
        ("channels", &["bbands", "keltner", "donchian"]),
        ("trend", &["supertrend", "sar", "ma", "vwma", "hma"]),
        (
            "averages",
            &[
                "ema_stack",
                "bbands",
                "chande_kroll_stop",
                "envelopes",
                "linreg_channel",
            ],
        ),
        ("profiles", &["frvp", "anchored_vp", "hvn_lvn"]),
        ("sessions", &["vwap", "vwap_bands", "pivot"]),
        ("heikin-patterns", &["heikin_ashi", "candlestick_patterns"]),
        (
            "oscillators",
            &["tsi", "cci", "cmo", "uo", "choppiness", "balance_of_power"],
        ),
        (
            "values",
            &["ad", "cvd", "elder_force_index", "ease_of_movement"],
        ),
        (
            "references",
            &["aroon", "bb_percent_b", "fisher_transform", "cmf"],
        ),
        (
            "waves",
            &["cipher_b", "wavetrend", "rvi", "awesome_oscillator"],
        ),
    ];
    for (name, names) in groups {
        let panels = names
            .iter()
            .map(|name| {
                indicators::by_name(name).unwrap_or_else(|| panic!("unknown indicator {name}"))
            })
            .collect::<Vec<_>>();
        renderer::render_chart(
            &data,
            &panels,
            &format!("{directory}/{name}.png"),
            1600,
            1000,
        )?;
    }
    // A large absolute value with a very narrow range must keep distinct ticks.
    let mut narrow = data.clone();
    narrow.symbol = Some("Narrow-range readability probe".into());
    for bar in &mut narrow.bars {
        for price in [&mut bar.open, &mut bar.high, &mut bar.low, &mut bar.close] {
            *price = 80_000.0 + (*price - 80_000.0) * 0.001;
        }
    }
    let oi = chartgen::indicator::PanelResult {
        label: "OI probe".into(),
        lines: vec![chartgen::indicator::Line {
            y: (0..200)
                .map(|i| 8_400_000_000.0 + i as f64 * 100.0)
                .collect(),
            color: plotters::style::RGBAColor(33, 150, 243, 1.0),
            width: 2,
            label: Some("OI".into()),
            breaks: false,
        }],
        ..Default::default()
    };
    renderer::render_results(
        &narrow,
        &[&oi],
        &format!("{directory}/narrow-axis.png"),
        1000,
        600,
    )?;
    Ok(())
}
