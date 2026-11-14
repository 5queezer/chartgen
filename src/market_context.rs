//! A symbol's market at a glance (ADR-0072): its price, the trend on 4h and 1d, and for a
//! Binance symbol the funding rate, the long/short split of accounts and the Fear & Greed
//! index. A wake-up carries it, and `get_market_context` answers it, so an agent that is
//! woken does not spend its first calls reading the same few numbers.
//!
//! Made from chartgen's own reads and indicators and held for a minute, so a wake-up
//! rarely waits for it. Whether a macro blackout is on is not here: chartgen has no
//! macro calendar.

use std::collections::HashMap;
use std::sync::{Condvar, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chartgen::data::OhlcvData;
use chartgen::indicator::ema;
use chartgen::indicators;
use serde_json::{json, Map, Value};

use crate::fetch::{self, DataSource};
use crate::mcp::{tool_err, tool_ok};

/// How long a symbol's context is served before it is read again.
const FRESH: Duration = Duration::from_secs(60);
/// How long a read in flight is waited for, and holds others off. Longer than a read
/// takes: three kline requests and two futures series of at most 20 s each.
const READING: Duration = Duration::from_secs(90);
/// How long the Fear & Greed index is kept: it moves once a day.
const FEAR_GREED_FRESH: Duration = Duration::from_secs(600);
/// Bars read of each interval: enough for the slow EMA to settle.
const BARS: usize = 150;
/// The EMAs a trend is read from.
const FAST: usize = 20;
const SLOW: usize = 50;
/// How long ago the newest 4h bar of a Binance symbol, which trades around the clock,
/// may have opened: the forming bar opened within 4 h, and one bar more is allowed.
/// Older, nothing has traded for a whole bar, as on a delisted symbol whose last bars
/// Binance still serves.
const STALE_AROUND_THE_CLOCK: Duration = Duration::from_secs(8 * 3600);
/// The same for a market with sessions, which is shut over weekends and holidays.
const STALE_IN_SESSIONS: Duration = Duration::from_secs(7 * 86_400);
/// Most symbols one `get_market_context` call takes.
pub const MAX_SYMBOLS: usize = 10;

/// A symbol's context: being read since then, or read then.
enum Held {
    Reading(Instant),
    Read(Instant, Value),
}

impl Held {
    fn current(&self) -> bool {
        match self {
            Held::Reading(since) => since.elapsed() < READING,
            Held::Read(at, _) => at.elapsed() < FRESH,
        }
    }
}

static HELD: LazyLock<Mutex<HashMap<String, Held>>> = LazyLock::new(Mutex::default);
/// Told when a read ends.
static READ: Condvar = Condvar::new();
/// When the index was last read, and what it was then.
static FEAR_GREED: Mutex<Option<(Instant, Option<f64>)>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The context of `symbol`, as held if it is fresh. Blocking: a read can take seconds.
/// A symbol whose bars cannot be read answers `{symbol, error}`. Callers that come while
/// a symbol is being read wait for that read rather than start another.
pub fn of(symbol: &str) -> Value {
    let mut held = lock(&HELD);
    loop {
        match held.get(symbol) {
            Some(Held::Read(at, context)) if at.elapsed() < FRESH => return context.clone(),
            Some(h @ Held::Reading(_)) if h.current() => {
                held = READ
                    .wait_timeout(held, READING)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            _ => break,
        }
    }
    held.insert(symbol.to_string(), Held::Reading(Instant::now()));
    drop(held);
    #[cfg(test)]
    tests::count_read(symbol);
    let context = read(symbol, source());
    let mut held = lock(&HELD);
    held.retain(|_, h| h.current());
    held.insert(
        symbol.to_string(),
        Held::Read(Instant::now(), context.clone()),
    );
    READ.notify_all();
    context
}

/// The context for a wake-up: what `of` has within `wait`, or none. A read that takes
/// longer goes on and is held for the next attempt.
#[cfg(feature = "scripting")]
pub async fn within(symbol: String, wait: Duration) -> Option<Value> {
    let read = tokio::task::spawn_blocking(move || of(&symbol));
    match tokio::time::timeout(wait, read).await {
        Ok(Ok(context)) if context.get("error").is_none() => Some(context),
        _ => None,
    }
}

/// Unit tests read the sample source: a test of a wake-up must not reach an exchange.
fn source() -> DataSource {
    if cfg!(test) {
        DataSource::Sample
    } else {
        DataSource::from_env()
    }
}

fn read(symbol: &str, source: DataSource) -> Value {
    let four = match bars(source, symbol, "4h") {
        Ok(data) => data,
        Err(e) => return json!({ "symbol": symbol, "error": e }),
    };
    if let Some(e) = stale(symbol, &four, fetch::now_ms()) {
        return json!({ "symbol": symbol, "error": e });
    }
    let day = bars(source, symbol, "1d").ok();
    let mut v = Map::new();
    v.insert("symbol".into(), json!(symbol));
    v.insert("at".into(), json!(fetch::now_ms()));
    if let Some(last) = four.bars.last() {
        v.insert("price".into(), json!(price(last.close)));
    }
    let mut trend = Map::new();
    for (interval, data) in [("4h", Some(&four)), ("1d", day.as_ref())] {
        if let Some(t) = data.and_then(trend_of) {
            trend.insert(interval.into(), json!(t));
        }
    }
    v.insert("trend".into(), Value::Object(trend));
    // The futures series come from Binance, or from their sample under
    // `CHARTGEN_DATA_SOURCE=sample`, whatever source the bars came from. A unit test
    // has neither, and so leaves them out.
    let futures = source == DataSource::Live || chartgen::data::sample_source();
    if futures && fetch::detect_source(symbol) == "binance" {
        // On 1h bars: the indicator lines a rate up with the nearest bar open, and a
        // symbol funded every hour has a newer rate than the 4h bar's open.
        let hourly = bars(source, symbol, "1h").ok();
        let funding = hourly.as_ref().map(|h| indicator("funding_rate", h));
        if let Some(rate) = last(&funding.as_ref().and_then(|f| f.bars.first()).map(|b| &b.y)) {
            v.insert("funding_pct".into(), json!(round(rate, 4)));
        }
        let ls = indicator("long_short", &four);
        let side = |i: usize| last(&ls.lines.get(i).map(|l| &l.y));
        if let (Some(long), Some(short)) = (side(0), side(1)) {
            let mut split = json!({ "long": round(long, 3) });
            if short > 0.0 {
                split["ratio"] = json!(round(long / short, 2));
            }
            v.insert("long_short".into(), split);
        }
        if let Some(value) = day.as_ref().and_then(|d| fear_greed(source, d)) {
            v.insert(
                "fear_greed".into(),
                json!({ "value": value.round() as i64, "label": fear_greed_label(value) }),
            );
        }
    }
    let line = line(&v);
    v.insert("line".into(), json!(line));
    Value::Object(v)
}

/// Why bars whose newest opened long ago are no market to describe, if they are: a
/// price from then is not the price now.
fn stale(symbol: &str, data: &OhlcvData, now_ms: i64) -> Option<String> {
    let opened = data.bars.last()?.date.parse::<i64>().ok()? * 1000;
    let limit = match fetch::detect_source(symbol) {
        "binance" => STALE_AROUND_THE_CLOCK,
        _ => STALE_IN_SESSIONS,
    };
    (now_ms - opened > limit.as_millis() as i64).then(|| {
        format!(
            "no bar since {}: not trading",
            chartgen::drawings::iso(opened)
        )
    })
}

fn bars(source: DataSource, symbol: &str, interval: &str) -> Result<OhlcvData, String> {
    let mut data = fetch::fetch_ohlcv(source, symbol, interval, BARS, None)?.data;
    if data.bars.is_empty() {
        return Err(format!("no {interval} bars for {symbol}"));
    }
    data.symbol = Some(symbol.to_string());
    data.interval = Some(interval.to_string());
    Ok(data)
}

fn indicator(name: &str, data: &OhlcvData) -> chartgen::indicator::PanelResult {
    indicators::by_name(name)
        .map(|i| i.compute(data))
        .unwrap_or_default()
}

/// The last value of a series, if it has one there: a missing reading is NaN.
fn last(series: &Option<&Vec<f64>>) -> Option<f64> {
    series
        .and_then(|y| y.last().copied())
        .filter(|v| v.is_finite())
}

/// A price to 6 significant digits and at least 2 decimals: Yahoo's carry float noise.
fn price(p: f64) -> f64 {
    if !(p.is_finite() && p > 0.0) {
        return p;
    }
    round(p, (5 - p.log10().floor() as i32).clamp(2, 12))
}

fn round(v: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (v * f).round() / f
}

/// `up` when the last price is above the 20-bar EMA and that is above the 50-bar EMA,
/// `down` when both are the other way round, `mixed` otherwise. Of the bars as read,
/// the forming one included, so the last price is the current one.
fn trend_of(data: &OhlcvData) -> Option<&'static str> {
    let closes: Vec<f64> = data.bars.iter().map(|b| b.close).collect();
    let price = *closes.last()?;
    let fast = *ema(&closes, FAST).last()?;
    let slow = *ema(&closes, SLOW).last()?;
    if !(fast.is_finite() && slow.is_finite()) {
        return None;
    }
    Some(if price > fast && fast > slow {
        "up"
    } else if price < fast && fast < slow {
        "down"
    } else {
        "mixed"
    })
}

/// Today's Fear & Greed index, kept for ten minutes. The sample source has none.
fn fear_greed(source: DataSource, day: &OhlcvData) -> Option<f64> {
    if !matches!(source, DataSource::Live) {
        return None;
    }
    if let Some((at, value)) = *lock(&FEAR_GREED) {
        if at.elapsed() < FEAR_GREED_FRESH {
            return value;
        }
    }
    let value = last(&indicator("fear_greed", day).lines.first().map(|l| &l.y));
    *lock(&FEAR_GREED) = Some((Instant::now(), value));
    value
}

/// The bands the Fear & Greed panel colours by.
fn fear_greed_label(value: f64) -> &'static str {
    match value {
        v if v < 25.0 => "extreme fear",
        v if v < 45.0 => "fear",
        v if v < 55.0 => "neutral",
        v if v < 75.0 => "greed",
        _ => "extreme greed",
    }
}

