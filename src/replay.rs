//! Replay: answer "as of" a time from fixture bars (ADR-0059).
//!
//! `CHARTGEN_REPLAY_FIXTURES=<dir>` starts the server on 1m bars read from that
//! directory, for an evaluation that gives the same answers on every run. It is read
//! at startup only. Every bar a tool sees comes through `Replay::bars`, which keeps a
//! 1m bar only if it closed at or before the asked time and builds every other
//! interval from those, so nothing after that time can reach an answer.

#[cfg(test)]
use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use chartgen::data::Bar;
use chartgen::interval::{Interval, Unit};
use chartgen::setups::Adjustment;
use serde::Deserialize;

const MINUTE_MS: i64 = 60_000;

/// What a call with `as_of` is told on a server that is not replaying.
pub const NOT_A_REPLAY: &str =
    "as_of is only for a replay server (one started with CHARTGEN_REPLAY_FIXTURES); leave it out";

/// The tools that read prices in a replay. Each needs `as_of`.
pub const PRICE_TOOLS: [&str; 4] = [
    "generate_chart",
    "inspect_chart",
    "get_indicators",
    "get_quotes",
];
/// The tools that read no prices, and answer in a replay as anywhere.
const PLAIN_TOOLS: [&str; 2] = ["list_indicators", "list_drawing_tools"];

/// The time the call running on this thread asks about, in a replay.
pub fn clock() -> Option<i64> {
    chartgen::data::clock()
}

/// Run `call` with the clock at `as_of`: every bar it reads and every "now" it asks
/// for, in the server and in the indicators, is cut there.
pub fn at<T>(as_of: Option<i64>, call: impl FnOnce() -> T) -> T {
    chartgen::data::at(as_of, call)
}

/// The `as_of` of a call in a replay: unix seconds, unix ms or an ISO 8601 time.
pub fn as_of(given: Option<&serde_json::Value>) -> Result<i64, String> {
    let given = given.filter(|v| !v.is_null()).ok_or(
        "This is a replay server: every call needs as_of, the time it is answered for (unix seconds, unix ms or an ISO 8601 time)",
    )?;
    chartgen::drawings::parse_time(given, i64::MAX / 2).map_err(|e| format!("as_of: {e}"))
}

/// Whether `ip` reaches this host or its tailnet only: loopback, or Tailscale's
/// 100.64.0.0/10 or fd7a:115c:a1e0::/48. Anyone who reaches a replay can move its clock
/// (ADR-0068).
pub fn private_address(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_loopback() || (a == 100 && b & 0xc0 == 64)
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback() || v6.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0]
        }
    }
}

/// The directory of fixtures, when the server is to replay.
pub fn fixtures_dir() -> Option<String> {
    std::env::var("CHARTGEN_REPLAY_FIXTURES")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// `{"symbol": "BTCUSDT", "bars": [[open_time_s, open, high, low, close, volume], ...]}`.
/// A stock's may add `"dividends": [[ex_date_s, amount], ...]` (ADR-0070).
#[derive(Deserialize)]
struct Fixture {
    symbol: String,
    bars: Vec<(i64, f64, f64, f64, f64, f64)>,
    #[serde(default)]
    dividends: Vec<(i64, f64)>,
}

pub struct Replay {
    /// 1m bars per symbol, oldest first.
    bars: BTreeMap<String, Vec<Bar>>,
    /// A stock's dividends per symbol, as (ex-date ms, amount), oldest first.
    dividends: BTreeMap<String, Vec<(i64, f64)>>,
    /// The latest time any call has asked about. One clock for every tool and every
    /// caller: one process is one run, and registration is open, so a clock per caller
    /// would let a second client read a time the first has already passed.
    clock: Mutex<Option<i64>>,
}

static GLOBAL: OnceLock<Replay> = OnceLock::new();

/// The replay this server runs, when it does.
#[cfg(not(test))]
pub fn global() -> Option<&'static Replay> {
    GLOBAL.get()
}

#[cfg(test)]
thread_local! {
    /// Tests share one process: a replay is on for the thread that turned it on only.
    static TEST: Cell<Option<&'static Replay>> = const { Cell::new(None) };
}

