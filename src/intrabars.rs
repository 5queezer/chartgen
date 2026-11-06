//! Lower-timeframe bars for footprints and volume profiles (ADR-0048): which
//! interval a chart can afford, and a bounded cache in front of the data sources.
//!
//! Binance: through the gate, with taker buy volume. Yahoo: no aggressor data, and
//! fine intervals only for recent history. The sample source: bars subdivided from
//! the chart's own, as its intervals are drawn independently.

use crate::fetch::{self, DataSource};
use chartgen::data::Bar;
use chartgen::orderflow::{self, Intrabar};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Most intrabars one request may use. On Binance, at 1000 per request, that is at
/// most 5 requests, or 6 when a page ends short of a boundary: the cost in ADR-0048.
pub const MAX_INTRABARS: usize = 5_000;
/// Intervals held in the cache, and intrabars per interval: 8 x 6000 x 64 B = 3 MB.
const MAX_ENTRIES: usize = 8;
const MAX_HELD: usize = 6_000;
/// How long the newest intrabars count as current: charts polling share one request.
const REFRESH: Duration = Duration::from_secs(5);
const MAX_PAGES: usize = 6;

const DAY: i64 = 86_400;
/// `(name, seconds, oldest age in seconds)`, finest first; `None`: no limit known.
const BINANCE: &[(&str, i64, Option<i64>)] = &[
    ("1s", 1, None),
    ("1m", 60, None),
    ("3m", 180, None),
    ("5m", 300, None),
    ("15m", 900, None),
    ("30m", 1_800, None),
    ("1h", 3_600, None),
    ("4h", 14_400, None),
    ("1d", DAY, None),
];
/// Yahoo's limits, as `fetch_yahoo` knows them. It serves 1m for 30 days only in
/// 7-day requests, so 1m is offered for what one request covers.
const YAHOO: &[(&str, i64, Option<i64>)] = &[
    ("1m", 60, Some(7 * DAY)),
    ("5m", 300, Some(59 * DAY)),
    ("15m", 900, Some(59 * DAY)),
    ("30m", 1_800, Some(59 * DAY)),
    ("1h", 3_600, Some(729 * DAY)),
    ("1d", DAY, None),
];

/// The finest interval under `chart_secs` that keeps `bars` bars, the oldest
/// opening `age` seconds ago, within `MAX_INTRABARS`. `None`: nothing finer exists
/// or can be afforded, and the bar itself is all there is.
pub fn plan(source: &str, chart_secs: i64, bars: usize, age: i64) -> Option<(&'static str, i64)> {
    let table = if source == "binance" { BINANCE } else { YAHOO };
    table
        .iter()
        .find(|(_, secs, oldest)| {
            let affordable =
                (bars as i64).saturating_mul(chart_secs / secs) <= MAX_INTRABARS as i64;
            *secs < chart_secs && affordable && oldest.is_none_or(|o| age <= o)
        })
        .map(|(name, secs, _)| (*name, *secs))
}

struct Held {
    /// Ascending by time. The last one may still be forming.
    bars: Vec<Intrabar>,
    /// Nothing exists before this: asking for older found no more.
    complete_from: Option<i64>,
    refreshed: Instant,
    used: Instant,
}

/// Newest `limit` bars opening before `end_s`, oldest first.
type Fetch<'a> = dyn FnMut(usize, Option<i64>) -> Result<Vec<Intrabar>, String> + 'a;

#[derive(Default)]
struct Cache {
    held: HashMap<(String, String), Held>,
}

impl Cache {
    fn evict(&mut self) {
        while self.held.len() > MAX_ENTRIES {
            let oldest = self
                .held
                .iter()
                .min_by_key(|(_, h)| h.used)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => self.held.remove(&k),
                None => return,
            };
        }
    }
}

fn merge(old: Vec<Intrabar>, mut newer: Vec<Intrabar>) -> Vec<Intrabar> {
    let Some(first) = newer.first().map(|b| b.t) else {
        return old;
    };
    let keep = old.partition_point(|b| b.t < first);
    let mut out = old;
    out.truncate(keep);
    out.append(&mut newer);
    out
}

