//! Bar intervals (ADR-0038): the one place that says what a timeframe string
//! means, what it may cost, and where its bars open.

use chrono::{Datelike, TimeZone, Utc};

const MINUTE: i64 = 60;
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;

/// Most native bars one bar of any interval may need from Binance, which pays
/// per request where Yahoo returns one window per request. It bounds what a
/// fetch costs per bar (see `fetch::MAX_BASE_BARS`).
pub const MAX_BASE_PER_BAR: i64 = 60;

/// Binance's intraday klines in minutes, coarsest first.
const BINANCE_INTRADAY: [(i64, &str); 11] = [
    (720, "12h"),
    (480, "8h"),
    (360, "6h"),
    (240, "4h"),
    (120, "2h"),
    (60, "1h"),
    (30, "30m"),
    (15, "15m"),
    (5, "5m"),
    (3, "3m"),
    (1, "1m"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
}

/// A validated interval in its canonical form: `120m` is 2 hours, `24h` one
/// day and `7d` one week, so equal bars have one name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub n: u32,
    pub unit: Unit,
}

impl Interval {
    /// Parse `<number><unit>`: `m` minutes, `M` or `mo` months, and `s`, `h`,
    /// `d`, `w` or `wk` in any case.
    pub fn parse(text: &str) -> Result<Self, String> {
        let split = text
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len());
        let (num, unit) = text.split_at(split);
        let unit = match unit {
            "m" => Some(Unit::Minute),
            "M" => Some(Unit::Month),
            u => match u.to_ascii_lowercase().as_str() {
                "s" => Some(Unit::Second),
                "h" => Some(Unit::Hour),
                "d" => Some(Unit::Day),
                "w" | "wk" => Some(Unit::Week),
                "mo" => Some(Unit::Month),
                _ => None,
            },
        };
        let (Some(unit), Some(n)) = (unit, num.parse::<u32>().ok().filter(|n| *n > 0)) else {
            return Err(format!(
                "Unsupported timeframe: {text}. Use a number and a unit, e.g. 5s, 7m, 45m, 8h, 2D, 2W, 3M (s seconds, m minutes, h hours, D days, W weeks, M months)."
            ));
        };
        let iv = Self { n, unit }.normalized();
        let max = match iv.unit {
            Unit::Second => 59,
            Unit::Minute => 1_439,
            Unit::Hour => 168,
            Unit::Day => 365,
            Unit::Week => 52,
            Unit::Month => 12,
        };
        if iv.n > max {
            return Err(format!(
                "Unsupported timeframe: {text}. The upper bounds are under 60s, 1440m, 168h, 365D, 52W and 12M."
            ));
        }
        if matches!(iv.unit, Unit::Minute | Unit::Hour) {
            let (base, per_bar) = iv.binance_base();
            if per_bar > MAX_BASE_PER_BAR {
                return Err(format!(
                    "Unsupported timeframe: {text}. One bar would take {per_bar} {base} bars and the limit is {MAX_BASE_PER_BAR}; pick a rounder interval."
                ));
            }
        }
        Ok(iv)
    }

    fn normalized(self) -> Self {
        let Self { n, unit } = self;
        match unit {
            Unit::Second if n % 60 == 0 => Self {
                n: n / 60,
                unit: Unit::Minute,
            }
            .normalized(),
            Unit::Minute if n % 60 == 0 => Self {
                n: n / 60,
                unit: Unit::Hour,
            }
            .normalized(),
            Unit::Hour if n % 24 == 0 => Self {
                n: n / 24,
                unit: Unit::Day,
            }
            .normalized(),
            Unit::Day if n % 7 == 0 => Self {
                n: n / 7,
                unit: Unit::Week,
            },
            _ => self,
        }
    }

    /// The canonical name: what clients are echoed and what keys a chart.
    pub fn canonical(&self) -> String {
        let unit = match self.unit {
            Unit::Second => "s",
            Unit::Minute => "m",
            Unit::Hour => "h",
            Unit::Day => "d",
            Unit::Week => "wk",
            Unit::Month => "mo",
        };
        format!("{}{unit}", self.n)
    }

    /// Bar length in seconds; a month counts as 30 days.
    pub fn secs(&self) -> i64 {
        let unit = match self.unit {
            Unit::Second => 1,
            Unit::Minute => MINUTE,
            Unit::Hour => HOUR,
            Unit::Day => DAY,
            Unit::Week => 7 * DAY,
            Unit::Month => 30 * DAY,
        };
        unit * i64::from(self.n)
    }

    /// Day, week and month bars stand for a date, not a time of day.
    pub fn is_date_bar(&self) -> bool {
        matches!(self.unit, Unit::Day | Unit::Week | Unit::Month)
    }

    /// Whole months since 1970-01 of the UTC date of `t`.
    fn month_index(t: i64) -> i64 {
        let d = Utc
            .timestamp_opt(t, 0)
            .single()
            .unwrap_or(chrono::DateTime::UNIX_EPOCH);
        i64::from(d.year() - 1970) * 12 + i64::from(d.month0())
    }

    fn month_start(index: i64) -> i64 {
        let year = 1970 + index.div_euclid(12);
        let month = index.rem_euclid(12) as u32 + 1;
        Utc.with_ymd_and_hms(year as i32, month, 1, 0, 0, 0)
            .single()
            .map_or(0, |d| d.timestamp())
    }

    /// Open time of the bar holding `t` on the stable grid, so a reload gives
    /// the same bars. Minutes and hours count from the epoch. Days count from
    /// epoch day 1, where Binance's 3d klines open, and weeks from Monday
    /// 1970-01-05, where its 1w klines open. Months count from 1970-01, so 3M
    /// is a calendar quarter and 12M a calendar year.
    pub fn grid_start(&self, t: i64) -> i64 {
        let floor = |offset: i64| offset + (t - offset).div_euclid(self.secs()) * self.secs();
        match self.unit {
            Unit::Second | Unit::Minute | Unit::Hour => floor(0),
            Unit::Day => floor(DAY),
            Unit::Week => floor(4 * DAY),
            Unit::Month => {
                let n = i64::from(self.n);
                Self::month_start(Self::month_index(t).div_euclid(n) * n)
            }
        }
    }

    /// Open time of the bar after the one holding `t`.
    pub fn grid_end(&self, t: i64) -> i64 {
        match self.unit {
            Unit::Month => {
                let n = i64::from(self.n);
                Self::month_start((Self::month_index(t).div_euclid(n) + 1) * n)
            }
            _ => self.grid_start(t) + self.secs(),
        }
    }

    /// The Binance kline this interval is built from, and how many of them
    /// make one bar: the coarsest that divides it exactly, so the fewest are
    /// fetched. Each grid above lies on its base's grid, so whole klines fill
    /// a bar. A count of 1 means Binance has it natively.
    pub fn binance_base(&self) -> (&'static str, i64) {
        let n = i64::from(self.n);
        match self.unit {
            // Binance's finest, and spot only. Under a minute it always fits
            // `MAX_BASE_PER_BAR`.
            Unit::Second => ("1s", n),
            Unit::Minute | Unit::Hour => {
                let minutes = self.secs() / MINUTE;
                BINANCE_INTRADAY
                    .iter()
                    .find(|(m, _)| minutes % m == 0)
                    .map_or(("1m", minutes), |(m, name)| (*name, minutes / m))
            }
            Unit::Day if n % 3 == 0 => ("3d", n / 3),
            Unit::Day => ("1d", n),
            Unit::Week => ("1w", n),
            Unit::Month => ("1M", n),
        }
    }

    /// Binance's name for its kline stream of this interval, if it has one.
    pub fn binance_stream(&self) -> Option<&'static str> {
        let (base, per_bar) = self.binance_base();
        (per_bar == 1).then_some(base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp()
    }

    /// The same cases drive the web parser (web/src/lib/timeframe.test.ts).
    #[test]
    fn grammar_matches_the_shared_fixture() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/interval_grammar.json")).unwrap();
        for case in fixture["valid"].as_array().unwrap() {
            let (input, want) = (case[0].as_str().unwrap(), case[1].as_str().unwrap());
            assert_eq!(Interval::parse(input).unwrap().canonical(), want, "{input}");
        }
        for case in fixture["invalid"].as_array().unwrap() {
            let input = case.as_str().unwrap();
            assert!(Interval::parse(input).is_err(), "{input}");
        }
    }

    #[test]
    fn errors_say_what_went_wrong() {
        assert!(Interval::parse("banana").unwrap_err().contains("7m, 45m"));
        assert!(Interval::parse("53W").unwrap_err().contains("52W"));
        let e = Interval::parse("61m").unwrap_err();
        assert!(e.contains("61 1m bars"), "{e}");
    }

    #[test]
    fn every_valid_interval_stays_within_the_fetch_bound() {
        for unit in ["m", "h"] {
            for n in 1..=3000 {
                if let Ok(iv) = Interval::parse(&format!("{n}{unit}")) {
                    if !iv.is_date_bar() {
                        assert!(iv.binance_base().1 <= MAX_BASE_PER_BAR, "{n}{unit}");
                    }
                }
            }
        }
    }

    #[test]
    fn binance_base_keeps_native_klines_and_picks_the_coarsest_divisor() {
        for (tf, base, per_bar) in [
            ("1s", "1s", 1),
            ("5s", "1s", 5),
            ("1m", "1m", 1),
            ("3m", "3m", 1),
            ("1h", "1h", 1),
            ("8h", "8h", 1),
            ("12h", "12h", 1),
            ("1d", "1d", 1),
            ("3d", "3d", 1),
            ("1wk", "1w", 1),
            ("1mo", "1M", 1),
            ("7m", "1m", 7),
            ("45m", "15m", 3),
            ("90m", "30m", 3),
            ("7h", "1h", 7),
            ("10h", "2h", 5),
            ("2d", "1d", 2),
            ("6d", "3d", 2),
            ("2wk", "1w", 2),
            ("3mo", "1M", 3),
        ] {
            assert_eq!(
                Interval::parse(tf).unwrap().binance_base(),
                (base, per_bar),
                "{tf}"
            );
        }
    }

    #[test]
    fn only_native_intervals_have_a_stream_and_it_has_binances_name() {
        let stream = |s| Interval::parse(s).unwrap().binance_stream();
        for (tf, name) in [
            ("1s", "1s"),
            ("60s", "1m"),
            ("1m", "1m"),
            ("60m", "1h"),
            ("120m", "2h"),
            ("1H", "1h"),
            ("4H", "4h"),
            ("24h", "1d"),
            ("1D", "1d"),
            ("3D", "3d"),
            ("7d", "1w"),
            ("1W", "1w"),
            ("1wk", "1w"),
            ("1M", "1M"),
            ("1mo", "1M"),
        ] {
            assert_eq!(stream(tf), Some(name), "{tf}");
        }
        for tf in ["5s", "7m", "45m", "7h", "2D", "2W", "3M"] {
            assert_eq!(stream(tf), None, "{tf}");
        }
    }

    #[test]
    fn the_grid_is_stable_and_lands_on_the_klines_of_its_base() {
        let iv = |s| Interval::parse(s).unwrap();
        // 7m counts from the epoch, wherever a request starts.
        let t = ts("2026-11-04T10:03:20Z");
        let start = iv("7m").grid_start(t);
        assert_eq!(start % 420, 0);
        assert!(start <= t && t < start + 420);
        assert_eq!(iv("7m").grid_start(start), start);
        // Weeks open on Monday, 2W every second one.
        let thu = ts("2026-11-05T12:00:00Z");
        assert_eq!(iv("1wk").grid_start(thu), ts("2026-11-02T00:00:00Z"));
        let two = iv("2wk").grid_start(thu);
        assert_eq!(two % DAY, 0);
        assert_eq!((two / DAY - 4) % 14, 0);
        // 3d matches Binance's klines (epoch day % 3 == 1), and 6d lies on them.
        let d = iv("3d").grid_start(thu);
        assert_eq!((d / DAY) % 3, 1);
        assert_eq!((iv("6d").grid_start(thu) / DAY) % 3, 1);
        // Months are calendar months; 3M is a quarter and 12M a year.
        assert_eq!(iv("1mo").grid_start(thu), ts("2026-11-01T00:00:00Z"));
        assert_eq!(iv("3mo").grid_start(thu), ts("2026-10-01T00:00:00Z"));
        assert_eq!(iv("12mo").grid_start(thu), ts("2026-01-01T00:00:00Z"));
        assert_eq!(iv("3mo").grid_end(thu), ts("2027-01-01T00:00:00Z"));
        // Before the epoch the grid keeps going.
        assert_eq!(iv("1mo").grid_start(-1), ts("1969-12-01T00:00:00Z"));
    }
}
