/// OHLCV bar and data container.
use crate::interval::{Interval, Unit};
use chrono::{Datelike, NaiveDate, NaiveTime, TimeZone};

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Bar {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub date: String,
}

#[derive(Clone, Debug)]
pub struct OhlcvData {
    pub bars: Vec<Bar>,
    pub symbol: Option<String>,
    pub interval: Option<String>,
    /// The exchange's regular session; `None` for a 24/7 market or an unknown one.
    pub session: Option<Session>,
}

impl OhlcvData {
    /// Heikin Ashi transform: haClose = ohlc4, haOpen = (prev haOpen + prev
    /// haClose) / 2 seeded with (open + close) / 2, haHigh/haLow widened to
    /// contain haOpen/haClose. Volume and timestamps are kept.
    pub fn heikin_ashi(&self) -> OhlcvData {
        let mut prev: Option<(f64, f64)> = None;
        let bars = self
            .bars
            .iter()
            .map(|b| {
                let ha_close = (b.open + b.high + b.low + b.close) / 4.0;
                let ha_open = match prev {
                    Some((o, c)) => (o + c) / 2.0,
                    None => (b.open + b.close) / 2.0,
                };
                prev = Some((ha_open, ha_close));
                Bar {
                    open: ha_open,
                    high: b.high.max(ha_open).max(ha_close),
                    low: b.low.min(ha_open).min(ha_close),
                    close: ha_close,
                    volume: b.volume,
                    date: b.date.clone(),
                }
            })
            .collect();
        OhlcvData {
            bars,
            symbol: self.symbol.clone(),
            interval: self.interval.clone(),
            session: self.session,
        }
    }

    pub fn closes(&self) -> Vec<f64> {
        self.bars.iter().map(|b| b.close).collect()
    }
    pub fn highs(&self) -> Vec<f64> {
        self.bars.iter().map(|b| b.high).collect()
    }
    pub fn lows(&self) -> Vec<f64> {
        self.bars.iter().map(|b| b.low).collect()
    }
    pub fn len(&self) -> usize {
        self.bars.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }
}

