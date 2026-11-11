//! Lower-timeframe bars for footprints and volume profiles (ADR-0048): which
//! interval a chart can afford, and reading them.
//!
//! Binance: the windows and pages charts read (ADR-0049), so one read per symbol and
//! kline with its deadlines and walk limit, with taker buy volume. Yahoo: no aggressor
//! data, and fine intervals only for recent history. A kline window does not fit it: it
//! has no gate, closes for the night, and has limits by age, so it has a bounded cache
//! of its own here. The sample source: bars subdivided from the chart's own, as its
//! intervals are drawn independently.

use crate::fetch::{self, BinanceClient, Claim, DataSource, Flight, Flights};
use chartgen::data::Bar;
use chartgen::orderflow::{self, Intrabar};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Most intrabars one request may use. On Binance, at 1000 per request, that is at
/// most 5 requests, or 6 when a page ends short of a boundary: the cost in ADR-0048.
pub const MAX_INTRABARS: usize = 5_000;
/// Yahoo intervals held in the cache, and intrabars per interval: 8 x 6000 x 64 B = 3 MB.
const MAX_ENTRIES: usize = 8;
/// Most intrabars read for one window.
const MAX_HELD: usize = 6_000;
/// How long Yahoo's newest intrabars count as current: charts polling share one request.
const REFRESH: Duration = Duration::from_secs(5);
/// A newest intrabar this far behind the clock, past its own end, means a market that
/// is closed or quiet, whatever the bar's grid says: a daily bar stays open all night.
const QUIET_AFTER: i64 = 120;
/// How often a quiet market is asked again: soon enough to see it open.
const QUIET_REFRESH: Duration = Duration::from_secs(60);
const MAX_PAGES: usize = 6;
/// How long one Yahoo read may take, waiting for another's included.
const READ_BUDGET: Duration = Duration::from_secs(20);

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
    table_of(source)
        .iter()
        .find(|e| fits(e, chart_secs, bars, age))
        .map(|(name, secs, _)| (*name, *secs))
}

/// What a live poll reads: `load`, the interval its load read (`NO_INTERVAL` for
/// nothing finer), so that its cells do not change with the age of its few bars. If
/// these bars cannot have that one, as after a long gap, whatever `plan` finds instead,
/// which is coarser.
pub fn plan_like_load(
    source: &str,
    chart_secs: i64,
    bars: usize,
    age: i64,
    load: &str,
) -> Result<Option<(&'static str, i64)>, String> {
    if load == NO_INTERVAL {
        return Ok(None);
    }
    let table = table_of(source);
    let Some(entry) = table.iter().find(|(name, _, _)| *name == load) else {
        let known: Vec<&str> = table.iter().map(|(name, _, _)| *name).collect();
        return Err(format!(
            "intrabar_interval must be {NO_INTERVAL} or one of {}.",
            known.join(", ")
        ));
    };
    Ok(if fits(entry, chart_secs, bars, age) {
        Some((entry.0, entry.1))
    } else {
        plan(source, chart_secs, bars, age)
    })
}

/// `plan_like_load`'s word for a load that read nothing finer.
pub const NO_INTERVAL: &str = "none";

fn table_of(source: &str) -> &'static [(&'static str, i64, Option<i64>)] {
    if source == "binance" {
        BINANCE
    } else {
        YAHOO
    }
}

fn fits(entry: &(&str, i64, Option<i64>), chart_secs: i64, bars: usize, age: i64) -> bool {
    let (_, secs, oldest) = *entry;
    let affordable = (bars as i64).saturating_mul(chart_secs / secs) <= MAX_INTRABARS as i64;
    secs < chart_secs && affordable && oldest.is_none_or(|o| age <= o)
}

#[derive(Clone)]
struct Held {
    /// Ascending by time. The last one may still be forming.
    bars: Vec<Intrabar>,
    /// Nothing exists before this: asking for older found no more.
    complete_from: Option<i64>,
    refreshed: Instant,
    /// How far the source has been asked, unix s: nothing newer than the bars exists
    /// up to here, so a window ending by then has no reason to ask again.
    asked_to: i64,
    used: Instant,
}

/// Newest `limit` bars opening before `end_s`, oldest first.
type Fetch<'a> = dyn FnMut(usize, Option<i64>) -> Result<Vec<Intrabar>, String> + 'a;