/// Walk back from `end_s` until `from_s` is covered, at most `MAX_PAGES` requests.
/// Returns the bars, and whether the source ran out before `from_s`.
fn walk(
    fetch: &mut Fetch<'_>,
    from_s: i64,
    end_s: Option<i64>,
    per_page: usize,
) -> Result<(Vec<Intrabar>, bool), String> {
    let mut out: Vec<Intrabar> = Vec::new();
    let mut end = end_s;
    for _ in 0..MAX_PAGES {
        let page = fetch(per_page, end)?;
        let short = page.len() < per_page;
        let Some(oldest) = page.first().map(|b| b.t) else {
            return Ok((out, true));
        };
        out = merge(page, out);
        if oldest <= from_s {
            return Ok((out, false));
        }
        if short {
            return Ok((out, true));
        }
        end = Some(oldest);
    }
    Ok((out, false))
}

/// Intrabars opening in `[from_s, to_s)`, from what is held where it can be. A window
/// reaching the present reads only what is new, at most once per `REFRESH`.
fn read(
    held: Option<Held>,
    from_s: i64,
    to_s: i64,
    now_s: i64,
    secs: i64,
    now: Instant,
    fetch: &mut Fetch<'_>,
) -> Result<(Vec<Intrabar>, Held), String> {
    let per_page = 1000usize;
    let live = to_s + secs > now_s;
    let mut h = held.unwrap_or_else(|| Held {
        bars: Vec::new(),
        complete_from: None,
        refreshed: now - REFRESH * 2,
        used: now,
    });
    let first = h.bars.first().map(|b| b.t);
    let last = h.bars.last().map(|b| b.t);
    // What is held overlaps the window, so it can grow; otherwise it starts over.
    let overlaps = matches!((first, last), (Some(a), Some(b)) if a < to_s && b + secs >= from_s);
    if !overlaps {
        h.bars.clear();
        h.complete_from = None;
    }
    let first = h.bars.first().map(|b| b.t);
    let last = h.bars.last().map(|b| b.t);
    let covered_back =
        h.complete_from.is_some_and(|c| c <= from_s) || first.is_some_and(|a| a <= from_s);
    // Older than what is held: read back from its oldest bar.
    if !covered_back {
        let end = first.or((!live).then_some(to_s));
        let (older, ran_out) = walk(fetch, from_s, end, per_page)?;
        if ran_out {
            h.complete_from = Some(from_s);
        }
        h.bars = merge(older, std::mem::take(&mut h.bars));
        if end.is_none() {
            h.refreshed = now;
        }
    }
    // Newer than what is held. The newest held may have been forming: read it again.
    let stale = now.saturating_duration_since(h.refreshed) >= REFRESH;
    if let Some(l) = last {
        let behind_window = !live && l + secs < to_s;
        if (live && stale) || behind_window {
            let want = ((to_s.min(now_s + secs) - l) / secs + 2).clamp(2, per_page as i64) as usize;
            let (newer, _) = walk(fetch, l, (!live).then_some(to_s), want)?;
            h.bars = merge(std::mem::take(&mut h.bars), newer);
            h.refreshed = now;
        }
    }
    // Bound it, keeping the end of the window being read.
    if h.bars.len() > MAX_HELD {
        let keep_from = h.bars.partition_point(|b| b.t < from_s);
        let cut = keep_from.min(h.bars.len() - MAX_HELD);
        h.bars.drain(..cut);
        if cut > 0 {
            h.complete_from = None;
        }
        h.bars.truncate(MAX_HELD);
    }
    h.used = now;
    let a = h.bars.partition_point(|b| b.t < from_s);
    let b = h.bars.partition_point(|b| b.t < to_s);
    Ok((h.bars[a..b].to_vec(), h))
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

fn fetch_live(
    symbol: &str,
    interval: &str,
    limit: usize,
    end_s: Option<i64>,
) -> Result<Vec<Intrabar>, String> {
    let end_ms = end_s.map(|e| e * 1000);
    if fetch::detect_source(symbol) == "binance" {
        let page = fetch::fetch_binance_taker_page(symbol, interval, limit, end_ms)?;
        return Ok(page
            .iter()
            .filter_map(|(bar, taker)| Intrabar::from_bar(bar, Some(*taker)))
            .collect());
    }
    let fetched = fetch::fetch_yahoo(symbol, interval, limit, end_ms)?;
    Ok(fetched
        .data
        .bars
        .iter()
        .filter_map(|bar| Intrabar::from_bar(bar, None))
        .collect())
}

fn sample_seed(symbol: &str) -> u64 {
    symbol.bytes().fold(0xCBF2_9CE4_8422_2325_u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01B3)
    })
}