/// Simple LCG PRNG (no external dep needed).
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_f64(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f64) / (u32::MAX as f64)
    }
    /// Approximate normal via Box-Muller.
    fn normal(&mut self, mean: f64, std: f64) -> f64 {
        let u1 = self.next_f64().max(1e-10);
        let u2 = self.next_f64();
        mean + std * (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

pub fn sample_data(n: usize) -> OhlcvData {
    let mut rng = Rng::new(42);
    let mut price = 30.0_f64;
    let mut bars = Vec::with_capacity(n);

    for i in 0..n {
        let o = price + rng.normal(0.0, 0.5);
        let c = o + rng.normal(-0.1, 1.0);
        let h = o.max(c) + rng.normal(0.0, 0.5).abs();
        let l = o.min(c) - rng.normal(0.0, 0.5).abs();
        let v = rng.normal(10.0, 0.8).exp();
        bars.push(Bar {
            open: o,
            high: h,
            low: l,
            close: c,
            volume: v,
            date: format!("D{}", i),
        });
        price = c;
    }
    OhlcvData {
        bars,
        symbol: None,
        interval: None,
        session: None,
    }
}

/// Length in seconds of a bar interval in the grammar of `interval::Interval`
/// ("1m", "45m", "4h", "2D", "1w"/"1wk", "1M"/"1mo", ...). Months are
/// approximated as 30 days. `None` for anything else, so callers can reject
/// the timeframe instead of doing arithmetic on it.
pub fn interval_secs(interval: &str) -> Option<i64> {
    Interval::parse(interval).ok().map(|iv| iv.secs())
}

/// New York Stock Exchange market holidays, as (year, month, day), from its
/// published calendar (https://www.nyse.com/markets/hours-calendars, read
/// 2026-11-06). Nasdaq closes on the same days. It ends with 2028: past that,
/// only weekends are skipped. Unscheduled closures cannot be listed.
const US_HOLIDAYS: [(i32, u32, u32); 29] = [
    (2026, 1, 1),
    (2026, 1, 19),
    (2026, 2, 16),
    (2026, 4, 3),
    (2026, 5, 25),
    (2026, 6, 19),
    (2026, 7, 3),
    (2026, 9, 7),
    (2026, 11, 26),
    (2026, 12, 25),
    (2027, 1, 1),
    (2027, 1, 18),
    (2027, 2, 15),
    (2027, 3, 26),
    (2027, 5, 31),
    (2027, 6, 18),
    (2027, 7, 5),
    (2027, 9, 6),
    (2027, 11, 25),
    (2027, 12, 24),
    (2028, 1, 17),
    (2028, 2, 21),
    (2028, 4, 14),
    (2028, 5, 29),
    (2028, 6, 19),
    (2028, 7, 4),
    (2028, 9, 4),
    (2028, 11, 23),
    (2028, 12, 25),
];

/// Days the same calendar closes the market at 13:00 New York time.
const US_EARLY_CLOSES: [(i32, u32, u32); 5] = [
    (2026, 11, 27),
    (2026, 12, 24),
    (2027, 11, 26),
    (2028, 7, 3),
    (2028, 11, 24),
];

fn listed(table: &[(i32, u32, u32)], d: NaiveDate) -> bool {
    table.contains(&(d.year(), d.month(), d.day()))
}

/// An exchange's regular session, read from the data source's metadata.
/// Times are the exchange's own, so its DST changes need no special case.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Session {
    pub tz: chrono_tz::Tz,
    pub open: NaiveTime,
    pub close: NaiveTime,
    /// A US-listed instrument: `US_HOLIDAYS` and `US_EARLY_CLOSES` apply.
    pub us_calendar: bool,
}

impl Session {
    fn close_on(&self, d: NaiveDate) -> NaiveTime {
        if self.us_calendar && listed(&US_EARLY_CLOSES, d) {
            NaiveTime::from_hms_opt(13, 0, 0).unwrap_or(self.close)
        } else {
            self.close
        }
    }
}

/// Which weekdays (Monday first) a market with these bar dates is closed, for
/// bars of `n` days. Saturday and Sunday each close when no bar falls on them, and
/// a Sunday with neither a Friday nor a Saturday is a Friday and Saturday weekend,
/// which finds such markets without a list of them. No other weekday ever closes,
/// so a holiday in a short window stays a holiday. A market with a `session` is
/// known to have closed days. Without one, as for 24/7 crypto, it takes bars that
/// could have shown them: a week of daily bars, or five cycles of 7 longer ones.
fn closed_weekdays(days: &[NaiveDate], n: u32, has_session: bool) -> [bool; 7] {
    let mut seen = [false; 7];
    for d in days {
        seen[d.weekday().num_days_from_monday() as usize] = true;
    }
    let span = match (days.first(), days.last()) {
        (Some(a), Some(b)) => (*b - *a).num_days(),
        _ => 0,
    };
    let can_show = (n == 1 && span >= 7) || span >= 5 * 7 * i64::from(n);
    if !has_session && !can_show {
        return [false; 7];
    }
    let (fri, sat, sun) = (seen[4], seen[5], seen[6]);
    let friday = sun && !fri && !sat;
    [false, false, false, false, friday, !sat, !sun]
}

/// Open times (unix s) of the `n` bars after the last bar of `data`, for a
/// projection past it (ADR-0033, ADR-0038). Month-based bars step by calendar
/// months. Day-based bars stay on their grid and skip days the market is closed.
/// Intraday bars of a market with a `session` stay inside it: they step from the
/// last bar to the close, then restart at the next open. Without one (24/7) they
/// step on regardless. `None` without bar times or a known interval.
pub fn future_bar_times(data: &OhlcvData, n: usize) -> Option<Vec<i64>> {
    use chrono::{DateTime, Months};
    let last = data.bars.last()?.date.parse::<i64>().ok()?;
    let iv = Interval::parse(data.interval.as_deref()?).ok()?;
    if iv.unit == Unit::Month {
        let start = DateTime::from_timestamp(last, 0)?;
        return (1..=n as u32)
            .map(|k| Some(start.checked_add_months(Months::new(iv.n * k))?.timestamp()))
            .collect();
    }
    let times: Vec<i64> = data
        .bars
        .iter()
        .filter_map(|b| b.date.parse().ok())
        .collect();
    if iv.is_date_bar() {
        return date_bar_times(data, &iv, &times, last, n);
    }
    if let Some(s) = &data.session {
        return session_bar_times(s, iv.secs(), &times, last, n);
    }
    Some((1..=n as i64).map(|k| last + k * iv.secs()).collect())
}

/// Date bars are stamped with their date in UTC (see `fetch::local_date_stamp`).
fn date_bar_times(
    data: &OhlcvData,
    iv: &Interval,
    times: &[i64],
    last: i64,
    n: usize,
) -> Option<Vec<i64>> {
    const DAY: i64 = 86_400;
    let date = |t: i64| chrono::DateTime::from_timestamp(t, 0).map(|d| d.date_naive());
    let days: Vec<NaiveDate> = times.iter().filter_map(|t| date(*t)).collect();
    let closed_day = closed_weekdays(&days, iv.n, data.session.is_some());
    let us = data.session.is_some_and(|s| s.us_calendar);
    let closed = |t: i64| {
        date(t).is_some_and(|d| {
            closed_day[d.weekday().num_days_from_monday() as usize]
                || (us && listed(&US_HOLIDAYS, d))
        })
    };
    let tod = last.rem_euclid(DAY);
    let mut grid = iv.grid_start(last);
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        grid += iv.secs();
        if iv.unit == Unit::Week {
            out.push(grid + tod);
        } else if let Some(t) = (0..i64::from(iv.n))
            .map(|k| grid + k * DAY + tod)
            .find(|t| !closed(*t))
        {
            out.push(t);
        }
    }
    Some(out)
}

fn session_bar_times(
    s: &Session,
    step: i64,
    times: &[i64],
    last: i64,
    n: usize,
) -> Option<Vec<i64>> {
    let local = |t: i64| s.tz.timestamp_opt(t, 0).single().map(|d| d.naive_local());
    let days: Vec<NaiveDate> = times
        .iter()
        .filter_map(|t| Some(local(*t)?.date()))
        .collect();
    let closed_day = closed_weekdays(&days, 1, true);
    let closed = |d: NaiveDate| {
        closed_day[d.weekday().num_days_from_monday() as usize]
            || (s.us_calendar && listed(&US_HOLIDAYS, d))
    };
    let mut t = last;
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let (here, there) = (local(t)?, local(t + step)?);
        let inside = there.date() == here.date()
            && !closed(here.date())
            && there.time() >= s.open
            && there.time() < s.close_on(here.date());
        t = if inside {
            t + step
        } else {
            let mut d = here.date().succ_opt()?;
            while closed(d) {
                d = d.succ_opt()?;
            }
            s.tz.from_local_datetime(&d.and_time(s.open))
                .earliest()?
                .timestamp()
        };
        out.push(t);
    }
    Some(out)
}

/// Bar length in seconds for indicator math (HV annualization, VWAP and
/// pivot anchoring). Unlike the strict `interval_secs` (which gates what the
/// data sources accept), this takes any multiple and any letter case:
/// "1D", "1W", "2w", "4H", "1y", "D". A capital `M` means months and a
/// lowercase `m` minutes (TradingView / Binance convention); "mo" is months
/// in any case. Months are 30 days, years 365. `None` when unrecognised:
/// callers treat that as an unknown timeframe, not as intraday.
pub fn bar_interval_secs(interval: &str) -> Option<i64> {
    const DAY: i64 = 86_400;
    let s = interval.trim();
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let k: i64 = if num.is_empty() { 1 } else { num.parse().ok()? };
    let unit_secs = match unit {
        "m" => 60,
        "M" => 30 * DAY,
        u => match u.to_ascii_lowercase().as_str() {
            "s" => 1,
            "min" => 60,
            "h" => 3_600,
            "d" => DAY,
            "w" | "wk" => 7 * DAY,
            "mo" => 30 * DAY,
            "y" => 365 * DAY,
            _ => return None,
        },
    };
    (k > 0).then(|| k.checked_mul(unit_secs)).flatten()
}

