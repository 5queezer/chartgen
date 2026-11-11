//! Runtime, Series operator, sandbox escape, persistence and wavetrend
//! parity tests for user scripts.

use super::runtime::{compile, compile_within, run, COMPUTE_TIMEOUT};
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
    assert!(e.message.contains("one value per bar"), "{e}");
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
    // Entry keys of the wire and the web client are not params (ADR-0028).
    for reserved in ["name", "id", "hidden", "style"] {
        let e = validate_err(&format!(
            "PARAMS = [{{\"name\": \"{reserved}\", \"type\": \"int\", \"default\": 1}}]\nMETA = {{}}\ndef compute(ohlcv, params, chart):\n    pass\n"
        ));
        assert_eq!(e.step, "presence");
        assert!(
            e.message
                .contains(&format!("cannot be called \"{reserved}\" (reserved)")),
            "{e}"
        );
    }

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
    let took = t.elapsed();
    eprintln!("[timing] rejected in {took:?}: {}", e.message);
    assert!(took < Duration::from_secs(1), "took {took:?}");
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
        // generous: a slow runner must hit the memory budget, not the deadline
        Duration::from_secs(30),
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
    store.save_oauth_client("c2", &[]).unwrap();
    store.mark_oauth_client_used("c1").unwrap();
    drop(store);
    // reopening runs no migration twice and keeps the data
    let store = persistence::Store::open(&path).unwrap();
    assert_eq!(store.count("u1").unwrap(), 1);
    let clients = store.oauth_clients().unwrap();
    assert_eq!(clients.len(), 2);
    let c1 = clients.iter().find(|c| c.client_id == "c1").unwrap();
    assert_eq!(c1.redirect_uris, ["https://x/cb"]);
    assert!(c1.last_token_at.is_some());
    // clients that never got a token are collected once old enough
    assert_eq!(store.gc_oauth_clients(86_400).unwrap(), 0);
    assert_eq!(store.gc_oauth_clients(-10).unwrap(), 1);
    let left: Vec<String> = store
        .oauth_clients()
        .unwrap()
        .into_iter()
        .map(|c| c.client_id)
        .collect();
    assert_eq!(left, ["c1"]);
    let _ = std::fs::remove_file(&path);
}

