//! The market of a stock setup (ADR-0070): its costs and its sessions.
//!
//! A Binance setup has no market and trades around the clock. A stock setup counts in
//! session time: its expiry, a skip's bracket, the time stop and its durations.

use chrono::{Datelike, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};

use super::Costs;
use crate::interval::{Interval, Unit};

/// The cost assumption of a stock setup, in basis points per side.
pub const STOCK_FEE_BPS_PER_SIDE: f64 = 1.0;
pub const STOCK_SLIPPAGE_BPS_PER_SIDE: f64 = 3.0;

/// Sessions a bar of a day or longer counts as: a week of 5, a month of 21.
const SESSIONS_PER_WEEK: i64 = 5;
const SESSIONS_PER_MONTH: i64 = 21;
/// A full session, for durations in bars of a day or longer.
const SESSION_MS: i64 = 390 * 60_000;
/// How far ahead a session is looked for: longer than any closure there has been.
const SEARCH_DAYS: i64 = 4000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Market {
    /// A stock or ETF listed in the US, read from Yahoo: the regular session, New York time.
    UsEquity,
}

impl Market {
    pub fn costs(self) -> Costs {
        Costs {
            fee_bps_per_side: STOCK_FEE_BPS_PER_SIDE,
            slippage_bps_per_side: STOCK_SLIPPAGE_BPS_PER_SIDE,
        }
    }

    /// Whether the calendar knows the holidays at `t` (ms): times past its last year
    /// would count holidays as sessions.
    pub fn covers(self, t: i64) -> bool {
        new_york(t).is_some_and(|d| d.year() <= crate::data::US_CALENDAR_LAST_YEAR)
    }

    /// The session on `d`, as (open, close) in ms.
    fn session_on(self, d: NaiveDate) -> Option<(i64, i64)> {
        let (open, close) = crate::data::us_session(d)?;
        let tz = chrono_tz::America::New_York;
        let at = |time| {
            tz.from_local_datetime(&d.and_time(time))
                .single()
                .map(|t| t.timestamp_millis())
        };
        Some((at(open)?, at(close)?))
    }

    /// The sessions that close after `t` (ms), in order: the one holding it first, if
    /// any.
    pub fn sessions_from(self, t: i64) -> impl Iterator<Item = (i64, i64)> {
        let first = new_york(t);
        (0..SEARCH_DAYS)
            .filter_map(move |k| first?.checked_add_days(chrono::Days::new(k as u64)))
            .filter_map(move |d| self.session_on(d))
            .filter(move |(_, close)| *close > t)
    }

    /// Whether `t` (ms) lies inside a session.
    pub fn in_session(self, t: i64) -> bool {
        self.sessions_from(t)
            .next()
            .is_some_and(|(open, _)| open <= t)
    }

    /// Session time from `from` to `to` (ms).
    pub fn session_ms_between(self, from: i64, to: i64) -> i64 {
        let mut total = 0;
        for (open, close) in self.sessions_from(from) {
            if open >= to {
                break;
            }
            total += close.min(to) - open.max(from);
        }
        total
    }

    /// The time when `ms` of session time have passed after `t`.
    pub fn add_session_ms(self, t: i64, ms: i64) -> Option<i64> {
        let mut left = ms;
        for (open, close) in self.sessions_from(t) {
            let start = open.max(t);
            if close - start >= left {
                return Some(start + left);
            }
            left -= close - start;
        }
        None
    }

    /// How many sessions a bar of `iv` counts as, when it is a day or longer.
    fn sessions_per_bar(iv: &Interval) -> Option<i64> {
        let n = i64::from(iv.n);
        match iv.unit {
            Unit::Day => Some(n),
            Unit::Week => Some(n * SESSIONS_PER_WEEK),
            Unit::Month => Some(n * SESSIONS_PER_MONTH),
            _ => None,
        }
    }

    /// Session time a bar of `iv` holds, for durations in bars.
    pub fn bar_ms(iv: &Interval) -> i64 {
        match Self::sessions_per_bar(iv) {
            Some(sessions) => sessions * SESSION_MS,
            None => iv.secs() * 1000,
        }
    }