/// Offset of the sample bar grid from the epoch, as on `Interval::grid_start`:
/// weekly bars open on Monday (1970-01-01 was a Thursday), day bars on epoch
/// day 1, like Binance's 3d. Sample months are a fixed 30 days from the epoch.
fn grid_offset_ms(iv: &Interval) -> i64 {
    match iv.unit {
        Unit::Week => 4 * 86_400_000,
        Unit::Day => 86_400_000,
        _ => 0,
    }
}

/// splitmix64 finalizer: a stateless hash so prices are a pure function of
/// (symbol, timestamp).
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Uniform in [0, 1) from a hash.
fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Close of the completed bar opening at `t` (unix seconds): three
/// sinusoids in bar units plus per-bar hash noise.
fn grid_close(seed: u64, t: i64, step: i64) -> f64 {
    use std::f64::consts::TAU;
    let x = t as f64 / step as f64;
    let phase = unit(mix(seed)) * TAU;
    let base = 50.0 + (seed % 200) as f64;
    let wave = 0.25 * (x / 600.0 * TAU + phase).sin()
        + 0.12 * (x / 150.0 * TAU + 2.0 * phase).sin()
        + 0.05 * (x / 37.0 * TAU + 3.0 * phase).sin();
    let noise = (unit(mix(seed ^ t as u64)) - 0.5) * 0.02;
    base * (1.0 + wave + noise)
}