type CacheKey = (String, &'static str);

#[derive(Default)]
struct Cache {
    /// What was read. A read works on a copy, so a read that fails or is still out
    /// takes nothing from anybody.
    held: HashMap<CacheKey, Arc<Held>>,
    /// Reads under way. Others wait for them rather than read the same again.
    reading: Flights<CacheKey, ()>,
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

#[derive(Default)]
struct SharedCache {
    state: Mutex<Cache>,
    ended: Condvar,
}

impl SharedCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
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

/// Walk back from `end_s` until `from_s` is covered, at most `MAX_PAGES` requests, none
/// begun after `deadline`. Returns the bars, and whether the source ran out before `from_s`.
fn walk(
    fetch: &mut Fetch<'_>,
    from_s: i64,
    end_s: Option<i64>,
    per_page: usize,
    deadline: Instant,
) -> Result<(Vec<Intrabar>, bool), String> {
    let mut out: Vec<Intrabar> = Vec::new();
    let mut end = end_s;
    for _ in 0..MAX_PAGES {
        if Instant::now() >= deadline {
            return Err("reading finer bars took too long".into());
        }
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
/// reaching the present reads only what is new, at most once per `REFRESH`, or per
/// `QUIET_REFRESH` while the market looks closed. An older one reads only what the
/// source has not been asked for yet.
#[allow(clippy::too_many_arguments)]
fn read(
    held: Option<&Held>,
    from_s: i64,
    to_s: i64,
    now_s: i64,
    secs: i64,
    now: Instant,
    deadline: Instant,
    fetch: &mut Fetch<'_>,
) -> Result<(Vec<Intrabar>, Held), String> {
    let per_page = 1000usize;
    let live = to_s + secs > now_s;
    let mut h = held.cloned().unwrap_or_else(|| Held {
        bars: Vec::new(),
        complete_from: None,
        refreshed: now - QUIET_REFRESH * 2,
        asked_to: i64::MIN,
        used: now,
    });
    let first = h.bars.first().map(|b| b.t);
    let last = h.bars.last().map(|b| b.t);
    // What is held overlaps the window, so it can grow; otherwise it starts over.
    let overlaps = matches!((first, last), (Some(a), Some(b)) if a < to_s && b + secs >= from_s);
    if !overlaps {
        h.bars.clear();
        h.complete_from = None;
        h.asked_to = i64::MIN;
    }
    let first = h.bars.first().map(|b| b.t);
    let last = h.bars.last().map(|b| b.t);
    let covered_back =
        h.complete_from.is_some_and(|c| c <= from_s) || first.is_some_and(|a| a <= from_s);
    // Older than what is held: read back from its oldest bar.
    if !covered_back {
        let end = first.or((!live).then_some(to_s));
        let (older, ran_out) = walk(fetch, from_s, end, per_page, deadline)?;
        if ran_out {
            h.complete_from = Some(from_s);
        }
        h.bars = merge(older, std::mem::take(&mut h.bars));
        if first.is_none() {
            h.asked_to = if live { now_s } else { to_s };
        }
        if end.is_none() {
            h.refreshed = now;
        }
    }
    // Newer than what is held. The newest held may have been forming: read it again.
    if let Some(l) = last {
        let quiet = now_s - (l + secs) >= QUIET_AFTER;
        let wait = if quiet { QUIET_REFRESH } else { REFRESH };
        let stale = now.saturating_duration_since(h.refreshed) >= wait;
        let behind_window = !live && l + secs < to_s && h.asked_to < to_s;
        if (live && stale) || behind_window {
            let want = ((to_s.min(now_s + secs) - l) / secs + 2).clamp(2, per_page as i64) as usize;
            let (newer, _) = walk(fetch, l, (!live).then_some(to_s), want, deadline)?;
            h.bars = merge(std::mem::take(&mut h.bars), newer);
            h.refreshed = now;
            h.asked_to = if live { now_s } else { to_s };
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

/// `read` of `key`, one at a time. Whoever finds one under way waits for it, until
/// `deadline`, and then reads what it left, or fails as it did rather than each in turn
/// against a source that is down. A read that fails leaves what was held as it was.
fn shared_read(
    shared: &SharedCache,
    key: CacheKey,
    (from_s, to_s, now_s, secs): (i64, i64, i64, i64),
    deadline: Instant,
    fetch: &mut Fetch<'_>,
) -> Result<Vec<Intrabar>, String> {
    let mut st = shared.lock();
    // Kept past its end: a read that failed has left `reading` by the time we wake.
    let mut joined: Option<Arc<Flight<()>>> = None;
    loop {
        if let Some(Err(e)) = joined.as_ref().and_then(|f| f.0.get()) {
            return Err(e.clone());
        }
        let Some(flight) = st.reading.get(&key).cloned() else {
            break;
        };
        joined = Some(flight);
        st = fetch::wait_for_flight(&shared.ended, st, deadline)
            .map_err(|_| "timed out waiting for another read of finer bars".to_string())?;
    }
    let held = st.held.get(&key).cloned();
    let flight = Arc::new(Flight(OnceLock::new()));
    st.reading.insert(key.clone(), flight.clone());
    let claim = Claim {
        state: &shared.state,
        ended: &shared.ended,
        flights: |c: &mut Cache| &mut c.reading,
        key: key.clone(),
        flight,
    };
    drop(st);

    let (out, held) = read(
        held.as_deref(),
        from_s,
        to_s,
        now_s,
        secs,
        Instant::now(),
        deadline,
        fetch,
    )
    .inspect_err(|e| claim.end(Err(e.clone())))?;
    let mut st = shared.lock();
    st.held.insert(key, Arc::new(held));
    st.evict();
    drop(st);
    claim.end(Ok(()));
    Ok(out)
}

fn cache() -> &'static SharedCache {
    static CACHE: OnceLock<SharedCache> = OnceLock::new();
    CACHE.get_or_init(SharedCache::default)
}

fn read_yahoo(
    symbol: &str,
    interval: &'static str,
    window: (i64, i64, i64, i64),
) -> Result<Vec<Intrabar>, String> {
    let mut fetcher = |limit: usize, end: Option<i64>| -> Result<Vec<Intrabar>, String> {
        let end_ms = end.map(|e| e * 1000);
        let fetched = fetch::fetch_yahoo(symbol, interval, limit, end_ms)?;
        Ok(fetched
            .data
            .bars
            .iter()
            .filter_map(|bar| Intrabar::from_bar(bar, None))
            .collect())
    };
    shared_read(
        cache(),
        (symbol.to_uppercase(), interval),
        window,
        Instant::now() + READ_BUDGET,
        &mut fetcher,
    )
}

pub(crate) fn read_binance(
    client: &BinanceClient,
    symbol: &str,
    kline: &'static str,
    (from_s, to_s, now_s, secs): (i64, i64, i64, i64),
) -> Result<Vec<Intrabar>, String> {
    // A window reaching the present reads the newest. An older one is a page, ending
    // where the window does. As many klines as it has slots always reach its start:
    // where they skip a time with no trades, as `1s` ones may, they only reach further.
    let end_ms = (to_s + secs <= now_s).then_some(to_s * 1000);
    let slots = (to_s.min(now_s + secs) - from_s).max(0) / secs + 2;
    let limit = (slots as usize).min(MAX_HELD);
    let klines = fetch::fetch_binance_klines(client, symbol, kline, limit, end_ms)?;
    Ok(klines
        .iter()
        .filter_map(|k| Intrabar::from_bar(&k.bar, k.taker_buy()))
        .filter(|b| (from_s..to_s).contains(&b.t))
        .collect())
}

fn read_live(
    client: &BinanceClient,
    symbol: &str,
    interval: &'static str,
    window: (i64, i64, i64, i64),
) -> Result<Vec<Intrabar>, String> {
    if fetch::detect_source(symbol) == "binance" {
        read_binance(client, symbol, interval, window)
    } else {
        read_yahoo(symbol, interval, window)
    }
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
    interval: &'static str,
    secs: i64,
    bars: &[Bar],
    last_end: i64,
) -> Result<Vec<Intrabar>, String> {
    let opens: Vec<i64> = bars.iter().filter_map(|b| b.date.parse().ok()).collect();
    let (Some(&from_s), now_s) = (opens.first(), fetch::now_ms() / 1000) else {
        return Ok(Vec::new());
    };
    if matches!(source, DataSource::Replay { .. }) {
        return Err("a replay holds no lower-timeframe bars".into());
    }
    if source != DataSource::Live {
        let last_len = last_end - opens.last().copied().unwrap_or(from_s);
        return Ok(sample_list(symbol, bars, &opens, last_len, secs, now_s));
    }
    read_live(
        fetch::binance(),
        symbol,
        interval,
        (from_s, last_end, now_s, secs),
    )
}

/// `load`, for a caller with a fallback, as a chart's load has in spreading its own
/// bars: gives up on waiting after `budget`, whatever the source is doing, even in the
/// middle of a request. The read goes on by itself, bounded by its own deadlines, so
/// what it finds is there for the next load.
pub fn load_within(
    budget: Duration,
    source: DataSource,
    symbol: &str,
    interval: &'static str,
    secs: i64,
    bars: &[Bar],
    last_end: i64,
) -> Result<Vec<Intrabar>, String> {
    if source != DataSource::Live {
        return load(source, symbol, interval, secs, bars, last_end);
    }
    let Some(from_s) = bars.iter().find_map(|b| b.date.parse().ok()) else {
        return Ok(Vec::new());
    };
    let window = (from_s, last_end, fetch::now_ms() / 1000, secs);
    read_within(budget, fetch::binance(), symbol, interval, window)
}

pub(crate) fn read_within(
    budget: Duration,
    client: &'static BinanceClient,
    symbol: &str,
    interval: &'static str,
    window: (i64, i64, i64, i64),
) -> Result<Vec<Intrabar>, String> {
    static RUNNING: AtomicUsize = AtomicUsize::new(0);
    let symbol = symbol.to_string();
    detached(&RUNNING, budget, start_thread, move || {
        read_live(client, &symbol, interval, window)
    })
}

/// Reads left running on their own at once. These threads are outside the blocking pool
/// that bounds requests, and each outlives its caller by up to its read's deadlines, so
/// a stalled source and a flood of loads would otherwise pile them up. Past this a load
/// just spreads its own bars, as it does for any other failure.
const MAX_DETACHED_READS: usize = 16;

fn start_thread(work: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("intrabars".into())
        .spawn(work)
        .map(drop)
}

/// `work` on a thread of its own, waiting for it up to `budget`. It goes on after that.
fn detached(
    running: &'static AtomicUsize,
    budget: Duration,
    start: fn(Box<dyn FnOnce() + Send>) -> std::io::Result<()>,
    work: impl FnOnce() -> Result<Vec<Intrabar>, String> + Send + 'static,
) -> Result<Vec<Intrabar>, String> {
    struct Slot(&'static AtomicUsize);
    impl Drop for Slot {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let slot = Slot(running);
    if running.fetch_add(1, Ordering::SeqCst) >= MAX_DETACHED_READS {
        return Err("too many reads of finer bars are running".into());
    }
    let (done, finished) = mpsc::channel();
    start(Box::new(move || {
        let _slot = slot;
        // Nobody may be listening any more: that is the point.
        let _ = done.send(work());
    }))
    .map_err(|e| format!("could not start a read of finer bars: {e}"))?;
    finished
        .recv_timeout(budget)
        .map_err(|_| "reading finer bars took too long".to_string())?
}

/// Bars of `block_secs` for a TPO: the source's own. The sample source draws each
/// interval on its own, so there they are built from its 1h bars, which a default
/// chart shows, or their prices would not be on that chart at all.
pub fn tpo_blocks(
    source: DataSource,
    symbol: &str,
    block: &str,
    block_secs: i64,
    limit: usize,
    end_ms: Option<i64>,
) -> Result<fetch::Fetched, String> {
    if source == DataSource::Live {
        return fetch::fetch_ohlcv(source, symbol, block, limit, end_ms);
    }
    const HOUR: i64 = 3_600;
    let parents = (limit as i64 * block_secs / HOUR + 2) as usize;
    let mut fetched = fetch::fetch_ohlcv(source, symbol, "1h", parents, end_ms)?;
    let parent_bars = std::mem::take(&mut fetched.data.bars);
    let now_s = fetch::now_ms() / 1000;
    let mut blocks: Vec<Bar> = Vec::new();
    if block_secs < HOUR {
        let opens: Vec<i64> = parent_bars
            .iter()
            .filter_map(|b| b.date.parse().ok())
            .collect();
        let list = sample_list(symbol, &parent_bars, &opens, HOUR, block_secs, now_s);
        blocks.extend(list.iter().map(|b| Bar {
            open: b.open,
            high: b.high,
            low: b.low,
            close: b.close,
            volume: b.volume,
            date: b.t.to_string(),
        }));
    } else {
        for bar in &parent_bars {
            let t: i64 = bar.date.parse().unwrap_or(0);
            let start = t - t.rem_euclid(block_secs);
            match blocks.last_mut() {
                Some(last) if last.date.parse::<i64>().ok() == Some(start) => {
                    last.high = last.high.max(bar.high);
                    last.low = last.low.min(bar.low);
                    last.close = bar.close;
                    last.volume += bar.volume;
                }
                _ => blocks.push(Bar {
                    date: start.to_string(),
                    ..bar.clone()
                }),
            }
        }
    }
    let excess = blocks.len().saturating_sub(limit);
    blocks.drain(..excess);
    fetched.data.bars = blocks;
    Ok(fetched)
}

/// Sample intrabars for `bars`, each `chart_secs` long or up to the next.
fn sample_list(
    symbol: &str,
    bars: &[Bar],
    opens: &[i64],
    last_len: i64,
    secs: i64,
    now_s: i64,
) -> Vec<Intrabar> {
    let seed = sample_seed(symbol);
    let taker = fetch::detect_source(symbol) == "binance";
    let mut out = Vec::new();
    for (k, (bar, &open)) in bars.iter().zip(opens).enumerate() {
        let end = opens.get(k + 1).copied().unwrap_or(open + last_len);
        let slots = ((end - open) / secs).max(1) as usize;
        // The forming bar has only what has passed of it.
        let count = ((now_s.min(end) - open) / secs + 1).clamp(1, slots as i64) as usize;
        out.extend(orderflow::sample_intrabars(
            seed, open, bar, secs, slots, count, taker,
        ));
    }
    out
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
    fn a_stocks_poll_keeps_the_interval_of_its_load() {
        // 60 hourly bars of a stock span 12 days: too old for 1m. Its poll's 3 bars are not.
        let load = plan("yahoo", 3_600, 60, 12 * DAY).unwrap().0;
        assert_eq!(load, "5m");
        assert_eq!(plan("yahoo", 3_600, 60, 3 * 3_600).unwrap().0, "1m");
        assert_eq!(
            plan_like_load("yahoo", 3_600, 3, 3 * 3_600, load),
            Ok(Some(("5m", 300)))
        );
        // A load with nothing finer stays that way, whatever a short window could read.
        assert_eq!(
            plan_like_load("yahoo", 3_600, 3, 3_600, NO_INTERVAL),
            Ok(None)
        );
        // Too many bars for it: what they can afford, which is coarser.
        assert_eq!(
            plan_like_load("binance", 60, 200, 0, "1s"),
            Ok(plan("binance", 60, 200, 0))
        );
        assert_eq!(
            plan_like_load("binance", 3_600, 200, 0, "1m"),
            Ok(Some(("3m", 180)))
        );
        // Yahoo has no 1s; nonsense is refused.
        assert!(plan_like_load("yahoo", 60, 3, 0, "1s").is_err());
        assert!(plan_like_load("binance", 60, 3, 0, "bogus").is_err());
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
        let far = Instant::now() + Duration::from_secs(600);
        read(held.as_ref(), from, to, now_s, 60, now, far, &mut f).unwrap()
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

    /// Polls of a market whose last bar opened at `CLOSE`, every 10 s, from `now_s`:
    /// how many of `polls` reached the source, after the load.
    fn reads_of_polls(to_s: i64, now_s: i64, polls: i64) -> usize {
        let source = Source::new(0, CLOSE);
        let start = Instant::now();
        let from = CLOSE - 600;
        let (_, mut held) = run(&source, None, from, to_s, now_s, start);
        source.requests.borrow_mut().clear();
        for k in 1..=polls {
            let at = start + Duration::from_secs(10 * k as u64);
            let (got, h) = run(&source, Some(held), from, to_s, now_s + 10 * k, at);
            assert_eq!(got.last().unwrap().t, CLOSE, "nothing lost by not asking");
            held = h;
        }
        let requests = source.requests.borrow().len();
        requests
    }

    /// Its last bar's open, on the 60 s grid.
    const CLOSE: i64 = 600_000;
    const HOUR: i64 = 3_600;

    #[test]
    fn a_closed_market_is_not_asked_on_every_poll() {
        let after = CLOSE + 6 * HOUR;
        // A bar ending with its last intrabar: over, and all of it read.
        assert_eq!(reads_of_polls(CLOSE + 60, after, 6), 0);
        // A bar whose grid ends 30 min after the close, and that time has passed.
        assert_eq!(reads_of_polls(CLOSE + 1_800, after, 6), 0);
        // The same bar in those 30 min, some way past the close: open by its grid.
        assert_eq!(reads_of_polls(CLOSE + 1_800, CLOSE + 900, 5), 0);
        // A daily bar, its session over and the bar not: no read in the first minute.
        assert_eq!(reads_of_polls(CLOSE + 8 * HOUR, after, 5), 0);
    }

    #[test]
    fn a_closed_market_is_asked_about_once_a_minute() {
        // A daily bar not over, for 6 minutes of polls: to see it open.
        let reads = reads_of_polls(CLOSE + 8 * HOUR, CLOSE + 2 * HOUR, 36);
        assert_eq!(reads, 6);
    }

    #[test]
    fn a_market_that_is_open_is_read_on_every_poll() {
        // The bar forming, and the last intrabar still within its own end.
        assert_eq!(reads_of_polls(CLOSE + HOUR, CLOSE + 30, 6), 6);
        // Just past the threshold for a quiet one is still not a closed market yet.
        assert_eq!(
            reads_of_polls(CLOSE + HOUR, CLOSE + 60 + QUIET_AFTER - 70, 6),
            6
        );
    }

    #[test]
    fn a_market_that_opens_is_read_as_often_again() {
        let source = Source::new(0, CLOSE);
        let start = Instant::now();
        let (from, to) = (CLOSE - 600, CLOSE + 8 * HOUR);
        let (_, held) = run(&source, None, from, to, CLOSE + 2 * HOUR, start);
        // It opened: a new bar, seen at the next minute's read.
        let opened = CLOSE + 2 * HOUR;
        let source = Source::new(0, opened);
        let at = start + QUIET_REFRESH;
        let (got, held) = run(&source, Some(held), from, to, opened + 5, at);
        assert_eq!(got.last().unwrap().t, opened);
        source.requests.borrow_mut().clear();
        // From there, as any open market.
        run(&source, Some(held), from, to, opened + 15, at + REFRESH);
        assert_eq!(source.requests.borrow().len(), 1);
    }

    #[test]
    fn an_older_window_past_what_was_asked_reads_at_once_though_the_market_looks_closed() {
        // Held up to 600,000, from a read that ended there. A window reaching further,
        // an hour later, must not wait for a minute's refresh with bars missing.
        let source = Source::new(0, 700_000);
        let now = Instant::now();
        let far = 10_000_000;
        let (_, held) = run(&source, None, 590_000, 600_000, far, now);
        assert_eq!(held.bars.last().unwrap().t, 599_940);
        let (got, held) = run(&source, Some(held), 590_000, 603_600, far, now);
        assert_eq!(got.last().unwrap().t, 603_540);
        source.requests.borrow_mut().clear();
        // Having asked, it does not again.
        run(&source, Some(held), 590_000, 603_600, far, now);
        assert_eq!(source.requests.borrow().len(), 0);
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
        // Nothing before the first bar on the source's grid at or after its start.
        assert_eq!(got.first().unwrap().t, 500_040);
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
                (k.to_string(), "1m"),
                Arc::new(Held {
                    bars: vec![],
                    complete_from: None,
                    refreshed: now,
                    asked_to: 0,
                    used: now + Duration::from_secs(k as u64),
                }),
            );
        }
        c.evict();
        assert_eq!(c.held.len(), MAX_ENTRIES);
        assert!(!c.held.contains_key(&("0".to_string(), "1m")));
    }

    /// Bars every 60 s up to 600,000, counting requests and answering slowly.
    struct SlowSource {
        requests: std::sync::atomic::AtomicUsize,
        fail: std::sync::atomic::AtomicBool,
        delay: Duration,
        /// While true a request does not answer: a test lets it go when it has seen
        /// what it waits for, rather than hope that a delay was long enough.
        held: Mutex<bool>,
        released: Condvar,
    }

    impl SlowSource {
        fn new(delay_ms: u64) -> Self {
            Self {
                requests: Default::default(),
                fail: Default::default(),
                delay: Duration::from_millis(delay_ms),
                held: Mutex::new(false),
                released: Condvar::new(),
            }
        }

        /// A source whose requests answer only after `release`.
        fn held() -> Self {
            let source = Self::new(0);
            *source.held.lock().unwrap() = true;
            source
        }

        fn release(&self) {
            *self.held.lock().unwrap() = false;
            self.released.notify_all();
        }

        fn requests(&self) -> usize {
            self.requests.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn get(&self, limit: usize, end: Option<i64>) -> Result<Vec<Intrabar>, String> {
            use std::sync::atomic::Ordering::SeqCst;
            self.requests.fetch_add(1, SeqCst);
            std::thread::sleep(self.delay);
            // The minute only ends a test that would hang.
            drop(
                self.released
                    .wait_timeout_while(
                        self.held.lock().unwrap(),
                        Duration::from_secs(60),
                        |held| *held,
                    )
                    .unwrap(),
            );
            if self.fail.load(SeqCst) {
                return Err("source is down".into());
            }
            let stop = end.unwrap_or(600_060).min(600_060);
            let first = ((stop - 1).div_euclid(60) * 60).saturating_sub(60 * (limit as i64 - 1));
            Ok((0..limit as i64).map(|k| ib(first + k * 60)).collect())
        }
    }

    const WINDOW: (i64, i64, i64, i64) = (540_000, 600_060, 600_030, 60);

    fn far() -> Instant {
        Instant::now() + Duration::from_secs(600)
    }

    /// Wait for `what`, which another thread brings about. The bound only ends a test
    /// that would hang; nothing is timed by it.
    fn until(what: impl Fn() -> bool) {
        let give_up = Instant::now() + Duration::from_secs(60);
        while !what() {
            assert!(Instant::now() < give_up, "waited a minute");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// How many callers wait on the read of `X` that is under way. Each holds the
    /// flight, as do the map and the reader.
    fn waiting(cache: &SharedCache) -> usize {
        let st = cache.lock();
        st.reading
            .get(&("X".to_string(), "1m"))
            .map_or(0, |flight| Arc::strong_count(flight) - 2)
    }

    fn shared(cache: &SharedCache, source: &SlowSource) -> Result<Vec<Intrabar>, String> {
        let mut f = |limit: usize, end: Option<i64>| source.get(limit, end);
        shared_read(cache, ("X".into(), "1m"), WINDOW, far(), &mut f)
    }

    #[test]
    fn callers_at_once_share_one_read() {
        let (cache, source) = (SharedCache::default(), SlowSource::new(100));
        let got: Vec<_> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..32)
                .map(|_| s.spawn(|| shared(&cache, &source)))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(source.requests(), 2, "one walk of two pages, whoever asks");
        for list in &got {
            assert_eq!(list.as_ref().unwrap().len(), 1001);
        }
    }

    #[test]
    fn a_read_that_fails_keeps_what_was_held() {
        use std::sync::atomic::Ordering::SeqCst;
        let (cache, source) = (SharedCache::default(), SlowSource::new(0));
        assert_eq!(shared(&cache, &source).unwrap().len(), 1001);
        let before = cache.lock().held[&("X".to_string(), "1m")].bars.len();
        // Old enough to read again, and the source has gone down.
        {
            let mut st = cache.lock();
            let held = st.held.get_mut(&("X".to_string(), "1m")).unwrap();
            Arc::get_mut(held).unwrap().refreshed = Instant::now() - QUIET_REFRESH * 2;
        }
        source.fail.store(true, SeqCst);
        assert_eq!(shared(&cache, &source).unwrap_err(), "source is down");
        assert_eq!(
            cache.lock().held[&("X".to_string(), "1m")].bars.len(),
            before,
            "still held"
        );
        // Back up: one short read of the newest, not the whole window again.
        source.fail.store(false, SeqCst);
        let requests = source.requests();
        assert_eq!(shared(&cache, &source).unwrap().len(), 1001);
        assert_eq!(source.requests(), requests + 1);
    }

    #[test]
    fn those_waiting_on_a_read_that_fails_fail_with_it() {
        use std::sync::atomic::Ordering::SeqCst;
        let (cache, source) = (SharedCache::default(), SlowSource::held());
        source.fail.store(true, SeqCst);
        let got: Vec<_> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|_| s.spawn(|| shared(&cache, &source)))
                .collect();
            // One reads and seven wait on it, and only then does the source answer: a
            // caller that came after the read had failed would rightly read again.
            until(|| source.requests() == 1 && waiting(&cache) == 7);
            source.release();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(got.iter().all(Result::is_err));
        assert_eq!(
            source.requests(),
            1,
            "not each in turn against a source that is down"
        );
        assert!(cache.lock().reading.is_empty());
    }

    #[test]
    fn waiting_on_a_read_gives_up_at_its_deadline_and_sends_nothing() {
        let (cache, source) = (SharedCache::default(), SlowSource::held());
        std::thread::scope(|s| {
            s.spawn(|| shared(&cache, &source).unwrap());
            // The read is under way, and stays so until the waiter has given up.
            until(|| source.requests() == 1);
            let mut f = |limit: usize, end: Option<i64>| source.get(limit, end);
            let soon = Instant::now() + Duration::from_millis(30);
            let err = shared_read(&cache, ("X".into(), "1m"), WINDOW, soon, &mut f).unwrap_err();
            assert!(err.contains("timed out"), "{err}");
            assert_eq!(source.requests(), 1, "it gave up before the read ended");
            assert!(!cache.lock().reading.is_empty());
            source.release();
        });
        assert_eq!(
            source.requests(),
            2,
            "the first's two pages, and none of the waiter's"
        );
    }

    /// A counter of its own, as other tests run theirs at the same time.
    fn counter() -> &'static AtomicUsize {
        Box::leak(Box::new(AtomicUsize::new(0)))
    }

    #[test]
    fn past_the_limit_a_detached_read_is_refused_and_a_slot_frees_when_one_ends() {
        let running = counter();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let hold = |release: Arc<(Mutex<bool>, Condvar)>| {
            move || {
                let (open, freed) = &*release;
                let guard = open.lock().unwrap();
                drop(freed.wait_while(guard, |o| !*o).unwrap());
                Ok(Vec::new())
            }
        };
        let quick = Duration::from_millis(5);
        for _ in 0..MAX_DETACHED_READS {
            let err = detached(running, quick, start_thread, hold(release.clone())).unwrap_err();
            assert!(err.contains("too long"), "{err}");
        }
        assert_eq!(running.load(Ordering::SeqCst), MAX_DETACHED_READS);
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let r = ran.clone();
        let err = detached(running, quick, start_thread, move || {
            r.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        })
        .unwrap_err();
        assert!(err.contains("too many"), "{err}");
        assert_eq!(
            running.load(Ordering::SeqCst),
            MAX_DETACHED_READS,
            "a refusal takes no slot"
        );

        // They end, and so the limit does not stay reached.
        *release.0.lock().unwrap() = true;
        release.1.notify_all();
        let give_up = Instant::now() + Duration::from_secs(10);
        while running.load(Ordering::SeqCst) > 0 && Instant::now() < give_up {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(running.load(Ordering::SeqCst), 0);
        let got = detached(running, Duration::from_secs(5), start_thread, || {
            Ok(vec![ib(1)])
        });
        assert_eq!(got.unwrap().len(), 1);
    }

    #[test]
    fn a_thread_the_system_refuses_is_an_error_and_takes_no_slot() {
        fn refused(_work: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "no threads",
            ))
        }
        let running = counter();
        let err = detached(running, Duration::from_secs(1), refused, || Ok(Vec::new()));
        assert!(err.unwrap_err().contains("could not start"));
        assert_eq!(running.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_read_that_panics_gives_its_slot_back_and_its_caller_an_error() {
        let running = counter();
        let err = detached(running, Duration::from_secs(5), start_thread, || {
            panic!("this panic is part of the test")
        });
        assert!(err.is_err());
        let give_up = Instant::now() + Duration::from_secs(5);
        while running.load(Ordering::SeqCst) > 0 && Instant::now() < give_up {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(running.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_walk_past_its_deadline_begins_no_other_request() {
        let source = SlowSource::new(30);
        let mut f = |limit: usize, end: Option<i64>| source.get(limit, end);
        // 3000 bars at 1000 a page, with time for one.
        let soon = Instant::now() + Duration::from_millis(10);
        let err = walk(&mut f, 420_000, Some(600_060), 1000, soon).unwrap_err();
        assert!(err.contains("too long"), "{err}");
        assert_eq!(source.requests(), 1);
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