/// The context in one line, for a receiver that shows a template rather than the JSON.
fn line(v: &Map<String, Value>) -> String {
    let mut parts = vec![match v.get("price") {
        Some(p) => format!("{} {p}", v["symbol"].as_str().unwrap_or_default()),
        None => v["symbol"].as_str().unwrap_or_default().to_string(),
    }];
    let trend: Vec<String> = ["4h", "1d"]
        .iter()
        .filter_map(|i| Some(format!("{i} {}", v["trend"].get(*i)?.as_str()?)))
        .collect();
    if !trend.is_empty() {
        parts.push(format!("trend {}", trend.join(", ")));
    }
    if let Some(f) = v.get("funding_pct") {
        parts.push(format!("funding {f}%"));
    }
    if let Some(ls) = v.get("long_short") {
        let long = ls["long"].as_f64().unwrap_or_default();
        let mut part = format!("longs {:.1}%", long * 100.0);
        if let Some(r) = ls.get("ratio") {
            part.push_str(&format!(" (L/S {r})"));
        }
        parts.push(part);
    }
    if let Some(fg) = v.get("fear_greed") {
        parts.push(format!(
            "F&G {} {}",
            fg["value"],
            fg["label"].as_str().unwrap_or_default()
        ));
    }
    parts.join(" | ")
}