/// Deterministic, network-free OHLCV source (`CHARTGEN_DATA_SOURCE=sample`).
///
/// Bars sit on the epoch-aligned `interval` grid (weeks start on Monday;
/// months are a fixed 30-day approximation). The newest bar is the last grid slot whose
/// open time is strictly before `end_time_ms`, capped at the bar forming at
/// `now_ms`. Every completed bar is a pure function of its open time and
/// `symbol`, so overlapping pages agree exactly. The forming bar moves
/// towards its final close by elapsed time and wobbles with wall-clock
/// seconds so a live poll observes change. No bars exist before the unix
/// epoch, so paging eventually runs out of history.
pub fn sample_ohlcv(
    symbol: &str,
    interval: &str,
    count: usize,
    end_time_ms: Option<i64>,
    now_ms: i64,
) -> Result<OhlcvData, String> {
    let iv = Interval::parse(interval)?;
    let step = iv.secs();
    let step_ms = step * 1000;
    // FNV-1a: stable across Rust versions, unlike DefaultHasher.
    let seed = symbol.bytes().fold(0xCBF2_9CE4_8422_2325_u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01B3)
    });

    let off = grid_offset_ms(&iv);
    let slot = |ms: i64| {
        ms.saturating_sub(off)
            .div_euclid(step_ms)
            .saturating_mul(step_ms)
            .saturating_add(off)
    };
    let current_open_ms = slot(now_ms);
    let last_open_ms = match end_time_ms {
        Some(end) => slot(end.saturating_sub(1)).min(current_open_ms),
        None => current_open_ms,
    };
    let available = if last_open_ms < 0 {
        0
    } else {
        ((last_open_ms - off) / step_ms + 1) as usize
    };
    let n = count.min(available);

    let bars = (0..n)
        .map(|i| {
            let open_ms = last_open_ms - (n - 1 - i) as i64 * step_ms;
            let t = open_ms / 1000;
            let o = grid_close(seed, t - step, step);
            let c = grid_close(seed, t, step);
            let wick_hi = unit(mix(seed ^ t as u64 ^ 0x4849)) * 0.006;
            let wick_lo = unit(mix(seed ^ t as u64 ^ 0x4C4F)) * 0.006;
            let h = o.max(c) * (1.0 + wick_hi);
            let l = o.min(c) * (1.0 - wick_lo);
            let v = 1_000.0 * (1.0 + 4.0 * unit(mix(seed ^ t as u64 ^ 0x564F)));
            if open_ms != current_open_ms {
                return Bar {
                    open: o,
                    high: h,
                    low: l,
                    close: c,
                    volume: v,
                    date: t.to_string(),
                };
            }
            let frac = ((now_ms - open_ms) as f64 / step_ms as f64).clamp(0.0, 1.0);
            let wobble = o * 0.003 * (now_ms as f64 / 1000.0).floor().sin();
            let close = o + (c - o) * frac + wobble;
            Bar {
                open: o,
                high: (o.max(c) + (h - o.max(c)) * frac).max(close),
                low: (o.min(c) - (o.min(c) - l) * frac).min(close),
                close,
                volume: v * frac,
                date: t.to_string(),
            }
        })
        .collect();

    Ok(OhlcvData {
        bars,
        symbol: None,
        interval: None,
        session: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR_MS: i64 = 3_600_000;

    fn at(times: &[i64], interval: &str) -> OhlcvData {
        OhlcvData {
            bars: times
                .iter()
                .map(|t| Bar {
                    open: 1.0,
                    high: 1.0,
                    low: 1.0,
                    close: 1.0,
                    volume: 1.0,
                    date: t.to_string(),
                })
                .collect(),
            symbol: None,
            interval: Some(interval.into()),
            session: None,
        }
    }

    #[test]
    fn future_bar_times_follow_the_calendar() {
        const DAY: i64 = 86_400;
        let ts = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp();
        // 4h: plain steps.
        let t = ts("2026-11-05T00:00:00Z");
        assert_eq!(
            future_bar_times(&at(&[t], "4h"), 2),
            Some(vec![t + 14_400, t + 28_800])
        );
        // 1M: calendar months, so slot 2 is the start of the month after next.
        let nov = ts("2026-11-01T00:00:00Z");
        let m = future_bar_times(&at(&[nov], "1M"), 3).unwrap();
        assert_eq!(m[0], ts("2026-12-01T00:00:00Z"));
        assert_eq!(m[1], ts("2027-01-01T00:00:00Z"));
        assert_eq!(m[2], ts("2027-02-01T00:00:00Z"));
        // 3mo: quarters.
        let q = future_bar_times(&at(&[nov], "3mo"), 2).unwrap();
        assert_eq!(
            q,
            vec![ts("2027-02-01T00:00:00Z"), ts("2027-05-01T00:00:00Z")]
        );
        // 1d of a stock: Thu, Fri, then Mon.
        let wed = ts("2026-11-04T14:30:00Z");
        let thu = wed + DAY;
        let stock = ny(&[wed - DAY, wed], "1d");
        assert_eq!(
            future_bar_times(&stock, 3),
            Some(vec![thu, thu + DAY, thu + 4 * DAY])
        );
        // 1d with weekend bars (crypto): every day.
        let sun = ts("2026-11-01T00:00:00Z");
        let crypto = at(&[sun, sun + DAY], "1d");
        assert_eq!(
            future_bar_times(&crypto, 6),
            Some((2..=7).map(|k| sun + k * DAY).collect()),
            "Saturday and Sunday stay"
        );
        // Intraday bars never skip.
        let fri = ts("2026-11-06T20:00:00Z");
        assert_eq!(
            future_bar_times(&at(&[fri], "4h"), 1),
            Some(vec![fri + 14_400])
        );
        // Custom intervals step like any other.
        assert_eq!(
            future_bar_times(&at(&[t], "7m"), 2),
            Some(vec![t + 420, t + 840])
        );
        let m = future_bar_times(&at(&[nov], "2mo"), 2).unwrap();
        assert_eq!(
            m,
            vec![ts("2027-01-01T00:00:00Z"), ts("2027-03-01T00:00:00Z")]
        );
        // No time or unknown interval: none.
        assert_eq!(future_bar_times(&at(&[t], "61m"), 2), None);
        let mut untimed = at(&[t], "1d");
        untimed.bars[0].date = "D0".into();
        assert_eq!(future_bar_times(&untimed, 2), None);
    }

    /// `bars` in the session's own zone; dates are local.
    fn ts_in(tz: chrono_tz::Tz, s: &str) -> i64 {
        tz.from_local_datetime(&chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap())
            .single()
            .unwrap()
            .timestamp()
    }

    fn in_session(
        mut d: OhlcvData,
        tz: chrono_tz::Tz,
        open: (u32, u32),
        close: (u32, u32),
    ) -> OhlcvData {
        d.session = Some(Session {
            tz,
            open: NaiveTime::from_hms_opt(open.0, open.1, 0).unwrap(),
            close: NaiveTime::from_hms_opt(close.0, close.1, 0).unwrap(),
            us_calendar: tz == chrono_tz::America::New_York,
        });
        d
    }

    fn ny(times: &[i64], interval: &str) -> OhlcvData {
        in_session(
            at(times, interval),
            chrono_tz::America::New_York,
            (9, 30),
            (16, 0),
        )
    }

    fn local_times(d: &OhlcvData, tz: chrono_tz::Tz, n: usize) -> Vec<String> {
        future_bar_times(d, n)
            .unwrap()
            .iter()
            .map(|t| {
                tz.timestamp_opt(*t, 0)
                    .single()
                    .unwrap()
                    .format("%a %m-%d %H:%M")
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn a_24h_market_stays_continuous_through_nights_and_weekends() {
        let ts = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp();
        let fri = ts("2026-11-06T22:00:00Z");
        // Crypto: no session, so the night and the weekend are bars like any other.
        let t = future_bar_times(&at(&[fri - 3_600, fri], "1h"), 60).unwrap();
        assert_eq!(t, (1..=60).map(|k| fri + k * 3_600).collect::<Vec<_>>());
        // Custom lengths too, and a holiday in the US table means nothing here.
        let eve = ts("2026-11-25T23:55:00Z");
        let t = future_bar_times(&at(&[eve], "7m"), 3).unwrap();
        assert_eq!(t, vec![eve + 420, eve + 840, eve + 1_260]);
        let wed = ts("2026-11-25T00:00:00Z");
        let t = future_bar_times(&at(&[wed - 86_400, wed], "1d"), 1).unwrap();
        assert_eq!(
            t,
            vec![wed + 86_400],
            "Thanksgiving is a day like any other"
        );
    }

    #[test]
    fn intraday_stock_bars_stay_in_the_session() {
        let tz = chrono_tz::America::New_York;
        // 1h from a Monday: to the 15:30 bar, then the next day's open.
        let d = ny(&[ts_in(tz, "2026-11-09 14:30")], "1h");
        assert_eq!(
            local_times(&d, tz, 3),
            ["Mon 11-09 15:30", "Tue 11-10 09:30", "Tue 11-10 10:30"]
        );
        // Friday's close leads to Monday.
        let d = ny(&[ts_in(tz, "2026-11-13 15:30")], "1h");
        assert_eq!(local_times(&d, tz, 1), ["Mon 11-16 09:30"]);
        // A custom length keeps the session's grid, and ends before the close.
        let d = ny(&[ts_in(tz, "2026-11-09 15:30")], "45m");
        assert_eq!(
            local_times(&d, tz, 3),
            ["Tue 11-10 09:30", "Tue 11-10 10:15", "Tue 11-10 11:00"]
        );
        let d = ny(&[ts_in(tz, "2026-11-09 14:38")], "7m");
        assert_eq!(local_times(&d, tz, 1), ["Mon 11-09 14:45"]);
        // A bar longer than the session is one bar per session.
        let d = ny(&[ts_in(tz, "2026-11-09 09:30")], "8h");
        assert_eq!(
            local_times(&d, tz, 2),
            ["Tue 11-10 09:30", "Wed 11-11 09:30"]
        );
    }

    #[test]
    fn every_future_bar_of_a_long_stock_projection_is_in_the_session() {
        let tz = chrono_tz::America::New_York;
        let d = ny(&[ts_in(tz, "2026-10-26 09:30")], "90m");
        let t = future_bar_times(&d, 500).unwrap();
        assert_eq!(t.len(), 500);
        assert!(t.windows(2).all(|w| w[0] < w[1]));
        for x in t {
            let l = tz.timestamp_opt(x, 0).single().unwrap().naive_local();
            let date = l.date();
            assert!(date.weekday().num_days_from_monday() < 5, "{l}");
            assert!(!listed(&US_HOLIDAYS, date), "{l}");
            let open = NaiveTime::from_hms_opt(9, 30, 0).unwrap();
            let close = d.session.unwrap().close_on(date);
            assert!(l.time() >= open && l.time() < close, "{l}");
        }
    }

    #[test]
    fn stock_sessions_keep_their_local_hours_across_dst() {
        let tz = chrono_tz::America::New_York;
        // US clocks go back on 2026-11-01: the open moves from 13:30 to 14:30 UTC.
        let fri = ts_in(tz, "2026-10-30 15:30");
        let t = future_bar_times(&ny(&[fri], "1h"), 1).unwrap();
        let want = chrono::DateTime::parse_from_rfc3339("2026-11-02T14:30:00Z").unwrap();
        assert_eq!(t[0], want.timestamp());
    }

    #[test]
    fn us_holidays_and_early_closes_are_skipped() {
        let tz = chrono_tz::America::New_York;
        // Thanksgiving is closed, and the Friday after closes at 13:00.
        let d = ny(&[ts_in(tz, "2026-11-25 15:30")], "1h");
        assert_eq!(
            local_times(&d, tz, 6),
            [
                "Fri 11-27 09:30",
                "Fri 11-27 10:30",
                "Fri 11-27 11:30",
                "Fri 11-27 12:30",
                "Mon 11-30 09:30",
                "Mon 11-30 10:30",
            ]
        );
        // Date bars skip them too: Christmas 2026 is a Friday.
        let d = ny(&[ts_in(tz, "2026-12-22 00:00") - 5 * 3_600], "1d");
        let t = future_bar_times(&d, 4).unwrap();
        let date = |x: i64| {
            chrono::DateTime::from_timestamp(x, 0)
                .unwrap()
                .format("%m-%d")
                .to_string()
        };
        assert_eq!(
            t.iter().map(|x| date(*x)).collect::<Vec<_>>(),
            ["12-23", "12-24", "12-28", "12-29"]
        );
        // Past the table's end it degrades to weekends: 2029-01-01 is not listed.
        let d = ny(&[ts_in(tz, "2028-12-29 15:30")], "1h");
        assert_eq!(local_times(&d, tz, 1), ["Mon 01-01 09:30"]);
    }

    /// The n-th (from 1) `weekday` of a month.
    fn nth(year: i32, month: u32, weekday: chrono::Weekday, n: u8) -> NaiveDate {
        NaiveDate::from_weekday_of_month_opt(year, month, weekday, n).unwrap()
    }

    /// Easter Sunday: the anonymous Gregorian computus.
    fn easter(y: i32) -> NaiveDate {
        let (a, b, c) = (y % 19, y / 100, y % 100);
        let (d, e) = (b / 4, b % 4);
        let g = (8 * b + 13) / 25;
        let h = (19 * a + b - d - g + 15) % 30;
        let (i, k) = (c / 4, c % 4);
        let l = (32 + 2 * e + 2 * i - h - k) % 7;
        let m = (a + 11 * h + 19 * l) / 433;
        let month = (h + l - 7 * m + 90) / 25;
        let day = (h + l - 7 * m + 33 * month + 19) % 32;
        NaiveDate::from_ymd_opt(y, month as u32, day as u32).unwrap()
    }

    /// Saturday's holiday is observed on Friday, Sunday's on Monday.
    fn observed(d: NaiveDate) -> NaiveDate {
        match d.weekday() {
            chrono::Weekday::Sat => d.pred_opt().unwrap(),
            chrono::Weekday::Sun => d.succ_opt().unwrap(),
            _ => d,
        }
    }

    #[test]
    fn the_us_tables_match_the_rules_the_holidays_follow() {
        use chrono::Weekday::{Mon, Thu};
        // Derived independently of the table, so a wrong date cannot pass for being
        // a weekday. New Year's Day on a Saturday is not observed, as its calendar says.
        let mut want = Vec::new();
        let mut early = Vec::new();
        for y in 2026..=2028 {
            let date = |m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
            if date(1, 1).weekday() != chrono::Weekday::Sat {
                want.push(observed(date(1, 1)));
            }
            let thanksgiving = nth(y, 11, Thu, 4);
            want.extend([
                nth(y, 1, Mon, 3),
                nth(y, 2, Mon, 3),
                easter(y) - chrono::Days::new(2),
                (25..=31)
                    .rev()
                    .map(|d| date(5, d))
                    .find(|d| d.weekday() == Mon)
                    .unwrap(),
                observed(date(6, 19)),
                observed(date(7, 4)),
                nth(y, 9, Mon, 1),
                thanksgiving,
                observed(date(12, 25)),
            ]);
            early.push(thanksgiving + chrono::Days::new(1));
        }
        let table = |t: &[(i32, u32, u32)]| -> Vec<NaiveDate> {
            t.iter()
                .map(|&(y, m, d)| NaiveDate::from_ymd_opt(y, m, d).unwrap())
                .collect()
        };
        want.sort();
        assert_eq!(table(&US_HOLIDAYS), want);
        // The day after Thanksgiving closes early every year. The others are the
        // eve of a midweek holiday, which the calendar decides year by year.
        let closes = table(&US_EARLY_CLOSES);
        assert!(early.iter().all(|d| closes.contains(d)), "{closes:?}");
        assert_eq!(
            closes
                .iter()
                .filter(|d| !early.contains(d))
                .collect::<Vec<_>>(),
            vec![
                &NaiveDate::from_ymd_opt(2026, 12, 24).unwrap(),
                &NaiveDate::from_ymd_opt(2028, 7, 3).unwrap(),
            ]
        );
    }

    #[test]
    fn other_exchanges_get_hours_but_no_holidays() {
        let tz = chrono_tz::Asia::Tokyo;
        let d = in_session(
            at(&[ts_in(tz, "2026-11-06 15:00")], "1h"),
            tz,
            (9, 0),
            (15, 30),
        );
        assert_eq!(
            local_times(&d, tz, 2),
            ["Mon 11-09 09:00", "Mon 11-09 10:00"]
        );
        // Culture Day, 2026-11-03, is a Tokyo holiday this table does not hold.
        let d = in_session(
            at(&[ts_in(tz, "2026-11-02 15:00")], "1h"),
            tz,
            (9, 0),
            (15, 30),
        );
        assert_eq!(local_times(&d, tz, 1), ["Tue 11-03 09:00"]);
    }

    #[test]
    fn trading_days_come_from_the_bars() {
        // Riyadh trades Sunday to Thursday, 10:00 to 15:00.
        let tz = chrono_tz::Asia::Riyadh;
        let thu = ts_in(tz, "2026-11-05 14:00");
        let bars: Vec<i64> = (0..30)
            .rev()
            .map(|k| thu - k * 86_400)
            .filter(|t| {
                let d = tz.timestamp_opt(*t, 0).single().unwrap().weekday();
                !matches!(d, chrono::Weekday::Fri | chrono::Weekday::Sat)
            })
            .collect();
        let d = in_session(at(&bars, "1h"), tz, (10, 0), (15, 0));
        assert_eq!(local_times(&d, tz, 1), ["Sun 11-08 10:00"]);
    }

    #[test]
    fn short_windows_of_a_sunday_to_thursday_market_keep_its_weekend() {
        let tz = chrono_tz::Asia::Riyadh;
        let riyadh = |t: &[i64]| in_session(at(t, "1h"), tz, (10, 0), (15, 0));
        // A week of bars, as a 1m, 7m or 2m chart loads: Thursday leads to Sunday.
        let thu = ts_in(tz, "2026-11-05 14:00");
        let sun = ts_in(tz, "2026-11-01 14:00");
        let week = [sun, sun + 86_400, sun + 2 * 86_400, sun + 3 * 86_400, thu];
        assert_eq!(local_times(&riyadh(&week), tz, 1), ["Sun 11-08 10:00"]);
        // A Sunday alone is enough to tell: no bar lands on a Friday or Saturday.
        for t in local_times(&riyadh(&[sun - 4 * 3_600]), tz, 40) {
            assert!(!t.starts_with("Fri") && !t.starts_with("Sat"), "{t}");
        }
        // A full load and a shorter tail of the same chart agree.
        let load: Vec<i64> = (0..50)
            .rev()
            .map(|k| thu - k * 86_400)
            .filter(|t| {
                let d = tz.timestamp_opt(*t, 0).single().unwrap().weekday();
                !matches!(d, chrono::Weekday::Fri | chrono::Weekday::Sat)
            })
            .collect();
        let tail = &load[load.len() - 10..];
        assert_eq!(
            future_bar_times(&riyadh(&load), 12),
            future_bar_times(&riyadh(tail), 12)
        );
    }

    #[test]
    fn saturday_and_sunday_close_independently() {
        let tz = chrono_tz::America::New_York;
        // A market seen trading on a Saturday keeps it, and closes Sunday.
        let sat = ts_in(tz, "2026-11-07 15:30");
        let d = ny(&[sat - 86_400, sat], "1h");
        assert_eq!(local_times(&d, tz, 1), ["Mon 11-09 09:30"]);
        let fri = ts_in(tz, "2026-11-13 15:30");
        let d = ny(&[sat, fri], "1h");
        assert_eq!(local_times(&d, tz, 1), ["Sat 11-14 09:30"]);
        // One seen trading on a Sunday and a Friday closes Saturday alone.
        let sun = ts_in(tz, "2026-11-08 15:30");
        let d = ny(&[sun, fri], "1h");
        assert_eq!(local_times(&d, tz, 1), ["Sun 11-15 09:30"]);
    }

    #[test]
    fn a_holiday_inside_a_short_window_is_not_a_closed_weekday() {
        // A week with Thanksgiving in it has no Thursday bar, and the Thursday
        // after is open.
        let tz = chrono_tz::America::New_York;
        let times: Vec<i64> = [
            "2026-11-23 15:30",
            "2026-11-24 15:30",
            "2026-11-25 15:30",
            "2026-11-27 12:30",
            "2026-11-30 15:30",
            "2026-12-01 15:30",
            "2026-12-02 15:30",
        ]
        .iter()
        .map(|s| ts_in(tz, s))
        .collect();
        let d = ny(&times, "1h");
        assert_eq!(local_times(&d, tz, 1), ["Thu 12-03 09:30"]);
    }

    #[test]
    fn crypto_date_bars_stay_on_their_grid_however_few_are_loaded() {
        const DAY: i64 = 86_400;
        let ts = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp();
        let mon = ts("2026-11-02T00:00:00Z");
        // Five 1d bars, Monday to Friday, with no session: Saturday is a bar.
        let week: Vec<i64> = (0..5).map(|k| mon + k * DAY).collect();
        assert_eq!(
            future_bar_times(&at(&week, "1d"), 3),
            Some(vec![mon + 5 * DAY, mon + 6 * DAY, mon + 7 * DAY])
        );
        // The two-bar tail CodeRabbit named.
        let fri = mon + 4 * DAY;
        assert_eq!(
            future_bar_times(&at(&[fri - DAY, fri], "1d"), 2),
            Some(vec![fri + DAY, fri + 2 * DAY])
        );
        // A few long bars show too few weekdays to judge any of them closed.
        for (tf, step, bars) in [("5d", 5, 6), ("10d", 10, 4), ("30d", 30, 3)] {
            let iv = Interval::parse(tf).unwrap();
            let last = iv.grid_start(mon);
            let times: Vec<i64> = (0..bars).rev().map(|k| last - k * step * DAY).collect();
            let t = future_bar_times(&at(&times, tf), 8).unwrap();
            assert_eq!(
                t,
                (1..=8).map(|k| last + k * step * DAY).collect::<Vec<_>>(),
                "{tf}"
            );
        }
    }

    #[test]
    fn without_a_session_only_enough_bars_can_close_a_day() {
        const DAY: i64 = 86_400;
        let ts = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp();
        // Two weeks of weekdays, with metadata missing: the bars show a weekend.
        let mon = ts("2026-11-02T00:00:00Z");
        let days: Vec<i64> = (0..12)
            .map(|k| mon + k * DAY)
            .filter(|t| {
                let d = chrono::DateTime::from_timestamp(*t, 0).unwrap().weekday();
                !matches!(d, chrono::Weekday::Sat | chrono::Weekday::Sun)
            })
            .collect();
        assert_eq!(days.last(), Some(&(mon + 11 * DAY)));
        assert_eq!(
            future_bar_times(&at(&days, "1d"), 3),
            Some(vec![mon + 14 * DAY, mon + 15 * DAY, mon + 16 * DAY]),
            "Friday, then Monday and Tuesday"
        );
    }

    #[test]
    fn multi_day_stock_bars_land_on_the_first_open_day_of_their_grid_slot() {
        const DAY: i64 = 86_400;
        let ts = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp();
        let iv = Interval::parse("3d").unwrap();
        // A stock's 3d bars with no weekend bars: each future bar is on the grid
        // or the first weekday of its slot, and none falls on a weekend.
        let wed = ts("2026-11-04T00:00:00Z");
        let t = future_bar_times(&ny(&[wed - DAY, wed], "3d"), 12).unwrap();
        let mut slots = std::collections::HashSet::new();
        for x in &t {
            let d = chrono::DateTime::from_timestamp(*x, 0).unwrap();
            assert!(d.weekday().num_days_from_monday() < 5, "{d}");
            assert!(slots.insert(iv.grid_start(*x)), "one bar per slot: {d}");
        }
        // Weekly bars stay on Mondays.
        let mon = ts("2026-11-02T00:00:00Z");
        assert_eq!(
            future_bar_times(&at(&[mon], "1wk"), 2).unwrap(),
            vec![mon + 7 * DAY, mon + 14 * DAY]
        );
    }

    #[test]
    fn bar_interval_secs_is_case_insensitive_and_strict_about_unknowns() {
        const DAY: i64 = 86_400;
        assert_eq!(bar_interval_secs("1D"), Some(DAY));
        assert_eq!(bar_interval_secs("1d"), Some(DAY));
        assert_eq!(bar_interval_secs("D"), Some(DAY));
        assert_eq!(bar_interval_secs("1W"), Some(7 * DAY));
        assert_eq!(bar_interval_secs("2w"), Some(14 * DAY));
        assert_eq!(bar_interval_secs("1WK"), Some(7 * DAY));
        assert_eq!(bar_interval_secs("4H"), Some(14_400));
        assert_eq!(bar_interval_secs("15m"), Some(900));
        assert_eq!(bar_interval_secs("1M"), Some(30 * DAY));
        assert_eq!(bar_interval_secs("3MO"), Some(90 * DAY));
        assert_eq!(bar_interval_secs("1y"), Some(365 * DAY));
        assert_eq!(bar_interval_secs("0d"), None);
        assert_eq!(bar_interval_secs("1x"), None);
        assert_eq!(bar_interval_secs(""), None);
        assert_eq!(bar_interval_secs("99999999999999999999d"), None);
    }

    #[test]
    fn interval_secs_parses_binance_and_yahoo_forms() {
        assert_eq!(interval_secs("1m"), Some(60));
        assert_eq!(interval_secs("90m"), Some(5_400));
        assert_eq!(interval_secs("4h"), Some(14_400));
        assert_eq!(interval_secs("1d"), Some(86_400));
        assert_eq!(interval_secs("1w"), Some(604_800));
        assert_eq!(interval_secs("1wk"), Some(604_800));
        assert_eq!(interval_secs("1M"), Some(2_592_000));
        assert_eq!(interval_secs("1mo"), Some(2_592_000));
        assert_eq!(interval_secs("h"), None);
        assert_eq!(interval_secs("0h"), None);
        assert_eq!(interval_secs("1x"), None);
        // Absurd multiples are rejected rather than overflowing.
        assert_eq!(interval_secs("4611686018427387904m"), None);
        assert_eq!(interval_secs("61m"), None);
        // Custom intervals are the grammar's.
        assert_eq!(interval_secs("7h"), Some(25_200));
        assert_eq!(interval_secs("2D"), Some(172_800));
        assert_eq!(interval_secs("2W"), Some(1_209_600));
    }

    #[test]
    fn sample_ohlcv_aligns_to_grid_and_respects_end_time() {
        let now = 1_750_000_123_456;
        // end_time off the grid: last bar is the slot strictly before it.
        let end = 1_700_000_000_000 + 1_234;
        let d = sample_ohlcv("BTCUSDT", "4h", 50, Some(end), now).unwrap();
        assert_eq!(d.len(), 50);
        let ts: Vec<i64> = d.bars.iter().map(|b| b.date.parse().unwrap()).collect();
        for w in ts.windows(2) {
            assert_eq!(w[1] - w[0], 14_400);
        }
        assert!(ts.iter().all(|t| t % 14_400 == 0));
        assert!(ts.last().unwrap() * 1000 < end);
        assert!(ts.last().unwrap() * 1000 + 14_400_000 >= end);

        // end_time on the grid is exclusive.
        let on_grid = 1_700_006_400_000;
        assert_eq!(on_grid % (4 * HOUR_MS), 0);
        let d = sample_ohlcv("BTCUSDT", "4h", 3, Some(on_grid), now).unwrap();
        let last: i64 = d.bars.last().unwrap().date.parse().unwrap();
        assert_eq!(last * 1000, on_grid - 4 * HOUR_MS);
    }

    #[test]
    fn sample_ohlcv_serves_every_supported_interval() {
        let now = 1_750_000_123_456;
        for (tf, secs) in [
            ("3m", 180),
            ("2h", 7_200),
            ("6h", 21_600),
            ("12h", 43_200),
            ("3d", 259_200),
            ("1mo", 2_592_000),
            ("7m", 420),
            ("90m", 5_400),
            ("8h", 28_800),
            ("2d", 172_800),
            ("2wk", 1_209_600),
        ] {
            let d = sample_ohlcv("BTCUSDT", tf, 20, None, now).unwrap();
            assert_eq!(d.len(), 20, "{tf}");
            let ts: Vec<i64> = d.bars.iter().map(|b| b.date.parse().unwrap()).collect();
            assert!(ts.windows(2).all(|w| w[1] - w[0] == secs), "{tf} spacing");
            // Binance's 3d klines open on epoch days with day % 3 == 1; weeks
            // on Monday, epoch day 4. The same as `Interval::grid_start`.
            let off = match tf {
                "3d" | "2d" => 86_400,
                "2wk" => 4 * 86_400,
                _ => 0,
            };
            if tf != "1mo" {
                let iv = Interval::parse(tf).unwrap();
                assert!(
                    ts.iter().all(|t| iv.grid_start(*t) == *t),
                    "{tf} grid_start"
                );
            }
            assert!(ts.iter().all(|t| (t - off) % secs == 0), "{tf} grid");
        }
    }

    #[test]
    fn sample_ohlcv_overlapping_pages_agree() {
        let now = 1_750_000_000_000;
        let end_a = 1_700_000_000_000;
        let a = sample_ohlcv("ETHUSDT", "1h", 100, Some(end_a), now).unwrap();
        let b = sample_ohlcv("ETHUSDT", "1h", 100, Some(end_a - 40 * HOUR_MS), now).unwrap();
        let mut overlap = 0;
        for bb in &b.bars {
            if let Some(ab) = a.bars.iter().find(|x| x.date == bb.date) {
                overlap += 1;
                assert_eq!(
                    (ab.open, ab.high, ab.low, ab.close, ab.volume),
                    (bb.open, bb.high, bb.low, bb.close, bb.volume)
                );
            }
        }
        assert_eq!(overlap, 60);
        // Different symbols produce different prices.
        let c = sample_ohlcv("SOLUSDT", "1h", 100, Some(end_a), now).unwrap();
        assert_ne!(a.bars[0].close, c.bars[0].close);
        // Candles are continuous (open == previous close) and well-formed.
        for w in a.bars.windows(2) {
            assert_eq!(w[1].open, w[0].close);
            assert!(w[1].high >= w[1].open.max(w[1].close));
            assert!(w[1].low <= w[1].open.min(w[1].close));
        }
    }

    #[test]
    fn sample_ohlcv_forming_bar_changes_with_wall_clock() {
        let open = 1_700_000_000_000 / HOUR_MS * HOUR_MS;
        let a = sample_ohlcv("BTCUSDT", "1h", 3, None, open + 600_000).unwrap();
        let b = sample_ohlcv("BTCUSDT", "1h", 3, None, open + 601_000).unwrap();
        assert_eq!(a.bars.last().unwrap().date, (open / 1000).to_string());
        // Completed bars identical, forming bar moved.
        assert_eq!(a.bars[1].close, b.bars[1].close);
        assert_ne!(a.bars[2].close, b.bars[2].close);
        assert_eq!(a.bars[2].open, b.bars[2].open);
        for bar in [&a.bars[2], &b.bars[2]] {
            assert!(bar.high >= bar.open.max(bar.close));
            assert!(bar.low <= bar.open.min(bar.close));
        }
        // end_time in the future is capped at the forming bar.
        let c = sample_ohlcv(
            "BTCUSDT",
            "1h",
            3,
            Some(open + 10 * HOUR_MS),
            open + 600_000,
        )
        .unwrap();
        assert_eq!(c.bars.last().unwrap().date, a.bars[2].date);
    }

    #[test]
    fn sample_ohlcv_weekly_bars_open_on_monday() {
        let d = sample_ohlcv("X", "1wk", 10, None, 1_750_000_000_000).unwrap();
        assert_eq!(d.len(), 10);
        for b in &d.bars {
            let day = b.date.parse::<i64>().unwrap() / 86_400;
            // Day 0 (1970-01-01) was a Thursday, so Mondays are day % 7 == 4.
            assert_eq!(day % 7, 4, "weekly bar must open on Monday");
        }
        // The first weekly bar after the epoch is Monday 1970-01-05.
        let d = sample_ohlcv("X", "1w", 5, Some(15 * 86_400_000), 1_750_000_000_000).unwrap();
        assert_eq!(d.bars[0].date, (4 * 86_400).to_string());
        assert_eq!(d.len(), 2);
        // end_time = i64::MIN saturates instead of overflowing.
        assert!(sample_ohlcv("X", "1h", 5, Some(i64::MIN), 0)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn sample_ohlcv_stops_at_epoch() {
        let d = sample_ohlcv("X", "1h", 50, Some(10 * HOUR_MS), 1_750_000_000_000).unwrap();
        assert_eq!(d.len(), 10);
        assert_eq!(d.bars[0].date, "0");
        assert!(sample_ohlcv("X", "1h", 5, Some(0), 1_750_000_000_000)
            .unwrap()
            .is_empty());
        assert!(sample_ohlcv("X", "7x", 5, None, 0).is_err());
    }
}