    /// The close (ms) of the `n`th bar of `iv` after the bar holding `t`. Intraday bars
    /// are on the session grid, as Yahoo's: from the open, the session's last bar
    /// shorter. A bar of a day or longer is that many sessions. A `t` outside a
    /// session counts from the first bar after it.
    pub fn bar_close_after(self, iv: &Interval, t: i64, n: i64) -> Option<i64> {
        if let Some(per) = Self::sessions_per_bar(iv) {
            return self
                .sessions_from(t)
                .nth(usize::try_from(n * per).ok()?)
                .map(|(_, close)| close);
        }
        let step = iv.secs() * 1000;
        let mut index = n;
        for (open, close) in self.sessions_from(t) {
            let bars = (close - open + step - 1) / step;
            if open <= t {
                index += (t - open) / step;
            }
            if index < bars {
                return Some((open + (index + 1) * step).min(close));
            }
            index -= bars;
        }
        None
    }

    /// When a skip made at `now` runs out: `bars` bars of `iv` of session time later.
    /// For a bar of a day or longer, the close of that many sessions after the one
    /// holding `now`, or the next.
    pub fn skip_expiry(self, iv: &Interval, now: i64, bars: i64) -> Option<i64> {
        match Self::sessions_per_bar(iv) {
            Some(per) => self
                .sessions_from(now)
                .nth(usize::try_from(bars * per).ok()?)
                .map(|(_, close)| close),
            None => self.add_session_ms(now, bars * iv.secs() * 1000),
        }
    }

    /// The open (ms) of the intraday bar of `iv` holding `t`, on the session grid; the
    /// epoch grid for a bar of a day or longer, or a `t` outside a session.
    pub fn bar_start(self, iv: &Interval, t: i64) -> i64 {
        let grid = iv.grid_start(t.div_euclid(1000)) * 1000;
        if Self::sessions_per_bar(iv).is_some() {
            return grid;
        }
        match self.sessions_from(t).next() {
            Some((open, _)) if open <= t => {
                let step = iv.secs() * 1000;
                open + (t - open) / step * step
            }
            _ => grid,
        }
    }

    /// The close (ms) of the bar of `iv` that opened at `start` (`bar_start`).
    pub fn bar_end(self, iv: &Interval, start: i64) -> i64 {
        if Self::sessions_per_bar(iv).is_some() {
            return iv.grid_end(start.div_euclid(1000)) * 1000;
        }
        match self.sessions_from(start).next() {
            Some((open, close)) if open <= start => (start + iv.secs() * 1000).min(close),
            _ => iv.grid_end(start.div_euclid(1000)) * 1000,
        }
    }
}

