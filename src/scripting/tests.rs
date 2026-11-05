//! Runtime, Series operator, sandbox escape, persistence and wavetrend
//! parity tests for user scripts.

use super::runtime::{compile, run, COMPUTE_TIMEOUT};
use super::*;
use crate::data::{sample_data, Bar, OhlcvData};
use crate::indicators::pine;
use std::time::{Duration, Instant};

fn script(body: &str) -> String {
    format!(
        "PARAMS = []\nMETA = {{\"title\": \"T\"}}\ndef compute(ohlcv, params, chart):\n{}\n",
        body.lines()
            .map(|l| format!("    {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// Run a compute body on `bars` sample bars; returns the chart or the error.
fn exec(body: &str, bars: usize) -> Result<PanelResult, ScriptError> {
    let c = compile("t", &script(body))?;
    run(
        &c,
        &sample_data(bars),
        &json!({}),
        COMPUTE_TIMEOUT,
        "runtime",
    )
}

/// The first line a compute body plotted.
fn line(body: &str) -> Vec<f64> {
    let pr = exec(body, 50).unwrap_or_else(|e| panic!("{e}"));
    pr.lines[0].y.clone()
}

fn closes(d: &OhlcvData) -> Vec<f64> {
    d.bars.iter().map(|b| b.close).collect()
}

// ---- runtime ----

#[test]
fn compiles_params_and_meta() {
    let src = r#"
PARAMS = [
    {"name": "n", "type": "int", "default": 14, "min": 1, "max": 500, "desc": "Length"},
    {"name": "k", "type": "float", "default": 2},
    {"name": "src", "type": "string", "default": "close", "options": ["close", "open"]},
]
META = {"title": "My RSI", "category": "overlay", "description": "d"}
def compute(ohlcv, params, chart):
    chart.line(ta.rsi(ohlcv.close, params["n"]) * params["k"])
"#;
    let c = compile("my_rsi", src).unwrap();
    assert_eq!(c.info.title, "My RSI");
    assert!(c.info.overlay);
    assert_eq!(c.info.params[0]["type"], "integer");
    assert_eq!(c.info.params[0]["description"], "Length");
    assert_eq!(c.info.params[1]["type"], "number");
    assert_eq!(c.info.params[2]["enum"], json!(["close", "open"]));
    // configured params override defaults; numbers coerce to the declared type
    let d = sample_data(40);
    let pr = run(&c, &d, &json!({"n": 5, "k": 1}), COMPUTE_TIMEOUT, "runtime").unwrap();
    pine::test_util::eq(&pr.lines[0].y, &pine::rsi(&closes(&d), 5));
    assert!(pr.is_overlay);
    assert_eq!(pr.label, "My RSI");
}

#[test]
fn ta_functions_match_pine_helpers() {
    let d = sample_data(50);
    let c = closes(&d);
    let eq = pine::test_util::eq;
    eq(
        &line("chart.line(ta.sma(ohlcv.close, 5))"),
        &pine::sma_n(&c, 5),
    );
    eq(
        &line("chart.line(ta.ema(ohlcv.close, 5))"),
        &pine::ema_n(&c, 5),
    );
    eq(
        &line("chart.line(ta.wma(ohlcv.close, 5))"),
        &pine::wma_n(&c, 5),
    );
    eq(
        &line("chart.line(ta.rma(ohlcv.close, 5))"),
        &pine::rma_n(&c, 5),
    );
    eq(
        &line("chart.line(ta.highest(ohlcv.close, 5))"),
        &pine::highest(&c, 5),
    );
    eq(
        &line("chart.line(ta.stdev(ohlcv.close, 5))"),
        &pine::stdev(&c, 5),
    );
    eq(
        &line("chart.line(ta.roc(ohlcv.close, 3))"),
        &pine::roc(&c, 3),
    );
    eq(
        &line("chart.line(ta.change(ohlcv.close))"),
        &pine::change_n(&c, 1),
    );
    eq(&line("chart.line(ta.atr(ohlcv, 5))"), &pine::atr(&d, 5));
    let hi: Vec<f64> = d.bars.iter().map(|b| b.high).collect();
    let lo: Vec<f64> = d.bars.iter().map(|b| b.low).collect();
    eq(
        &line("chart.line(ta.stoch(ohlcv.close, ohlcv.high, ohlcv.low, 5))"),
        &pine::stoch(&c, &hi, &lo, 5),
    );
    let m = line("m, s, h = ta.macd(ohlcv.close, 3, 6, 2)\nchart.line(h)");
    let mac: Vec<f64> = pine::ema_n(&c, 3)
        .iter()
        .zip(pine::ema_n(&c, 6))
        .map(|(a, b)| a - b)
        .collect();
    let sig = pine::ema_n(&mac, 2);
    eq(
        &m,
        &mac.iter().zip(&sig).map(|(a, b)| a - b).collect::<Vec<_>>(),
    );
    eq(
        &line("chart.line(ta.pivot_high(ohlcv.high, 2, 2))"),
        &pine::pivot_high(&hi, 2, 2),
    );
}

#[test]
fn per_bar_loops_work() {
    let y = line(
        "vals = ohlcv.close.list()\nout = []\ntotal = 0.0\nfor v in vals:\n    total += v\n    out.append(total)\nchart.line(ta.series(out))",
    );
    let c = closes(&sample_data(50));
    assert!((y[49] - c.iter().sum::<f64>()).abs() < 1e-9);
}

#[test]
fn chart_builder_fills_every_panel_field() {
    let pr = exec(
        r##"
c = ohlcv.close
chart.line(c, title = "c", color = "red", width = 3)
chart.fill(c, c * 2, color = "#00ff0080", color_below = "blue")
chart.bars(c - c[1], color = "teal")
chart.dot(c > c[1], c, label = "up")
chart.dot(3, 1.5, size = 7)
chart.hline(10, color = "gray")
chart.hbar(30.0, 0.5, 0.4, left = True)
chart.divline(1, 2.0, 5, 3.0, dashed = True)
chart.set_y_range(-1, 1)
chart.set_label("Custom")
"##,
        20,
    )
    .unwrap();
    assert_eq!(pr.lines[0].label.as_deref(), Some("c"));
    assert_eq!(pr.lines[0].width, 3);
    assert_eq!(
        pr.fills[0].color,
        crate::indicator::rgba(0x00ff00, 128.0 / 255.0)
    );
    assert!(pr.fills[0].color_below.is_some());
    assert_eq!(pr.bars[0].colors.len(), 20);
    assert!(pr.dots.iter().any(|d| d.label.as_deref() == Some("up")));
    assert!(pr.dots.iter().any(|d| d.x == 3 && d.size == 7));
    assert_eq!(pr.hlines[0].y, 10.0);
    assert!(pr.hbars[0].left);
    assert!(pr.divlines[0].dashed);
    assert_eq!(pr.y_range, Some((-1.0, 1.0)));
    assert_eq!(pr.label, "Custom");
}

// ---- Series operators ----

#[test]
fn series_arithmetic_both_operand_orders() {
    let c = closes(&sample_data(50));
    let eq = pine::test_util::eq;
    let map = |f: fn(f64) -> f64| c.iter().map(|&x| f(x)).collect::<Vec<_>>();
    eq(&line("chart.line(ohlcv.close + 1)"), &map(|x| x + 1.0));
    eq(&line("chart.line(1 + ohlcv.close)"), &map(|x| 1.0 + x));
    eq(&line("chart.line(10 - ohlcv.close)"), &map(|x| 10.0 - x));
    eq(&line("chart.line(ohlcv.close * 2)"), &map(|x| x * 2.0));
    eq(&line("chart.line(2 * ohlcv.close)"), &map(|x| 2.0 * x));
    eq(&line("chart.line(1 / ohlcv.close)"), &map(|x| 1.0 / x));
    eq(&line("chart.line(-ohlcv.close)"), &map(|x| -x));
    eq(
        &line("x = ohlcv.close\nx -= 1\nchart.line(x)"),
        &map(|x| x - 1.0),
    );
    // division by zero is na, not inf
    assert!(line("chart.line(ohlcv.close / 0)")
        .iter()
        .all(|v| v.is_nan()));
    assert!(
        line("chart.line(ohlcv.close / (ohlcv.close - ohlcv.close))")
            .iter()
            .all(|v| v.is_nan())
    );
}

#[test]
fn series_history_offset() {
    let c = closes(&sample_data(50));
    let y = line("chart.line(ohlcv.close[2])");
    assert!(y[0].is_nan() && y[1].is_nan());
    assert_eq!(y[2..], c[..48]);
    let err = exec("chart.line(ohlcv.close[-1])", 10).err().unwrap();
    assert!(err.message.contains("bars ago"), "{err}");
}

#[test]
fn series_comparisons_return_bool_series() {
    let c = closes(&sample_data(50));
    let y = line("chart.line(ohlcv.close > ohlcv.close[1])");
    assert_eq!(y[0], 0.0, "na comparison is false");
    for i in 1..50 {
        assert_eq!(y[i], if c[i] > c[i - 1] { 1.0 } else { 0.0 });
    }
    let both = line("up = ohlcv.close > ohlcv.open\nchart.line(up & ~up)");
    assert!(both.iter().all(|v| *v == 0.0));
    let either = line("up = ohlcv.close > ohlcv.open\nchart.line(up | ~up)");
    assert!(either.iter().all(|v| *v == 1.0));
    let eq = line("chart.line(ohlcv.close == ohlcv.close)");
    assert!(eq.iter().all(|v| *v == 1.0));
    let ne = line("chart.line(30 != ohlcv.close)");
    assert!(ne.iter().all(|v| *v == 1.0));
}

#[test]
fn series_methods_and_scalars_keep_plain_semantics() {
    let pr = exec(
        r#"
c = ohlcv.close
if len(c) != 10 or c.get(-1) != c.list()[9]:
    fail("len/get/list")
if not (1 < 2 and "a" < "b" and [1] == [1] and 3 // 2 == 1 and 7 % 3 == 1):
    fail("scalar operators")
if "a" + "b" != "ab" or 2 * "ab" != "abab" or "%s-%d" % ("x", 3) != "x-3":
    fail("strings")
n = 1
n += 2
n *= 3
chart.hline(n)
"#,
        10,
    )
    .unwrap();
    assert_eq!(pr.hlines[0].y, 9.0);
}

#[test]
fn integer_overflow_is_an_error() {
    let e = exec("x = 1 << 62\ny = x * 4", 10).err().unwrap();
    assert!(e.message.contains("overflow"), "{e}");
}

#[test]
fn length_mismatch_is_an_error() {
    let e = exec("chart.line(ohlcv.close + ta.series([1, 2]))", 10)
        .err()
        .unwrap();
    assert!(e.message.contains("length mismatch"), "{e}");
}

// ---- validation pipeline ----

fn validate_err(src: &str) -> ScriptError {
    validate("t", src)
        .err()
        .expect("expected a validation error")
}

#[test]
fn pipeline_reports_step_and_line() {
    let e = validate_err("PARAMS = []\nMETA = {}\ndef compute(ohlcv, params, chart)\n    pass\n");
    assert_eq!((e.step, e.line), ("parse", Some(3)));

    let e = validate_err(
        "PARAMS = []\nMETA = {}\ndef compute(ohlcv, params, chart):\n    x = undefined_name\n",
    );
    assert_eq!(e.step, "static");
    assert_eq!(e.line, Some(4), "{e}");

    let e = validate_err("PARAMS = []\nMETA = {}\n");
    assert_eq!(e.step, "presence");
    assert!(e.message.contains("compute"));
    let e = validate_err("META = {}\ndef compute(ohlcv, params, chart):\n    pass\n");
    assert!(e.message.contains("PARAMS"), "{e}");
    let e = validate_err("PARAMS = [{\"name\": \"n\", \"type\": \"int\", \"default\": 1.5}]\nMETA = {}\ndef compute(ohlcv, params, chart):\n    pass\n");
    assert!(e.message.contains("default must be a integer"), "{e}");

    let e = validate_err(&script("x = 1\nchart.line(ohlcv.close * \"a\")"));
    assert_eq!(e.step, "dry_run");
    assert_eq!(e.line, Some(5), "{e}");

    let e = validate_err(&script("return 1"));
    assert!(e.message.contains("must not return"), "{e}");
}

#[test]
fn runtime_errors_fail_with_line() {
    let e = exec("a = 1\nb = a.nope", 10).err().unwrap();
    assert_eq!(e.step, "runtime");
    assert_eq!(e.line, Some(5));
}

// ---- sandbox escape suite: clean errors, never a panic or a hang ----

fn assert_rejected(src: &str, expect: &str) {
    let t = Instant::now();
    let e = validate("t", src).err().expect("script must be rejected");
    assert!(
        e.message.to_lowercase().contains(expect),
        "expected {expect:?} in {e}"
    );
    assert!(
        t.elapsed() < Duration::from_secs(3),
        "took {:?}",
        t.elapsed()
    );
}

#[test]
fn sandbox_load_is_rejected() {
    assert_rejected("load(\"//:stdlib.star\", \"x\")\n", "load");
}

#[test]
fn sandbox_recursion_bomb() {
    assert_rejected(&script("def f(n):\n    return f(n + 1)\nf(0)"), "stack");
}

#[test]
fn sandbox_infinite_loop() {
    assert_rejected(
        &script("for i in range(20000):\n    for j in range(20000):\n        pass"),
        "limit",
    );
}

#[test]
fn sandbox_huge_allocations() {
    assert_rejected(&script("x = [0] * 1000000000"), "too large");
    assert_rejected(&script("x = \"x\" * (1 << 30)"), "too large");
    assert_rejected(&script("x = list(range(1000000000))"), "range");
    assert_rejected(
        &script("s = \"x\"\nfor i in range(64):\n    s = s + s"),
        "too large",
    );
    assert_rejected(
        &script("a = [0]\nfor i in range(64):\n    a += a"),
        "too large",
    );
    assert_rejected(
        &script("s = \"x\" * 10000\nt = str([s] * 10000)"),
        "too large",
    );
    assert_rejected(
        &script("a = list(range(10000))\nz = zip(*([a] * 10000))"),
        "too large",
    );
    assert_rejected(
        &script("s = \"%s\" * 5000 % tuple([\"x\" * 10000] * 5000)"),
        "too large",
    );
    assert_rejected(&script("x = \"\".join([\"a\"] * 10)"), "not available");
    assert_rejected(&script("x = getattr([], \"extend\")"), "not available");
}

#[test]
fn sandbox_series_allocations_are_budgeted() {
    // Series data lives outside the Starlark heap, so it is counted exactly.
    let c = compile(
        "t",
        &script("for i in range(10000):\n    x = ohlcv.close + 1"),
    )
    .unwrap();
    let e = run(
        &c,
        &sample_data(5000),
        &json!({}),
        COMPUTE_TIMEOUT,
        "runtime",
    )
    .err()
    .unwrap();
    assert!(e.message.contains("memory limit"), "{e}");
}

#[test]
fn sandbox_shadowed_builtins_are_rejected() {
    assert_rejected(
        "ta = {}\nPARAMS = []\nMETA = {}\ndef compute(o, p, c):\n    pass\n",
        "shadows",
    );
    assert_rejected(
        "def len(x):\n    return 0\nPARAMS = []\nMETA = {}\ndef compute(o, p, c):\n    pass\n",
        "shadows",
    );
    assert_rejected(
        "_op_mul = None\nPARAMS = []\nMETA = {}\ndef compute(o, p, c):\n    pass\n",
        "shadows",
    );
    // no I/O builtins exist at all
    assert_rejected(&script("print(1)"), "not found");
    assert_rejected(&script("open(\"/etc/passwd\")"), "not found");
}

#[test]
fn compute_timeout_caps_wall_clock() {
    let c = compile(
        "t",
        &script("for i in range(20000):\n    for j in range(20000):\n        x = i"),
    )
    .unwrap();
    let t = Instant::now();
    let e = run(
        &c,
        &sample_data(10),
        &json!({}),
        Duration::from_millis(100),
        "runtime",
    )
    .err()
    .unwrap();
    assert!(e.message.contains("limit"), "{e}");
    assert!(t.elapsed() < Duration::from_secs(2));
}

// ---- persistence ----

#[test]
fn store_round_trip_on_a_temp_db() {
    let path = std::env::temp_dir().join(format!("chartgen-store-{}.db", uuid::Uuid::new_v4()));
    let store = persistence::Store::open(&path).unwrap();
    let meta = json!({"title": "A"});
    store.upsert("u1", "a", "src1", &meta, "h1").unwrap();
    store.upsert("u1", "b", "src2", &meta, "h2").unwrap();
    store.upsert("u2", "a", "other", &meta, "h3").unwrap();
    let a = store.get("u1", "a").unwrap().unwrap();
    assert_eq!((a.source.as_str(), a.meta.clone()), ("src1", meta.clone()));
    store.upsert("u1", "a", "src1b", &meta, "h4").unwrap();
    let a2 = store.get("u1", "a").unwrap().unwrap();
    assert_eq!(a2.source, "src1b");
    assert_eq!(a2.created_at, a.created_at);
    assert_eq!(store.count("u1").unwrap(), 2);
    let names: Vec<String> = store
        .list("u1")
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(names, ["a", "b"]);
    assert!(store.delete("u1", "a").unwrap());
    assert!(!store.delete("u1", "a").unwrap());
    assert_eq!(store.get("u2", "a").unwrap().unwrap().source, "other");
    store
        .save_oauth_client("c1", &["https://x/cb".into()])
        .unwrap();
    drop(store);
    // reopening runs no migration twice and keeps the data
    let store = persistence::Store::open(&path).unwrap();
    assert_eq!(store.count("u1").unwrap(), 1);
    assert_eq!(
        store.oauth_clients().unwrap(),
        [("c1".to_string(), vec!["https://x/cb".to_string()])]
    );
    let _ = std::fs::remove_file(&path);
}

// ---- refimpl parity ----

/// Fixture battery: trending up and down, low and high volatility, flat and
/// sparse (gapped) prices.
fn fixtures() -> Vec<OhlcvData> {
    let mk = |f: &dyn Fn(usize) -> f64| OhlcvData {
        bars: (0..300)
            .map(|i| {
                let c = f(i);
                let o = f(i.saturating_sub(1));
                Bar {
                    open: o,
                    high: o.max(c) + 0.3 + (i % 7) as f64 * 0.05,
                    low: o.min(c) - 0.2 - (i % 5) as f64 * 0.04,
                    close: c,
                    volume: 100.0 + (i % 13) as f64,
                    date: (1_700_000_000 + i * 3600).to_string(),
                }
            })
            .collect(),
        symbol: None,
        interval: None,
    };
    vec![
        sample_data(500),
        mk(&|i| 100.0 + i as f64 * 0.5 + (i as f64 / 5.0).sin()),
        mk(&|i| 300.0 - i as f64 * 0.7 + (i as f64 / 3.0).cos() * 2.0),
        mk(&|i| 50.0 + (i as f64 / 9.0).sin() * 0.01),
        mk(&|i| 50.0 + (i as f64 / 4.0).sin() * 25.0),
        mk(&|i| {
            if (i / 20) % 2 == 0 {
                10.0
            } else {
                10.0 + (i % 20) as f64
            }
        }),
    ]
}

fn close_enough(a: &[f64], b: &[f64], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: length");
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        let same = (x.is_nan() && y.is_nan()) || (x - y).abs() <= 1e-9;
        assert!(same, "{what}[{i}]: {x} vs {y}");
    }
}

#[test]
fn wavetrend_refimpl_matches_rust() {
    let src = refimpl("wavetrend").unwrap();
    let c = validate("wavetrend", src).unwrap();
    for params in [json!({}), json!({"ch_len": 5, "avg_len": 21})] {
        for d in fixtures() {
            let mut rust = crate::indicators::by_name("wavetrend").unwrap();
            rust.configure(&params);
            let want = rust.compute(&d);
            let got = run(&c, &d, &params, COMPUTE_TIMEOUT, "runtime").unwrap();
            assert_eq!(got.label, want.label);
            assert_eq!(got.is_overlay, want.is_overlay);
            assert_eq!(got.y_range, want.y_range);
            assert_eq!(got.lines.len(), want.lines.len());
            for (g, w) in got.lines.iter().zip(&want.lines) {
                close_enough(&g.y, &w.y, "line");
                assert_eq!((g.color, g.width, &g.label), (w.color, w.width, &w.label));
            }
            assert_eq!(got.fills.len(), want.fills.len());
            for (g, w) in got.fills.iter().zip(&want.fills) {
                close_enough(&g.y1, &w.y1, "fill.y1");
                close_enough(&g.y2, &w.y2, "fill.y2");
                assert_eq!((g.color, g.color_below), (w.color, w.color_below));
            }
            let key = |d: &crate::indicator::Dot| (d.x, d.label.clone());
            let mut gd = got.dots.clone();
            let mut wd = want.dots.clone();
            gd.sort_by_key(key);
            wd.sort_by_key(key);
            assert_eq!(gd.len(), wd.len(), "dots");
            for (g, w) in gd.iter().zip(&wd) {
                assert_eq!(
                    (g.x, &g.label, g.size, g.color),
                    (w.x, &w.label, w.size, w.color)
                );
                assert!((g.y - w.y).abs() <= 1e-9);
            }
            let hl = |p: &PanelResult| p.hlines.iter().map(|h| (h.y, h.color)).collect::<Vec<_>>();
            assert_eq!(hl(&got), hl(&want));
        }
    }
}

#[test]
fn every_refimpl_validates() {
    for (name, src) in REFIMPLS {
        validate(name, src).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            crate::indicators::by_name(name).is_some(),
            "{name} is a built-in"
        );
    }
}

#[test]
fn names() {
    assert!(valid_name("my_ind2"));
    assert!(!valid_name("2x") && !valid_name("A") && !valid_name("") && !valid_name("a-b"));
    assert!(!valid_name(&"a".repeat(49)));
}

/// The examples in website/src/content/docs/guides/scripted-indicators.md.
#[test]
fn guide_examples_validate() {
    let rsi_bands = r#"
PARAMS = [
    {"name": "length", "type": "int", "default": 14, "min": 2, "max": 200, "desc": "RSI length"},
    {"name": "smooth", "type": "int", "default": 5, "min": 1, "max": 50, "desc": "Signal EMA"},
    {"name": "ob", "type": "float", "default": 70.0, "desc": "Overbought"},
    {"name": "os", "type": "float", "default": 30.0, "desc": "Oversold"},
]
META = {"title": "RSI + signal", "overlay": False, "description": "RSI with an EMA signal and band crosses"}

def compute(ohlcv, params, chart):
    r = ta.rsi(ohlcv.close, params["length"])
    sig = ta.ema(r, params["smooth"])
    chart.line(r, title = "RSI", color = "purple")
    chart.line(sig, title = "Signal", color = "orange", width = 1)
    chart.fill(r, sig, color = "green", color_below = "red", alpha = 0.15)
    chart.dot(ta.crossover(r, params["os"]), r, color = "lime", size = 5, label = "rsi_oversold_exit")
    chart.dot(ta.crossunder(r, params["ob"]), r, color = "red", size = 5, label = "rsi_overbought_exit")
    chart.hline(params["ob"], color = "gray")
    chart.hline(params["os"], color = "gray")
    chart.set_y_range(0, 100)
"#;
    validate("rsi_bands", rsi_bands).unwrap_or_else(|e| panic!("{e}"));
    let streak = r#"PARAMS = []
META = {"title": "Streak"}

def compute(ohlcv, params, chart):
    out = []
    streak = 0
    closes = ohlcv.close.list()
    for i in range(len(closes)):
        if i > 0 and closes[i] > closes[i - 1]:
            streak += 1
        else:
            streak = 0
        out.append(streak)
    chart.bars(ta.series(out), color = "teal")
"#;
    let c = validate("streak", streak).unwrap_or_else(|e| panic!("{e}"));
    let d = sample_data(30);
    let pr = run(&c, &d, &json!({}), COMPUTE_TIMEOUT, "runtime").unwrap();
    let cl = closes(&d);
    assert_eq!(pr.bars[0].y[1], if cl[1] > cl[0] { 1.0 } else { 0.0 });
}