#[cfg(test)]
pub fn global() -> Option<&'static Replay> {
    TEST.with(Cell::get)
}

/// Run `test` on a replay server. Its callers are the setup tests, which exist with
/// the `scripting` feature only.
#[cfg(all(test, feature = "scripting"))]
pub fn on<T>(replay: Replay, test: impl FnOnce(&'static Replay) -> T) -> T {
    let replay: &'static Replay = Box::leak(Box::new(replay));
    let before = TEST.with(|t| t.replace(Some(replay)));
    let out = test(replay);
    TEST.with(|t| t.set(before));
    out
}

/// Load the fixtures and make this a replay server. At startup.
#[cfg_attr(test, allow(dead_code))]
pub fn start(dir: &str) -> Result<&'static Replay, String> {
    let replay = Replay::load(Path::new(dir))?;
    Ok(GLOBAL.get_or_init(|| replay))
}

fn open_ms(bar: &Bar) -> i64 {
    bar.date.parse::<i64>().unwrap_or(0) * 1000
}

impl Replay {
    /// Read every `*.json` in `dir`. A fixture that is not minute bars in order is an
    /// error: a replay on bad bars would be scored on them.
    pub fn load(dir: &Path) -> Result<Replay, String> {
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .filter_map(|entry| Some(entry.ok()?.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        paths.sort();
        let mut bars = BTreeMap::new();
        let mut dividends = BTreeMap::new();
        for path in paths {
            let name = path.display();
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{name}: {e}"))?;
            let fixture: Fixture =
                serde_json::from_str(&text).map_err(|e| format!("{name}: {e}"))?;
            let parsed = Self::fixture_bars(&fixture).map_err(|e| format!("{name}: {e}"))?;
            if bars.insert(fixture.symbol.clone(), parsed).is_some() {
                return Err(format!("{name}: {} has a fixture already", fixture.symbol));
            }
            let mut paid = Vec::new();
            for &(t, amount) in &fixture.dividends {
                if !amount.is_finite() || amount <= 0.0 {
                    return Err(format!(
                        "{name}: the dividend at {t} is not an amount above 0"
                    ));
                }
                paid.push((t * 1000, amount));
            }
            paid.sort_by_key(|d| d.0);
            dividends.insert(fixture.symbol, paid);
        }
        if bars.is_empty() {
            return Err(format!("{} holds no fixture", dir.display()));
        }
        Ok(Replay {
            dividends,
            ..Replay::of(bars)
        })
    }

    pub fn of(bars: BTreeMap<String, Vec<Bar>>) -> Replay {
        Replay {
            bars,
            dividends: BTreeMap::new(),
            clock: Mutex::default(),
        }
    }

    /// The dividends of a stock that went ex from `from_ms` to `to_ms`.
    pub fn dividends(&self, symbol: &str, from_ms: i64, to_ms: i64) -> Vec<Adjustment> {
        self.dividends
            .get(symbol)
            .into_iter()
            .flatten()
            .filter(|(at, _)| (from_ms..=to_ms).contains(at))
            .map(|&(at, amount)| Adjustment::Dividend { at, amount })
            .collect()
    }

    fn fixture_bars(fixture: &Fixture) -> Result<Vec<Bar>, String> {
        let mut last = None;
        fixture
            .bars
            .iter()
            .map(|&(t, open, high, low, close, volume)| {
                let prices = [open, high, low, close];
                if t % 60 != 0 || last.is_some_and(|l| t <= l) {
                    return Err(format!("bar {t} is not a later whole minute"));
                }
                if prices.iter().any(|p| !p.is_finite() || *p <= 0.0)
                    || low > open.min(close)
                    || high < open.max(close)
                    || !volume.is_finite()
                    || volume < 0.0
                {
                    return Err(format!("bar {t} is not a possible bar"));
                }
                last = Some(t);
                Ok(Bar {
                    open,
                    high,
                    low,
                    close,
                    volume,
                    date: t.to_string(),
                })
            })
            .collect()
    }

    /// Admit a tool call that is not a setup's. A tool that reads prices needs `as_of`
    /// and moves the clock to it; one that would read anything but the fixtures
    /// is refused. Returns the time to run the call at.
    pub fn admit(&self, tool: &str, args: &serde_json::Value) -> Result<Option<i64>, String> {
        if PLAIN_TOOLS.contains(&tool) {
            return Ok(None);
        }
        if !PRICE_TOOLS.contains(&tool) {
            return Err(format!(
                "This is a replay server: {tool} is not available, because it reads what the fixtures do not hold"
            ));
        }
        let now = as_of(args.get("as_of"))?;
        self.advance(now)?;
        Ok(Some(now))
    }

    /// The last price of `symbol` and its last 24 hours, from the 1m bars closed by
    /// `as_of`: (last, the open 24 h before, high, low, volume).
    pub fn quote(&self, symbol: &str, as_of: i64) -> Result<(f64, f64, f64, f64, f64), String> {
        if !self.bars.contains_key(symbol) {
            return Err(format!("this replay has no bars for {symbol}"));
        }
        let day = self.minute_bars(symbol, as_of - 1440 * MINUTE_MS, as_of);
        let (Some(first), Some(last)) = (day.first(), day.last()) else {
            return Err(format!("{symbol} has no bars in the 24 hours before as_of"));
        };
        let high = day.iter().map(|b| b.high).fold(f64::MIN, f64::max);
        let low = day.iter().map(|b| b.low).fold(f64::MAX, f64::min);
        let volume = day.iter().map(|b| b.volume).sum();
        Ok((last.close, first.open, high, low, volume))
    }

    /// Move the clock to `as_of`. A time before it is refused.
    pub fn advance(&self, as_of: i64) -> Result<(), String> {
        let mut clock = self.clock.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(latest) = clock.filter(|latest| as_of < *latest) {
            return Err(format!(
                "as_of {} is before {}, the latest time this replay has answered for; a replay only moves forward",
                chartgen::drawings::iso(as_of),
                chartgen::drawings::iso(latest)
            ));
        }
        *clock = Some(as_of);
        Ok(())
    }

    /// The 1m bars of `symbol` that opened at or after `from_ms` and closed at or
    /// before `as_of`.
    pub fn minute_bars(&self, symbol: &str, from_ms: i64, as_of: i64) -> &[Bar] {
        let Some(bars) = self.bars.get(symbol) else {
            return &[];
        };
        let start = bars.partition_point(|b| open_ms(b) < from_ms);
        let end = bars.partition_point(|b| open_ms(b) + MINUTE_MS <= as_of);
        &bars[start.min(end)..end]
    }

    /// The newest `limit` bars of `interval` that closed at or before `as_of`, and
    /// opened before `end_ms` when that is given; and whether older ones exist.
    pub fn bars(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
        as_of: i64,
        end_ms: Option<i64>,
    ) -> Result<(Vec<Bar>, bool), String> {
        if !self.bars.contains_key(symbol) {
            return Err(format!("this replay has no bars for {symbol}"));
        }
        let iv = Interval::parse(interval)?;
        if iv.unit == Unit::Second {
            return Err(format!(
                "this replay holds 1m bars, so it has no {interval} bars"
            ));
        }
        let minutes = self.minute_bars(symbol, i64::MIN, as_of);
        // A stock's intraday bars start at its session's open, as Yahoo's do (ADR-0070).
        let market = crate::fetch::market_of(symbol);
        let start_of = |t: i64| match market {
            Some(m) => m.bar_start(&iv, t * 1000) / 1000,
            None => iv.grid_start(t),
        };
        let end_of = |start: i64| match market {
            Some(m) => m.bar_end(&iv, start * 1000) / 1000,
            None => iv.grid_end(start),
        };
        let mut out: Vec<Bar> = Vec::new();
        let mut bucket = None;
        for bar in minutes {
            let t = open_ms(bar) / 1000;
            let start = start_of(t);
            if bucket != Some(start) {
                bucket = Some(start);
                out.push(Bar {
                    date: start.to_string(),
                    ..bar.clone()
                });
                continue;
            }
            if let Some(b) = out.last_mut() {
                b.high = b.high.max(bar.high);
                b.low = b.low.min(bar.low);
                b.close = bar.close;
                b.volume += bar.volume;
            }
        }
        // Whole bars only: one that began before the fixture, or has not closed by
        // `as_of`, would be a bar that never existed.
        let first = minutes.first().map_or(0, |b| open_ms(b) / 1000);
        out.retain(|b| {
            let start = open_ms(b) / 1000;
            start >= first
                && end_of(start) * 1000 <= as_of
                && end_ms.is_none_or(|end| start * 1000 < end)
        });
        let has_more = out.len() > limit;
        Ok((out.split_off(out.len().saturating_sub(limit)), has_more))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_700_006_400;

    /// `n` minutes from `T0`, an hour boundary; bar `i` closes at `100 + i`.
    fn minutes(n: usize, lift: f64) -> Vec<Bar> {
        (0..n)
            .map(|i| {
                let c = 100.0 + i as f64 + lift;
                Bar {
                    open: c - 0.5,
                    high: c + 1.0,
                    low: c - 1.0,
                    close: c,
                    volume: 1.0,
                    date: (T0 + i as i64 * 60).to_string(),
                }
            })
            .collect()
    }

    fn replay(n: usize) -> Replay {
        Replay::of(BTreeMap::from([("BTCUSDT".to_string(), minutes(n, 0.0))]))
    }

    #[test]
    fn as_of_drops_the_bar_that_contains_it() {
        let r = replay(300);
        // 90 s in: the first bar has closed, the second has not.
        let (one, _) = r
            .bars("BTCUSDT", "1m", 10, T0 * 1000 + 90_000, None)
            .unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].close, 100.0);
        // On the boundary a bar has just closed.
        let (two, _) = r
            .bars("BTCUSDT", "1m", 10, T0 * 1000 + 120_000, None)
            .unwrap();
        assert_eq!(two.len(), 2);

        // 1h: nothing until the hour is whole, then exactly its 60 minutes.
        let almost = (T0 + 3599) * 1000;
        assert!(r
            .bars("BTCUSDT", "1h", 10, almost, None)
            .unwrap()
            .0
            .is_empty());
        let (hour, _) = r.bars("BTCUSDT", "1h", 10, almost + 1000, None).unwrap();
        assert_eq!(hour.len(), 1);
        assert_eq!((hour[0].open, hour[0].close), (99.5, 159.0));
        assert_eq!(
            (hour[0].high, hour[0].low, hour[0].volume),
            (160.0, 99.0, 60.0)
        );
        // 90 minutes in, the second hour is still forming and is not there.
        let later = (T0 + 5400) * 1000;
        assert_eq!(r.bars("BTCUSDT", "1h", 10, later, None).unwrap().0.len(), 1);
    }

    #[test]
    fn a_bar_that_began_before_the_fixture_is_not_built() {
        let mut bars = minutes(300, 0.0);
        bars.drain(..30);
        let r = Replay::of(BTreeMap::from([("BTCUSDT".to_string(), bars)]));
        let (hours, _) = r
            .bars("BTCUSDT", "1h", 10, (T0 + 7200) * 1000, None)
            .unwrap();
        assert_eq!(hours.len(), 1);
        assert_eq!(hours[0].date, (T0 + 3600).to_string());
    }

    #[test]
    fn a_gap_of_hours_leaves_out_whole_buckets_and_shortens_the_rest() {
        // No minutes from 00:30 to 03:20 (T0 is on the 4h grid): real history has
        // gaps like it, and a fixture builder does not fill them (ADR-0068).
        let mut bars = minutes(600, 0.0);
        bars.drain(30..200);
        let r = Replay::of(BTreeMap::from([("BTCUSDT".to_string(), bars)]));
        let as_of = (T0 + 6 * 3600) * 1000;
        let (hours, _) = r.bars("BTCUSDT", "1h", 10, as_of, None).unwrap();
        // Hours 1 and 2 had no minute, so there is no bar for them, as on Binance.
        let starts: Vec<i64> = hours.iter().map(|b| open_ms(b) / 1000 - T0).collect();
        assert_eq!(starts, [0, 3 * 3600, 4 * 3600, 5 * 3600]);
        // An hour with a gap is built from the minutes it has, on its grid start.
        assert_eq!((hours[0].close, hours[0].volume), (129.0, 30.0));
        assert_eq!((hours[1].open, hours[1].volume), (299.5, 40.0));
        // The 4h bar over the gap: the minutes before and after it.
        let (four, _) = r.bars("BTCUSDT", "4h", 10, as_of, None).unwrap();
        assert_eq!(four.len(), 1, "the second 4h bar has not closed");
        assert_eq!(four[0].date, T0.to_string());
        assert_eq!(
            (four[0].open, four[0].high, four[0].low, four[0].close),
            (99.5, 340.0, 99.0, 339.0)
        );
        assert_eq!(four[0].volume, 70.0);
        // In the gap, the cut is still by time: the hour that held the last minute is
        // whole once its end has passed, and nothing later is there yet.
        let (early, _) = r
            .bars("BTCUSDT", "1h", 10, (T0 + 9000) * 1000, None)
            .unwrap();
        assert_eq!(early.len(), 1);
    }

    #[test]
    fn end_time_and_limit_page_within_the_cut() {
        let r = replay(300);
        let as_of = (T0 + 600) * 1000;
        let (page, more) = r.bars("BTCUSDT", "1m", 3, as_of, None).unwrap();
        assert_eq!(
            page.iter().map(|b| b.close).collect::<Vec<_>>(),
            [107.0, 108.0, 109.0]
        );
        assert!(more);
        let end = Some((T0 + 120) * 1000);
        let (older, more) = r.bars("BTCUSDT", "1m", 3, as_of, end).unwrap();
        assert_eq!(older.len(), 2);
        assert!(!more);
        // An end time past the cut cannot widen it.
        let (same, _) = r.bars("BTCUSDT", "1m", 3, as_of, Some(i64::MAX)).unwrap();
        let closes = |bars: &[Bar]| bars.iter().map(|b| b.close).collect::<Vec<_>>();
        assert_eq!(closes(&same), closes(&page));
    }

    #[test]
    fn only_loopback_and_tailscale_addresses_are_private() {
        for (ip, private) in [
            ("127.0.0.1", true),
            ("::1", true),
            ("100.64.0.1", true),
            ("100.101.102.103", true),
            ("100.127.255.255", true),
            ("fd7a:115c:a1e0::1", true),
            ("0.0.0.0", false),
            ("::", false),
            ("100.63.255.255", false),
            ("100.128.0.1", false),
            ("192.168.1.10", false),
            ("fd7a:115c:a1e1::1", false),
        ] {
            assert_eq!(private_address(ip.parse().unwrap()), private, "{ip}");
        }
    }

    #[test]
    fn the_clock_only_moves_forward() {
        let r = replay(10);
        r.advance(2000).unwrap();
        r.advance(2000).unwrap();
        assert!(r.advance(1999).unwrap_err().contains("only moves forward"));
        r.advance(2001).unwrap();
    }

    #[test]
    fn unknown_symbols_and_second_bars_are_refused() {
        let r = replay(10);
        assert!(r.bars("ETHUSDT", "1m", 5, i64::MAX, None).is_err());
        assert!(r.bars("BTCUSDT", "5s", 5, i64::MAX, None).is_err());
    }

    #[test]
    fn a_fixture_must_be_minute_bars_in_order() {
        let fixture = |bars| Fixture {
            symbol: "BTCUSDT".into(),
            bars,
            dividends: Vec::new(),
        };
        let ok = (T0, 1.0, 2.0, 0.5, 1.5, 3.0);
        assert!(
            Replay::fixture_bars(&fixture(vec![ok, (T0 + 60, 1.0, 2.0, 0.5, 1.5, 3.0)])).is_ok()
        );
        for bad in [
            vec![ok, ok],
            vec![(T0 + 1, 1.0, 2.0, 0.5, 1.5, 3.0)],
            vec![(T0, 1.0, 0.9, 0.5, 1.5, 3.0)],
            vec![(T0, 1.0, 2.0, 0.5, f64::NAN, 3.0)],
        ] {
            assert!(Replay::fixture_bars(&fixture(bad)).is_err());
        }
    }
}