/// Intrabars for `bars` (the chart's, ascending) at `secs` apart.
pub fn load(
    source: DataSource,
    symbol: &str,
    interval: &str,
    secs: i64,
    bars: &[Bar],
    last_end: i64,
) -> Result<Vec<Intrabar>, String> {
    let opens: Vec<i64> = bars.iter().filter_map(|b| b.date.parse().ok()).collect();
    let (Some(&from_s), now_s) = (opens.first(), fetch::now_ms() / 1000) else {
        return Ok(Vec::new());
    };
    if source != DataSource::Live {
        let seed = sample_seed(symbol);
        let taker = fetch::detect_source(symbol) == "binance";
        let mut out = Vec::new();
        for (k, (bar, &open)) in bars.iter().zip(&opens).enumerate() {
            let end = opens.get(k + 1).copied().unwrap_or(last_end);
            let slots = ((end - open) / secs).max(1) as usize;
            // The forming bar has only what has passed of it.
            let count = ((now_s.min(end) - open) / secs + 1).clamp(1, slots as i64) as usize;
            out.extend(orderflow::sample_intrabars(
                seed, open, bar, secs, slots, count, taker,
            ));
        }
        return Ok(out);
    }
    let key = (symbol.to_uppercase(), interval.to_string());
    let held = lock().held.remove(&key);
    let mut fetcher = |limit: usize, end: Option<i64>| fetch_live(symbol, interval, limit, end);
    let (out, held) = read(
        held,
        from_s,
        last_end,
        now_s,
        secs,
        Instant::now(),
        &mut fetcher,
    )?;
    let mut c = lock();
    c.held.insert(key, held);
    c.evict();
    Ok(out)
}