/// The New York date of `t` (ms).
fn new_york(t: i64) -> Option<NaiveDate> {
    chrono_tz::America::New_York
        .timestamp_millis_opt(t)
        .single()
        .map(|d| d.date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;

    const US: Market = Market::UsEquity;

    /// ms of a New York local time.
    fn ny(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        chrono_tz::America::New_York
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .unwrap()
            .timestamp_millis()
    }

    const HOUR: i64 = 3_600_000;

    #[test]
    fn sessions_skip_weekends_holidays_and_end_early_on_an_early_close() {
        // Wednesday 2026-11-25, Thanksgiving on Thursday, 13:00 close on Friday, then
        // the weekend.
        let sessions: Vec<_> = US.sessions_from(ny(2026, 11, 25, 12, 0)).take(3).collect();
        assert_eq!(
            sessions,
            [
                (ny(2026, 11, 25, 9, 30), ny(2026, 11, 25, 16, 0)),
                (ny(2026, 11, 27, 9, 30), ny(2026, 11, 27, 13, 0)),
                (ny(2026, 11, 30, 9, 30), ny(2026, 11, 30, 16, 0)),
            ]
        );
        // Across a DST change too: 2026-11-02 is the Monday after it.
        let monday = US.sessions_from(ny(2026, 10, 31, 0, 0)).next().unwrap();
        assert_eq!(monday, (ny(2026, 11, 2, 9, 30), ny(2026, 11, 2, 16, 0)));
        assert_eq!(monday.0 % (24 * HOUR), (14 * 60 + 30) * 60_000);
        assert!(US.in_session(ny(2026, 11, 25, 9, 30)));
        assert!(!US.in_session(ny(2026, 11, 25, 16, 0)));
        assert!(!US.in_session(ny(2026, 11, 26, 11, 0)));
    }

    #[test]
    fn session_time_leaves_out_nights_weekends_and_holidays() {
        // Friday 15:00 to Monday 10:30: an hour on Friday, an hour on Monday.
        let between = US.session_ms_between(ny(2026, 11, 6, 15, 0), ny(2026, 11, 9, 10, 30));
        assert_eq!(between, 2 * HOUR);
        // Wednesday 15:00 plus 2h of session time: Thursday is Thanksgiving, so Friday
        // 10:30.
        let later = US.add_session_ms(ny(2026, 11, 25, 15, 0), 2 * HOUR);
        assert_eq!(later, Some(ny(2026, 11, 27, 10, 30)));
        // From a closed market, the time starts at the next open.
        let later = US.add_session_ms(ny(2026, 11, 7, 12, 0), HOUR);
        assert_eq!(later, Some(ny(2026, 11, 9, 10, 30)));
    }

    #[test]
    fn bars_are_counted_on_the_session_grid() {
        let hour = Interval::parse("1h").unwrap();
        // The 1h bars of a session open at 09:30, 10:30 ... 15:30, the last a half hour.
        // From inside the 14:30 bar on Friday: its close, then the next two.
        let t = ny(2026, 11, 6, 14, 45);
        assert_eq!(
            US.bar_close_after(&hour, t, 0),
            Some(ny(2026, 11, 6, 15, 30))
        );
        assert_eq!(
            US.bar_close_after(&hour, t, 1),
            Some(ny(2026, 11, 6, 16, 0))
        );
        assert_eq!(
            US.bar_close_after(&hour, t, 2),
            Some(ny(2026, 11, 9, 10, 30))
        );
        // From a closed market: the first bar after it is bar 0.
        assert_eq!(
            US.bar_close_after(&hour, ny(2026, 11, 7, 12, 0), 0),
            Some(ny(2026, 11, 9, 10, 30))
        );
        let day = Interval::parse("1D").unwrap();
        // Day bars are sessions: Wednesday's close, then Friday's 13:00 close.
        let wednesday = ny(2026, 11, 25, 11, 0);
        assert_eq!(
            US.bar_close_after(&day, wednesday, 0),
            Some(ny(2026, 11, 25, 16, 0))
        );
        assert_eq!(
            US.bar_close_after(&day, wednesday, 1),
            Some(ny(2026, 11, 27, 13, 0))
        );
        assert_eq!(
            US.bar_start(&hour, ny(2026, 11, 6, 15, 59)),
            ny(2026, 11, 6, 15, 30)
        );
        assert_eq!(
            US.bar_end(&hour, ny(2026, 11, 6, 15, 30)),
            ny(2026, 11, 6, 16, 0)
        );
    }

    #[test]
    fn a_skip_runs_out_after_its_bars_of_session_time() {
        let hour = Interval::parse("1h").unwrap();
        // 24 trading hours from Wednesday 15:00: an hour that day, none on Thanksgiving,
        // 3.5h to Friday's 13:00 close, then 6.5h on each of Monday, Tuesday and
        // Wednesday: the next Wednesday's close.
        let expiry = US.skip_expiry(&hour, ny(2026, 11, 25, 15, 0), 24);
        assert_eq!(expiry, Some(ny(2026, 12, 2, 16, 0)));
        let day = Interval::parse("1D").unwrap();
        // 2 day bars from a Saturday: the close of the second session after Monday's.
        let expiry = US.skip_expiry(&day, ny(2026, 11, 7, 12, 0), 2);
        assert_eq!(expiry, Some(ny(2026, 11, 11, 16, 0)));
    }

    #[test]
    fn the_calendar_covers_its_tables_only() {
        assert!(US.covers(ny(2028, 12, 29, 12, 0)));
        assert!(!US.covers(ny(2029, 1, 2, 12, 0)));
    }
}