/// `get_market_context`: the context of each symbol, in the order asked. One symbol
/// that cannot be read answers `{symbol, error}` and never fails the call.
pub fn call_tool(id: Option<Value>, args: &Value) -> Value {
    if crate::replay::global().is_some() {
        return tool_err(
            id,
            "This is a replay server: get_market_context is not available, because it reads what the fixtures do not hold",
        );
    }
    let Some(raw) = args.get("symbols").and_then(|v| v.as_array()) else {
        return tool_err(id, "symbols is required and must be an array of strings");
    };
    if raw.is_empty() || raw.len() > MAX_SYMBOLS {
        return tool_err(id, &format!("Give 1 to {MAX_SYMBOLS} symbols"));
    }
    let mut asked: Vec<String> = Vec::new();
    for v in raw {
        let Some(text) = v.as_str() else {
            return tool_err(id, "symbols must be an array of strings");
        };
        let Some(symbol) = crate::mcp::normalize_quote_symbol(text) else {
            return tool_err(id, &format!("{text:?} is not a symbol"));
        };
        if !asked.contains(&symbol) {
            asked.push(symbol);
        }
    }
    let contexts: Vec<Value> = asked.iter().map(|s| of(s)).collect();
    tool_ok(id, json!({ "contexts": contexts }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chartgen::data::Bar;

    static READS: LazyLock<Mutex<HashMap<String, usize>>> = LazyLock::new(Mutex::default);

    pub(super) fn count_read(symbol: &str) {
        *lock(&READS).entry(symbol.to_string()).or_default() += 1;
    }

    /// Callers at once, for a symbol nobody holds, share one read.
    #[test]
    fn callers_at_once_share_one_read() {
        let symbol = "SHAREDREADUSDT";
        let start = std::sync::Barrier::new(8);
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    start.wait();
                    of(symbol)
                });
            }
        });
        assert_eq!(lock(&READS).get(symbol), Some(&1));
    }

    fn data(closes: &[f64]) -> OhlcvData {
        OhlcvData {
            bars: closes
                .iter()
                .enumerate()
                .map(|(i, &c)| Bar {
                    date: (i as i64 * 14_400).to_string(),
                    open: c,
                    high: c,
                    low: c,
                    close: c,
                    volume: 1.0,
                })
                .collect(),
            symbol: None,
            interval: None,
            session: None,
        }
    }

    #[test]
    fn a_trend_is_the_price_and_the_two_emas_in_order() {
        let rising: Vec<f64> = (0..120).map(|i| 100.0 + i as f64).collect();
        assert_eq!(trend_of(&data(&rising)), Some("up"));
        let falling: Vec<f64> = rising.iter().rev().copied().collect();
        assert_eq!(trend_of(&data(&falling)), Some("down"));
        // Rising, then a last price under the fast EMA: not a trend either way.
        let mut dipped = rising.clone();
        *dipped.last_mut().unwrap() = 150.0;
        assert_eq!(trend_of(&data(&dipped)), Some("mixed"));
        // Too few bars for the slow EMA: no reading rather than a guess.
        assert_eq!(trend_of(&data(&rising[..SLOW - 1])), None);
    }

    /// A unit test leaves the futures series out rather than reach Binance, unless
    /// `CHARTGEN_DATA_SOURCE=sample` turns their sample series on. Never the index.
    #[test]
    fn a_unit_test_reads_the_price_and_the_trends_and_no_live_series() {
        for symbol in ["BTCUSDT", "AAPL"] {
            let v = read(symbol, source());
            assert!(v["price"].as_f64().is_some(), "{v}");
            assert!(
                v["trend"]["4h"].is_string() && v["trend"]["1d"].is_string(),
                "{v}"
            );
            let futures = symbol == "BTCUSDT" && chartgen::data::sample_source();
            for part in ["funding_pct", "long_short"] {
                assert_eq!(v.get(part).is_some(), futures, "{part}: {v}");
            }
            assert!(v.get("fear_greed").is_none(), "{v}");
            let line = v["line"].as_str().unwrap();
            assert!(
                line.starts_with(&format!("{symbol} ")) && line.contains(" | trend 4h "),
                "{line}"
            );
        }
    }

    #[test]
    fn the_line_says_each_part_that_is_there() {
        let v = json!({
            "symbol": "BTCUSDT", "price": 91000.5,
            "trend": { "4h": "down", "1d": "mixed" },
            "funding_pct": 0.006, "long_short": { "long": 0.764, "ratio": 3.24 },
            "fear_greed": { "value": 13, "label": "extreme fear" },
        });
        assert_eq!(
            line(v.as_object().unwrap()),
            "BTCUSDT 91000.5 | trend 4h down, 1d mixed | funding 0.006% | longs 76.4% (L/S 3.24) | F&G 13 extreme fear"
        );
    }

    #[test]
    fn a_price_keeps_six_digits_and_two_decimals() {
        assert_eq!(price(356.2699890136719), 356.27);
        assert_eq!(price(64493.45), 64493.45);
        assert_eq!(price(1774.774), 1774.77);
        assert_eq!(price(0.51234567), 0.512346);
        assert_eq!(price(0.000012345678), 0.0000123457);
    }

    /// A delisted symbol's last bars are still served: they are not a price now.
    #[test]
    fn a_symbol_whose_last_bar_is_old_answers_an_error() {
        let v = read("STALEUSDT", DataSource::Unavailable);
        assert_eq!(v["symbol"], "STALEUSDT", "{v}");
        let error = v["error"].as_str().unwrap();
        assert!(
            error.starts_with("no bar since 2024-02-1") && error.ends_with(": not trading"),
            "{v}"
        );
        assert!(v.get("price").is_none(), "{v}");
    }

    #[test]
    fn stale_is_a_whole_bar_without_trade_or_a_week_without_a_session() {
        let opened = |hours_ago: i64| {
            let mut d = data(&[1.0]);
            d.bars[0].date = (1_800_000_000 - hours_ago * 3600).to_string();
            d
        };
        let now = 1_800_000_000_000;
        assert_eq!(stale("BTCUSDT", &opened(4), now), None);
        assert_eq!(stale("BTCUSDT", &opened(8), now), None);
        assert!(stale("BTCUSDT", &opened(9), now).is_some());
        // A stock's weekend and a holiday after it.
        assert_eq!(stale("AAPL", &opened(4 * 24), now), None);
        assert!(stale("AAPL", &opened(8 * 24), now).is_some());
    }

    #[test]
    fn the_index_bands_are_the_panels() {
        let labels: Vec<&str> = [0.0, 24.9, 25.0, 50.0, 55.0, 74.9, 75.0]
            .into_iter()
            .map(fear_greed_label)
            .collect();
        assert_eq!(
            labels,
            [
                "extreme fear",
                "extreme fear",
                "fear",
                "neutral",
                "greed",
                "greed",
                "extreme greed"
            ]
        );
    }

    #[test]
    fn a_call_takes_one_to_ten_symbols_and_answers_in_order() {
        let none = call_tool(Some(json!(1)), &json!({ "symbols": [] }));
        assert!(none["result"]["isError"] == json!(true), "{none}");
        let eleven: Vec<String> = (0..11).map(|i| format!("S{i}USDT")).collect();
        let many = call_tool(Some(json!(1)), &json!({ "symbols": eleven }));
        assert!(many["result"]["isError"] == json!(true), "{many}");
        let bad = call_tool(Some(json!(1)), &json!({ "symbols": ["BTC USDT"] }));
        assert!(bad["result"]["isError"] == json!(true), "{bad}");
    }
}
