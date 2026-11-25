//! Recording open interest beyond the 30 days Binance keeps (ADR-0082).
//!
//! A thread reads the 5m open interest of each watched symbol, forward from the newest
//! row it recorded (or as far back as Binance keeps, the first time), writes it to the
//! store, and drops rows older than the retention. Watched are the symbols of the
//! setups and server alerts in the store, and those of `CHARTGEN_OI_RECORD_SYMBOLS`.
//! A coarser period's row is the 5m row of the same time, so one series answers every
//! interval (`recorded_rows`).

use super::{
    hist_page, is_crypto_symbol, now_ms, Query, DAY_MS, OI, PAGE_ROWS, RETENTION_MARGIN_MS,
    RETENTION_MS,
};
use crate::scripting::persistence::{self, Store};
use std::time::Duration;

pub(super) const KIND: &str = "oi";
pub(super) const PERIOD: &str = "5m";
const STEP_MS: i64 = 5 * 60 * 1000;
/// How often the watched symbols are caught up. Far inside the 30 days, so a symbol
/// that failed a round has many more before a row ages out at Binance.
const SWEEP: Duration = Duration::from_secs(6 * 3600);
/// The wait after each request, so a first read of many symbols (18 requests each)
/// takes at most 150 of the 800 requests per 5 minutes the charts share
/// (`binance_gate::futures_data_budget`).
const PACE: Duration = Duration::from_secs(2);
const DEFAULT_DAYS: i64 = 365;

/// `CHARTGEN_OI_RECORD_DAYS`: how long a row is kept, default 365; 0 records nothing.
fn days() -> Result<i64, String> {
    match std::env::var("CHARTGEN_OI_RECORD_DAYS") {
        Err(_) => Ok(DEFAULT_DAYS),
        Ok(v) => v
            .trim()
            .parse()
            .ok()
            .filter(|d| *d >= 0)
            .ok_or_else(|| format!("CHARTGEN_OI_RECORD_DAYS must be a number of days, not {v}")),
    }
}