#[cfg(unix)]
#[test]
fn store_files_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("chartgen-perm-{}", uuid::Uuid::new_v4()));
    let path = dir.join("nested").join("x.db");
    persistence::Store::open(&path).unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(path.parent().unwrap()), 0o700);
    assert_eq!(mode(&path), 0o600);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn store_keeps_the_mode_of_an_existing_directory() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("chartgen-perm-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();
    let path = dir.join("x.db");
    persistence::Store::open(&path).unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode(&dir), 0o1777);
    assert_eq!(mode(&path), 0o600);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn gc_spares_clients_that_own_scripts() {
    let path = std::env::temp_dir().join(format!("chartgen-gc-{}.db", uuid::Uuid::new_v4()));
    let store = persistence::Store::open(&path).unwrap();
    store.save_oauth_client("owner", &[]).unwrap();
    store.save_oauth_client("other", &[]).unwrap();
    store
        .upsert("owner", "x", "", &serde_json::json!({}), "")
        .unwrap();
    assert_eq!(store.gc_oauth_clients(-10).unwrap(), 1);
    let left: Vec<String> = store
        .oauth_clients()
        .unwrap()
        .into_iter()
        .map(|c| c.client_id)
        .collect();
    assert_eq!(left, ["owner"]);
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
        session: None,
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

// ---- security review regressions (each must fail fast and cleanly) ----

/// Run `body` on `bars` sample bars; the error must contain `expect`. Time
/// limit cases run under the chart timeout and must arrive within 1.2 s.
/// Other (memory/size) cases get a generous timeout and no wall-time check, so
/// a slow runner cannot hit the deadline before the guard under test.
fn assert_run_fails(body: &str, bars: usize, expect: &str) {
    let timed = expect.contains("time limit");
    let timeout = if timed {
        COMPUTE_TIMEOUT
    } else {
        Duration::from_secs(30)
    };
    let c = compile("t", &script(body)).unwrap_or_else(|e| panic!("{e}"));
    let t = Instant::now();
    let e = run(&c, &sample_data(bars), &json!({}), timeout, "runtime")
        .err()
        .expect("script must fail");
    let took = t.elapsed();
    eprintln!("[timing] failed in {took:?}: {}", e.message);
    assert!(e.message.contains(expect), "expected {expect:?} in {e}");
    if timed {
        assert!(took < Duration::from_millis(1200), "took {took:?}");
    }
}

#[test]
fn bigints_never_reach_arithmetic() {
    assert_rejected(&script("x = int(1e300)"), "64-bit");
    assert_rejected(&script("x = int(\"9\" * 400)"), "64-bit");
    assert_rejected(&script("x = 123456789012345678901234567890"), "64-bit");
    assert_rejected(
        &script("x = 2147483647\nfor i in range(16):\n    x = x * x"),
        "overflow",
    );
    // -(-2**63) is the one way to make a bigint; no operator accepts it.
    assert_rejected(
        &script("x = -9223372036854775807 - 1\ny = -x\nz = y // 3"),
        "64-bit",
    );
    // 64-bit ints still work, and float conversion truncates
    let pr = exec(
        "chart.hline(int(2.9) + int(\"40\") + int(\"ff\", 16) + 9000000000 // 3)",
        5,
    )
    .unwrap();
    assert_eq!(pr.hlines[0].y, (2 + 40 + 255 + 3_000_000_000i64) as f64);
}

/// A limit no load stretches a passing run to, for tests of what bounds the
/// work rather than of how fast this machine does it.
const PATIENT: Duration = Duration::from_secs(30);

#[test]
fn ta_work_is_bounded_by_the_bar_count() {
    let patient = |body: &str, bars: usize| {
        let c = compile_within("t", &script(body), PATIENT)?;
        run(&c, &sample_data(bars), &json!({}), PATIENT, "runtime")
    };
    let rejected = |body: &str, bars: usize, expect: &str| {
        let e = patient(body, bars).err().expect("script must be rejected");
        let msg = e.message.to_lowercase();
        assert!(msg.contains(expect), "expected {expect:?} in {e}");
    };
    // What bounds a call: a series is exactly one value per bar, so a list of
    // 1M values (the 13.9 s report) never reaches ta.*, and a window is capped.
    rejected("s = ta.series([1.0] * 20000)", 50, "one value per bar");
    rejected("s = ta.series(list(range(1000000)))", 50, "range");
    rejected("x = ta.highest(ohlcv.close, 10001)", 50, "length must be");
    // The worst single call (window = half the bars) returns, and its result is
    // checked against the deadline, so one that outlasts PATIENT fails here.
    let t = Instant::now();
    patient("x = ta.highest(ohlcv.close, 3000)", 6000).unwrap_or_else(|e| panic!("{e}"));
    eprintln!(
        "[timing] worst-case ta.highest on 6000 bars: {:?}",
        t.elapsed()
    );
    // The per-call deadline check stops a loop of them, whatever the load.
    for body in [
        "for i in range(20000):\n    x = ta.stdev(ohlcv.close, 3000)",
        "for i in range(20000):\n    x = ta.pivot_high(ohlcv.close, 1500, 1500)",
    ] {
        let c = compile_within("t", &script(body), PATIENT).unwrap_or_else(|e| panic!("{e}"));
        let t = Instant::now();
        let e = run(
            &c,
            &sample_data(6000),
            &json!({}),
            COMPUTE_TIMEOUT,
            "runtime",
        )
        .err()
        .expect("the loop must be stopped");
        assert!(e.message.contains("time limit"), "{e}");
        assert!(t.elapsed() < PATIENT, "took {:?}", t.elapsed());
    }
}

#[test]
fn list_and_dict_growth_is_capped() {
    // Not `assert_rejected`: under load, building these can outlast its 200 ms
    // dry run, and the deadline would fire before the guard under test.
    assert_run_fails("a = [0] * 20000\na.append(1)", 50, "list too large");
    assert_run_fails("a = [0] * 20000\na.insert(0, 1)", 50, "list too large");
    assert_run_fails(
        "d = {i: 0 for i in range(20000)}\nd[-1] = 0",
        50,
        "dict too large",
    );
    assert_rejected(&script("d = {}\nd.update({1: 2})"), "not available");
    assert_rejected(&script("d = {}\nd.setdefault(1, 2)"), "not available");
    // Memory outside the Starlark heap (series data here; dict tables
    // likewise) is counted on every native call.
    assert_run_fails(
        "keep = []\nfor i in range(20000):\n    keep.append(ohlcv.close + i)",
        6000,
        "memory limit",
    );
    // the rewrites keep normal semantics
    let pr = exec(
        "a = [1]\na.append(len(a))\na.insert(0, 7)\nd = {}\nd[\"k\"] = 2\nd[\"k\"] = d[\"k\"] + 1\na[0] = d[\"k\"]\nchart.hline(a[0] * 100 + a[1] * 10 + a[2])",
        5,
    )
    .unwrap();
    assert_eq!(pr.hlines[0].y, 311.0);
}

#[test]
fn large_top_level_modules_are_rejected() {
    // Under load, building this can outlast the 200 ms production gives top-level
    // code, and that limit would fire before the size guard under test.
    let source = format!(
        "BIG = [[{}] * 20000 for _ in range(10)]\n{}",
        "0.5",
        script("pass")
    );
    let e = compile_within("t", &source, PATIENT)
        .err()
        .expect("module must be rejected");
    assert!(e.message.contains("top-level values"), "{e}");
}

#[test]
fn operator_rewrite_edge_cases() {
    assert_rejected(&script("x = [0]\nx[0] += 1"), "plain name");
    assert_rejected(&script("x = [0, 0]\nx[0], x[1] = 1, 2"), "one at a time");
    for cond in [
        "if ohlcv.close > ohlcv.open:\n    pass",
        "x = 1 if ohlcv.close[1] < 3 else 2",
        "x = not ta.crossover(ohlcv.close, 3)",
        "x = ohlcv.close > 1 and True",
        "x = [1 for i in range(3) if ta.rsi(ohlcv.close, 3) > 50]",
    ] {
        assert_rejected(&script(cond), "not a bool");
    }
    // // | & ^ >> go through the guards, on series and on plain values
    let c = closes(&sample_data(50));
    let y = line("chart.line(ohlcv.close // 2)");
    assert_eq!(y[5], (c[5] / 2.0).floor());
    let y = line("up = ohlcv.close > ohlcv.open\nchart.line(up | True)");
    assert!(y.iter().all(|v| *v == 1.0));
    let y = line("up = ohlcv.close > ohlcv.open\nchart.line(False & up)");
    assert!(y.iter().all(|v| *v == 0.0));
    let y = line("up = ohlcv.close > ohlcv.open\nchart.line(up ^ up)");
    assert!(y.iter().all(|v| *v == 0.0));
    let pr = exec(
        "n = (12 >> 2) | 8\nn ^= 1\nn //= 2\nd = {1: 2} | {3: 4}\nchart.hline(n + len(d))",
        5,
    )
    .unwrap();
    assert_eq!(pr.hlines[0].y, ((((12 >> 2) | 8) ^ 1) / 2 + 2) as f64);
}

#[test]
fn concurrency_and_chart_budget() {
    use crate::indicator::Indicator;
    let far = || Instant::now() + Duration::from_millis(50);
    // one evaluation per user, four in total
    let a = acquire("conc-a", far(), "runtime").unwrap();
    let busy = acquire("conc-a", far(), "runtime").err().unwrap();
    assert!(busy.message.contains("busy"), "{busy}");
    let others: Vec<_> = (0..3)
        .map(|i| acquire(&format!("conc-{i}"), far(), "runtime").unwrap())
        .collect();
    assert!(acquire("conc-z", far(), "runtime").is_err(), "fifth slot");
    drop(others);
    drop(a);
    assert!(acquire("conc-a", far(), "runtime").is_ok());

    // At most 3 scripts per chart, sharing one wall-clock budget.
    let slow = script("for i in range(20000):\n    for j in range(20000):\n        x = i");
    let inds: Vec<ScriptIndicator> = (0..4)
        .map(|i| ScriptIndicator::new("conc-chart", &format!("s{i}"), &slow))
        .collect();
    let d = sample_data(20);
    let t = Instant::now();
    let results: Vec<_> = with_chart_budget(|| inds.iter().map(|i| i.try_compute(&d)).collect());
    let took = t.elapsed();
    eprintln!("[timing] 4 slow scripts in one chart: {took:?}");
    assert!(took < Duration::from_millis(1300), "took {took:?}");
    let msgs: Vec<String> = results
        .into_iter()
        .map(|r| r.err().unwrap().message)
        .collect();
    assert!(msgs[0].contains("time limit"), "{msgs:?}");
    assert!(
        msgs[1..3].iter().all(|m| m.contains("time limit")),
        "{msgs:?}"
    );
    assert!(msgs[3].contains("at most 3"), "{msgs:?}");
}

/// ADR-0044: users, sessions and approvals survive reopening, and migrating an
/// existing file keeps its clients.
#[test]
fn sign_in_rows_survive_reopening() {
    let dir = std::env::temp_dir().join(format!("chartgen-signin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("s.db");
    let store = persistence::Store::open(&path).unwrap();
    store.save_oauth_client("c1", &[]).unwrap();
    store.set_oauth_client_name("c1", "Claude").unwrap();
    store.save_user("github:1", "octo").unwrap();
    store.save_user("github:1", "renamed").unwrap();
    let session = |hash: &str, expires_at| persistence::StoredSession {
        token_hash: hash.into(),
        user_id: "github:1".into(),
        created_at: 1,
        expires_at,
    };
    let now = chrono::Utc::now().timestamp();
    store.save_session(&session("live", now + 100)).unwrap();
    store.save_session(&session("gone", now - 1)).unwrap();
    store
        .save_session(&session("logged-out", now + 100))
        .unwrap();
    store.delete_session("logged-out").unwrap();
    store.save_grant("github:1", "c1").unwrap();
    store.save_grant("github:1", "evicted").unwrap();
    store.save_grant("github:2", "c1").unwrap();
    store.delete_grant("github:2", "c1").unwrap();
    drop(store);

    let store = persistence::Store::open(&path).unwrap();
    assert_eq!(
        store.users().unwrap(),
        [persistence::StoredUser {
            id: "github:1".into(),
            name: "renamed".into()
        }]
    );
    assert_eq!(
        store.live_sessions(now).unwrap(),
        [session("live", now + 100)]
    );
    assert_eq!(
        store.grants().unwrap(),
        [("github:1".to_string(), "c1".to_string())],
        "a grant of a client no longer registered goes"
    );
    assert_eq!(
        store.oauth_clients().unwrap()[0].client_name.as_deref(),
        Some("Claude")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// ADR-0044: what clients own goes to the account named, from every client, and
/// nothing is overwritten or lost. Only a client id's shape counts as a client.
#[test]
fn collecting_waiting_scripts_takes_every_client_without_loss() {
    const CLIENT_1: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1";
    const CLIENT_2: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa2";
    let dir = std::env::temp_dir().join(format!("chartgen-claim-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = persistence::Store::open(&dir.join("s.db")).unwrap();
    let meta = serde_json::json!({});
    for name in ["a", "b", "c"] {
        store
            .upsert(CLIENT_1, name, "client's", &meta, "h")
            .unwrap();
    }
    store
        .upsert(CLIENT_2, "d", "other client's", &meta, "h")
        .unwrap();
    store.upsert("github:1", "b", "user's", &meta, "h").unwrap();
    store
        .upsert("google:2", "own", "someone else's", &meta, "h")
        .unwrap();
    store
        .upsert("device:9", "kept", "not a client's", &meta, "h")
        .unwrap();
    let waiting = store.waiting_rows(Some("github:1")).unwrap();
    assert_eq!((waiting.scripts, waiting.name_collisions), (4, 1));
    assert_eq!(store.waiting_rows(None).unwrap().name_collisions, 0);

    assert_eq!(store.claim_waiting_scripts("github:1").unwrap(), 3);

    assert_eq!(
        store.get("github:1", "a").unwrap().unwrap().source,
        "client's"
    );
    assert_eq!(
        store.get("github:1", "d").unwrap().unwrap().source,
        "other client's"
    );
    assert_eq!(
        store.get("github:1", "b").unwrap().unwrap().source,
        "user's",
        "a name the user has is not overwritten"
    );
    assert_eq!(
        store.get(CLIENT_1, "b").unwrap().unwrap().source,
        "client's",
        "and the client's stays where it was"
    );
    assert_eq!(
        store.get("google:2", "own").unwrap().unwrap().source,
        "someone else's",
        "another account's rows are not a client's"
    );
    assert_eq!(
        store.get("device:9", "kept").unwrap().unwrap().source,
        "not a client's",
        "nor is any other kind of owner"
    );
    let left = store.waiting_rows(Some("github:1")).unwrap();
    assert_eq!((left.scripts, left.name_collisions), (1, 1));
    assert_eq!(store.claim_waiting_scripts("github:1").unwrap(), 0, "once");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_client_id_is_the_shape_register_gives() {
    assert!(persistence::is_client_id(&uuid::Uuid::new_v4().to_string()));
    for other in [
        "github:1",
        "google:abc",
        "device:9",
        "",
        "aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaa1",
        "not-a-uuid-but-exactly-36-chars-long",
    ] {
        assert!(!persistence::is_client_id(other), "{other}");
    }
}

/// The schema as each deployed version left it: migrations only append, so a file
/// from any of these must arrive with every table and keep its rows. No version with
/// sign-in in slot 3 ever shipped, so that order is not supported (ADR-0044).
const SCHEMA_V2: &str = "
    CREATE TABLE user_indicators (
        user_id TEXT NOT NULL, name TEXT NOT NULL, source TEXT NOT NULL,
        meta_json TEXT NOT NULL, source_hash TEXT NOT NULL,
        created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
        PRIMARY KEY (user_id, name)
    );
    CREATE TABLE oauth_clients (
        client_id TEXT PRIMARY KEY, redirect_uris TEXT NOT NULL,
        created_at INTEGER NOT NULL, last_token_at INTEGER
    );
    INSERT INTO oauth_clients VALUES ('old-client', '[\"https://x/cb\"]', 1, 2);
    INSERT INTO user_indicators VALUES ('old-client', 's', 'src', '{}', 'h', 1, 1);
    PRAGMA user_version = 2;
";
const SCHEMA_V4_PUSH: &str = "
    ALTER TABLE server_alert_events ADD COLUMN device TEXT;
    CREATE TABLE push_devices (
        id TEXT PRIMARY KEY, owner TEXT NOT NULL, endpoint TEXT NOT NULL,
        p256dh TEXT NOT NULL, auth TEXT NOT NULL, created_at INTEGER NOT NULL,
        UNIQUE (owner, endpoint)
    );
    CREATE TABLE server_settings (name TEXT PRIMARY KEY, value TEXT NOT NULL);
    INSERT INTO push_devices VALUES
        ('d1', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1', 'https://x/1', 'k', 'a', 1);
    PRAGMA user_version = 4;
";
const SCHEMA_V3_ALERTS: &str = "
    CREATE TABLE server_alerts (
        id TEXT PRIMARY KEY, owner TEXT NOT NULL, record_json TEXT NOT NULL
    );
    CREATE INDEX server_alerts_owner ON server_alerts (owner);
    CREATE TABLE server_alert_events (
        id TEXT PRIMARY KEY, alert_id TEXT NOT NULL, body TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    INSERT INTO server_alerts VALUES ('a1', 'old-client', '{}');
    PRAGMA user_version = 3;
";

fn schema_is_complete(store: &persistence::Store) {
    store.save_user("github:1", "octo").unwrap();
    store.mark_user_used("github:1").unwrap();
    assert_eq!(store.user_activity().unwrap().len(), 1);
    store.save_grant("github:1", "old-client").unwrap();
    assert!(store.live_sessions(0).unwrap().is_empty());
    assert!(store.refresh_tokens(0).unwrap().is_empty());
    store.save_server_alert("a2", "github:1", "{}").unwrap();
    store
        .workspace_sync(
            "github:1",
            0,
            &[ws_write("chartgen.layouts", Some("{}"), 0)],
            &WS_LIMITS,
        )
        .unwrap()
        .parts();
    assert_eq!(store.waiting_rows(None).unwrap().scripts, 0);
    assert!(store.server_alert_events().unwrap().is_empty());
    assert!(store.server_alert_owners().unwrap().contains("github:1"));
}

fn upgraded(name: &str, setup: &[&str]) -> persistence::Store {
    let dir = std::env::temp_dir().join(format!("chartgen-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    for sql in setup {
        conn.execute_batch(sql).unwrap();
    }
    drop(conn);
    let store = persistence::Store::open(&path).unwrap();
    // Reopening migrates nothing twice.
    drop(store);
    persistence::Store::open(&path).unwrap()
}

#[test]
fn an_empty_file_gets_every_table() {
    schema_is_complete(&upgraded("upgrade-empty", &[]));
}

#[test]
fn a_file_from_before_alerts_upgrades_and_keeps_its_rows() {
    let store = upgraded("upgrade-v2", &[SCHEMA_V2]);
    schema_is_complete(&store);
    let client = &store.oauth_clients().unwrap()[0];
    assert_eq!(client.client_id, "old-client");
    assert_eq!(client.client_name, None);
    assert_eq!(store.get("old-client", "s").unwrap().unwrap().source, "src");
}

#[test]
fn a_file_with_alerts_upgrades_and_keeps_them() {
    let store = upgraded("upgrade-v3", &[SCHEMA_V2, SCHEMA_V3_ALERTS]);
    schema_is_complete(&store);
    assert!(store.server_alert_owners().unwrap().contains("old-client"));
    assert_eq!(store.oauth_clients().unwrap().len(), 1);
}

#[test]
fn a_file_with_push_devices_upgrades_and_keeps_them() {
    let store = upgraded("upgrade-v4", &[SCHEMA_V2, SCHEMA_V3_ALERTS, SCHEMA_V4_PUSH]);
    schema_is_complete(&store);
    let devices = store.push_devices().unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].endpoint, "https://x/1");
    assert_eq!(store.waiting_rows(None).unwrap().devices, 1);
    assert!(store.server_alert_owners().unwrap().contains("old-client"));
    // Slot 5 came on top of them: what it adds is there too.
    store.set_oauth_client_name("old-client", "Claude").unwrap();
    assert_eq!(
        store.oauth_clients().unwrap()[0].client_name.as_deref(),
        Some("Claude")
    );
}

// --- Refresh tokens (ADR-0058) ---

fn refresh_row(hash: &str, family: &str, seq: i64) -> persistence::StoredRefreshToken {
    persistence::StoredRefreshToken {
        token_hash: hash.to_string(),
        family: family.to_string(),
        seq,
        client_id: "c1".to_string(),
        user_id: (family == "users").then(|| "github:1".to_string()),
        created_at: 1000 + seq,
        spent_at: None,
    }
}

/// A deploy is a new build on the file the last one left: one from before refresh
/// tokens gets the table, keeps its clients, users and approvals, and stores a token.
#[test]
fn a_file_from_before_refresh_tokens_upgrades_and_stores_them() {
    let before = persistence::MIGRATIONS
        .iter()
        .position(|sql| sql.contains("CREATE TABLE refresh_tokens"))
        .unwrap();
    let mut setup: Vec<String> = persistence::MIGRATIONS[..before]
        .iter()
        .map(|sql| sql.to_string())
        .collect();
    setup.push(format!(
        "INSERT INTO oauth_clients (client_id, redirect_uris, created_at, last_token_at)
             VALUES ('c1', '[\"https://x/cb\"]', 1, 2);
         INSERT INTO users VALUES ('github:1', 'octo', 1, 1, 1);
         INSERT INTO client_grants VALUES ('github:1', 'c1', 1);
         PRAGMA user_version = {before};"
    ));
    let setup: Vec<&str> = setup.iter().map(String::as_str).collect();
    let store = upgraded("upgrade-refresh", &setup);
    assert_eq!(store.oauth_clients().unwrap()[0].client_id, "c1");
    assert_eq!(store.users().unwrap()[0].id, "github:1");
    assert_eq!(store.grants().unwrap().len(), 1);
    assert!(store.refresh_tokens(0).unwrap().is_empty());
    let row = refresh_row("h0", "users", 0);
    store.save_refresh_token(&row, &[], 0).unwrap();
    assert_eq!(store.refresh_tokens(0).unwrap(), [row]);
}

#[test]
fn a_rotation_is_stored_whole_or_not_at_all() {
    let store = upgraded("refresh-rotation", &[]);
    store
        .save_refresh_token(&refresh_row("h0", "f", 0), &[], 0)
        .unwrap();
    let next = refresh_row("h1", "f", 1);
    store
        .save_refresh_token(&next, &["h0".to_string()], 0)
        .unwrap();
    let rows = store.refresh_tokens(0).unwrap();
    let spent = rows.iter().find(|r| r.token_hash == "h0").unwrap();
    assert_eq!(
        spent.spent_at,
        Some(next.created_at),
        "kept, to recognise reuse"
    );
    assert_eq!(rows.iter().filter(|r| r.spent_at.is_none()).count(), 1);

    // The successor cannot be stored: the token it would replace stays unspent.
    let clash = refresh_row("h0", "f", 2);
    assert!(store
        .save_refresh_token(&clash, &["h1".to_string()], 0)
        .is_err());
    let rows = store.refresh_tokens(0).unwrap();
    assert_eq!(rows.len(), 2);
    let live = rows.iter().find(|r| r.spent_at.is_none()).unwrap();
    assert_eq!(live.token_hash, "h1");

    // Spent tokens below the floor are dropped, the unspent one never.
    store
        .save_refresh_token(&refresh_row("h2", "f", 2), &["h1".to_string()], 2)
        .unwrap();
    let left: Vec<String> = store
        .refresh_tokens(0)
        .unwrap()
        .into_iter()
        .map(|r| r.token_hash)
        .collect();
    assert_eq!(left, ["h2"]);
}

#[test]
fn refresh_families_end_together_and_with_their_client() {
    let store = upgraded("refresh-families", &[]);
    store
        .save_oauth_client("c1", &["https://x/cb".to_string()])
        .unwrap();
    for (hash, family, seq) in [
        ("a0", "a", 0),
        ("a1", "a", 1),
        ("b0", "b", 0),
        ("u0", "users", 0),
    ] {
        store
            .save_refresh_token(&refresh_row(hash, family, seq), &[], 0)
            .unwrap();
    }
    let families = |store: &persistence::Store, cutoff: i64| -> Vec<String> {
        let mut f: Vec<String> = store
            .refresh_tokens(cutoff)
            .unwrap()
            .into_iter()
            .map(|r| r.family)
            .collect();
        f.dedup();
        f
    };
    store.delete_refresh_families(&["b".to_string()]).unwrap();
    assert_eq!(families(&store, 0), ["a", "users"]);
    // A family is as old as its newest token: `a` was exchanged at 1001.
    assert_eq!(families(&store, 1001), ["a"]);
    store.delete_oauth_client("c1").unwrap();
    assert!(store.refresh_tokens(0).unwrap().is_empty());
    assert!(store.oauth_clients().unwrap().is_empty());
}

/// What counts as a user's activity for their server alerts: a login or a token.
#[test]
fn user_activity_is_the_later_of_login_and_token() {
    let store = upgraded("activity", &[]);
    store.save_user("github:1", "a").unwrap();
    let login = store.user_activity().unwrap()[0].1;
    assert!(login > 0);
    store.mark_user_used("github:1").unwrap();
    assert!(store.user_activity().unwrap()[0].1 >= login);
}

// --- Workspace documents (ADR-0050) ---

impl persistence::WorkspaceSynced {
    fn parts(
        self,
    ) -> (
        Vec<persistence::WorkspaceOutcome>,
        Vec<persistence::WorkspaceDoc>,
        i64,
    ) {
        (self.outcomes, self.changes, self.cursor)
    }
}

const WS_LIMITS: persistence::WorkspaceLimits = persistence::WorkspaceLimits {
    doc_bytes: 100,
    user_bytes: 250,
    user_docs: 4,
    user_tombstones: 3,
    total_bytes: 400,
};

fn ws_write(key: &str, value: Option<&str>, base_version: i64) -> persistence::WorkspaceWrite {
    persistence::WorkspaceWrite {
        key: key.to_string(),
        value: value.map(str::to_string),
        base_version,
    }
}

fn ws_store(name: &str) -> persistence::Store {
    let dir = std::env::temp_dir().join(format!("chartgen-ws-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    persistence::Store::open(&dir.join("s.db")).unwrap()
}

#[test]
fn a_stale_write_gets_the_current_document_instead_of_replacing_it() {
    use persistence::WorkspaceOutcome::{Conflict, Stored};
    let store = ws_store("conflict");
    let (o, _, cursor) = store
        .workspace_sync("u", 0, &[ws_write("k", Some("one"), 0)], &WS_LIMITS)
        .unwrap()
        .parts();
    assert_eq!(o, [Stored { version: 1 }]);
    assert_eq!(cursor, 1);
    let (o, _, _) = store
        .workspace_sync("u", 1, &[ws_write("k", Some("two"), 1)], &WS_LIMITS)
        .unwrap()
        .parts();
    assert_eq!(o, [Stored { version: 2 }]);

    let (o, _, _) = store
        .workspace_sync("u", 2, &[ws_write("k", Some("stale"), 1)], &WS_LIMITS)
        .unwrap()
        .parts();
    assert_eq!(
        o,
        [Conflict {
            version: 2,
            value: Some("two".to_string())
        }]
    );
    // What a lost answer's retry looks like: already so, whatever version it names.
    let (o, _, cursor) = store
        .workspace_sync("u", 2, &[ws_write("k", Some("two"), 1)], &WS_LIMITS)
        .unwrap()
        .parts();
    assert_eq!(o, [Stored { version: 2 }]);
    assert_eq!(cursor, 2, "it wrote nothing");
    // A new document has to say so: another device may have made it first.
    let (o, _, _) = store
        .workspace_sync("u", 2, &[ws_write("k", Some("new"), 0)], &WS_LIMITS)
        .unwrap()
        .parts();
    assert!(matches!(o[0], Conflict { version: 2, .. }), "{o:?}");
}

#[test]
fn changes_are_what_moved_since_and_never_anothers() {
    let store = ws_store("changes");
    store
        .workspace_sync("ann", 0, &[ws_write("a", Some("1"), 0)], &WS_LIMITS)
        .unwrap()
        .parts();
    store
        .workspace_sync("ann", 1, &[ws_write("b", Some("2"), 0)], &WS_LIMITS)
        .unwrap()
        .parts();
    store
        .workspace_sync("bob", 0, &[ws_write("a", Some("bobs"), 0)], &WS_LIMITS)
        .unwrap()
        .parts();

    let (_, changes, cursor) = store
        .workspace_sync("ann", 0, &[], &WS_LIMITS)
        .unwrap()
        .parts();
    let got: Vec<_> = changes
        .iter()
        .map(|d| (d.key.as_str(), d.value.as_deref()))
        .collect();
    assert_eq!(got, [("a", Some("1")), ("b", Some("2"))]);
    assert_eq!(cursor, 2);
    let (_, changes, _) = store
        .workspace_sync("ann", 1, &[], &WS_LIMITS)
        .unwrap()
        .parts();
    assert_eq!(changes.len(), 1, "only what moved since");
    assert_eq!(changes[0].key, "b");

    // A write through one account cannot name another's, and its version is its own.
    let (_, changes, _) = store
        .workspace_sync("bob", 0, &[], &WS_LIMITS)
        .unwrap()
        .parts();
    assert_eq!(changes[0].value.as_deref(), Some("bobs"));
    assert_eq!(changes.len(), 1);

    // What this call just stored is not news to it; what another device did is.
    let (_, changes, _) = store
        .workspace_sync("ann", 0, &[ws_write("c", Some("3"), 0)], &WS_LIMITS)
        .unwrap()
        .parts();
    let keys: Vec<_> = changes.iter().map(|d| d.key.as_str()).collect();
    assert_eq!(keys, ["a", "b"]);
}

#[test]
fn a_deletion_reaches_other_devices_and_frees_its_bytes() {
    let store = ws_store("tombstone");
    store
        .workspace_sync(
            "u",
            0,
            &[ws_write("k", Some(&"x".repeat(100)), 0)],
            &WS_LIMITS,
        )
        .unwrap()
        .parts();
    let (o, _, _) = store
        .workspace_sync("u", 1, &[ws_write("k", None, 1)], &WS_LIMITS)
        .unwrap()
        .parts();
    assert_eq!(o, [persistence::WorkspaceOutcome::Stored { version: 2 }]);
    let (_, changes, _) = store
        .workspace_sync("u", 1, &[], &WS_LIMITS)
        .unwrap()
        .parts();
    assert_eq!(changes[0].value, None, "other devices learn of it");
    // Its bytes are back: two more of the largest fit only if they are.
    for (i, base) in [("a", 0), ("b", 0)] {
        let (o, _, _) = store
            .workspace_sync(
                "u",
                0,
                &[ws_write(i, Some(&"x".repeat(100)), base)],
                &WS_LIMITS,
            )
            .unwrap()
            .parts();
        assert!(
            matches!(o[0], persistence::WorkspaceOutcome::Stored { .. }),
            "{o:?}"
        );
    }
}

#[test]
fn limits_hold_per_document_per_user_and_in_total() {
    use persistence::WorkspaceOutcome::{OverQuota, TooLarge};
    let store = ws_store("limits");
    let big = "x".repeat(100);
    let (o, _, _) = store
        .workspace_sync(
            "u",
            0,
            &[ws_write("k", Some(&"x".repeat(101)), 0)],
            &WS_LIMITS,
        )
        .unwrap()
        .parts();
    assert_eq!(o, [TooLarge]);

    // Per user, in bytes: 250 fit two of these and a part.
    for key in ["a", "b"] {
        store
            .workspace_sync("u", 0, &[ws_write(key, Some(&big), 0)], &WS_LIMITS)
            .unwrap()
            .parts();
    }
    let (o, _, _) = store
        .workspace_sync("u", 0, &[ws_write("c", Some(&big), 0)], &WS_LIMITS)
        .unwrap()
        .parts();
    assert_eq!(o, [OverQuota], "over the user's bytes");
    // Shrinking is always allowed, and what was there stays.
    let (o, _, _) = store
        .workspace_sync("u", 2, &[ws_write("a", Some("s"), 1)], &WS_LIMITS)
        .unwrap()
        .parts();
    assert!(matches!(o[0], persistence::WorkspaceOutcome::Stored { .. }));

    // Per user, in documents.
    let store = ws_store("limits-docs");
    for key in ["a", "b", "c", "d"] {
        store
            .workspace_sync("u", 0, &[ws_write(key, Some("v"), 0)], &WS_LIMITS)
            .unwrap()
            .parts();
    }
    let (o, _, _) = store
        .workspace_sync("u", 0, &[ws_write("e", Some("v"), 0)], &WS_LIMITS)
        .unwrap()
        .parts();
    assert_eq!(o, [OverQuota], "over the user's documents");

    // In total: another account is refused once the file is full, and keeps nothing out.
    let store = ws_store("limits-total");
    for (user, key) in [("a", "1"), ("a", "2"), ("b", "1"), ("b", "2")] {
        store
            .workspace_sync(user, 0, &[ws_write(key, Some(&big), 0)], &WS_LIMITS)
            .unwrap()
            .parts();
    }
    let (o, _, _) = store
        .workspace_sync("c", 0, &[ws_write("1", Some("v"), 0)], &WS_LIMITS)
        .unwrap()
        .parts();
    assert_eq!(o, [OverQuota], "over the total");
}

#[test]
fn the_last_tenth_of_the_total_is_for_accounts_already_here() {
    use persistence::WorkspaceOutcome::{OverQuota, Stored};
    let store = ws_store("limits-reserve");
    let big = "x".repeat(100);
    for (user, key) in [("a", "1"), ("b", "1"), ("c", "1")] {
        let (o, _, _) = store
            .workspace_sync(user, 0, &[ws_write(key, Some(&big), 0)], &WS_LIMITS)
            .unwrap()
            .parts();
        assert!(matches!(o[0], Stored { .. }), "{user}");
    }
    // 300 of 400: a hundred more passes 360, which is all a new account may fill.
    let (o, _, _) = store
        .workspace_sync("new", 0, &[ws_write("1", Some(&big), 0)], &WS_LIMITS)
        .unwrap()
        .parts();
    assert_eq!(o, [OverQuota], "a new account stops at nine tenths");
    let (o, _, _) = store
        .workspace_sync("a", 0, &[ws_write("2", Some(&big), 0)], &WS_LIMITS)
        .unwrap()
        .parts();
    assert!(matches!(o[0], Stored { .. }), "one already here goes on");
    let (o, _, _) = store
        .workspace_sync("b", 0, &[ws_write("2", Some("over"), 0)], &WS_LIMITS)
        .unwrap()
        .parts();
    assert_eq!(o, [OverQuota], "and it too stops at the total");
}

#[test]
fn deleted_documents_leave_room_and_are_not_held_against_anyone() {
    use persistence::WorkspaceOutcome::{OverQuota, Stored};
    let store = ws_store("tombstones");
    let put = |key: &str, value: Option<&str>, base: i64| {
        store
            .workspace_sync("u", 0, &[ws_write(key, value, base)], &WS_LIMITS)
            .unwrap()
            .parts()
            .0
            .remove(0)
    };
    let mut versions = Vec::new();
    for key in ["a", "b", "c", "d"] {
        let Stored { version } = put(key, Some("v"), 0) else {
            panic!("{key}");
        };
        versions.push(version);
    }
    assert_eq!(put("e", Some("v"), 0), OverQuota, "the limit holds");
    let Stored { version: gone } = put("a", None, versions[0]) else {
        panic!("deleted");
    };
    // The deletion made room, and bringing the same key back counts it once again.
    let Stored { version: back } = put("a", Some("again"), gone) else {
        panic!("{:?}", put("a", Some("again"), gone));
    };
    assert_eq!(put("e", Some("v"), 0), OverQuota, "four again");
    assert!(back > gone);
    put("a", None, back);
    assert!(matches!(put("e", Some("v"), 0), Stored { .. }));

    // A device with no cursor has nothing to delete. Any other is told.
    let (_, fresh, _) = store
        .workspace_sync("u", 0, &[], &WS_LIMITS)
        .unwrap()
        .parts();
    assert!(fresh.iter().all(|d| d.value.is_some()), "{fresh:?}");
    let (_, known, _) = store
        .workspace_sync("u", 1, &[], &WS_LIMITS)
        .unwrap()
        .parts();
    assert!(
        known.iter().any(|d| d.key == "a" && d.value.is_none()),
        "{known:?}"
    );
}

#[test]
fn tombstones_are_bounded_per_user_keeping_the_newest() {
    let store = ws_store("tombstone-bound");
    for i in 0..6 {
        let key = format!("k{i}");
        let (o, _, _) = store
            .workspace_sync("u", 0, &[ws_write(&key, Some("v"), 0)], &WS_LIMITS)
            .unwrap()
            .parts();
        let persistence::WorkspaceOutcome::Stored { version } = o[0] else {
            panic!("{o:?}");
        };
        store
            .workspace_sync("u", 0, &[ws_write(&key, None, version)], &WS_LIMITS)
            .unwrap();
    }
    let (_, rows, _) = store
        .workspace_sync("u", 1, &[], &WS_LIMITS)
        .unwrap()
        .parts();
    let keys: Vec<_> = rows.iter().map(|d| d.key.as_str()).collect();
    assert_eq!(keys, ["k3", "k4", "k5"]);
}

#[test]
fn a_restored_file_is_said_even_when_this_request_writes_past_the_gap() {
    let store = ws_store("restored");
    let sync = |since, writes: &[persistence::WorkspaceWrite]| {
        store
            .workspace_sync("u", since, writes, &WS_LIMITS)
            .unwrap()
    };
    assert!(!sync(0, &[ws_write("k", Some("v"), 0)]).restored);
    assert!(!sync(1, &[]).restored);
    let writes = [ws_write("a", Some("1"), 0), ws_write("b", Some("2"), 0)];
    let synced = sync(2, &writes);
    assert!(synced.restored, "seq 1 is behind a cursor of 2");
    assert!(synced.cursor >= 2, "the cursor alone would not have told");
    // Deletions included, or a device would send back what was deleted here.
    let v = sync(0, &[ws_write("k", None, 1)]).cursor;
    let told = sync(v + 5, &[]);
    assert!(told.restored);
    assert!(told
        .changes
        .iter()
        .any(|d| d.key == "k" && d.value.is_none()));
}

#[test]
fn a_cursor_from_a_restored_file_starts_over() {
    let store = ws_store("cursor");
    store
        .workspace_sync("u", 0, &[ws_write("k", Some("v"), 0)], &WS_LIMITS)
        .unwrap()
        .parts();
    let (_, changes, _) = store
        .workspace_sync("u", 99, &[], &WS_LIMITS)
        .unwrap()
        .parts();
    assert_eq!(changes.len(), 1);
}