fn lock() -> std::sync::MutexGuard<'static, Cache> {
    cache().lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chartgen::data;
    use std::cell::RefCell;

    fn ib(t: i64) -> Intrabar {
        Intrabar {
            t,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
            taker_buy: None,
        }
    }

    /// A source with bars every 60 s from `start` to `end`, counting requests.
    struct Source {
        start: i64,
        end: i64,
        requests: RefCell<Vec<(usize, Option<i64>)>>,
    }

    impl Source {
        fn new(start: i64, end: i64) -> Self {
            Self {
                start,
                end,
                requests: RefCell::default(),
            }
        }

        fn get(&self, limit: usize, end: Option<i64>) -> Result<Vec<Intrabar>, String> {
            self.requests.borrow_mut().push((limit, end));
            let stop = end.unwrap_or(self.end + 60).min(self.end + 60);
            let mut out = Vec::new();
            let mut t = (stop - 1).div_euclid(60) * 60;
            while t >= self.start && out.len() < limit {
                out.push(ib(t));
                t -= 60;
            }
            out.reverse();
            Ok(out)
        }
    }

    #[test]
    fn plan_takes_the_finest_interval_it_can_afford() {
        // 60 1h bars of 1m: 3600.
        assert_eq!(plan("binance", 3_600, 60, 0), Some(("1m", 60)));
        // 200 1h bars of 1m would be 12000: next is 3m, at 4000.
        assert_eq!(plan("binance", 3_600, 200, 0), Some(("3m", 180)));
        // 1m bars read seconds.
        assert_eq!(plan("binance", 60, 60, 0), Some(("1s", 1)));
        // 60 daily bars of 1m is 86400; 30m is 2880.
        assert_eq!(plan("binance", DAY, 60, 0), Some(("30m", 1_800)));
    }

    #[test]
    fn plan_stays_under_the_budget_for_any_chart() {
        for chart in [60, 180, 900, 3_600, 14_400, DAY, 7 * DAY, 30 * DAY] {
            for bars in [1, 3, 60, 120, 200] {
                if let Some((_, secs)) = plan("binance", chart, bars, 0) {
                    assert!(bars as i64 * chart / secs <= MAX_INTRABARS as i64);
                    assert!(secs < chart);
                }
            }
        }
    }

    #[test]
    fn nothing_finer_than_the_chart_is_no_plan() {
        // Yahoo's finest is 1m.
        assert_eq!(plan("yahoo", 60, 60, 0), None);
        assert_eq!(plan("binance", 1, 60, 0), None);
    }

    #[test]
    fn yahoo_goes_coarser_with_age() {
        assert_eq!(plan("yahoo", 3_600, 60, DAY), Some(("1m", 60)));
        assert_eq!(plan("yahoo", 3_600, 60, 30 * DAY), Some(("5m", 300)));
        // Too old for anything under an hour.
        assert_eq!(plan("yahoo", 3_600, 60, 100 * DAY), None);
        assert_eq!(plan("yahoo", DAY, 60, 100 * DAY), Some(("1h", 3_600)));
        assert_eq!(plan("yahoo", DAY, 60, 800 * DAY), None);
    }

    fn run(
        source: &Source,
        held: Option<Held>,
        from: i64,
        to: i64,
        now_s: i64,
        now: Instant,
    ) -> (Vec<Intrabar>, Held) {
        let mut f = |limit: usize, end: Option<i64>| source.get(limit, end);
        read(held, from, to, now_s, 60, now, &mut f).unwrap()
    }

    #[test]
    fn a_first_read_walks_back_over_pages_for_the_window() {
        let source = Source::new(0, 600_000);
        let now = Instant::now();
        let (got, _) = run(&source, None, 480_000, 540_000, 700_000, now);
        assert_eq!(got.len(), 1000);
        assert_eq!(got[0].t, 480_000);
        assert_eq!(got.last().unwrap().t, 539_940);
        assert_eq!(source.requests.borrow().len(), 1);
        // 2000 bars take two pages, newest first.
        let source = Source::new(0, 600_000);
        let (got, _) = run(&source, None, 420_000, 540_000, 700_000, now);
        assert_eq!(got.len(), 2000);
        assert_eq!(source.requests.borrow().len(), 2);
    }

    #[test]
    fn the_same_window_again_costs_nothing() {
        let source = Source::new(0, 600_000);
        let now = Instant::now();
        let (a, held) = run(&source, None, 480_000, 540_000, 700_000, now);
        let requests = source.requests.borrow().len();
        let (b, _) = run(&source, Some(held), 480_000, 540_000, 700_000, now);
        assert_eq!(a, b);
        assert_eq!(source.requests.borrow().len(), requests);
    }

    #[test]
    fn a_live_window_reads_only_the_newest_and_not_within_the_refresh() {
        let source = Source::new(0, 600_000);
        let now = Instant::now();
        let (_, held) = run(&source, None, 540_000, 600_060, 600_030, now);
        let first = source.requests.borrow().len();
        // Within the refresh: nothing asked.
        let (_, held) = run(
            &source,
            Some(held),
            540_000,
            600_060,
            600_031,
            now + Duration::from_secs(1),
        );
        assert_eq!(source.requests.borrow().len(), first);
        // After it: one short read of the tail, not the window.
        let (got, _) = run(
            &source,
            Some(held),
            540_000,
            600_060,
            600_040,
            now + REFRESH,
        );
        let requests = source.requests.borrow();
        assert_eq!(requests.len(), first + 1);
        assert!(requests.last().unwrap().0 <= 3, "{:?}", requests.last());
        assert_eq!(got.last().unwrap().t, 600_000);
    }

    #[test]
    fn the_forming_bar_is_replaced_not_duplicated() {
        let source = Source::new(0, 600_000);
        let now = Instant::now();
        let (a, held) = run(&source, None, 599_000, 600_060, 600_030, now);
        let (b, _) = run(
            &source,
            Some(held),
            599_000,
            600_060,
            600_030,
            now + REFRESH,
        );
        assert_eq!(a.len(), b.len());
        let times: Vec<i64> = b.iter().map(|x| x.t).collect();
        let mut sorted = times.clone();
        sorted.dedup();
        assert_eq!(times, sorted);
    }

    #[test]
    fn scrolling_back_reads_from_the_oldest_held_bar() {
        let source = Source::new(0, 600_000);
        let now = Instant::now();
        let (_, held) = run(&source, None, 540_000, 600_000, 700_000, now);
        source.requests.borrow_mut().clear();
        let (got, _) = run(&source, Some(held), 510_000, 600_000, 700_000, now);
        assert_eq!(got.first().unwrap().t, 510_000);
        let requests = source.requests.borrow();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, Some(540_000));
    }

    #[test]
    fn a_window_elsewhere_starts_over() {
        let source = Source::new(0, 600_000);
        let now = Instant::now();
        let (_, held) = run(&source, None, 540_000, 600_000, 700_000, now);
        let (got, held) = run(&source, Some(held), 60_000, 63_000, 700_000, now);
        assert_eq!(got.first().unwrap().t, 60_000);
        assert!(held.bars.iter().all(|b| b.t < 100_000));
    }

    #[test]
    fn running_out_of_history_is_remembered() {
        let source = Source::new(500_000, 600_000);
        let now = Instant::now();
        let (got, held) = run(&source, None, 480_000, 540_000, 700_000, now);
        assert_eq!(got.first().unwrap().t, 500_040.min(got[0].t));
        let requests = source.requests.borrow().len();
        let (_, _) = run(&source, Some(held), 480_000, 540_000, 700_000, now);
        assert_eq!(source.requests.borrow().len(), requests);
    }

    #[test]
    fn what_is_held_stays_bounded() {
        let source = Source::new(0, 10_000_000);
        let now = Instant::now();
        let mut held = None;
        let mut from = 9_000_000;
        for _ in 0..6 {
            let (_, h) = run(&source, held.take(), from, from + 300_000, 20_000_000, now);
            assert!(h.bars.len() <= MAX_HELD, "{}", h.bars.len());
            held = Some(h);
            from -= 240_000;
        }
    }

    #[test]
    fn a_page_of_requests_never_passes_the_limit() {
        let source = Source::new(0, 10_000_000);
        let now = Instant::now();
        let _ = run(&source, None, 0, 10_000_000, 20_000_000, now);
        assert!(source.requests.borrow().len() <= MAX_PAGES);
    }

    #[test]
    fn the_cache_evicts_the_longest_unused() {
        let now = Instant::now();
        let mut c = Cache::default();
        for k in 0..=MAX_ENTRIES {
            c.held.insert(
                (k.to_string(), "1m".into()),
                Held {
                    bars: vec![],
                    complete_from: None,
                    refreshed: now,
                    used: now + Duration::from_secs(k as u64),
                },
            );
        }
        c.evict();
        assert_eq!(c.held.len(), MAX_ENTRIES);
        assert!(!c.held.contains_key(&("0".to_string(), "1m".to_string())));
    }

    #[test]
    fn sample_intrabars_cover_each_chart_bar() {
        let bars = data::sample_ohlcv("BTCUSDT", "1h", 3, Some(7_200_000 * 1000), fetch::now_ms())
            .unwrap();
        let opens: Vec<i64> = bars.bars.iter().map(|b| b.date.parse().unwrap()).collect();
        let last_end = opens[2] + 3600;
        let got = load(
            DataSource::Sample,
            "BTCUSDT",
            "1m",
            60,
            &bars.bars,
            last_end,
        )
        .unwrap();
        assert_eq!(got.len(), 180);
        assert!(got.iter().all(|b| b.taker_buy.is_some()));
        // No aggressor data for a stock, as on Yahoo.
        let got = load(DataSource::Sample, "AAPL", "1m", 60, &bars.bars, last_end).unwrap();
        assert!(got.iter().all(|b| b.taker_buy.is_none()));
    }
}