/// `CHARTGEN_OI_RECORD_SYMBOLS`: symbols recorded besides those watched, comma-separated.
fn listed() -> Vec<String> {
    std::env::var("CHARTGEN_OI_RECORD_SYMBOLS")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Start recording, unless turned off or the server has no store.
pub fn start() {
    let days = match days() {
        Ok(0) => return,
        Ok(days) => days,
        Err(e) => {
            eprintln!("[oi-record] {e}; open interest is not recorded");
            return;
        }
    };
    let Some(store) = persistence::global() else {
        eprintln!("[oi-record] the store is unavailable; open interest is not recorded");
        return;
    };
    let listed = listed();
    let spawned = std::thread::Builder::new()
        .name("oi-record".into())
        .spawn(move || loop {
            sweep(
                store,
                &listed,
                days,
                now_ms(),
                &mut |symbol, q| hist_page(&OI, symbol, PERIOD, &q),
                &mut || std::thread::sleep(PACE),
            );
            std::thread::sleep(SWEEP);
        });
    if let Err(e) = spawned {
        eprintln!("[oi-record] cannot start: {e}");
    }
}

type Page<'a> = dyn FnMut(&str, Query) -> Result<Vec<(i64, f64)>, String> + 'a;

/// One round at `now`: each symbol caught up, then rows older than `days` dropped.
fn sweep(
    store: &Store,
    listed: &[String],
    days: i64,
    now: i64,
    page: &mut Page,
    pace: &mut dyn FnMut(),
) {
    let watched = store.watched_symbols().unwrap_or_else(|e| {
        eprintln!("[oi-record] cannot read the watched symbols: {e}");
        Vec::new()
    });
    let mut symbols: Vec<String> = listed
        .iter()
        .cloned()
        .chain(watched)
        .filter(|s| is_crypto_symbol(s))
        .collect();
    symbols.sort();
    symbols.dedup();
    let (mut rows, mut failed) = (0, Vec::new());
    for symbol in &symbols {
        match catch_up(store, symbol, now, page, pace) {
            Ok(n) => rows += n,
            Err(e) => failed.push(format!("{symbol} ({e})")),
        }
    }
    let dropped = store
        .forget_futures_rows(KIND, now - days * DAY_MS)
        .unwrap_or_else(|e| {
            eprintln!("[oi-record] cannot drop old rows: {e}");
            0
        });
    let failed = match failed.is_empty() {
        true => String::new(),
        false => format!("; failed: {}", failed.join(", ")),
    };
    eprintln!(
        "[oi-record] {rows} rows recorded for {} symbols, {dropped} older than {days} days dropped{failed}",
        symbols.len()
    );
}

/// Read `symbol`'s 5m open interest from after its newest recorded row (or from as far
/// back as Binance keeps) up to `now`, a page of 500 rows at a time, and write each page
/// as it comes, so a round that fails keeps what it read. The rows of the last 10
/// minutes wait for the next round, so a row is never recorded before it is final.
/// Answers the rows written.
fn catch_up(
    store: &Store,
    symbol: &str,
    now: i64,
    page: &mut Page,
    pace: &mut dyn FnMut(),
) -> Result<usize, String> {
    let floor = now - RETENTION_MS + RETENTION_MARGIN_MS;
    let newest = now - 2 * STEP_MS;
    let last = store
        .last_futures_row(KIND, symbol, PERIOD)
        .map_err(|e| e.to_string())?;
    let from = last.map_or(floor, |t| t + STEP_MS).max(floor);
    // The first time on the grid at or after `from`.
    let mut start = (from + STEP_MS - 1).div_euclid(STEP_MS) * STEP_MS;
    let mut written = 0;
    while start <= newest {
        let end = (start + (PAGE_ROWS as i64 - 1) * STEP_MS).min(newest);
        let mut rows = page(
            symbol,
            Query {
                start: Some(start),
                end: Some(end),
            },
        )?;
        pace();
        rows.retain(|(t, _)| (start..=end).contains(t));
        store
            .save_futures_rows(KIND, symbol, PERIOD, &rows)
            .map_err(|e| e.to_string())?;
        written += rows.len();
        start = end + STEP_MS;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    /// A Binance that has a row every 5 minutes within its 30 days, valued by time,
    /// recording what was asked for.
    fn binance(
        asked: &mut Vec<(String, i64, i64)>,
    ) -> impl FnMut(&str, Query) -> Result<Vec<(i64, f64)>, String> + '_ {
        move |symbol, q| {
            let (start, end) = (q.start.unwrap(), q.end.unwrap());
            asked.push((symbol.to_string(), start, end));
            if symbol == "FAILUSDT" {
                return Err("Binance returned status 400 Bad Request (code -1121)".into());
            }
            assert!(
                start >= NOW - RETENTION_MS,
                "never asks past what Binance keeps"
            );
            Ok((start.div_euclid(STEP_MS)..=end.div_euclid(STEP_MS))
                .map(|i| i * STEP_MS)
                .filter(|t| *t >= start)
                .map(|t| (t, t as f64 / 1e6))
                .collect())
        }
    }

    fn rows(store: &Store, symbol: &str) -> Vec<(i64, f64)> {
        store
            .futures_rows(KIND, symbol, PERIOD, (0, i64::MAX), STEP_MS)
            .unwrap()
    }

    #[test]
    fn a_symbol_is_read_back_30_days_then_forward_from_its_newest_row() {
        let store = Store::in_memory().unwrap();
        let mut asked = Vec::new();
        let mut paced = 0;
        let n = catch_up(
            &store,
            "BTCUSDT",
            NOW,
            &mut binance(&mut asked),
            &mut || paced += 1,
        )
        .unwrap();
        // 30 days less the margin and the last 10 minutes, at 5m: 18 pages.
        assert_eq!((asked.len(), paced), (18, 18));
        let got = rows(&store, "BTCUSDT");
        assert_eq!(got.len(), n);
        let (first, last) = (got[0].0, got.last().unwrap().0);
        assert!(first >= NOW - RETENTION_MS + RETENTION_MARGIN_MS && first % STEP_MS == 0);
        assert!(last <= NOW - 2 * STEP_MS && last > NOW - 3 * STEP_MS);
        assert!(
            got.windows(2).all(|w| w[1].0 - w[0].0 == STEP_MS),
            "no hole"
        );
        assert!(asked
            .iter()
            .all(|(_, s, e)| (e - s) / STEP_MS < PAGE_ROWS as i64));

        // Six hours on: one request, from the row after the newest.
        asked.clear();
        let later = NOW + 6 * 3_600_000;
        let n = catch_up(
            &store,
            "BTCUSDT",
            later,
            &mut binance(&mut asked),
            &mut || {},
        )
        .unwrap();
        assert_eq!(
            asked,
            [("BTCUSDT".into(), last + STEP_MS, later - 2 * STEP_MS)]
        );
        assert_eq!(n, 72);
        // Nothing new yet: nothing asked.
        asked.clear();
        catch_up(
            &store,
            "BTCUSDT",
            later,
            &mut binance(&mut asked),
            &mut || {},
        )
        .unwrap();
        assert!(asked.is_empty());
    }

    #[test]
    fn a_server_down_longer_than_30_days_starts_again_at_what_binance_keeps() {
        let store = Store::in_memory().unwrap();
        let old = NOW - 90 * DAY_MS;
        store
            .save_futures_rows(KIND, "BTCUSDT", PERIOD, &[(old, 1.0)])
            .unwrap();
        let mut asked = Vec::new();
        catch_up(&store, "BTCUSDT", NOW, &mut binance(&mut asked), &mut || {}).unwrap();
        assert!(asked[0].1 >= NOW - RETENTION_MS + RETENTION_MARGIN_MS);
        assert_eq!(rows(&store, "BTCUSDT")[0], (old, 1.0), "what it had stays");
    }

    #[test]
    fn a_round_records_the_watched_and_listed_symbols_and_drops_old_rows() {
        let store = Store::in_memory().unwrap();
        store
            .save_server_alert("a1", "owner", r#"{"symbol":"ethusdt","interval":"4h"}"#)
            .unwrap();
        store
            .save_setups(&[persistence::StoredSetup {
                id: "s1".into(),
                owner: "owner".into(),
                record: r#"{"symbol":"BTCUSDT"}"#.into(),
                revisions: vec![],
            }])
            .unwrap();
        store
            .save_server_alert("a2", "owner", r#"{"symbol":"AAPL","interval":"1d"}"#)
            .unwrap();
        assert_eq!(
            store.watched_symbols().unwrap(),
            ["AAPL", "BTCUSDT", "ETHUSDT"]
        );
        let year_ago = NOW - 400 * DAY_MS;
        store
            .save_futures_rows(KIND, "BTCUSDT", PERIOD, &[(year_ago, 1.0)])
            .unwrap();

        let mut asked = Vec::new();
        let listed = ["FAILUSDT".to_string(), "BTCUSDT".to_string()];
        sweep(
            &store,
            &listed,
            365,
            NOW,
            &mut binance(&mut asked),
            &mut || {},
        );
        let symbols: std::collections::BTreeSet<_> = asked.iter().map(|a| a.0.as_str()).collect();
        // A stock has no futures; one that fails does not stop the others.
        assert_eq!(
            symbols.into_iter().collect::<Vec<_>>(),
            ["BTCUSDT", "ETHUSDT", "FAILUSDT"]
        );
        assert_eq!(rows(&store, "ETHUSDT").len(), rows(&store, "BTCUSDT").len());
        assert!(
            rows(&store, "BTCUSDT")[0].0 > year_ago,
            "older than a year: dropped"
        );
    }

    #[test]
    fn a_coarser_period_reads_the_5m_rows_at_its_own_times() {
        let store = Store::in_memory().unwrap();
        let rows: Vec<_> = (0..48 * 12).map(|i| (i * STEP_MS, i as f64)).collect();
        store
            .save_futures_rows(KIND, "BTCUSDT", PERIOD, &rows)
            .unwrap();
        let four = store
            .futures_rows(KIND, "BTCUSDT", PERIOD, (0, i64::MAX), 4 * 3_600_000)
            .unwrap();
        assert_eq!(
            four,
            [
                (0, 0.0),
                (14_400_000, 48.0),
                (28_800_000, 96.0),
                (43_200_000, 144.0),
                (57_600_000, 192.0),
                (72_000_000, 240.0),
                (86_400_000, 288.0),
                (100_800_000, 336.0),
                (115_200_000, 384.0),
                (129_600_000, 432.0),
                (144_000_000, 480.0),
                (158_400_000, 528.0)
            ]
        );
    }
}
