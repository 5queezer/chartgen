use crate::data::{self, Bar, OhlcvData, Session};
use chartgen::binance_gate::{self, Gate};
use chartgen::interval::{Interval, Unit};
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// Binance klines REST caps `limit` at 1000 per request.
const BINANCE_MAX_LIMIT: usize = 1000;
/// Overall budget for one paginated Binance fetch.
const BINANCE_DEADLINE: Duration = Duration::from_secs(20);
/// Most native bars one custom-interval fetch reads from Binance: 20 requests.
/// A larger ask returns the newest bars that fit, with `has_more` set, and the
/// client pages on (ADR-0038).
pub(crate) const MAX_BASE_BARS: usize = 20 * BINANCE_MAX_LIMIT;

/// Where OHLCV data comes from. `Sample` (env `CHARTGEN_DATA_SOURCE=sample`)
/// never touches the network — see `data::sample_ohlcv`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataSource {
    Live,
    Sample,
    /// Always fails, for exercising upstream-error paths in tests.
    #[cfg(test)]
    Unavailable,
}

impl DataSource {
    pub fn from_env() -> Self {
        match std::env::var("CHARTGEN_DATA_SOURCE").as_deref() {
            Ok("sample") => DataSource::Sample,
            _ => DataSource::Live,
        }
    }
}

/// Bars from a source plus whether older bars may exist before them.
pub struct Fetched {
    pub data: OhlcvData,
    pub has_more: bool,
}

/// Fetch the newest `limit` bars whose open time is strictly before
/// `end_time_ms` (or up to the still-forming bar when `None`). With
/// `end_time_ms` set, running out of history yields fewer (possibly zero)
/// bars rather than an error.
pub fn fetch_ohlcv(
    source: DataSource,
    symbol: &str,
    interval: &str,
    limit: usize,
    end_time_ms: Option<i64>,
) -> Result<Fetched, String> {
    match source {
        DataSource::Sample => {
            // As Binance would: the bars a custom interval is built from. Sample
            // months are a fixed 30 days, so they stay on their own grid.
            let iv = Interval::parse(interval)?;
            let (base, per_bar) = iv.binance_base();
            if per_bar > 1 && iv.unit != Unit::Month {
                let (want, end) = resample_window(&iv, per_bar, limit, end_time_ms);
                let fetched = data::sample_ohlcv(symbol, base, want, end, now_ms())?;
                let (bars, has_more) = resampled_bars(&fetched.bars, &iv, limit, want);
                return Ok(Fetched {
                    data: OhlcvData {
                        bars,
                        symbol: None,
                        interval: None,
                        session: None,
                    },
                    has_more,
                });
            }
            let data = data::sample_ohlcv(symbol, interval, limit, end_time_ms, now_ms())?;
            let has_more = data.len() >= limit;
            Ok(Fetched { data, has_more })
        }
        DataSource::Live => match detect_source(symbol) {
            "binance" => fetch_binance(symbol, interval, limit, end_time_ms),
            _ => fetch_yahoo(symbol, interval, limit, end_time_ms),
        },
        #[cfg(test)]
        DataSource::Unavailable => Err("upstream unavailable".into()),
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Shared blocking HTTP client; every upstream request times out after 10 s.
pub(crate) fn http() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new())
    })
}

fn bar_ts(bar: &Bar) -> Option<i64> {
    bar.date.parse::<i64>().ok()
}

/// Open time in ms of a bar whose `date` holds unix seconds.
fn bar_open_ms(bar: &Bar) -> Option<i64> {
    bar_ts(bar).map(|s| s.saturating_mul(1000))
}

fn binance_klines_url(symbol: &str, interval: &str, limit: usize, end_ms: Option<i64>) -> String {
    let mut url = format!(
        "https://api.binance.com/api/v3/klines?symbol={}&interval={}&limit={}",
        percent_encode(&symbol.to_uppercase()),
        interval,
        limit
    );
    if let Some(end) = end_ms {
        url.push_str(&format!("&endTime={end}"));
    }
    url
}

/// Collect `limit` bars by walking backwards from `end_time_ms` (exclusive)
/// in pages of at most `BINANCE_MAX_LIMIT`. `page(page_limit, end_inclusive)`
/// performs one request; a short page means history is exhausted. Fails if
/// `deadline` passes before the walk completes.
fn paginate_backwards<F>(
    limit: usize,
    end_time_ms: Option<i64>,
    deadline: Instant,
    mut page: F,
) -> Result<Vec<Bar>, String>
where
    F: FnMut(usize, Option<i64>) -> Result<Vec<Bar>, String>,
{
    let mut out: Vec<Bar> = Vec::new();
    // Binance's endTime is inclusive; ours is exclusive.
    let mut end = end_time_ms.map(|e| e.saturating_sub(1));
    while out.len() < limit {
        if Instant::now() >= deadline {
            return Err("Binance pagination exceeded its time budget".into());
        }
        let want = (limit - out.len()).min(BINANCE_MAX_LIMIT);
        let mut chunk = page(want, end)?;
        if let Some(e) = end_time_ms {
            chunk.retain(|b| bar_open_ms(b).is_some_and(|t| t < e));
        }
        let got = chunk.len();
        let Some(oldest) = chunk.first().and_then(bar_open_ms) else {
            break;
        };
        chunk.append(&mut out);
        out = chunk;
        if got < want {
            break;
        }
        end = Some(oldest.saturating_sub(1));
    }
    Ok(out)
}

pub fn fetch_binance(
    symbol: &str,
    interval: &str,
    limit: usize,
    end_time_ms: Option<i64>,
) -> Result<Fetched, String> {
    fetch_binance_via(binance(), symbol, interval, limit, end_time_ms)
}

fn fetch_binance_via(
    client: &BinanceClient,
    symbol: &str,
    interval: &str,
    limit: usize,
    end_time_ms: Option<i64>,
) -> Result<Fetched, String> {
    let iv = Interval::parse(interval)?;
    let (base, per_bar) = iv.binance_base();
    let resampled = per_bar > 1;
    let (want, end) = if resampled {
        resample_window(&iv, per_bar, limit, end_time_ms)
    } else {
        (limit, end_time_ms)
    };
    let deadline = Instant::now() + BINANCE_DEADLINE;
    let page = |page_limit: usize, end: Option<i64>| {
        fetch_binance_page(client, &binance_klines_url(symbol, base, page_limit, end))
    };
    let symbol_key = symbol.to_uppercase();
    let fetched = match end {
        None => newest_klines(
            &client.klines,
            &client.walks,
            (symbol_key, base),
            want,
            &*client.now,
            deadline,
            page,
        )?,
        Some(end) => shared_page(
            &client.pages,
            (symbol_key, base, end, want),
            &*client.now,
            deadline,
            || walk_klines(&client.walks, want, Some(end), deadline, page),
        )?,
    };

    let (bars, has_more) = if resampled {
        resampled_bars(&fetched, &iv, limit, want)
    } else {
        let more = fetched.len() >= want;
        (fetched, more)
    };

    if bars.is_empty() && end_time_ms.is_none() {
        return Err(NO_BARS.into());
    }

    Ok(Fetched {
        data: OhlcvData {
            bars,
            symbol: None,
            interval: None,
            session: None,
        },
        has_more,
    })
}

/// Native bars to read for `limit` bars of `iv`, and where they end.
fn resample_window(
    iv: &Interval,
    per_bar: i64,
    limit: usize,
    end_time_ms: Option<i64>,
) -> (usize, Option<i64>) {
    // One bar more than asked: the oldest may be cut off by the window.
    let want = limit
        .saturating_add(1)
        .saturating_mul(per_bar as usize)
        .min(MAX_BASE_BARS);
    // A bar opening before `end_time_ms` takes in klines up to its own end.
    let end = end_time_ms.map(|e| {
        iv.grid_end(e.saturating_sub(1).div_euclid(1000))
            .saturating_mul(1000)
    });
    (want, end)
}

/// The newest `limit` bars of `iv` from the `want` native bars asked for, and
/// whether older ones exist.
fn resampled_bars(native: &[Bar], iv: &Interval, limit: usize, want: usize) -> (Vec<Bar>, bool) {
    let reached_want = native.len() >= want;
    let mut bars = resample(native, iv, reached_want);
    let older = bars.len() > limit;
    if older {
        bars.drain(..bars.len() - limit);
    }
    (bars, reached_want || older)
}

/// Both clocks a cache reads: monotonic for ages, unix ms for kline boundaries.
#[derive(Clone, Copy)]
struct Now {
    mono: Instant,
    wall_ms: i64,
}

type Clock<'a> = &'a (dyn Fn() -> Now + Send + Sync);

/// How many symbol and kline pairs `newest_klines` holds, how many klines in all,
/// and for how long since last used. A kline is about 96 bytes (64 for the `Bar`, 32
/// for its date), so at most about 25 MB: 14 of the longest custom charts, or 256
/// ordinary ones, which read a page of 1000.
const KLINE_CACHE_ENTRIES: usize = 256;
const KLINE_CACHE_BARS: usize = 256_000;
const KLINE_CACHE_TTL: Duration = Duration::from_secs(120);
/// Klines a refresh reads: enough to overlap after up to `KLINE_CACHE_TTL` on
/// the finest kline, with the forming one.
const KLINE_REFRESH: usize = 16;
/// A repeat this soon after the read it would repeat sends nothing. Half the web
/// app's 2 s poll, so any number of pollers of a chart cost one request a second
/// and each still sees the candle move on every poll.
const KLINE_FRESH: Duration = Duration::from_secs(1);
/// While one caller refreshes, others get the klines held if they are this young,
/// rather than a blocking thread each. Older, they wait for the refresh.
const KLINE_STALE: Duration = Duration::from_secs(3);
/// `end_time` pages kept once read, in entries, klines (about 6 MB) and seconds.
const PAGE_CACHE_ENTRIES: usize = 128;
const PAGE_CACHE_BARS: usize = 64_000;
const PAGE_CACHE_TTL: Duration = Duration::from_secs(60);

const NO_BARS: &str = "Binance returned zero bars";
/// What joiners of a read get if its thread panicked.
const READ_ABORTED: &str = "Binance read failed unexpectedly; try again shortly";

/// A read under way that others may join, and how it ended once it has.
struct Flight<T>(OnceLock<Result<T, String>>);

impl<T> Flight<T> {
    fn running(&self) -> bool {
        self.0.get().is_none()
    }
}

/// The turn to run a flight. Ends however the read does, panics included, so
/// nobody who joined it hangs.
struct Claim<'a, S, T> {
    state: &'a Mutex<S>,
    ended: &'a Condvar,
    flight: Arc<Flight<T>>,
}

impl<S, T> Claim<'_, S, T> {
    /// How the read ended, for those who joined it. They learn when this drops.
    fn end(&self, how: Result<T, String>) {
        let _ = self.flight.0.set(how);
    }
}

impl<S, T> Drop for Claim<'_, S, T> {
    fn drop(&mut self) {
        self.end(Err(READ_ABORTED.into()));
        // Under the lock, so a joiner between its check and its wait still hears.
        let _state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.ended.notify_all();
    }
}

/// Sleep until someone's read ends or `deadline` passes, which gives `BUSY`.
fn wait_for_flight<'a, S>(
    ended: &Condvar,
    state: MutexGuard<'a, S>,
    deadline: Instant,
) -> Result<MutexGuard<'a, S>, String> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(BUSY.into());
    }
    Ok(ended
        .wait_timeout(state, left)
        .unwrap_or_else(|e| e.into_inner())
        .0)
}

/// Drop the least recently used entries, other than `keep`, down to the bounds.
/// `info` gives an entry's last use and its klines.
fn evict<K: Eq + Hash + Clone, V>(
    held: &mut HashMap<K, V>,
    keep: &K,
    (entries, bars): (usize, usize),
    info: impl Fn(&V) -> (Instant, usize),
) {
    let mut total: usize = held.values().map(|v| info(v).1).sum();
    while held.len() > entries || total > bars {
        let oldest = held
            .iter()
            .filter(|(k, _)| *k != keep)
            .min_by_key(|(_, v)| info(v).0)
            .map(|(k, _)| k.clone());
        let Some(gone) = oldest.and_then(|k| held.remove(&k)) else {
            break;
        };
        total -= info(&gone).1;
    }
}

/// Unix ms at which the `kline` holding `t_ms` closes.
fn kline_end_ms(kline: &str, t_ms: i64) -> i64 {
    Interval::parse(kline).map_or(i64::MAX, |iv| {
        iv.grid_end(t_ms.div_euclid(1000)).saturating_mul(1000)
    })
}

struct HeldKlines {
    used: Instant,
    /// When the read behind the newest klines was sent.
    read: Instant,
    /// When the kline forming at that moment closes. Nothing is served past it, so
    /// a closed kline is never missing or short of its last trades.
    read_slot_end_ms: i64,
    bars: Arc<Vec<Bar>>,
    /// A full read found less than it asked for: nothing older exists.
    complete: bool,
}

impl HeldKlines {
    fn serves(&self, now: Now, age: Duration) -> bool {
        now.wall_ms < self.read_slot_end_ms && now.mono.saturating_duration_since(self.read) < age
    }

    fn stamp(&mut self, sent: Now, kline: &str) {
        self.used = sent.mono;
        self.read = sent.mono;
        self.read_slot_end_ms = kline_end_ms(kline, sent.wall_ms);
    }
}

type KlineKey = (String, &'static str);
type KlineFlight = Arc<Flight<()>>;

#[derive(Default)]
struct KlineState {
    held: HashMap<KlineKey, HeldKlines>,
    /// Reads of klines older than those held, or of the first ones.
    walking: HashMap<KlineKey, KlineFlight>,
    /// Reads of the newest klines over those held. They do not touch the older end,
    /// so a chart's refresh need not wait for another chart's long walk.
    refreshing: HashMap<KlineKey, KlineFlight>,
}

#[derive(Default)]
struct KlineCache {
    state: Mutex<KlineState>,
    ended: Condvar,
}

type KlineClaim<'a> = Claim<'a, KlineState, ()>;

enum KlineRead<'a> {
    Held(Arc<Vec<Bar>>),
    Refresh(KlineClaim<'a>),
    /// Read on from the open time of the oldest held, of how many held.
    Walk(KlineClaim<'a>, Option<(i64, usize)>),
}

impl KlineCache {
    fn lock(&self) -> MutexGuard<'_, KlineState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn claim(&self, flight: &KlineFlight) -> KlineClaim<'_> {
        Claim {
            state: &self.state,
            ended: &self.ended,
            flight: flight.clone(),
        }
    }

    /// Klines to serve `want` from, or this thread's turn to read, waiting up to
    /// `deadline` for a read already under way. Whoever waits for a refresh takes
    /// its result however long it took, or a slow Binance would have each caller
    /// find it too old and refresh again until all time out. A walk says nothing
    /// about the newest klines, which were read when it began, so those who wait
    /// for one look again. A failed read fails those who joined it, not each in turn.
    fn begin(
        &self,
        key: &KlineKey,
        want: usize,
        clock: Clock<'_>,
        deadline: Instant,
    ) -> Result<KlineRead<'_>, String> {
        let mut guard = self.lock();
        // The flight waited for, and whether it is a refresh.
        let mut joined: Option<(KlineFlight, bool)> = None;
        let mut refreshed = false;
        loop {
            if let Some(end) = joined.as_ref().and_then(|(f, _)| f.0.get().cloned()) {
                end?;
                refreshed = joined.take().is_some_and(|(_, refresh)| refresh);
            }
            let now = clock();
            let st = &mut *guard;
            st.held
                .retain(|_, h| now.mono.saturating_duration_since(h.used) < KLINE_CACHE_TTL);
            st.walking.retain(|_, f| f.running());
            st.refreshing.retain(|_, f| f.running());
            let refresh = st.refreshing.get(key).cloned();
            let covering = st
                .held
                .get_mut(key)
                .filter(|h| h.complete || h.bars.len() >= want);
            joined = Some(if let Some(h) = covering {
                let age = match (refreshed, &refresh) {
                    (true, _) => Duration::MAX,
                    (false, Some(_)) => KLINE_STALE,
                    (false, None) => KLINE_FRESH,
                };
                if h.serves(now, age) {
                    h.used = now.mono;
                    return Ok(KlineRead::Held(h.bars.clone()));
                }
                let Some(flight) = refresh else {
                    let flight = Arc::new(Flight(OnceLock::new()));
                    st.refreshing.insert(key.clone(), flight.clone());
                    return Ok(KlineRead::Refresh(self.claim(&flight)));
                };
                (flight, true)
            } else if let Some(flight) = st.walking.get(key) {
                (flight.clone(), false)
            } else {
                let flight = Arc::new(Flight(OnceLock::new()));
                st.walking.insert(key.clone(), flight.clone());
                let from = st
                    .held
                    .get(key)
                    .and_then(|h| Some((bar_open_ms(h.bars.first()?)?, h.bars.len())));
                return Ok(KlineRead::Walk(self.claim(&flight), from));
            });
            guard = wait_for_flight(&self.ended, guard, deadline)?;
        }
    }
}

struct HeldPage {
    read: Instant,
    bars: Arc<Vec<Bar>>,
}

/// Symbol, kline, exclusive end and klines asked for.
type PageKey = (String, &'static str, i64, usize);

#[derive(Default)]
struct PageState {
    held: HashMap<PageKey, HeldPage>,
    flying: HashMap<PageKey, Arc<Flight<Arc<Vec<Bar>>>>>,
}

#[derive(Default)]
struct PageCache {
    state: Mutex<PageState>,
    ended: Condvar,
}

/// The klines of an `end_time` page: from a recent read of the same page, or a read
/// under way, or `read`. A page reaching the forming kline is shared but not kept,
/// since keeping it could not bound how old that kline gets. Anything else is closed
/// and never changes, so history that ran out is kept as well.
fn shared_page<F>(
    cache: &PageCache,
    key: PageKey,
    clock: Clock<'_>,
    deadline: Instant,
    read: F,
) -> Result<Vec<Bar>, String>
where
    F: FnOnce() -> Result<Vec<Bar>, String>,
{
    let mut guard = cache.state.lock().unwrap_or_else(|e| e.into_inner());
    let now = clock();
    let st = &mut *guard;
    st.held
        .retain(|_, p| now.mono.saturating_duration_since(p.read) < PAGE_CACHE_TTL);
    st.flying.retain(|_, f| f.running());
    if let Some(bars) = st.held.get(&key).map(|p| p.bars.clone()) {
        drop(guard);
        return Ok(bars.to_vec());
    }
    if let Some(flight) = st.flying.get(&key).cloned() {
        loop {
            if let Some(end) = flight.0.get() {
                drop(guard);
                return end.clone().map(|bars| bars.to_vec());
            }
            guard = wait_for_flight(&cache.ended, guard, deadline)?;
        }
    }
    let flight = Arc::new(Flight(OnceLock::new()));
    st.flying.insert(key.clone(), flight.clone());
    let claim = Claim {
        state: &cache.state,
        ended: &cache.ended,
        flight,
    };
    drop(guard);

    let mut bars = read().inspect_err(|e| claim.end(Err(e.clone())))?;
    bars.shrink_to_fit();
    let bars = Arc::new(bars);
    let closed = bars
        .last()
        .and_then(bar_open_ms)
        .is_none_or(|t| kline_end_ms(key.1, t) <= now.wall_ms);
    if closed {
        let mut st = cache.state.lock().unwrap_or_else(|e| e.into_inner());
        let page = HeldPage {
            read: now.mono,
            bars: bars.clone(),
        };
        st.held.insert(key.clone(), page);
        evict(
            &mut st.held,
            &key,
            (PAGE_CACHE_ENTRIES, PAGE_CACHE_BARS),
            |p| (p.read, p.bars.len()),
        );
    }
    claim.end(Ok(bars.clone()));
    Ok(bars.to_vec())
}

/// Paged walks of Binance's history allowed at once. At about 0.3 s a page, four
/// cost about 1600 weight a minute, well under the gate's threshold.
const MAX_WALKS: usize = 4;
/// Says a call gave up waiting; it spent no request. Not the rate limit: others'
/// reads may just be slow, as in an outage.
const BUSY: &str = "Binance: timed out waiting for other history reads; try again shortly";

#[derive(Default)]
struct Walks {
    running: Mutex<usize>,
    free: Condvar,
}

struct WalkPermit<'a>(&'a Walks);

impl Drop for WalkPermit<'_> {
    fn drop(&mut self) {
        *self.0.running.lock().unwrap_or_else(|e| e.into_inner()) -= 1;
        self.0.free.notify_one();
    }
}

impl Walks {
    fn enter(&self, deadline: Instant) -> Result<WalkPermit<'_>, String> {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        while *running >= MAX_WALKS {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(BUSY.into());
            }
            running = self
                .free
                .wait_timeout(running, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        *running += 1;
        Ok(WalkPermit(self))
    }
}

/// `paginate_backwards`, holding a walk permit when it takes more than one page.
/// The wait for one comes out of `deadline`, and the walk then gets its own.
fn walk_klines<F>(
    walks: &Walks,
    want: usize,
    end: Option<i64>,
    deadline: Instant,
    page: F,
) -> Result<Vec<Bar>, String>
where
    F: FnMut(usize, Option<i64>) -> Result<Vec<Bar>, String>,
{
    if want <= BINANCE_MAX_LIMIT {
        return paginate_backwards(want, end, deadline, page);
    }
    let _permit = walks.enter(deadline)?;
    paginate_backwards(want, end, Instant::now() + BINANCE_DEADLINE, page)
}

/// Replace the end of `bars` with `fresh`, from its first bar on. False, leaving
/// `bars` alone, if `fresh` does not reach them.
fn lay_over(bars: &mut Vec<Bar>, fresh: &[Bar]) -> bool {
    let reaches = fresh.first().and_then(bar_ts).filter(|first| {
        bars.last()
            .and_then(bar_ts)
            .is_some_and(|last| last >= *first)
    });
    let Some(first) = reaches else {
        return false;
    };
    bars.truncate(bars.partition_point(|b| bar_ts(b).is_some_and(|t| t < first)));
    // Exact, so a held window does not double its memory for one new kline.
    bars.reserve_exact(fresh.len());
    bars.extend_from_slice(fresh);
    true
}

/// The newest `want` klines of `key`, for every read without an `end_time`. A closed
/// kline never changes, so one window per symbol and kline serves every chart on it,
/// whatever its size: only klines it lacks are read, in whole pages since a page
/// costs the same weight at any size, so reads of mixed sizes add up to one walk. A
/// repeat within `KLINE_FRESH` sends nothing, and after that one caller reads a small
/// page and lays it over the window (see `KlineCache::begin` for the others). A
/// refused or failed read stores nothing. `page` is as for `paginate_backwards`.
fn newest_klines<F>(
    cache: &KlineCache,
    walks: &Walks,
    key: KlineKey,
    want: usize,
    clock: Clock<'_>,
    deadline: Instant,
    mut page: F,
) -> Result<Vec<Bar>, String>
where
    F: FnMut(usize, Option<i64>) -> Result<Vec<Bar>, String>,
{
    let want = want.min(MAX_BASE_BARS);
    let newest = |bars: &[Bar]| bars[bars.len().saturating_sub(want)..].to_vec();
    let bounds = (KLINE_CACHE_ENTRIES, KLINE_CACHE_BARS);
    let info = |h: &HeldKlines| (h.used, h.bars.len());
    loop {
        match cache.begin(&key, want, clock, deadline)? {
            KlineRead::Held(bars) => return Ok(newest(&bars)),
            KlineRead::Refresh(claim) => {
                let sent = clock();
                let fresh = page(KLINE_REFRESH, None).inspect_err(|e| claim.end(Err(e.clone())))?;
                let mut st = cache.lock();
                let laid = st.held.get_mut(&key).and_then(|h| {
                    let bars = Arc::make_mut(&mut h.bars);
                    if !lay_over(bars, &fresh) {
                        return None;
                    }
                    if bars.len() > MAX_BASE_BARS {
                        bars.drain(..bars.len() - MAX_BASE_BARS);
                        h.complete = false;
                    }
                    h.stamp(sent, key.1);
                    Some(h.bars.clone())
                });
                let Some(bars) = laid else {
                    // A gap means the page does not reach what we hold: read it all
                    // again. Or it was evicted meanwhile, which comes to the same.
                    st.held.remove(&key);
                    drop(st);
                    claim.end(Ok(()));
                    continue;
                };
                evict(&mut st.held, &key, bounds, info);
                drop(st);
                claim.end(Ok(()));
                return Ok(newest(&bars));
            }
            KlineRead::Walk(claim, from) => {
                let sent = clock();
                let (end, have) = from.map_or((None, 0), |(first, len)| (Some(first), len));
                let read = want
                    .next_multiple_of(BINANCE_MAX_LIMIT)
                    .min(MAX_BASE_BARS)
                    .saturating_sub(have);
                let mut got = walk_klines(walks, read, end, deadline, &mut page)
                    .inspect_err(|e| claim.end(Err(e.clone())))?;
                let complete = got.len() < read;
                let mut st = cache.lock();
                let Some(first) = end else {
                    if got.is_empty() {
                        drop(st);
                        claim.end(Err(NO_BARS.into()));
                        return Err(NO_BARS.into());
                    }
                    got.shrink_to_fit();
                    let bars = Arc::new(got);
                    let mut held = HeldKlines {
                        used: sent.mono,
                        read: sent.mono,
                        read_slot_end_ms: 0,
                        bars: bars.clone(),
                        complete,
                    };
                    held.stamp(sent, key.1);
                    st.held.insert(key.clone(), held);
                    evict(&mut st.held, &key, bounds, info);
                    drop(st);
                    claim.end(Ok(()));
                    return Ok(newest(&bars));
                };
                // Only if the window still starts where this read went on from: it
                // may have been read again from a gap meanwhile.
                let same =
                    |h: &&mut HeldKlines| h.bars.first().and_then(bar_open_ms) == Some(first);
                if let Some(h) = st.held.get_mut(&key).filter(same) {
                    got.reserve_exact(h.bars.len());
                    got.extend_from_slice(&h.bars);
                    got.shrink_to_fit();
                    h.bars = Arc::new(got);
                    h.complete = complete;
                    h.used = sent.mono;
                    evict(&mut st.held, &key, bounds, info);
                }
                drop(st);
                claim.end(Ok(()));
                // Serve it as any other reader would: its newest klines are the
                // window's, which may need a refresh by now.
            }
        }
    }
}

/// Bars of `iv` from ascending native bars that lie on its grid (24/7 markets,
/// so no sessions). `may_be_cut`: older klines may exist, so a first bar that
/// does not start on its grid slot is incomplete and dropped. Otherwise it
/// is the symbol's first and stays.
pub(crate) fn resample(bars: &[Bar], iv: &Interval, may_be_cut: bool) -> Vec<Bar> {
    let min_start = match bars.first().and_then(bar_ts) {
        Some(first) if may_be_cut => first,
        _ => i64::MIN,
    };
    if iv.is_date_bar() {
        aggregate_calendar(bars, iv, 0, min_start)
    } else {
        aggregate_bars(bars, iv.secs(), min_start)
    }
}

/// One REST response, whatever its status.
pub(crate) struct BinanceResponse {
    pub status: u16,
    retry_after: Option<String>,
    used_weight: Option<String>,
    pub body: String,
}

type BinanceSend = Box<dyn Fn(&str) -> Result<BinanceResponse, String> + Send + Sync>;

/// Every spot REST request goes through here, so all of them answer to one budget.
pub(crate) struct BinanceClient {
    gate: &'static Gate,
    walks: Walks,
    klines: KlineCache,
    pages: PageCache,
    now: Box<dyn Fn() -> Now + Send + Sync>,
    send: BinanceSend,
}

impl BinanceClient {
    /// GET `url` if the gate admits it, and tell the gate how it went.
    pub(crate) fn get(&self, url: &str) -> Result<BinanceResponse, String> {
        let sent = self.gate.admit()?;
        let resp = (self.send)(url)?;
        self.gate.observe(
            sent,
            resp.status,
            resp.retry_after.as_deref(),
            resp.used_weight.as_deref(),
        );
        Ok(resp)
    }
}

fn http_send(url: &str) -> Result<BinanceResponse, String> {
    let resp = http()
        .get(url)
        .send()
        .map_err(|e| format!("Binance request failed: {e}"))?;
    let header = |name: &str| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let (status, retry_after, used_weight) = (
        resp.status().as_u16(),
        header("retry-after"),
        header("x-mbx-used-weight-1m"),
    );
    let body = resp
        .text()
        .map_err(|e| format!("Failed to read Binance response: {e}"))?;
    Ok(BinanceResponse {
        status,
        retry_after,
        used_weight,
        body,
    })
}

#[cfg(test)]
thread_local! {
    /// What `binance()` answers on this thread, so a test of a whole tool call can
    /// reach a pretend exchange.
    static TEST_CLIENT: std::cell::Cell<Option<&'static BinanceClient>> =
        const { std::cell::Cell::new(None) };
}

pub(crate) fn binance() -> &'static BinanceClient {
    #[cfg(test)]
    if let Some(client) = TEST_CLIENT.get() {
        return client;
    }
    static CLIENT: OnceLock<BinanceClient> = OnceLock::new();
    CLIENT.get_or_init(|| BinanceClient {
        gate: binance_gate::spot(),
        walks: Walks::default(),
        klines: KlineCache::default(),
        pages: PageCache::default(),
        now: Box::new(|| Now {
            mono: Instant::now(),
            wall_ms: now_ms(),
        }),
        send: Box::new(http_send),
    })
}

fn fetch_binance_page(client: &BinanceClient, url: &str) -> Result<Vec<Bar>, String> {
    let resp = client.get(url)?;

    if !(200..300).contains(&resp.status) {
        return Err(format!("Binance API returned status {}", resp.status));
    }

    let body: serde_json::Value = serde_json::from_str(&resp.body)
        .map_err(|e| format!("Failed to parse Binance response: {}", e))?;

    let klines = body.as_array().ok_or("Binance response is not an array")?;

    let mut bars = Vec::with_capacity(klines.len());
    for kline in klines {
        let arr = kline
            .as_array()
            .ok_or("Binance kline entry is not an array")?;
        if arr.len() < 6 {
            return Err("Binance kline entry has fewer than 6 elements".into());
        }

        let open_time = arr[0].as_i64().ok_or("open_time is not a number")?;
        let open: f64 = arr[1]
            .as_str()
            .ok_or("open is not a string")?
            .parse()
            .map_err(|e| format!("Failed to parse open: {}", e))?;
        let high: f64 = arr[2]
            .as_str()
            .ok_or("high is not a string")?
            .parse()
            .map_err(|e| format!("Failed to parse high: {}", e))?;
        let low: f64 = arr[3]
            .as_str()
            .ok_or("low is not a string")?
            .parse()
            .map_err(|e| format!("Failed to parse low: {}", e))?;
        let close: f64 = arr[4]
            .as_str()
            .ok_or("close is not a string")?
            .parse()
            .map_err(|e| format!("Failed to parse close: {}", e))?;
        let volume: f64 = arr[5]
            .as_str()
            .ok_or("volume is not a string")?
            .parse()
            .map_err(|e| format!("Failed to parse volume: {}", e))?;

        bars.push(Bar {
            open,
            high,
            low,
            close,
            volume,
            date: format!("{}", open_time / 1000),
        });
    }

    Ok(bars)
}

pub fn fetch_yahoo(
    symbol: &str,
    interval: &str,
    limit: usize,
    end_time_ms: Option<i64>,
) -> Result<Fetched, String> {
    let interval = Interval::parse(interval)?.canonical();
    let interval = interval.as_str();
    let window = yahoo_window(interval, limit, end_time_ms, now_ms() / 1000);

    if window.period1 >= window.period2 {
        // Entire window lies beyond Yahoo's intraday lookback.
        return match end_time_ms {
            Some(_) => Ok(Fetched {
                data: OhlcvData {
                    bars: Vec::new(),
                    symbol: None,
                    interval: None,
                    session: None,
                },
                has_more: false,
            }),
            None => Err("Requested window is outside Yahoo's history limit".into()),
        };
    }

    let url = yahoo_chart_url(symbol, &window)?;

    let resp = http()
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .map_err(|e| format!("Yahoo request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Yahoo API returned status {}", resp.status()));
    }

    let body: serde_json::Value = resp
        .json()
        .map_err(|e| format!("Failed to parse Yahoo response: {}", e))?;

    parse_yahoo_chart(&body, interval, limit, end_time_ms, &window)
}

/// Turn a Yahoo chart response into at most `limit` bars of `interval`:
/// drops bars at or after `end_time_ms`, snaps the live bar onto its period,
/// and resamples the intervals Yahoo lacks (see `yahoo_api_interval`). Pure, so fixture JSON can drive it in tests.
fn parse_yahoo_chart(
    body: &serde_json::Value,
    interval: &str,
    limit: usize,
    end_time_ms: Option<i64>,
    window: &YahooWindow,
) -> Result<Fetched, String> {
    let iv = Interval::parse(interval)?;
    let needs_aggregation = data::interval_secs(window.api_interval) != Some(iv.secs());
    let empty = Fetched {
        data: OhlcvData {
            bars: Vec::new(),
            symbol: None,
            interval: None,
            session: None,
        },
        has_more: false,
    };

    let result = body
        .get("chart")
        .and_then(|c| c.get("result"))
        .and_then(|r| r.as_array())
        .and_then(|a| a.first())
        .ok_or("Yahoo response missing chart.result[0]")?;

    let meta = result.get("meta");
    let meta_i64 = |path: &[&str]| {
        path.iter()
            .try_fold(meta?, |v, k| v.get(k))
            .and_then(|v| v.as_i64())
    };
    let session_start = meta_i64(&["currentTradingPeriod", "regular", "start"]);
    let session_end = meta_i64(&["currentTradingPeriod", "regular", "end"]);
    let gmtoffset = meta_i64(&["gmtoffset"]).unwrap_or(0);
    // 24/7 instruments have no session to anchor resampled buckets on.
    let all_day = matches!(
        meta.and_then(|m| m.get("instrumentType"))
            .and_then(|v| v.as_str()),
        Some("CRYPTOCURRENCY" | "CURRENCY")
    ) || meta
        .and_then(|m| m.get("symbol"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| s.ends_with("=X"));
    // Older bars exist unless period1 hit Yahoo's lookback limit or the
    // symbol's first trade lies inside the window.
    let has_more = !window.age_clamped
        && meta_i64(&["firstTradeDate"]).is_none_or(|first| first < window.period1);

    // Yahoo omits `timestamp` when the window holds no bars.
    let timestamps = match result.get("timestamp").and_then(|t| t.as_array()) {
        Some(t) => t,
        None if end_time_ms.is_some() => return Ok(empty),
        None => return Err("Yahoo response missing timestamps".into()),
    };

    let quote = result
        .get("indicators")
        .and_then(|i| i.get("quote"))
        .and_then(|q| q.as_array())
        .and_then(|a| a.first())
        .ok_or("Yahoo response missing indicators.quote[0]")?;

    let opens = quote
        .get("open")
        .and_then(|v| v.as_array())
        .ok_or("Missing open array")?;
    let highs = quote
        .get("high")
        .and_then(|v| v.as_array())
        .ok_or("Missing high array")?;
    let lows = quote
        .get("low")
        .and_then(|v| v.as_array())
        .ok_or("Missing low array")?;
    let closes = quote
        .get("close")
        .and_then(|v| v.as_array())
        .ok_or("Missing close array")?;
    let volumes = quote
        .get("volume")
        .and_then(|v| v.as_array())
        .ok_or("Missing volume array")?;

    let date_bars = iv.is_date_bar();
    // Weekly and monthly bars are stamped at local midnight, in the offset of
    // their own date: across a DST change the current offset reads that as
    // 23:00 the day before. Rounding to the nearest midnight keeps the date.
    let stamp_ts = |ts: i64| match iv.unit {
        Unit::Week | Unit::Month => ts + DAY / 2,
        _ => ts,
    };
    let mut bars = Vec::new();
    #[allow(clippy::needless_range_loop)] // indexing 6 parallel arrays
    for i in 0..timestamps.len() {
        let ts = match timestamps[i].as_i64() {
            Some(v) => v,
            None => continue,
        };
        // A date bar is compared by its stamp (see `local_date_stamp`), so a
        // history page that ends at a bar's stamp does not return it again.
        let stamp = if date_bars {
            local_date_stamp(stamp_ts(ts), gmtoffset)
        } else {
            ts
        };
        if end_time_ms.is_some_and(|e| stamp.saturating_mul(1000) >= e) {
            continue;
        }
        let open = match opens.get(i).and_then(|v| v.as_f64()) {
            Some(v) => v,
            None => continue,
        };
        let high = match highs.get(i).and_then(|v| v.as_f64()) {
            Some(v) => v,
            None => continue,
        };
        let low = match lows.get(i).and_then(|v| v.as_f64()) {
            Some(v) => v,
            None => continue,
        };
        let close = match closes.get(i).and_then(|v| v.as_f64()) {
            Some(v) => v,
            None => continue,
        };
        let volume = volumes.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0);

        bars.push(Bar {
            open,
            high,
            low,
            close,
            volume,
            date: format!("{}", ts),
        });
    }

    if let Some(step) = data::interval_secs(window.api_interval) {
        // Today's session bounds only describe the live tail, not a history page.
        let (open, close) = if end_time_ms.is_none() {
            (session_start, session_end)
        } else {
            (None, None)
        };
        snap_live_bar(&mut bars, step, open, close);
    }

    if needs_aggregation {
        let secs = iv.secs();
        // Session open as an exchange-local time of day.
        let open_tod = session_start.map(|s| (s + gmtoffset).rem_euclid(DAY));
        bars = if iv.is_date_bar() {
            let offset = if all_day { 0 } else { gmtoffset };
            aggregate_calendar(&bars, &iv, offset, window.period1)
        } else if all_day || 900 % secs == 0 {
            // 3m divides every exchange's 15-minute open, so the epoch grid
            // is the session grid.
            aggregate_bars(&bars, secs, window.period1)
        } else {
            // Rows without a price still mark when the session opened.
            let raw: Vec<i64> = timestamps.iter().filter_map(|t| t.as_i64()).collect();
            let opens: Vec<i64> = raw
                .iter()
                .enumerate()
                .filter(|(i, t)| *i == 0 || **t - raw[*i - 1] > SESSION_GAP)
                .map(|(_, t)| *t)
                .collect();
            aggregate_sessions(&bars, secs, gmtoffset, window.period1, open_tod, &opens)
        };
    }

    if date_bars {
        for b in &mut bars {
            if let Some(t) = bar_ts(b) {
                b.date = local_date_stamp(stamp_ts(t), gmtoffset).to_string();
            }
        }
    }

    if bars.is_empty() {
        return match end_time_ms {
            Some(_) => Ok(empty),
            None => Err("Yahoo returned zero bars".into()),
        };
    }

    // Take last N bars
    if bars.len() > limit {
        bars = bars.split_off(bars.len() - limit);
    }

    let session = if all_day {
        None
    } else {
        meta.and_then(session_from_meta)
    };

    Ok(Fetched {
        data: OhlcvData {
            bars,
            symbol: None,
            interval: None,
            session,
        },
        has_more,
    })
}

/// The regular session from a Yahoo `meta`: exchange time zone, today's open,
/// and the latest close among `tradingPeriods` (intraday responses list recent
/// days), because today may be an early close. `None` for markets open all day
/// (futures report 23 h 59 m), which then step through time like crypto, or for
/// anything missing. US-listed means an instrument with New York's zone.
fn session_from_meta(meta: &serde_json::Value) -> Option<Session> {
    use chrono::TimeZone;
    let regular = meta.get("currentTradingPeriod")?.get("regular")?;
    let start = regular.get("start")?.as_i64()?;
    let end = regular.get("end")?.as_i64()?;
    if end - start >= 23 * 3_600 {
        return None;
    }
    let tz: chrono_tz::Tz = meta.get("exchangeTimezoneName")?.as_str()?.parse().ok()?;
    let local = |t: i64| tz.timestamp_opt(t, 0).single().map(|d| d.time());
    let recent_ends = meta
        .get("tradingPeriods")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter_map(|day| day.as_array())
        .flatten()
        .filter_map(|p| local(p.get("end")?.as_i64()?));
    let close = recent_ends.fold(local(end)?, |a, b| a.max(b));
    let listed_type = matches!(
        meta.get("instrumentType").and_then(|v| v.as_str()),
        Some("EQUITY" | "ETF" | "INDEX" | "MUTUALFUND")
    );
    Some(Session {
        tz,
        open: local(start)?,
        close,
        us_calendar: listed_type && tz == chrono_tz::America::New_York,
    })
}

/// Stamp of a date bar: 00:00 UTC of the exchange-local date of `ts`.
/// Yahoo stamps these bars at the session open (10:00 AEDT is the previous
/// day in UTC), so reading the stamp as a UTC date would show the date one
/// day early for ASX or NZX. Binance date bars already open at 00:00 UTC.
fn local_date_stamp(ts: i64, gmtoffset: i64) -> i64 {
    (ts + gmtoffset).div_euclid(DAY) * DAY
}

/// Yahoo stamps the live bar with the last trade time instead of its period
/// start. Snap it onto the `step` grid anchored at the current session open
/// (intraday/daily, `session_start`) or at the previous bar (weekly and up;
/// months by calendar), and merge it into the previous bar when both fall in
/// the same period. Volume merges with `max` so a repeated snapshot of the
/// forming bar is not double-counted. A zero-volume intraday print outside
/// the regular session (Yahoo's phantom print at the close) is not a period
/// of its own: it merges into the previous bar.
fn snap_live_bar(
    bars: &mut Vec<Bar>,
    step: i64,
    session_start: Option<i64>,
    session_end: Option<i64>,
) {
    let n = bars.len();
    let Some(ts) = bars.last().and_then(bar_ts) else {
        return;
    };
    let prev = n.checked_sub(2).and_then(|i| bar_ts(&bars[i]));
    let outside = session_start.is_some_and(|s| ts < s) || session_end.is_some_and(|e| ts >= e);
    if step < DAY && prev.is_some() && outside && bars[n - 1].volume == 0.0 {
        let live = bars.pop().unwrap();
        let into = bars.last_mut().unwrap();
        into.high = into.high.max(live.high);
        into.low = into.low.min(live.low);
        into.close = live.close;
        return;
    }
    let anchor = match (session_start, prev) {
        (Some(s), _) if step <= DAY && ts >= s => s,
        (_, Some(p)) => p,
        _ => return,
    };
    let snapped = if step >= 28 * DAY {
        month_start(ts, anchor)
    } else {
        anchor + (ts - anchor).div_euclid(step) * step
    };
    if Some(snapped) == prev {
        let live = bars.pop().unwrap();
        let into = bars.last_mut().unwrap();
        into.high = into.high.max(live.high);
        into.low = into.low.min(live.low);
        into.close = live.close;
        into.volume = into.volume.max(live.volume);
    } else {
        bars[n - 1].date = snapped.to_string();
    }
}

/// Start of the UTC calendar month containing `ts`, at `anchor`'s time of day.
fn month_start(ts: i64, anchor: i64) -> i64 {
    // Howard Hinnant's civil-from-days / days-from-civil.
    let z = ts.div_euclid(DAY) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153; // month, March = 0
    let first_doy = (153 * mp + 2) / 5; // day-of-year of the 1st
    let first = era * 146_097 + 365 * yoe + yoe / 4 - yoe / 100 + first_doy - 719_468;
    first * DAY + anchor.rem_euclid(DAY)
}

/// Yahoo chart request window: unix-second `period1..period2` sized to hold
/// `limit` bars of `interval`. `age_clamped` marks a period1 raised to
/// Yahoo's maximum lookback: nothing older can be fetched.
#[derive(Debug, PartialEq, Eq)]
struct YahooWindow<'a> {
    api_interval: &'a str,
    period1: i64,
    period2: i64,
    age_clamped: bool,
}

const DAY: i64 = 86_400;

/// Yahoo's minute and hour intervals, coarsest first. Whole hours come from
/// 1h, for its 730 days of history; 90m is exact only, so that 3h does not
/// trade them for 90m's 60 days.
const YAHOO_INTRADAY: [(i64, &str); 6] = [
    (60, "1h"),
    (30, "30m"),
    (15, "15m"),
    (5, "5m"),
    (2, "2m"),
    (1, "1m"),
];

/// Yahoo interval that `interval` is fetched as: itself where Yahoo has it,
/// else the coarsest that divides it. Those are resampled from it.
fn yahoo_api_interval(interval: &str) -> &str {
    let Ok(iv) = Interval::parse(interval) else {
        return interval;
    };
    match iv.unit {
        Unit::Minute | Unit::Hour if iv.secs() == 5_400 => "90m",
        Unit::Minute | Unit::Hour => {
            let minutes = iv.secs() / 60;
            YAHOO_INTRADAY
                .iter()
                .find(|(m, _)| minutes % m == 0)
                .map_or("1m", |(_, name)| name)
        }
        Unit::Day => "1d",
        Unit::Week => "1wk",
        // Not Yahoo's 3mo: its rows follow `period1`, not calendar quarters.
        Unit::Month => "1mo",
    }
}

fn yahoo_window(
    interval: &str,
    limit: usize,
    end_time_ms: Option<i64>,
    now_s: i64,
) -> YahooWindow<'_> {
    let api_interval = yahoo_api_interval(interval);
    let step = data::interval_secs(interval).unwrap_or(DAY);
    // Calendar time per bar is longer than the bar itself for exchange-
    // traded symbols: ~6.5 trading hours per day, 5 days per week, holidays.
    let calendar_factor = if step < 6 * 3_600 {
        6.0
    } else if step < DAY {
        3.0 // a 6h or 12h bar spans about one session
    } else if step < 7 * DAY {
        2.0
    } else {
        1.2
    };
    // Floor keeps tiny tail polls (bars=2) from landing in a weekend gap.
    let span = ((limit as f64 * step as f64 * calendar_factor) as i64).max(10 * DAY);

    let period2 = end_time_ms.map_or(now_s, |e| e.saturating_add(999).div_euclid(1000));
    let mut period1 = period2.saturating_sub(span);
    let mut age_clamped = false;

    // Yahoo's intraday limits: (max window per request, max age).
    let (max_span, max_age) = match api_interval {
        "1m" => (Some(7 * DAY), Some(30 * DAY)),
        "2m" | "5m" | "15m" | "30m" | "90m" => (None, Some(60 * DAY)),
        "60m" | "1h" => (None, Some(730 * DAY)),
        _ => (None, None),
    };
    if let Some(s) = max_span {
        period1 = period1.max(period2 - s);
    }
    if let Some(a) = max_age {
        // One-hour margin: Yahoo rejects windows touching the boundary.
        let oldest = now_s - a + 3_600;
        if oldest >= period1 {
            period1 = oldest;
            age_clamped = true;
        }
    }

    YahooWindow {
        api_interval,
        period1,
        period2,
        age_clamped,
    }
}

/// `symbol` as exactly one URL path segment: everything but RFC 3986
/// unreserved characters is percent-encoded, so `/`, `?` or `#` cannot
/// change the request path. Dot-only input is rejected because URL
/// parsers resolve `.`/`..` (even as `%2E`) as relative segments.
fn path_segment(symbol: &str) -> Result<String, String> {
    if symbol.chars().all(|c| c == '.') {
        return Err(format!("Invalid symbol: {symbol:?}"));
    }
    Ok(percent_encode(symbol))
}

/// Percent-encode every byte except RFC 3986 unreserved characters.
pub(crate) fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn yahoo_chart_url(symbol: &str, w: &YahooWindow) -> Result<String, String> {
    Ok(format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&period1={}&period2={}",
        path_segment(symbol)?,
        w.api_interval,
        w.period1,
        w.period2
    ))
}

pub fn detect_source(symbol: &str) -> &'static str {
    let s = symbol.to_uppercase();
    let crypto_quotes = ["USDT", "BUSD", "BTC", "ETH", "BNB", "USDC", "FDUSD"];
    if crypto_quotes
        .iter()
        .any(|q| s.ends_with(q) && s.len() > q.len())
    {
        "binance"
    } else {
        "yahoo"
    }
}

/// Group consecutive bars into epoch-aligned `bucket_secs` buckets (1h → 4h,
/// 1m → 3m) for 24/7 instruments and for 3m. Each bucket is dated by its grid
/// start. Grid alignment keeps bucket boundaries identical across overlapping
/// requests, which history paging relies on. Buckets starting before
/// `min_start` (unix seconds) may be missing members and are dropped.
fn aggregate_bars(bars: &[Bar], bucket_secs: i64, min_start: i64) -> Vec<Bar> {
    let bucket = |b: &Bar| bar_ts(b).map(|t| t.div_euclid(bucket_secs) * bucket_secs);
    bars.chunk_by(|a, b| bucket(a) == bucket(b))
        .filter(|chunk| bucket(&chunk[0]).is_some_and(|s| s >= min_start))
        .map(|chunk| {
            let mut bar = merge_bars(chunk);
            if let Some(start) = bucket(&chunk[0]) {
                bar.date = start.to_string();
            }
            bar
        })
        .collect()
}

/// Bars more than this far apart (seconds) lie in different sessions.
const SESSION_GAP: i64 = 3_600;

/// Session-anchored buckets for exchange-traded symbols (TradingView style):
/// buckets of `bucket_secs` start at a session's open, and a session's last
/// bucket may be shorter. A session ends where consecutive bars are more than
/// an hour apart: the overnight gap, a futures daily break, or a lunch break
/// that leaves a gap in the rows. `opens` holds those starts from Yahoo's rows,
/// priced or not: a first minute with no price (Yahoo sends nulls) must not shift
/// the session's grid. Tokyo, Shanghai and Hong Kong keep null rows through lunch,
/// so their afternoon stays on the morning's grid (ADR-0038). Each bucket's `t` is its slot on
/// that grid. `gmtoffset` (seconds) maps UTC to exchange time; it is the
/// current offset, but a DST shift of an hour never moves a session across a
/// day boundary. The local day containing `min_start` may be cut off by the
/// window and is dropped, unless the window starts at or before the session
/// open time of day (`open_tod`, seconds after local midnight).
fn aggregate_sessions(
    bars: &[Bar],
    bucket_secs: i64,
    gmtoffset: i64,
    min_start: i64,
    open_tod: Option<i64>,
    opens: &[i64],
) -> Vec<Bar> {
    let day = |t: i64| (t + gmtoffset).div_euclid(DAY);
    let before_open = open_tod.is_some_and(|o| (min_start + gmtoffset).rem_euclid(DAY) <= o);
    let first_full_day = if before_open {
        day(min_start)
    } else {
        day(min_start.saturating_sub(1)) + 1
    };
    let same_session = |a: &Bar, b: &Bar| match (bar_ts(a), bar_ts(b)) {
        (Some(x), Some(y)) => y - x <= SESSION_GAP,
        _ => false,
    };
    let mut out = Vec::new();
    for session in bars.chunk_by(same_session) {
        let Some(first) = bar_ts(&session[0]) else {
            continue;
        };
        let open = match opens.partition_point(|o| *o <= first) {
            0 => first,
            i => opens[i - 1],
        };
        if day(open) < first_full_day {
            continue;
        }
        let slot = |b: &Bar| bar_ts(b).map(|t| (t - open).div_euclid(bucket_secs));
        out.extend(session.chunk_by(|a, b| slot(a) == slot(b)).map(|chunk| {
            let mut bar = merge_bars(chunk);
            if let Some(k) = slot(&chunk[0]) {
                bar.date = (open + k * bucket_secs).to_string();
            }
            bar
        }));
    }
    out
}

/// Date-bar buckets of `iv` on its calendar grid (`Interval::grid_start`,
/// exchange-local via `gmtoffset` seconds), so boundaries stay identical across
/// overlapping requests and match Binance's 3d and 1w klines. The bucket holding
/// `min_start` may be missing older members and is dropped; the last bucket may
/// be partial (still forming). Weekly and monthly members round to the nearest
/// local midnight, as `parse_yahoo_chart` explains.
fn aggregate_calendar(bars: &[Bar], iv: &Interval, gmtoffset: i64, min_start: i64) -> Vec<Bar> {
    let round = match iv.unit {
        Unit::Week | Unit::Month => DAY / 2,
        _ => 0,
    };
    let key = |b: &Bar| bar_ts(b).map(|t| iv.grid_start(t + gmtoffset + round));
    bars.chunk_by(|a, b| key(a) == key(b))
        .filter(|chunk| key(&chunk[0]).is_some_and(|k| k - gmtoffset >= min_start))
        .map(merge_bars)
        .collect()
}

fn merge_bars(chunk: &[Bar]) -> Bar {
    let open = chunk[0].open;
    let close = chunk.last().unwrap().close;
    let high = chunk
        .iter()
        .map(|b| b.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let low = chunk.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
    let volume: f64 = chunk.iter().map(|b| b.volume).sum();
    let date = chunk[0].date.clone();
    Bar {
        open,
        high,
        low,
        close,
        volume,
        date,
    }
}

// ---------------------------------------------------------------------------
// Quotes (watchlist): last price and change for many symbols at once.
// ---------------------------------------------------------------------------

/// Concurrent upstream requests per upstream per `get_quotes` call.
const QUOTE_CONCURRENCY: usize = 8;
/// How long a fetched quote (or per-symbol error) is served from the cache.
const QUOTE_TTL_MS: i64 = 3_000;
/// How long a symbol Binance rejected as unknown stays out of batches.
const BAD_SYMBOL_TTL_MS: i64 = 300_000;

/// Last price of one symbol and the reference it changed from. Yahoo and
/// sample: `prev_close` is the previous daily close (TradingView's
/// definition). Binance: the price at the start of the rolling 24 h window,
/// which is what Binance's own ticker reports (ADR-0011).
#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    pub last: f64,
    pub prev_close: f64,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub volume: Option<f64>,
    pub source: &'static str,
}

/// One quote per symbol, in input order. A failing symbol yields its own
/// `Err` and never fails the others.
pub fn fetch_quotes(source: DataSource, symbols: &[String]) -> Vec<Result<Quote, String>> {
    match source {
        DataSource::Sample => {
            let now = now_ms();
            symbols.iter().map(|s| sample_quote(s, now)).collect()
        }
        DataSource::Live => cached_quotes(
            quote_cache(),
            symbols,
            now_ms(),
            |syms| fetch_binance_quotes(syms, binance_get),
            fetch_yahoo_quote,
        ),
        #[cfg(test)]
        DataSource::Unavailable => symbols
            .iter()
            .map(|_| Err("upstream unavailable".into()))
            .collect(),
    }
}

/// Live quotes shared by every client: recent results per symbol, and
/// symbols Binance rejected as unknown. Both keyed by symbol and stamped
/// with the ms time they were stored.
#[derive(Default)]
struct QuoteCache {
    quotes: HashMap<String, (i64, Result<Quote, String>)>,
    bad: HashMap<String, (i64, String)>,
}

fn quote_cache() -> &'static Mutex<QuoteCache> {
    static CACHE: OnceLock<Mutex<QuoteCache>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

/// A Binance per-symbol failure; the flag marks a symbol Binance rejected
/// as unknown or malformed.
type BinanceQuote = Result<Quote, (bool, String)>;

/// Serve `symbols` from `cache` when fresh at `now` (ms), fetch the rest —
/// Binance and Yahoo concurrently — and store the results. Symbols Binance
/// rejected as unknown are answered from memory for `BAD_SYMBOL_TTL_MS`
/// instead of breaking every batch. Clock and upstreams are injected so
/// tests run offline.
fn cached_quotes<B, Y>(
    cache: &Mutex<QuoteCache>,
    symbols: &[String],
    now: i64,
    binance: B,
    yahoo: Y,
) -> Vec<Result<Quote, String>>
where
    B: FnOnce(&[&str]) -> Vec<BinanceQuote> + Send,
    Y: Fn(&str) -> Result<Quote, String> + Sync,
{
    let mut out: Vec<Option<Result<Quote, String>>> = vec![None; symbols.len()];
    let (mut need_binance, mut need_yahoo) = (Vec::new(), Vec::new());
    {
        let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
        c.quotes.retain(|_, (t, _)| now - *t < QUOTE_TTL_MS);
        c.bad.retain(|_, (t, _)| now - *t < BAD_SYMBOL_TTL_MS);
        for (i, s) in symbols.iter().enumerate() {
            if let Some((_, q)) = c.quotes.get(s) {
                out[i] = Some(q.clone());
            } else if let Some((_, e)) = c.bad.get(s) {
                out[i] = Some(Err(e.clone()));
            } else if detect_source(s) == "binance" {
                need_binance.push(i);
            } else {
                need_yahoo.push(i);
            }
        }
    }

    let pick = |idx: &[usize]| -> Vec<&str> { idx.iter().map(|&i| symbols[i].as_str()).collect() };
    let (binance_syms, yahoo_syms) = (pick(&need_binance), pick(&need_yahoo));
    let (from_binance, from_yahoo) = std::thread::scope(|s| {
        let b = s.spawn(|| {
            if binance_syms.is_empty() {
                Vec::new()
            } else {
                binance(&binance_syms)
            }
        });
        let y = par_map(&yahoo_syms, &yahoo);
        (b.join().expect("binance quote worker panicked"), y)
    });

    let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
    let binance_results = need_binance.iter().zip(from_binance).map(|(&i, r)| {
        let r = r.map_err(|(unknown, e)| {
            if unknown {
                c.bad.insert(symbols[i].clone(), (now, e.clone()));
            }
            e
        });
        (i, r)
    });
    let results: Vec<_> = binance_results
        .chain(need_yahoo.iter().copied().zip(from_yahoo))
        .collect();
    for (i, r) in results {
        c.quotes.insert(symbols[i].clone(), (now, r.clone()));
        out[i] = Some(r);
    }
    out.into_iter()
        .map(|r| r.unwrap_or_else(|| Err("quote missing".into())))
        .collect()
}

/// `f` over `items` on a work queue of at most `QUOTE_CONCURRENCY`
/// workers; results keep input order.
fn par_map<T, F>(items: &[&str], f: F) -> Vec<T>
where
    T: Send,
    F: Fn(&str) -> T + Sync,
{
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<T>>> = Mutex::new(items.iter().map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..QUOTE_CONCURRENCY.min(items.len()) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(i) else { break };
                let r = f(item);
                results.lock().unwrap_or_else(|e| e.into_inner())[i] = Some(r);
            });
        }
    });
    results
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .into_iter()
        .map(|r| r.expect("every queued item is processed"))
        .collect()
}

/// Sample-source quote: the forming daily bar against the previous one, so
/// it is deterministic per second and moves over time.
fn sample_quote(symbol: &str, now_ms: i64) -> Result<Quote, String> {
    let data = data::sample_ohlcv(symbol, "1d", 2, None, now_ms)?;
    let [prev, cur] = data.bars.as_slice() else {
        return Err("sample source has no previous daily bar".into());
    };
    Ok(Quote {
        last: cur.close,
        prev_close: prev.close,
        high: Some(cur.high),
        low: Some(cur.low),
        volume: Some(cur.volume),
        source: "sample",
    })
}

fn binance_ticker_url(symbols: &[&str]) -> String {
    let list = symbols
        .iter()
        .map(|s| format!("%22{}%22", percent_encode(s)))
        .collect::<Vec<_>>()
        .join(",");
    format!("https://api.binance.com/api/v3/ticker/24hr?symbols=%5B{list}%5D")
}

/// One batched 24 h ticker request for all symbols via `get`. Binance
/// rejects the whole batch when any symbol is unknown; only then is the
/// batch retried per symbol to isolate the bad one. Other failures (rate
/// limits 429/418, outages) fail every symbol without fanning out.
fn fetch_binance_quotes<G>(symbols: &[&str], get: G) -> Vec<BinanceQuote>
where
    G: Fn(&str) -> Result<serde_json::Value, (bool, String)> + Sync,
{
    let parse = |body: &serde_json::Value, syms: &[&str]| -> Vec<BinanceQuote> {
        parse_binance_tickers(body, syms)
            .into_iter()
            .map(|r| r.map_err(|e| (false, e)))
            .collect()
    };
    match get(&binance_ticker_url(symbols)) {
        Ok(body) => parse(&body, symbols),
        Err((true, _)) if symbols.len() > 1 => par_map(symbols, |s| {
            get(&binance_ticker_url(&[s])).and_then(|body| parse(&body, &[s]).remove(0))
        }),
        Err(e) => symbols.iter().map(|_| Err(e.clone())).collect(),
    }
}

/// True when a Binance error response means the request named an unknown
/// (-1121 Invalid symbol) or malformed (-1100 Illegal characters) symbol.
fn binance_unknown_symbol(status: u16, body: &serde_json::Value) -> bool {
    status == 400
        && matches!(
            body.get("code").and_then(|c| c.as_i64()),
            Some(-1121 | -1100)
        )
}

/// GET a Binance JSON endpoint. On failure the flag is true when Binance
/// rejected a symbol as unknown (see `binance_unknown_symbol`).
fn binance_get(url: &str) -> Result<serde_json::Value, (bool, String)> {
    binance_get_via(binance(), url)
}

fn binance_get_via(client: &BinanceClient, url: &str) -> Result<serde_json::Value, (bool, String)> {
    let resp = client.get(url).map_err(|e| (false, e))?;
    let status = resp.status;
    let body: serde_json::Value = serde_json::from_str(&resp.body).map_err(|e| {
        (
            false,
            format!("Failed to parse Binance response ({status}): {e}"),
        )
    })?;
    if (200..300).contains(&status) {
        return Ok(body);
    }
    let msg = body.get("msg").and_then(|m| m.as_str()).map_or_else(
        || format!("Binance API returned status {status}"),
        |m| format!("Binance: {m}"),
    );
    Err((binance_unknown_symbol(status, &body), msg))
}

/// Match a `/ticker/24hr?symbols=` array to `symbols` (in order). Pure, so
/// fixture JSON can drive it in tests.
fn parse_binance_tickers(body: &serde_json::Value, symbols: &[&str]) -> Vec<Result<Quote, String>> {
    let Some(entries) = body.as_array() else {
        return symbols
            .iter()
            .map(|_| Err("Binance ticker response is not an array".into()))
            .collect();
    };
    symbols
        .iter()
        .map(|s| {
            let entry = entries
                .iter()
                .find(|e| e.get("symbol").and_then(|v| v.as_str()) == Some(s))
                .ok_or("Binance returned no ticker for this symbol")?;
            let num = |k: &str| -> Result<f64, String> {
                entry
                    .get(k)
                    .and_then(|v| v.as_str())
                    .and_then(|v| v.parse::<f64>().ok())
                    .ok_or(format!("Binance ticker missing {k}"))
            };
            Ok(Quote {
                last: num("lastPrice")?,
                prev_close: num("openPrice")?,
                high: num("highPrice").ok(),
                low: num("lowPrice").ok(),
                volume: num("volume").ok(),
                source: "binance",
            })
        })
        .collect()
}

fn yahoo_quote_url(symbol: &str) -> Result<String, String> {
    Ok(format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?range=1d&interval=1d",
        path_segment(symbol)?
    ))
}

fn fetch_yahoo_quote(symbol: &str) -> Result<Quote, String> {
    let resp = http()
        .get(yahoo_quote_url(symbol)?)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .map_err(|e| format!("Yahoo request failed: {e}"))?;
    let status = resp.status();
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| format!("Failed to parse Yahoo response ({status}): {e}"))?;
    parse_yahoo_quote(&body)
}

/// Quote from a v8 chart response's `meta`: `regularMarketPrice` against
/// `chartPreviousClose` (with `range=1d`, the previous session's close).
/// Pure, so fixture JSON can drive it in tests.
fn parse_yahoo_quote(body: &serde_json::Value) -> Result<Quote, String> {
    let chart = body.get("chart").ok_or("Yahoo response missing chart")?;
    if let Some(desc) = chart
        .get("error")
        .and_then(|e| e.get("description"))
        .and_then(|d| d.as_str())
    {
        return Err(format!("Yahoo: {desc}"));
    }
    let meta = chart
        .get("result")
        .and_then(|r| r.get(0))
        .and_then(|r| r.get("meta"))
        .ok_or("Yahoo response missing chart.result[0].meta")?;
    let num = |k: &str| meta.get(k).and_then(|v| v.as_f64());
    Ok(Quote {
        last: num("regularMarketPrice").ok_or("Yahoo meta missing regularMarketPrice")?,
        prev_close: num("chartPreviousClose")
            .or_else(|| num("previousClose"))
            .ok_or("Yahoo meta missing chartPreviousClose")?,
        high: num("regularMarketDayHigh"),
        low: num("regularMarketDayLow"),
        volume: num("regularMarketVolume"),
        source: "yahoo",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const HOUR_MS: i64 = 3_600_000;

    fn far() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    fn bar_at(t_s: i64, close: f64) -> Bar {
        Bar {
            open: close,
            high: close + 1.0,
            low: close - 1.0,
            close,
            volume: 1.0,
            date: t_s.to_string(),
        }
    }

    /// Fake exchange: hourly bars from `first_ms` up to `last_ms`, answering
    /// like Binance (newest `limit` bars with open <= end, inclusive).
    fn fake_exchange(
        first_ms: i64,
        last_ms: i64,
    ) -> impl FnMut(usize, Option<i64>) -> Result<Vec<Bar>, String> {
        move |limit, end| {
            assert!(limit <= BINANCE_MAX_LIMIT, "page exceeds Binance cap");
            let end = end.unwrap_or(last_ms).min(last_ms);
            let last = end.div_euclid(HOUR_MS) * HOUR_MS;
            let mut v = Vec::new();
            let mut t = last;
            while t >= first_ms && v.len() < limit {
                v.push(bar_at(t / 1000, t as f64));
                t -= HOUR_MS;
            }
            v.reverse();
            Ok(v)
        }
    }

    #[test]
    fn binance_url_includes_end_time_only_when_set() {
        assert_eq!(
            binance_klines_url("btcusdt", "1h", 500, None),
            "https://api.binance.com/api/v3/klines?symbol=BTCUSDT&interval=1h&limit=500"
        );
        assert_eq!(
            binance_klines_url("BTCUSDT", "4h", 1000, Some(1_700_000_000_000)),
            "https://api.binance.com/api/v3/klines?symbol=BTCUSDT&interval=4h&limit=1000&endTime=1700000000000"
        );
    }

    #[test]
    fn the_sample_source_builds_custom_intervals_from_native_bars() {
        // A fixed end, so every bar is closed whatever the wall clock says.
        let end_ms = 1_700_000_000_000 / 420_000 * 420_000;
        let got = fetch_ohlcv(DataSource::Sample, "BTCUSDT", "7m", 3, Some(end_ms)).unwrap();
        let native = data::sample_ohlcv("BTCUSDT", "1m", 21, Some(end_ms), now_ms()).unwrap();
        assert_eq!(got.data.len(), 3);
        for (i, bar) in got.data.bars.iter().enumerate() {
            let bucket = &native.bars[i * 7..(i + 1) * 7];
            assert_eq!(bar.date, bucket[0].date);
            assert_eq!(bar.open, bucket[0].open);
            assert_eq!(bar.close, bucket[6].close);
            assert_eq!(bar.volume, bucket.iter().map(|b| b.volume).sum::<f64>());
        }
        // Their own months, on their own grid, as the sample source has always had.
        let months = fetch_ohlcv(DataSource::Sample, "BTCUSDT", "2mo", 3, Some(end_ms)).unwrap();
        let direct = data::sample_ohlcv("BTCUSDT", "2mo", 3, Some(end_ms), now_ms()).unwrap();
        assert_eq!(months.data.bars[2].date, direct.bars[2].date);
    }

    #[test]
    fn binance_urls_carry_the_kline_name() {
        for native in [
            "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h", "1d", "3d", "1w",
            "1M",
        ] {
            let url = binance_klines_url("BTCUSDT", native, 10, None);
            assert!(url.contains(&format!("&interval={native}&")), "{url}");
        }
        // A symbol cannot inject query parameters.
        let url = binance_klines_url("BTC&limit=1#x", "1h", 10, None);
        assert!(url.contains("symbol=BTC%26LIMIT%3D1%23X&interval=1h&limit=10"));
        // Interval durations agree for both spellings (sample grid, Yahoo window).
        assert_eq!(data::interval_secs("1wk"), data::interval_secs("1w"));
        assert_eq!(data::interval_secs("1mo"), data::interval_secs("1M"));
    }

    #[test]
    fn paginate_walks_backwards_in_capped_pages() {
        let last = 1_700_000_000_000 / HOUR_MS * HOUR_MS;
        let mut calls: Vec<(usize, Option<i64>)> = Vec::new();
        let mut exchange = fake_exchange(0, last);
        let bars = paginate_backwards(2500, None, far(), |limit, end| {
            calls.push((limit, end));
            exchange(limit, end)
        })
        .unwrap();
        assert_eq!(bars.len(), 2500);
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0], (1000, None));
        assert_eq!(calls[1], (1000, Some(last - 999 * HOUR_MS - 1)));
        assert_eq!(calls[2], (500, Some(last - 1999 * HOUR_MS - 1)));
        // Contiguous, ascending, no duplicates at page seams.
        for w in bars.windows(2) {
            assert_eq!(
                bar_open_ms(&w[1]).unwrap() - bar_open_ms(&w[0]).unwrap(),
                HOUR_MS
            );
        }
        assert_eq!(bar_open_ms(bars.last().unwrap()), Some(last));
    }

    #[test]
    fn paginate_end_time_is_exclusive() {
        let last = 1_700_000_000_000 / HOUR_MS * HOUR_MS;
        let end = last - 10 * HOUR_MS; // on the grid
        let mut first_end = None;
        let mut exchange = fake_exchange(0, last);
        let bars = paginate_backwards(5, Some(end), far(), |limit, e| {
            first_end.get_or_insert(e);
            exchange(limit, e)
        })
        .unwrap();
        assert_eq!(first_end, Some(Some(end - 1)));
        assert_eq!(bars.len(), 5);
        assert_eq!(bar_open_ms(bars.last().unwrap()), Some(end - HOUR_MS));

        // Even if the exchange ignores endTime, nothing >= end_time leaks.
        let bars = paginate_backwards(5, Some(end), far(), |limit, _| {
            fake_exchange(0, last)(limit, None)
        })
        .unwrap();
        assert!(bars.iter().all(|b| bar_open_ms(b).unwrap() < end));
    }

    #[test]
    fn paginate_stops_when_history_runs_out() {
        let first = 1_600_000_000_000 / HOUR_MS * HOUR_MS;
        let last = first + 1499 * HOUR_MS;
        let mut calls = 0;
        let mut exchange = fake_exchange(first, last);
        let bars = paginate_backwards(3000, None, far(), |limit, end| {
            calls += 1;
            exchange(limit, end)
        })
        .unwrap();
        assert_eq!(bars.len(), 1500);
        assert_eq!(calls, 2);
        assert_eq!(bar_open_ms(&bars[0]), Some(first));

        // Paging past the listing date yields zero bars, not an error.
        let bars = paginate_backwards(10, Some(first), far(), fake_exchange(first, last)).unwrap();
        assert!(bars.is_empty());
    }

    #[test]
    fn yahoo_window_sizes_and_end_time() {
        let now = 1_750_000_000;
        let w = yahoo_window("1d", 200, None, now);
        assert_eq!(w.api_interval, "1d");
        assert_eq!(w.period2, now);
        assert_eq!(w.period1, now - 200 * DAY * 2);

        // end_time (ms) → period2 (s), rounded up so the bar before it stays in.
        let w = yahoo_window("1d", 200, Some(1_700_000_000_500), now);
        assert_eq!(w.period2, 1_700_000_001);
        assert_eq!(w.period1, 1_700_000_001 - 400 * DAY);

        // Tiny tail polls get the 10-day floor.
        let w = yahoo_window("1h", 2, None, now);
        assert_eq!(w.period1, now - 10 * DAY);

        // 4h is fetched as 1h; span sized in 4h bars.
        let w = yahoo_window("4h", 100, None, now);
        assert_eq!(w.api_interval, "1h");
        assert_eq!(w.period1, now - 100 * 14_400 * 6);
    }

    #[test]
    fn yahoo_window_sizes_long_intraday_bars_tighter() {
        let now = 1_750_000_000;
        assert_eq!(
            yahoo_window("4h", 100, None, now).period1,
            now - 100 * 14_400 * 6
        );
        assert_eq!(
            yahoo_window("12h", 100, None, now).period1,
            now - 100 * 43_200 * 3
        );
    }

    #[test]
    fn yahoo_window_clamps_intraday_lookback() {
        let now = 1_750_000_000;
        // 1m: at most 7 days per request.
        let w = yahoo_window("1m", 50_000, None, now);
        assert_eq!(w.period1, now - 7 * DAY);
        // 5m: no older than 60 days.
        let w = yahoo_window("5m", 100_000, None, now);
        assert_eq!(w.period1, now - 60 * DAY + 3_600);
        // end_time beyond the 1h limit: empty window.
        let w = yahoo_window("1h", 100, Some((now - 800 * DAY) * 1000), now);
        assert!(w.period1 >= w.period2);
        assert_eq!(
            yahoo_chart_url("AAPL", &yahoo_window("1d", 200, None, now)).unwrap(),
            format!(
                "https://query1.finance.yahoo.com/v8/finance/chart/AAPL?interval=1d&period1={}&period2={}",
                now - 400 * DAY,
                now
            )
        );
    }

    #[test]
    fn aggregate_bars_uses_epoch_grid() {
        let h = 3_600;
        let base = 1_700_006_400; // multiple of 4h
        assert_eq!(base % (4 * h), 0);
        // Stock-like session: 13:30..19:30 style offsets, gaps between days.
        let ts = [
            base - h,
            base,
            base + h,
            base + 2 * h,
            base + 4 * h,
            base + 5 * h,
        ];
        let bars: Vec<Bar> = ts
            .iter()
            .enumerate()
            .map(|(i, t)| bar_at(*t, i as f64))
            .collect();

        let agg = aggregate_bars(&bars, 4 * h, 0);
        assert_eq!(agg.len(), 3);
        assert_eq!(agg[1].date, base.to_string());
        assert_eq!(agg[1].open, 1.0);
        assert_eq!(agg[1].close, 3.0);
        assert_eq!(agg[1].high, 4.0);
        assert_eq!(agg[1].low, 0.0);
        assert_eq!(agg[1].volume, 3.0);

        // Same bucket boundaries regardless of where the window starts.
        let shifted = aggregate_bars(&bars[2..], 4 * h, 0);
        assert_eq!(shifted.last().unwrap().date, agg.last().unwrap().date);
        // A bucket starting before the window start is dropped as partial.
        let trimmed = aggregate_bars(&bars[2..], 4 * h, base + h);
        assert_eq!(trimmed.len(), 1);
        assert_eq!(trimmed[0].date, (base + 4 * h).to_string());
    }

    #[test]
    fn paginate_fails_after_deadline() {
        let err = paginate_backwards(10, None, Instant::now(), fake_exchange(0, HOUR_MS * 100));
        assert!(err.is_err());
    }

    // --- Yahoo response parsing (fixtures) ---

    const AAPL_1H_LIVE: &str = include_str!("../tests/fixtures/yahoo_aapl_1h_live.json");
    const H: i64 = 3_600;
    // 2026-11-04 14:30 UTC = 09:30 EST, the fixture's regular session start.
    const NOV4_OPEN: i64 = 1_793_802_600;
    const NOV3_OPEN: i64 = NOV4_OPEN - DAY;

    fn fixture() -> serde_json::Value {
        serde_json::from_str(AAPL_1H_LIVE).unwrap()
    }

    fn win(api_interval: &str, period1: i64) -> YahooWindow<'_> {
        YahooWindow {
            api_interval,
            period1,
            period2: i64::MAX,
            age_clamped: false,
        }
    }

    fn dates(f: &Fetched) -> Vec<i64> {
        f.data.bars.iter().map(|b| bar_ts(b).unwrap()).collect()
    }

    /// Minimal Yahoo chart body: closes are 0, 1, 2, ... in timestamp order.
    fn yahoo_body(ts: &[i64], symbol: &str, instrument: &str, gmtoffset: i64) -> serde_json::Value {
        let c: Vec<f64> = (0..ts.len()).map(|i| i as f64).collect();
        serde_json::json!({ "chart": { "result": [{
            "meta": { "symbol": symbol, "instrumentType": instrument, "gmtoffset": gmtoffset },
            "timestamp": ts,
            "indicators": { "quote": [{ "open": c, "high": c, "low": c, "close": c, "volume": c }] }
        }]}})
    }

    #[test]
    fn yahoo_live_bar_snaps_to_its_period() {
        let f = parse_yahoo_chart(&fixture(), "1h", 100, None, &win("1h", 0)).unwrap();
        let ts = dates(&f);
        assert_eq!(ts.len(), 10);
        // The live point at 16:47:13 UTC becomes the 16:30 bar.
        assert_eq!(*ts.last().unwrap(), NOV4_OPEN + 2 * H);
        assert!(ts.iter().all(|t| (t - NOV4_OPEN) % H == 0));
    }

    #[test]
    fn yahoo_live_bar_merges_into_existing_period() {
        let mut body = fixture();
        // Live point inside the 15:30 bar that Yahoo already returned.
        body["chart"]["result"][0]["timestamp"][9] = serde_json::json!(NOV4_OPEN + H + 1_700);
        let f = parse_yahoo_chart(&body, "1h", 100, None, &win("1h", 0)).unwrap();
        let ts = dates(&f);
        assert_eq!(ts.len(), 9);
        assert_eq!(*ts.last().unwrap(), NOV4_OPEN + H);
        let last = f.data.bars.last().unwrap();
        assert_eq!(last.open, 207.5); // existing bar's open
        assert_eq!(last.close, 209.0); // live close
        assert_eq!(last.high, 210.0);
        assert_eq!(last.low, 207.0);
        assert_eq!(last.volume, 1009.0);
    }

    #[test]
    fn snap_live_bar_daily_weekly_monthly() {
        // Daily: live print at 15:47 snaps to the session open (14:30 after
        // the DST change, while the previous bar opened at 13:30).
        let mut bars = vec![
            bar_at(NOV4_OPEN - DAY - H, 1.0),
            bar_at(NOV4_OPEN + 4_620, 2.0),
        ];
        snap_live_bar(&mut bars, DAY, Some(NOV4_OPEN), None);
        assert_eq!(bar_ts(&bars[1]), Some(NOV4_OPEN));
        // Weekly: a mid-week print belongs to the previous (current-week) bar.
        let monday = 1_792_972_800; // 2026-10-26 00:00 UTC, a Monday
        let mut bars = vec![bar_at(monday, 1.0), bar_at(monday + 2 * DAY + 5 * H, 2.0)];
        snap_live_bar(&mut bars, 7 * DAY, None, None);
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].close, 2.0);
        // ... and a print in the next week starts a new Monday bar.
        let mut bars = vec![bar_at(monday, 1.0), bar_at(monday + 8 * DAY, 2.0)];
        snap_live_bar(&mut bars, 7 * DAY, None, None);
        assert_eq!(bar_ts(&bars[1]), Some(monday + 7 * DAY));
        // Monthly: snap to the 1st of the month at the previous bar's time of day.
        let oct1 = 1_790_827_200; // 2026-10-01 04:00 UTC
        let mut bars = vec![bar_at(oct1, 1.0), bar_at(NOV4_OPEN, 2.0)];
        snap_live_bar(&mut bars, 30 * DAY, None, None);
        assert_eq!(bar_ts(&bars[1]), Some(oct1 + 31 * DAY));
        // On-grid bars are left alone.
        let mut bars = vec![bar_at(NOV3_OPEN, 1.0), bar_at(NOV4_OPEN, 2.0)];
        snap_live_bar(&mut bars, DAY, Some(NOV4_OPEN), None);
        assert_eq!(bar_ts(&bars[1]), Some(NOV4_OPEN));
    }

    #[test]
    fn month_start_handles_leap_years_and_pre_epoch() {
        assert_eq!(month_start(952_214_400, 0), 951_868_800); // 2000-03-05 → 2000-03-01
        assert_eq!(month_start(0, 0), 0); // 1970-01-01
        assert_eq!(month_start(-DAY, 0), -31 * DAY); // 1969-12-31 → 1969-12-01
        assert_eq!(month_start(1_709_164_800, 7_200), 1_706_745_600 + 7_200); // 2024-02-29 → 2024-02-01 02:00
    }

    #[test]
    fn yahoo_4h_equity_buckets_anchor_at_session_open() {
        let f = parse_yahoo_chart(&fixture(), "4h", 100, None, &win("1h", 0)).unwrap();
        // Nov 3: 09:30-13:30 and 13:30-16:00 ET; Nov 4: the forming 09:30 bucket.
        assert_eq!(dates(&f), vec![NOV3_OPEN, NOV3_OPEN + 4 * H, NOV4_OPEN]);
        let first = &f.data.bars[0];
        assert_eq!((first.open, first.close), (199.5, 203.0));
        assert_eq!(first.volume, 1000.0 + 1001.0 + 1002.0 + 1003.0);
        assert_eq!(f.data.bars[2].close, 209.0);

        // A window starting mid-session drops that (incomplete) day.
        let f = parse_yahoo_chart(&fixture(), "4h", 100, None, &win("1h", NOV3_OPEN + H)).unwrap();
        assert_eq!(dates(&f), vec![NOV4_OPEN]);
    }

    #[test]
    fn yahoo_4h_equity_buckets_across_dst_change() {
        // Fri 2026-03-06 opens 14:30 UTC (EST); Mon 2026-03-09 opens 13:30 UTC
        // (EDT). gmtoffset is Yahoo's current offset (EDT) for both days.
        let fri = 1_772_807_400;
        let mon = 1_773_063_000;
        let ts: Vec<i64> = (0..7)
            .map(|i| fri + i * H)
            .chain((0..7).map(|i| mon + i * H))
            .collect();
        let body = yahoo_body(&ts, "AAPL", "EQUITY", -14_400);
        let f = parse_yahoo_chart(&body, "4h", 100, None, &win("1h", 0)).unwrap();
        assert_eq!(dates(&f), vec![fri, fri + 4 * H, mon, mon + 4 * H]);
        let sizes: Vec<f64> = f.data.bars.iter().map(|b| b.close - b.open + 1.0).collect();
        assert_eq!(sizes, vec![4.0, 3.0, 4.0, 3.0]);
    }

    #[test]
    fn yahoo_4h_round_the_clock_symbols_use_utc_grid() {
        let base = 1_700_006_400; // multiple of 4h
        let ts: Vec<i64> = (1..10).map(|i| base + i * H).collect();
        for (symbol, instrument) in [("BTC-USD", "CRYPTOCURRENCY"), ("EURUSD=X", "")] {
            let body = yahoo_body(&ts, symbol, instrument, 0);
            let f = parse_yahoo_chart(&body, "4h", 100, None, &win("1h", 0)).unwrap();
            assert_eq!(
                dates(&f),
                vec![base, base + 4 * H, base + 8 * H],
                "{symbol}"
            );
        }
    }

    #[test]
    fn yahoo_resampled_intervals_fetch_a_native_one() {
        let now = 1_750_000_000;
        for (tf, api) in [
            ("3m", "1m"),
            ("2h", "1h"),
            ("4h", "1h"),
            ("6h", "1h"),
            ("12h", "1h"),
            ("3d", "1d"),
            ("5m", "5m"),
            ("60m", "1h"),
            ("1wk", "1wk"),
            // Custom intervals: the coarsest that divides, for depth of history.
            ("7m", "1m"),
            ("6m", "2m"),
            ("10m", "5m"),
            ("45m", "15m"),
            ("90m", "90m"),
            ("150m", "30m"),
            ("3h", "1h"),
            ("8h", "1h"),
            ("2d", "1d"),
            ("5d", "1d"),
            ("2wk", "1wk"),
            ("2mo", "1mo"),
            ("3mo", "1mo"),
        ] {
            assert_eq!(yahoo_window(tf, 100, None, now).api_interval, api, "{tf}");
        }
    }

    /// Reads through `newest_klines` against a fake exchange, counting requests.
    fn cached_read(
        cache: &KlineCache,
        key: &str,
        want: usize,
        now: Instant,
        (first_ms, last_ms): (i64, i64),
    ) -> (Vec<Bar>, usize) {
        let mut calls = 0;
        let mut exchange = fake_exchange(first_ms, last_ms);
        let bars = newest_klines(
            cache,
            &Walks::default(),
            (key.to_string(), "1h"),
            want,
            &move || Now {
                mono: now,
                wall_ms: last_ms,
            },
            now + Duration::from_secs(60),
            |limit, end| {
                calls += 1;
                exchange(limit, end)
            },
        )
        .unwrap();
        (bars, calls)
    }

    #[test]
    fn a_repeat_read_takes_one_request_and_equals_a_full_one() {
        // A 59m tail with warmup 300: 303 bars of 59 klines each.
        let want = 303 * 59;
        let last = 1_700_000_000_000 / HOUR_MS * HOUR_MS;
        let history = (0, last);
        let cache = KlineCache::default();
        let t0 = Instant::now();

        let (first, calls) = cached_read(&cache, "BTCUSDT", want, t0, history);
        assert_eq!(calls, 18, "the first read walks the history");
        assert_eq!(first.len(), want);

        // The forming kline moved on by two klines since then.
        let later = (0, last + 2 * HOUR_MS);
        let (again, calls) = cached_read(&cache, "BTCUSDT", want, t0, later);
        assert_eq!(calls, 1, "a repeat reads one page");
        let full = paginate_backwards(want, None, far(), fake_exchange(0, later.1)).unwrap();
        let key = |b: &[Bar]| {
            b.iter()
                .map(|x| (x.date.clone(), ohlcv(x)))
                .collect::<Vec<_>>()
        };
        assert_eq!(key(&again), key(&full), "the same klines, oldest to newest");

        // A smaller ask is served as it is, and a larger one reads only what it lacks.
        let (small, calls) = cached_read(&cache, "BTCUSDT", 2_000, t0, later);
        assert_eq!((small.len(), calls), (2_000, 0));
        let (big, calls) = cached_read(&cache, "BTCUSDT", want + 1_000, t0, later);
        assert_eq!((big.len(), calls), (want + 1_000, 1));
        let full = paginate_backwards(want + 1_000, None, far(), fake_exchange(0, later.1));
        assert_eq!(key(&big), key(&full.unwrap()));
    }

    #[test]
    fn a_short_read_landing_after_a_long_walk_does_not_cut_its_window() {
        let last = 1_700_000_000_000 / HOUR_MS * HOUR_MS;
        let cache = KlineCache::default();
        let t0 = Instant::now();
        cached_read(&cache, "BTCUSDT", 1_500, t0, (0, last));

        // A 7m chart's refresh is out while a 59m chart's walk finishes.
        let later = t0 + KLINE_FRESH;
        let mut exchange = fake_exchange(0, last);
        let got = newest_klines(
            &cache,
            &Walks::default(),
            ("BTCUSDT".to_string(), "1h"),
            1_500,
            &move || Now {
                mono: later,
                wall_ms: last,
            },
            far(),
            |limit, end| {
                cached_read(&cache, "BTCUSDT", 5_000, later, (0, last));
                exchange(limit, end)
            },
        )
        .unwrap();
        assert_eq!(got.len(), 1_500);
        assert_eq!(
            cache.lock().held[&("BTCUSDT".to_string(), "1h")].bars.len(),
            5_000
        );

        // So the long chart's next read costs one page, not a walk.
        let (long, calls) = cached_read(&cache, "BTCUSDT", 5_000, later, (0, last + HOUR_MS));
        assert_eq!((long.len(), calls), (5_000, 1));
        assert_eq!(bar_open_ms(long.last().unwrap()), Some(last + HOUR_MS));
        assert!(long
            .windows(2)
            .all(|w| { bar_open_ms(&w[1]).unwrap() - bar_open_ms(&w[0]).unwrap() == HOUR_MS }));
    }

    #[test]
    fn the_kline_cache_reads_again_after_a_gap_or_its_lifetime() {
        let want = 2_500;
        let last = 1_700_000_000_000 / HOUR_MS * HOUR_MS;
        let cache = KlineCache::default();
        let t0 = Instant::now();
        assert_eq!(cached_read(&cache, "A", want, t0, (0, last)).1, 3);
        // More new klines than a refresh reads: the page does not reach ours.
        let far_on = (0, last + (KLINE_REFRESH as i64 + 5) * HOUR_MS);
        let (bars, calls) = cached_read(&cache, "A", want, t0, far_on);
        assert_eq!(calls, 1 + 3, "the refresh, then a full read");
        assert_eq!(bar_open_ms(bars.last().unwrap()), Some(far_on.1));
        assert!(bars
            .windows(2)
            .all(|w| { bar_open_ms(&w[1]).unwrap() - bar_open_ms(&w[0]).unwrap() == HOUR_MS }));
        // Past its lifetime an entry is gone.
        let later = t0 + KLINE_CACHE_TTL + Duration::from_secs(1);
        assert_eq!(cached_read(&cache, "A", want, later, far_on).1, 3);
    }

    #[test]
    fn the_kline_cache_knows_a_short_history_and_stays_bounded() {
        let last = 1_700_000_000_000 / HOUR_MS * HOUR_MS;
        let cache = KlineCache::default();
        let t0 = Instant::now();
        // A symbol listed 1500 klines ago cannot fill the ask, yet only the first
        // read walks it.
        let listed = (last - 1_499 * HOUR_MS, last);
        let (bars, calls) = cached_read(&cache, "NEW", 5_000, t0, listed);
        assert_eq!((bars.len(), calls), (1_500, 2));
        let (bars, calls) = cached_read(&cache, "NEW", 5_000, t0 + KLINE_FRESH, listed);
        assert_eq!((bars.len(), calls), (1_500, 1));
        // The least recently used entry goes when the cache is full.
        for i in 0..KLINE_CACHE_ENTRIES {
            let at = t0 + Duration::from_secs(2 + i as u64 / 10);
            cached_read(&cache, &format!("S{i}"), 900, at, (0, last));
        }
        let st = cache.lock();
        assert_eq!(st.held.len(), KLINE_CACHE_ENTRIES);
        assert!(!st.held.contains_key(&("NEW".to_string(), "1h")));
    }

    /// A pretend Binance behind `BinanceClient`'s HTTP boundary: real URLs in, real
    /// JSON, statuses and weight headers out, with a clock the test moves.
    struct FakeBinance {
        now_s: Arc<std::sync::atomic::AtomicI64>,
        /// What the clock shows past `now_s`, for moves shorter than a second.
        extra_ms: Arc<std::sync::atomic::AtomicI64>,
        /// Open time of the first kline.
        listed_ms: std::sync::atomic::AtomicI64,
        /// While true, a request that has been counted does not answer.
        paused: (Mutex<bool>, Condvar),
        /// Its weight per minute, and the gate's. Small, so a test stays cheap.
        limit: u32,
        minute_weight: Mutex<HashMap<i64, u32>>,
        requests: AtomicUsize,
        in_flight: AtomicUsize,
        peak: AtomicUsize,
        over_limit: AtomicUsize,
        /// Answer the k-th request (counted from 1) with this status and Retry-After.
        fail_at: Mutex<Option<(usize, u16, &'static str)>>,
        delay: Duration,
    }

    impl FakeBinance {
        fn new(limit: u32, delay_ms: u64) -> Arc<Self> {
            Arc::new(Self {
                now_s: Arc::new(std::sync::atomic::AtomicI64::new(1_700_000_040)),
                extra_ms: Arc::default(),
                listed_ms: Default::default(),
                paused: Default::default(),
                limit,
                minute_weight: Mutex::default(),
                requests: AtomicUsize::new(0),
                in_flight: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                over_limit: AtomicUsize::new(0),
                fail_at: Mutex::default(),
                delay: Duration::from_millis(delay_ms),
            })
        }

        fn requests(&self) -> usize {
            self.requests.load(Ordering::SeqCst)
        }

        fn advance(&self, secs: i64) {
            self.now_s.fetch_add(secs, Ordering::SeqCst);
        }

        fn advance_ms(&self, ms: i64) {
            self.extra_ms.fetch_add(ms, Ordering::SeqCst);
        }

        fn pause(&self, paused: bool) {
            *self.paused.0.lock().unwrap() = paused;
            self.paused.1.notify_all();
        }

        /// Until `n` requests have been counted, answered or not.
        fn wait_for_requests(&self, n: usize) {
            let give_up = Instant::now() + Duration::from_secs(10);
            while self.requests() < n {
                assert!(Instant::now() < give_up, "{} requests", self.requests());
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        fn most_in_a_minute(&self) -> u32 {
            let weights = self.minute_weight.lock().unwrap();
            weights.values().copied().max().unwrap_or(0)
        }

        fn answer(&self, url: &str) -> Result<BinanceResponse, String> {
            let k = self.requests.fetch_add(1, Ordering::SeqCst) + 1;
            let flying = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(flying, Ordering::SeqCst);
            std::thread::sleep(self.delay);
            // Not for ever: a test that fails while paused must end, not hang.
            drop(
                self.paused
                    .1
                    .wait_timeout_while(
                        self.paused.0.lock().unwrap(),
                        Duration::from_secs(15),
                        |paused| *paused,
                    )
                    .unwrap(),
            );
            self.in_flight.fetch_sub(1, Ordering::SeqCst);

            let now_s = self.now_s.load(Ordering::SeqCst);
            let now_ms = now_s * 1000 + self.extra_ms.load(Ordering::SeqCst);
            let listed_ms = self.listed_ms.load(Ordering::SeqCst);
            let used = {
                let mut weights = self.minute_weight.lock().unwrap();
                let w = weights.entry(now_s.div_euclid(60)).or_insert(0);
                *w += 2;
                *w
            };
            if used > self.limit {
                self.over_limit.fetch_add(1, Ordering::SeqCst);
            }
            let reply = |status, retry: Option<&str>, body: String| {
                Ok(BinanceResponse {
                    status,
                    retry_after: retry.map(str::to_string),
                    used_weight: Some(used.to_string()),
                    body,
                })
            };
            let scripted = *self.fail_at.lock().unwrap();
            if let Some((_, status, retry)) = scripted.filter(|(at, _, _)| *at == k) {
                return reply(
                    status,
                    Some(retry),
                    r#"{"code":-1003,"msg":"Too many"}"#.into(),
                );
            }
            let param = |name: &str| {
                url.split(['?', '&'])
                    .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
            };
            let Some(interval) = param("interval") else {
                return reply(200, None, "[]".into());
            };
            let step_ms = data::interval_secs(interval).unwrap() * 1000;
            let limit: i64 = param("limit").unwrap().parse().unwrap();
            let end: i64 = param("endTime").map_or(i64::MAX, |e| e.parse().unwrap());
            // Binance's klines lie on the grid: weeks open on Monday, for example.
            let grid = Interval::parse(interval).unwrap();
            let newest = grid.grid_start(end.min(now_ms).div_euclid(1000)) * 1000;
            let rows: Vec<String> = (0..limit)
                .rev()
                .map(|i| newest - i * step_ms)
                .filter(|t| *t >= listed_ms)
                .map(|t| {
                    let p = 100.0 + (t / step_ms % 97) as f64;
                    // The forming kline moves with the clock, and stops as it closes.
                    let close = p + (now_ms - t).min(step_ms) as f64 / 1000.0;
                    format!(r#"[{t},"{p}","{}","{}","{close}","1"]"#, p + 1.0, p - 1.0)
                })
                .collect();
            reply(200, None, format!("[{}]", rows.join(",")))
        }
    }

    fn client_of(fake: &Arc<FakeBinance>) -> BinanceClient {
        let (wall, mono) = (fake.now_s.clone(), fake.now_s.clone());
        let (start, base) = (fake.now_s.load(Ordering::SeqCst), Instant::now());
        let (seconds, extra) = (fake.now_s.clone(), fake.extra_ms.clone());
        let exchange = fake.clone();
        BinanceClient {
            gate: Box::leak(Box::new(Gate::with_clocks(
                fake.limit,
                Box::new(move || wall.load(Ordering::SeqCst)),
                Box::new(move || {
                    base + Duration::from_secs((mono.load(Ordering::SeqCst) - start) as u64)
                }),
            ))),
            walks: Walks::default(),
            klines: KlineCache::default(),
            pages: PageCache::default(),
            now: Box::new(move || {
                let ms = seconds.load(Ordering::SeqCst) * 1000 + extra.load(Ordering::SeqCst);
                Now {
                    mono: base + Duration::from_millis((ms - start * 1000).max(0) as u64),
                    wall_ms: ms,
                }
            }),
            send: Box::new(move |url| exchange.answer(url)),
        }
    }

    /// A `59m` tail with warmup 300, as the web app polls it.
    fn tail_59m(client: &BinanceClient, symbol: &str) -> Result<Fetched, String> {
        fetch_binance_via(client, symbol, "59m", 302, None)
    }

    const REFUSED: &str = "Binance rate limit";

    fn no_read_under_way(client: &BinanceClient) -> bool {
        let st = client.klines.lock();
        st.walking
            .values()
            .chain(st.refreshing.values())
            .all(|f| !f.running())
    }

    #[test]
    fn failure_a_parallel_history_pages_stop_at_the_budget() {
        // Parallel 20-request history reads, each with its own end_time, so no
        // cache helps. Weight limit 600: the gate refuses from 480, 240 requests.
        let fake = FakeBinance::new(600, 1);
        let client = client_of(&fake);
        let base_ms = fake.now_s.load(Ordering::SeqCst) * 1000;
        let results: Vec<Result<Fetched, String>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..10i64)
                .flat_map(|i| (0..3i64).map(move |j| (i, j)))
                .map(|(i, j)| {
                    let (client, end) = (&client, base_ms - (i * 7 + j) * HOUR_MS);
                    s.spawn(move || fetch_binance_via(client, "BTCUSDT", "59m", 5000, Some(end)))
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        // 240 requests, and at most one more from each walk already past the gate.
        let most = 240 + MAX_WALKS;
        assert!(fake.requests() <= most, "{} requests", fake.requests());
        assert_eq!(
            fake.over_limit.load(Ordering::SeqCst),
            0,
            "Binance never said no"
        );
        assert!(
            fake.peak.load(Ordering::SeqCst) <= MAX_WALKS,
            "walks at once"
        );
        let refused: Vec<&String> = results.iter().filter_map(|r| r.as_ref().err()).collect();
        assert!(!refused.is_empty());
        let clear = |e: &&String| e.contains(REFUSED) || e.as_str() == BUSY;
        assert!(refused.iter().all(clear), "{refused:?}");
        // The next minute the budget is back.
        fake.advance(60);
        assert!(fetch_binance_via(&client, "BTCUSDT", "59m", 5000, Some(base_ms)).is_ok());
    }

    #[test]
    fn failure_b_tail_polls_thrashing_the_cache_stop_at_the_budget() {
        // One symbol more than the cache holds: every poll evicts the next one's
        // klines, and each costs a full 18-request walk.
        let fake = FakeBinance::new(600, 0);
        let client = client_of(&fake);
        let holds = KLINE_CACHE_BARS / 18_000;
        let symbols: Vec<String> = (0..=holds).map(|i| format!("S{i}USDT")).collect();
        let (mut ok, mut refused) = (0, 0);
        for _round in 0..50 {
            for s in &symbols {
                match tail_59m(&client, s) {
                    Ok(_) => ok += 1,
                    Err(e) => {
                        assert!(e.contains(REFUSED), "{e}");
                        refused += 1;
                    }
                }
            }
            fake.advance(2);
        }
        assert!(ok > 0 && refused > 0, "ok {ok}, refused {refused}");
        assert!(
            fake.most_in_a_minute() <= 480 + 2,
            "{}",
            fake.most_in_a_minute()
        );
        assert_eq!(fake.over_limit.load(Ordering::SeqCst), 0);

        // As many symbols as it holds cost one request a poll, after the first.
        let fake = FakeBinance::new(6_000, 0);
        let client = client_of(&fake);
        for s in &symbols[..holds] {
            tail_59m(&client, s).unwrap();
        }
        let first = fake.requests();
        assert_eq!(first, holds * 18);
        fake.advance(2);
        for s in &symbols[..holds] {
            tail_59m(&client, s).unwrap();
        }
        assert_eq!(fake.requests(), first + holds);
    }

    #[test]
    fn failure_c_subscribe_churn_seeds_share_the_klines() {
        // A push seed of a 59m chart asks for its native 1m klines.
        let want = 303 * 59;
        let fake = FakeBinance::new(6_000, 0);
        let client = client_of(&fake);
        let seed = |c: &BinanceClient| fetch_binance_via(c, "BTCUSDT", "1m", want, None);
        let first = seed(&client).unwrap();
        assert_eq!(fake.requests(), 18);
        for _ in 0..9 {
            fake.advance(6);
            seed(&client).unwrap();
        }
        assert_eq!(fake.requests(), 18 + 9, "ten subscribes, one walk");
        assert_eq!(first.data.len(), want);
    }

    #[test]
    fn every_binance_path_stops_after_a_429_or_418_until_retry_after() {
        for status in [429, 418] {
            let fake = FakeBinance::new(6_000, 0);
            let client = client_of(&fake);
            // The fifth page of a walk is answered with the status.
            *fake.fail_at.lock().unwrap() = Some((5, status, "120"));
            let e = tail_59m(&client, "BTCUSDT").err().unwrap();
            assert!(e.contains(&status.to_string()), "{e}");
            assert_eq!(fake.requests(), 5, "the walk stops there");

            // Nothing leaves now: this chart, another, a native seed, a single
            // page, the quotes and the symbol list's path.
            for call in [
                tail_59m(&client, "BTCUSDT").map(|_| ()),
                tail_59m(&client, "ETHUSDT").map(|_| ()),
                fetch_binance_via(&client, "BTCUSDT", "1m", 5000, None).map(|_| ()),
                fetch_binance_via(&client, "BTCUSDT", "4h", 10, None).map(|_| ()),
                client
                    .get("https://api.binance.com/api/v3/exchangeInfo")
                    .map(|_| ()),
            ] {
                let e = call.err().unwrap();
                assert!(e.contains(REFUSED) && e.contains("120s"), "{e}");
            }
            // A refused quote is not a symbol Binance called unknown.
            let (unknown, e) = binance_get_via(&client, &binance_ticker_url(&["BTCUSDT"]))
                .err()
                .unwrap();
            assert!(!unknown && e.contains(REFUSED), "{e}");
            assert_eq!(fake.requests(), 5, "not one request during the cooldown");

            // The refused reads left nothing behind: the next is a full, right one.
            assert!(client.klines.lock().held.is_empty());
            assert!(no_read_under_way(&client));
            fake.advance(120);
            let after = tail_59m(&client, "BTCUSDT").unwrap();
            assert_eq!(fake.requests(), 5 + 18);
            let clean = tail_59m(&client_of(&fake), "BTCUSDT").unwrap();
            let key = |f: &Fetched| {
                f.data
                    .bars
                    .iter()
                    .map(|b| (b.date.clone(), ohlcv(b)))
                    .collect::<Vec<_>>()
            };
            assert_eq!(key(&after), key(&clean));
        }
    }

    #[test]
    fn a_refused_refresh_keeps_the_klines_it_held() {
        let fake = FakeBinance::new(6_000, 0);
        let client = client_of(&fake);
        let held = tail_59m(&client, "BTCUSDT").unwrap();
        fake.advance(1);
        *fake.fail_at.lock().unwrap() = Some((19, 429, "30"));
        assert!(tail_59m(&client, "BTCUSDT").is_err());
        assert!(tail_59m(&client, "BTCUSDT")
            .err()
            .unwrap()
            .contains(REFUSED));
        fake.advance(30);
        let again = tail_59m(&client, "BTCUSDT").unwrap();
        assert_eq!(fake.requests(), 18 + 1 + 1, "one refresh, no new walk");
        assert_eq!(again.data.len(), held.data.len());
    }

    #[test]
    fn concurrent_first_reads_of_a_chart_share_one_walk() {
        let fake = FakeBinance::new(6_000, 1);
        let client = client_of(&fake);
        let results: Vec<Result<Fetched, String>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..6)
                .map(|_| {
                    let client = &client;
                    s.spawn(move || tail_59m(client, "BTCUSDT"))
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(results.iter().all(|r| r.is_ok()));
        assert_eq!(fake.requests(), 18, "one walk, and the clock has not moved");
        assert!(no_read_under_way(&client));
    }

    #[test]
    fn a_walk_waits_for_a_permit_and_gives_up_cleanly() {
        let walks = Walks::default();
        let permits: Vec<_> = (0..MAX_WALKS)
            .map(|_| walks.enter(far()).unwrap())
            .collect();
        let soon = Instant::now() + Duration::from_millis(20);
        let e = walk_klines(&walks, BINANCE_MAX_LIMIT + 1, None, soon, |_, _| {
            panic!("a call that gave up sends nothing")
        })
        .err()
        .unwrap();
        // Others' reads may just be slow, so it does not blame the rate limit.
        assert!(e.contains("timed out") && !e.contains(REFUSED), "{e}");
        // One page needs no permit, and a freed permit is free again.
        let one = walk_klines(&walks, 1, None, far(), fake_exchange(0, HOUR_MS * 3)).unwrap();
        assert_eq!(one.len(), 1);
        drop(permits);
        assert!(walks.enter(far()).is_ok());
    }

    type Seen = (Vec<(String, (f64, f64, f64, f64, f64))>, bool);

    fn seen(f: &Fetched) -> Seen {
        let bars = f.data.bars.iter();
        (
            bars.map(|b| (b.date.clone(), ohlcv(b))).collect(),
            f.has_more,
        )
    }

    /// `fetch_binance_via` with no cache: every read straight to Binance, on a client
    /// of its own, so what is compared against has shared nothing.
    fn direct(
        fake: &Arc<FakeBinance>,
        symbol: &str,
        interval: &str,
        limit: usize,
        end_time_ms: Option<i64>,
    ) -> Seen {
        let client = client_of(fake);
        let iv = Interval::parse(interval).unwrap();
        let (base, per_bar) = iv.binance_base();
        let (want, end) = if per_bar > 1 {
            resample_window(&iv, per_bar, limit, end_time_ms)
        } else {
            (limit, end_time_ms)
        };
        let native = paginate_backwards(want, end, far(), |n, e| {
            fetch_binance_page(&client, &binance_klines_url(symbol, base, n, e))
        })
        .unwrap();
        let (bars, has_more) = if per_bar > 1 {
            resampled_bars(&native, &iv, limit, want)
        } else {
            let more = native.len() >= want;
            (native, more)
        };
        let data = OhlcvData {
            bars,
            symbol: None,
            interval: None,
            session: None,
        };
        seen(&Fetched { data, has_more })
    }

    /// `n` concurrent reads of `f`, in order of `i`.
    fn together<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..n)
                .map(|i| {
                    let f = &f;
                    s.spawn(move || f(i))
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        })
    }

    /// Until `n` callers besides the one reading wait on this refresh.
    fn wait_for_joiners(client: &BinanceClient, key: &KlineKey, n: usize) {
        let give_up = Instant::now() + Duration::from_secs(10);
        loop {
            let waiting = client
                .klines
                .lock()
                .refreshing
                .get(key)
                .map_or(0, |f| Arc::strong_count(f).saturating_sub(2));
            if waiting >= n {
                return;
            }
            assert!(Instant::now() < give_up, "{waiting} joined");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_hundred_reads_of_mixed_sizes_share_one_walk() {
        let sizes = [50, 300, 800, 1_000, 1_500, 5_000, 17_877];
        let fake = FakeBinance::new(600_000, 1);
        let client = client_of(&fake);
        let got = together(100, |i| {
            fetch_binance_via(&client, "BTCUSDT", "1h", sizes[i % sizes.len()], None).unwrap()
        });
        assert_eq!(fake.requests(), 18, "as many pages as the largest needs");
        assert!(no_read_under_way(&client));
        for (i, f) in got.iter().enumerate() {
            let size = sizes[i % sizes.len()];
            assert_eq!(
                seen(f),
                direct(&fake, "BTCUSDT", "1h", size, None),
                "{size}"
            );
        }
    }

    #[test]
    fn a_repeat_sends_nothing_until_the_window_ends_then_one_caller_does() {
        let fake = FakeBinance::new(600_000, 1);
        let client = client_of(&fake);
        let read = |limit| fetch_binance_via(&client, "BTCUSDT", "1h", limit, None);
        let first = read(800).unwrap();
        assert_eq!(fake.requests(), 1);
        for _ in 0..50 {
            assert_eq!(seen(&read(800).unwrap()), seen(&first));
        }
        fake.advance_ms(999);
        assert_eq!(seen(&read(300).unwrap()).0.len(), 300);
        assert_eq!(fake.requests(), 1, "still inside the window");

        fake.advance_ms(1);
        let after = together(100, |_| read(800).unwrap());
        assert_eq!(fake.requests(), 2, "one of a hundred refreshes");
        assert_eq!(after.len(), 100);
        // Whoever was served during the refresh has the old read, the rest the new.
        let (old, new) = (seen(&first), direct(&fake, "BTCUSDT", "1h", 800, None));
        assert!(after.iter().all(|f| seen(f) == old || seen(f) == new));
        assert_eq!(seen(&read(800).unwrap()), new);

        // One poller costs a request per window, however often it polls.
        let before = fake.requests();
        for _ in 0..20 {
            fake.advance_ms(250);
            read(800).unwrap();
        }
        assert_eq!(fake.requests() - before, 5);
    }

    #[test]
    fn during_a_refresh_others_are_served_until_the_klines_are_too_old() {
        let fake = FakeBinance::new(600_000, 0);
        let client = client_of(&fake);
        let read = || fetch_binance_via(&client, "BTCUSDT", "1h", 800, None).unwrap();
        let held = seen(&read());
        fake.advance(1);
        fake.pause(true);
        std::thread::scope(|s| {
            let refreshing = s.spawn(read);
            fake.wait_for_requests(2);
            // Upstream has not answered, and these do not wait for it.
            for _ in 0..20 {
                assert_eq!(seen(&read()), held);
            }
            assert_eq!(fake.requests(), 2);

            // Past the limit they wait. A refresh this slow still serves them: were it
            // too old to take, each would refresh in turn until all timed out.
            fake.advance(9);
            let waiting = s.spawn(read);
            wait_for_joiners(&client, &("BTCUSDT".to_string(), "1h"), 1);
            assert!(!waiting.is_finished());
            fake.pause(false);
            let (a, b) = (refreshing.join().unwrap(), waiting.join().unwrap());
            assert_eq!(seen(&a), seen(&b));
            assert_ne!(seen(&a), held, "the forming kline moved");
            assert_eq!(seen(&a), direct(&fake, "BTCUSDT", "1h", 800, None));
        });
        assert_eq!(
            fake.requests(),
            2 + 1,
            "the refresh, and the direct read above"
        );
    }

    #[test]
    fn identical_scroll_back_pages_share_a_read_and_others_do_not() {
        let fake = FakeBinance::new(600_000, 5);
        let client = client_of(&fake);
        let end = fake.now_s.load(Ordering::SeqCst) * 1000 - 500 * HOUR_MS;
        let got = together(20, |_| {
            fetch_binance_via(&client, "BTCUSDT", "1h", 800, Some(end)).unwrap()
        });
        assert_eq!(fake.requests(), 1, "twenty identical pages, one read");
        for f in &got {
            assert_eq!(seen(f), direct(&fake, "BTCUSDT", "1h", 800, Some(end)));
        }
        let fake = FakeBinance::new(600_000, 5);
        let client = client_of(&fake);
        let different: [(&str, &str, usize, i64); 4] = [
            ("BTCUSDT", "1h", 800, end),
            ("BTCUSDT", "1h", 800, end - HOUR_MS),
            ("BTCUSDT", "1h", 700, end),
            ("ETHUSDT", "1h", 800, end),
        ];
        together(4, |i| {
            let (symbol, kline, limit, end) = different[i];
            fetch_binance_via(&client, symbol, kline, limit, Some(end)).unwrap()
        });
        assert_eq!(fake.requests(), 4, "different pages read separately");
        fetch_binance_via(&client, "BTCUSDT", "4h", 800, Some(end)).unwrap();
        assert_eq!(fake.requests(), 5, "a different kline too");

        // Case does not matter, and a kept page sends nothing until it expires.
        fetch_binance_via(&client, "btcusdt", "1h", 800, Some(end)).unwrap();
        assert_eq!(fake.requests(), 5);
        fake.advance(PAGE_CACHE_TTL.as_secs() as i64);
        fetch_binance_via(&client, "BTCUSDT", "1h", 800, Some(end)).unwrap();
        assert_eq!(fake.requests(), 6);
    }

    #[test]
    fn a_page_reaching_the_forming_kline_is_shared_but_not_kept() {
        let fake = FakeBinance::new(600_000, 0);
        let client = client_of(&fake);
        let end = fake.now_s.load(Ordering::SeqCst) * 1000 + 2 * HOUR_MS;
        let read = || fetch_binance_via(&client, "BTCUSDT", "1h", 100, Some(end)).unwrap();
        read();
        fake.advance(30);
        let later = read();
        assert_eq!(fake.requests(), 2);
        assert_eq!(seen(&later), direct(&fake, "BTCUSDT", "1h", 100, Some(end)));
        assert!(client.pages.state.lock().unwrap().held.is_empty());
    }

    #[test]
    fn a_failed_page_fails_who_joined_it_and_stores_nothing() {
        let fake = FakeBinance::new(600_000, 5);
        let client = client_of(&fake);
        let end = fake.now_s.load(Ordering::SeqCst) * 1000 - 500 * HOUR_MS;
        *fake.fail_at.lock().unwrap() = Some((1, 500, "0"));
        fake.pause(true);
        let read = || fetch_binance_via(&client, "BTCUSDT", "1h", 800, Some(end));
        let key: PageKey = ("BTCUSDT".to_string(), "1h", end, 800);
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..5).map(|_| s.spawn(read)).collect();
            let give_up = Instant::now() + Duration::from_secs(10);
            loop {
                let flying = client.pages.state.lock().unwrap().flying.get(&key).cloned();
                if flying.is_some_and(|f| Arc::strong_count(&f) >= 7) {
                    break;
                }
                assert!(Instant::now() < give_up);
                std::thread::sleep(Duration::from_millis(1));
            }
            fake.pause(false);
            for h in handles {
                let e = h.join().unwrap().err().unwrap();
                assert!(e.contains("500"), "{e}");
            }
        });
        assert_eq!(fake.requests(), 1);
        assert!(client.pages.state.lock().unwrap().held.is_empty());
        assert!(read().is_ok());
        assert_eq!(fake.requests(), 2);
    }

    #[test]
    fn a_kline_closing_in_the_window_gives_what_a_direct_read_gives() {
        for (tf, limit) in [("1m", 800), ("7m", 60), ("59m", 302), ("1m", 3_000)] {
            let fake = FakeBinance::new(600_000, 0);
            let client = client_of(&fake);
            let read = || fetch_binance_via(&client, "BTCUSDT", tf, limit, None).unwrap();
            // Half a second before the minute ends.
            fake.advance(59);
            fake.advance_ms(500);
            let before = read();
            let requests = fake.requests();
            fake.advance_ms(200);
            assert_eq!(seen(&read()), seen(&before), "{tf}: inside the window");
            assert_eq!(fake.requests(), requests);

            // Past the boundary, 0.7 s after that read: not served from it.
            fake.advance_ms(500);
            let closed = read();
            assert_eq!(fake.requests(), requests + 1, "{tf}");
            assert_eq!(
                seen(&closed),
                direct(&fake, "BTCUSDT", tf, limit, None),
                "{tf}"
            );
            let times: Vec<i64> = closed
                .data
                .bars
                .iter()
                .map(|b| bar_ts(b).unwrap())
                .collect();
            assert!(times.windows(2).all(|w| w[0] < w[1]), "{tf}: no duplicates");
        }
    }

    #[test]
    fn a_kline_closing_during_a_refresh_is_not_served_from_it() {
        let fake = FakeBinance::new(600_000, 0);
        let client = client_of(&fake);
        let read = || fetch_binance_via(&client, "BTCUSDT", "1m", 800, None).unwrap();
        fake.advance(58);
        fake.advance_ms(500);
        read();
        fake.advance(1);
        fake.pause(true);
        std::thread::scope(|s| {
            // Sent at 59.5 s, so its kline is the one about to close.
            let refreshing = s.spawn(read);
            fake.wait_for_requests(2);
            fake.advance_ms(700);
            let waiting = s.spawn(read);
            wait_for_joiners(&client, &("BTCUSDT".to_string(), "1m"), 1);
            fake.pause(false);
            refreshing.join().unwrap();
            let served = waiting.join().unwrap();
            assert_eq!(seen(&served), direct(&fake, "BTCUSDT", "1m", 800, None));
        });
        assert_eq!(
            fake.requests(),
            3 + 1,
            "its own refresh, and the direct read"
        );
    }

    #[test]
    fn a_failing_refresh_keeps_the_window_and_a_later_read_recovers() {
        let fake = FakeBinance::new(600_000, 0);
        let client = client_of(&fake);
        let read = || fetch_binance_via(&client, "BTCUSDT", "1h", 800, None);
        let held = seen(&read().unwrap());
        fake.advance(1);
        *fake.fail_at.lock().unwrap() = Some((2, 500, "0"));
        let e = read().err().unwrap();
        assert!(e.contains("500"), "{e}");
        assert_eq!(client.klines.lock().held.len(), 1);
        let key = ("BTCUSDT".to_string(), "1h");
        assert_eq!(client.klines.lock().held[&key].bars.len(), 1_000);

        // The next read is not told the klines are fresh: it refreshes, and is right.
        let after = read().unwrap();
        assert_eq!(fake.requests(), 3);
        assert_ne!(seen(&after), held);
        assert_eq!(seen(&after), direct(&fake, "BTCUSDT", "1h", 800, None));

        // Those waiting on a refresh that fails share its error, and send nothing.
        let fake = FakeBinance::new(600_000, 0);
        let client = client_of(&fake);
        let read = || fetch_binance_via(&client, "BTCUSDT", "1h", 800, None);
        read().unwrap();
        fake.advance(5);
        *fake.fail_at.lock().unwrap() = Some((2, 500, "0"));
        fake.pause(true);
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..8).map(|_| s.spawn(read)).collect();
            wait_for_joiners(&client, &key, 7);
            fake.pause(false);
            for h in handles {
                let e = h.join().unwrap().err().unwrap();
                assert!(e.contains("500"), "{e}");
            }
        });
        assert_eq!(fake.requests(), 2, "eight callers, one failure");
        assert!(read().is_ok());
    }

    #[test]
    fn a_failing_first_read_stores_nothing_and_shares_its_error() {
        let fake = FakeBinance::new(600_000, 0);
        let client = client_of(&fake);
        let read = || fetch_binance_via(&client, "BTCUSDT", "1h", 800, None);
        *fake.fail_at.lock().unwrap() = Some((1, 500, "0"));
        fake.pause(true);
        let key = ("BTCUSDT".to_string(), "1h");
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..6).map(|_| s.spawn(read)).collect();
            let give_up = Instant::now() + Duration::from_secs(10);
            loop {
                let flying = client.klines.lock().walking.get(&key).cloned();
                if flying.is_some_and(|f| Arc::strong_count(&f) >= 8) {
                    break;
                }
                assert!(Instant::now() < give_up);
                std::thread::sleep(Duration::from_millis(1));
            }
            fake.pause(false);
            for h in handles {
                assert!(h.join().unwrap().err().unwrap().contains("500"));
            }
        });
        assert_eq!(fake.requests(), 1);
        assert!(client.klines.lock().held.is_empty());
        assert_eq!(seen(&read().unwrap()).0.len(), 800);
    }

    #[test]
    fn a_panic_in_the_read_frees_who_waits_for_it() {
        let cache = KlineCache::default();
        let walks = Walks::default();
        let key: KlineKey = ("BTCUSDT".to_string(), "1h");
        let t0 = Instant::now();
        let clock = move || Now {
            mono: t0,
            wall_ms: 1_700_000_000_000,
        };
        let (started, panic_now) = (std::sync::Barrier::new(2), std::sync::Barrier::new(2));
        std::thread::scope(|s| {
            let reader = s.spawn(|| {
                newest_klines(&cache, &walks, key.clone(), 10, &clock, far(), |_, _| {
                    started.wait();
                    panic_now.wait();
                    panic!("the read panics");
                })
            });
            started.wait();
            let waiter = s.spawn(|| {
                newest_klines(&cache, &walks, key.clone(), 10, &clock, far(), |_, _| {
                    panic!("a joiner sends nothing")
                })
            });
            let give_up = Instant::now() + Duration::from_secs(10);
            loop {
                let flying = cache.lock().walking.get(&key).cloned();
                if flying.is_some_and(|f| Arc::strong_count(&f) >= 4) {
                    break;
                }
                assert!(Instant::now() < give_up);
                std::thread::sleep(Duration::from_millis(1));
            }
            panic_now.wait();
            assert!(reader.join().is_err());
            assert_eq!(waiter.join().unwrap().err().unwrap(), READ_ABORTED);
        });
        // Nothing is stuck: the next read takes the turn.
        let bars = newest_klines(&cache, &walks, key, 5, &clock, far(), |limit, end| {
            fake_exchange(0, 1_700_000_000_000)(limit, end)
        });
        assert_eq!(bars.unwrap().len(), 5);
    }

    #[test]
    fn nobody_waits_past_the_deadline_for_a_read_that_hangs() {
        let cache = KlineCache::default();
        let walks = Walks::default();
        let key: KlineKey = ("BTCUSDT".to_string(), "1h");
        let t0 = Instant::now();
        let clock = move || Now {
            mono: t0,
            wall_ms: 1_700_000_000_000,
        };
        let (started, finish) = (std::sync::Barrier::new(2), std::sync::Barrier::new(2));
        std::thread::scope(|s| {
            let reader = s.spawn(|| {
                newest_klines(
                    &cache,
                    &walks,
                    key.clone(),
                    10,
                    &clock,
                    far(),
                    |limit, end| {
                        started.wait();
                        finish.wait();
                        fake_exchange(0, 1_700_000_000_000)(limit, end)
                    },
                )
            });
            started.wait();
            let soon = Instant::now() + Duration::from_millis(30);
            let e = newest_klines(&cache, &walks, key.clone(), 10, &clock, soon, |_, _| {
                panic!("a joiner sends nothing")
            });
            assert_eq!(e.err().unwrap(), BUSY);
            finish.wait();
            assert_eq!(reader.join().unwrap().unwrap().len(), 10);
        });

        // The same for a page.
        let pages = PageCache::default();
        let key: PageKey = ("BTCUSDT".to_string(), "1h", 1_000_000, 10);
        let (started, finish) = (std::sync::Barrier::new(2), std::sync::Barrier::new(2));
        std::thread::scope(|s| {
            let reader = s.spawn(|| {
                shared_page(&pages, key.clone(), &clock, far(), || {
                    started.wait();
                    finish.wait();
                    Ok(vec![bar_at(0, 1.0)])
                })
            });
            started.wait();
            let soon = Instant::now() + Duration::from_millis(30);
            let e = shared_page(&pages, key.clone(), &clock, soon, || {
                panic!("a joiner sends nothing")
            });
            assert_eq!(e.err().unwrap(), BUSY);
            finish.wait();
            assert_eq!(reader.join().unwrap().unwrap().len(), 1);
        });
    }

    #[test]
    fn eviction_holds_the_memory_bound() {
        let last = 1_700_000_000_000 / HOUR_MS * HOUR_MS;
        let cache = KlineCache::default();
        let t0 = Instant::now();
        let at = |i: usize| t0 + Duration::from_millis(i as u64);
        // By klines: 20,000 each would hold 12.
        for i in 0..40 {
            cached_read(&cache, &format!("L{i}"), 20_000, at(i), (0, last));
            let st = cache.lock();
            let total: usize = st.held.values().map(|h| h.bars.len()).sum();
            assert!(total <= KLINE_CACHE_BARS, "{total}");
            assert!(
                st.held.contains_key(&(format!("L{i}"), "1h")),
                "kept the new"
            );
        }
        {
            let st = cache.lock();
            assert_eq!(st.held.len(), KLINE_CACHE_BARS / 20_000);
            assert!(
                !st.held.contains_key(&("L0".to_string(), "1h")),
                "oldest gone"
            );
            assert!(st.held.contains_key(&("L39".to_string(), "1h")));
        }
        // A read of a held window counts as use.
        cached_read(&cache, "L28", 10, at(100), (0, last));
        cached_read(&cache, "X", 20_000, at(101), (0, last));
        assert!(cache.lock().held.contains_key(&("L28".to_string(), "1h")));
        assert!(!cache.lock().held.contains_key(&("L29".to_string(), "1h")));

        // A refresh adds no slack, or the bound in bars would not bound the bytes,
        // and a window stays at its cap.
        let t1 = at(200) + KLINE_FRESH;
        cached_read(&cache, "X", 20_000, t1, (0, last + HOUR_MS));
        let st = cache.lock();
        let held = &st.held[&("X".to_string(), "1h")];
        assert_eq!(held.bars.len(), MAX_BASE_BARS);
        assert!(held.bars.capacity() <= MAX_BASE_BARS + KLINE_REFRESH);
        drop(st);

        // Pages, by entries and by klines.
        let pages = PageCache::default();
        let clock = move || Now {
            mono: t0,
            wall_ms: last,
        };
        let page = |n: usize| (0..n).map(|i| bar_at(i as i64, 1.0)).collect::<Vec<_>>();
        for i in 0..200 {
            let key = ("A".to_string(), "1h", i, 10);
            shared_page(&pages, key, &clock, far(), || Ok(page(10))).unwrap();
        }
        assert_eq!(pages.state.lock().unwrap().held.len(), PAGE_CACHE_ENTRIES);
        for i in 0..10 {
            let key = ("B".to_string(), "1h", i, 20_000);
            shared_page(&pages, key, &clock, far(), || Ok(page(20_000))).unwrap();
            let st = pages.state.lock().unwrap();
            let total: usize = st.held.values().map(|p| p.bars.len()).sum();
            assert!(total <= PAGE_CACHE_BARS, "{total}");
        }
    }

    #[test]
    fn has_more_and_bars_equal_a_direct_read_in_any_order() {
        let sizes = [1, 5, 500, 999, 1_000, 1_001, 1_500, 2_000, 6_000];
        let histories = [1, 5, 999, 1_000, 1_001, 1_500, 2_000, 2_500, 5_000];
        for klines in histories {
            for tf in ["1h", "7h", "2d"] {
                for reversed in [false, true] {
                    let fake = FakeBinance::new(6_000_000, 0);
                    let client = client_of(&fake);
                    let step = Interval::parse(tf).unwrap().binance_base();
                    let step_ms = data::interval_secs(step.0).unwrap() * 1000;
                    let now = fake.now_s.load(Ordering::SeqCst) * 1000;
                    let newest = now.div_euclid(step_ms) * step_ms;
                    let listed = newest - (klines - 1) * step_ms;
                    fake.listed_ms.store(listed, Ordering::SeqCst);
                    let mut order = sizes.to_vec();
                    if reversed {
                        order.reverse();
                    }
                    for limit in order {
                        let what = format!("{tf}, {klines} klines, {limit}, reversed {reversed}");
                        let got = fetch_binance_via(&client, "NEWUSDT", tf, limit, None);
                        let want = direct(&fake, "NEWUSDT", tf, limit, None);
                        match got {
                            Ok(f) => assert_eq!(seen(&f), want, "{what}"),
                            // Too few klines for one whole bar, as for a direct read.
                            Err(e) => {
                                assert_eq!(e, NO_BARS, "{what}");
                                assert!(want.0.is_empty(), "{what}");
                            }
                        }
                    }
                    // A new kline opens: the window of a short history grows by it.
                    fake.advance(step_ms / 1000);
                    let got = fetch_binance_via(&client, "NEWUSDT", tf, 1_000, None);
                    let want = direct(&fake, "NEWUSDT", tf, 1_000, None);
                    assert_eq!(got.map(|f| seen(&f)).ok(), Some(want), "{tf} {klines}");
                    // And pages that run out of history stop as a direct read does.
                    let end = listed + 100 * step_ms;
                    for limit in [50, 100, 101, 2_000] {
                        let got = fetch_binance_via(&client, "NEWUSDT", tf, limit, Some(end));
                        let want = direct(&fake, "NEWUSDT", tf, limit, Some(end));
                        assert_eq!(seen(&got.unwrap()), want, "{tf} {klines} page {limit}");
                    }
                }
            }
        }
    }

    #[test]
    fn symbols_and_klines_never_mix_and_case_does_not_matter() {
        let fake = FakeBinance::new(600_000, 0);
        let client = client_of(&fake);
        let read = |symbol: &str, tf: &str| fetch_binance_via(&client, symbol, tf, 100, None);
        let btc = seen(&read("BTCUSDT", "1h").unwrap());
        assert_eq!(seen(&read("btcusdt", "1h").unwrap()), btc);
        assert_eq!(seen(&read("BtcUsdt", "1h").unwrap()), btc);
        assert_eq!(fake.requests(), 1, "one entry, whatever the case");
        read("ETHUSDT", "1h").unwrap();
        read("BTCUSDT", "4h").unwrap();
        read("BTCUSDT", "1M").unwrap();
        read("BTCUSDT", "1m").unwrap();
        assert_eq!(fake.requests(), 5, "each its own entry");
        assert_ne!(seen(&read("BTCUSDT", "4h").unwrap()), btc);
        // A custom interval shares its base's window with the native chart.
        read("BTCUSDT", "7h").unwrap();
        assert_eq!(fake.requests(), 5);
    }

    /// One whole tool call, with the fake behind `binance()` on this thread.
    fn chart_call(client: &'static BinanceClient, args: serde_json::Value) -> serde_json::Value {
        TEST_CLIENT.set(Some(client));
        let resp =
            crate::mcp::generate_chart(Some(serde_json::json!(1)), &args, DataSource::Live, None);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"))
    }

    #[test]
    fn whole_chart_loads_share_one_request_and_a_tail_equals_a_cold_read() {
        let fake = FakeBinance::new(600_000, 1);
        let client: &'static BinanceClient = Box::leak(Box::new(client_of(&fake)));
        let args = |bars: u64, warmup: u64, ticker: &str| {
            serde_json::json!({
                "ticker": ticker, "timeframe": "1h", "bars": bars, "warmup": warmup,
                "indicators": ["rsi", "donchian", "macd"], "format": "series"
            })
        };
        let loads = together(100, |_| chart_call(client, args(500, 300, "BTCUSDT")));
        let measured = fake.requests();
        eprintln!("100 concurrent chart loads through generate_chart: {measured} request(s)");
        assert_eq!(measured, 1);
        assert!(loads.iter().all(|p| p == &loads[0]));
        assert_eq!(loads[0]["bars"].as_array().unwrap().len(), 500);

        // A 600-bar warmup, as `chart_patterns` asks: a tail off the shared window
        // equals the same read on a server that has read nothing yet.
        let hot = chart_call(client, args(2, 600, "BTCUSDT"));
        assert_eq!(fake.requests(), measured, "the window serves the tail");
        let cold_client: &'static BinanceClient = Box::leak(Box::new(client_of(&fake)));
        let cold = chart_call(cold_client, args(2, 600, "BTCUSDT"));
        assert_eq!(hot, cold);
        // And its bars and rolling values are those of a full load of the same moment.
        let full = chart_call(client, args(500, 600, "BTCUSDT"));
        let tail_of = |p: &serde_json::Value, path: &[&str]| {
            let mut v = p;
            for key in path {
                v = &v[*key];
            }
            let all = v.as_array().unwrap();
            all[all.len() - 2..].to_vec()
        };
        assert_eq!(tail_of(&hot, &["bars"]), tail_of(&full, &["bars"]));
        let rolling = |p: &serde_json::Value| {
            let lines = &p["indicators"]["donchian"]["lines"][0];
            let values = lines["values"].as_array().unwrap();
            values[values.len() - 2..].to_vec()
        };
        assert!(rolling(&hot).iter().all(|v| v.is_number()));
        assert_eq!(rolling(&hot), rolling(&full));
    }

    #[test]
    fn binance_paging_a_custom_interval_joins_up_to_one_read() {
        // The glue around the walk: want, end and has_more, against real URLs.
        let fake = FakeBinance::new(600_000, 0);
        let client = client_of(&fake);
        let dates = |f: &Fetched| {
            f.data
                .bars
                .iter()
                .map(|b| b.date.clone())
                .collect::<Vec<_>>()
        };
        for (tf, n) in [("7m", 60), ("45m", 40), ("7h", 30), ("2d", 20), ("2wk", 12)] {
            let whole = fetch_binance_via(&client, "BTCUSDT", tf, n, None).unwrap();
            assert!(whole.has_more, "{tf}");
            assert_eq!(whole.data.len(), n, "{tf}");
            let mid_ms = bar_open_ms(&whole.data.bars[n / 2]).unwrap();
            let newer = fetch_binance_via(&client, "BTCUSDT", tf, n / 2, None).unwrap();
            let older = fetch_binance_via(&client, "BTCUSDT", tf, n / 2, Some(mid_ms)).unwrap();
            let mut joined = dates(&older);
            joined.extend(dates(&newer));
            assert_eq!(joined, dates(&whole), "{tf}");
            let key = |f: &Fetched| f.data.bars.iter().map(ohlcv).collect::<Vec<_>>();
            assert_eq!(key(&older), key(&whole)[..n / 2].to_vec(), "{tf}");
        }
    }

    #[test]
    fn binance_resample_is_stable_across_requests_and_keeps_only_whole_bars() {
        let base = 1_700_006_400 / 420 * 420; // on the 7m grid
        let bars: Vec<Bar> = (0..30)
            .map(|i| bar_at(base + 60 + i * 60, i as f64))
            .collect();
        let iv = Interval::parse("7m").unwrap();
        let all = resample(&bars, &iv, true);
        // The first bucket lacks its first minute, so it goes.
        assert_eq!(bar_ts(&all[0]), Some(base + 420));
        assert!(all.iter().all(|b| bar_ts(b).unwrap() % 420 == 0));
        let first = &all[0];
        assert_eq!((first.open, first.close, first.volume), (6.0, 12.0, 7.0));
        // A reload from a later start gives the same bars.
        let reload = resample(&bars[10..], &iv, true);
        for b in &reload {
            let same = all.iter().find(|a| a.date == b.date).unwrap();
            assert_eq!(ohlcv(same), ohlcv(b));
        }
        // The first bars of a symbol stay when nothing older exists.
        let listed = resample(&bars, &iv, false);
        assert_eq!(bar_ts(&listed[0]), Some(base));
        assert_eq!(listed[0].volume, 6.0);
    }

    #[test]
    fn binance_resample_two_weeks_and_two_months_follow_the_calendar() {
        let w = |d: i64| (4 + d * 7) * DAY; // Mondays
        let n = 2_900; // an even week from the Monday origin
        let bars: Vec<Bar> = (n..n + 5).map(|i| bar_at(w(i), i as f64)).collect();
        let f = resample(&bars, &Interval::parse("2W").unwrap(), true);
        assert_eq!(
            f.iter().map(|b| bar_ts(b).unwrap()).collect::<Vec<_>>(),
            vec![w(n), w(n + 2), w(n + 4)]
        );
        assert_eq!(f[0].volume, 2.0);

        let ts = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp();
        let months = [
            "2026-01-01T00:00:00Z",
            "2026-02-01T00:00:00Z",
            "2026-03-01T00:00:00Z",
            "2026-04-01T00:00:00Z",
        ];
        let bars: Vec<Bar> = months.iter().map(|m| bar_at(ts(m), 1.0)).collect();
        let f = resample(&bars, &Interval::parse("2M").unwrap(), true);
        assert_eq!(
            f.iter().map(|b| bar_ts(b).unwrap()).collect::<Vec<_>>(),
            vec![ts(months[0]), ts(months[2])]
        );
        assert!(f.iter().all(|b| b.volume == 2.0));
    }

    #[test]
    fn yahoo_custom_intraday_buckets_anchor_at_session_open() {
        // 1m bars of a 09:30 open and of the next day's: 45m stays anchored.
        let mut ts: Vec<i64> = (0..390).map(|i| NOV3_OPEN + i * 60).collect();
        ts.extend((0..100).map(|i| NOV4_OPEN + i * 60));
        let body = yahoo_body(&ts, "AAPL", "EQUITY", -18_000);
        let f = parse_yahoo_chart(&body, "45m", 100, None, &win("1m", 0)).unwrap();
        let d = dates(&f);
        assert_eq!(d[0], NOV3_OPEN);
        assert_eq!(d[1], NOV3_OPEN + 45 * 60);
        // 390 minutes make 8 full buckets and a short one, then the next day.
        assert_eq!(d[9], NOV4_OPEN);
        assert_eq!(f.data.bars[8].volume, (360..390).sum::<i32>() as f64);
        // 7m on a crypto pair is the epoch grid, as Binance's would be.
        let base = 1_700_006_400 / 420 * 420;
        let ts: Vec<i64> = (0..20).map(|i| base + i * 60).collect();
        let body = yahoo_body(&ts, "BTC-USD", "CRYPTOCURRENCY", 0);
        let f = parse_yahoo_chart(&body, "7m", 100, None, &win("1m", 0)).unwrap();
        assert_eq!(dates(&f), vec![base, base + 420, base + 840]);
    }

    #[test]
    fn yahoo_meta_gives_the_regular_session() {
        use chrono::Timelike;
        // The AAPL fixture: 09:30 to 16:00 New York, a US listing.
        let f = parse_yahoo_chart(&fixture(), "1h", 100, None, &win("1h", 0)).unwrap();
        let s = f.data.session.expect("an equity has a session");
        assert_eq!(s.tz, chrono_tz::America::New_York);
        assert_eq!((s.open.hour(), s.open.minute()), (9, 30));
        assert_eq!((s.close.hour(), s.close.minute()), (16, 0));
        assert!(s.us_calendar);

        let meta = |instrument: &str, tz: &str, start: i64, end: i64| {
            serde_json::json!({
                "instrumentType": instrument,
                "exchangeTimezoneName": tz,
                "currentTradingPeriod": { "regular": { "start": start, "end": end } },
            })
        };
        // 09:00 to 15:30 in Tokyo: hours, and no US calendar.
        let tokyo = 1_793_923_200;
        let s = session_from_meta(&meta("EQUITY", "Asia/Tokyo", tokyo, tokyo + 23_400)).unwrap();
        assert_eq!(
            (s.open.hour(), s.close.hour(), s.close.minute()),
            (9, 15, 30)
        );
        assert!(!s.us_calendar);
        // A New York future runs all day, so it has no session.
        let future = meta("FUTURE", "America/New_York", tokyo, tokyo + 86_340);
        assert_eq!(session_from_meta(&future), None);
        // Unknown zone: no session rather than a wrong one.
        assert_eq!(
            session_from_meta(&meta("EQUITY", "Mars/Base", tokyo, tokyo + 23_400)),
            None
        );

        // Today an early close: the recent days say what a normal close is.
        let open = NOV4_OPEN;
        let mut early = meta("EQUITY", "America/New_York", open, open + 12_600);
        assert_eq!(session_from_meta(&early).unwrap().close.hour(), 13);
        early["tradingPeriods"] = serde_json::json!([
            [{ "start": open - DAY, "end": open - DAY + 23_400 }],
            [{ "start": open, "end": open + 12_600 }],
        ]);
        assert_eq!(session_from_meta(&early).unwrap().close.hour(), 16);

        // 24/7 instruments have none, whatever their metadata says.
        let body = yahoo_body(&[1_700_006_400], "BTC-USD", "CRYPTOCURRENCY", 0);
        let f = parse_yahoo_chart(&body, "1h", 100, None, &win("1h", 0)).unwrap();
        assert_eq!(f.data.session, None);
    }

    #[test]
    fn yahoo_quarters_are_calendar_quarters_built_from_months() {
        let ts = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp();
        // Yahoo's monthly stamps across a DST change, then its live print, which
        // the snap merges into November's row.
        let rows = [
            "2026-07-01T04:00:00Z",
            "2026-08-01T04:00:00Z",
            "2026-09-01T04:00:00Z",
            "2026-10-01T04:00:00Z",
            "2026-11-01T04:00:00Z",
            "2026-11-05T21:00:00Z",
        ];
        let stamps: Vec<i64> = rows.iter().map(|s| ts(s)).collect();
        let body = yahoo_body(&stamps, "AAPL", "EQUITY", -18_000);
        let window = yahoo_window("3mo", 40, None, ts("2026-11-06T00:00:00Z"));
        assert_eq!(window.api_interval, "1mo");
        let f = parse_yahoo_chart(&body, "3mo", 100, None, &win(window.api_interval, 0)).unwrap();
        // Two quarters, with no bar for the month the live print falls in.
        assert_eq!(
            dates(&f),
            vec![ts("2026-07-01T00:00:00Z"), ts("2026-10-01T00:00:00Z")]
        );
        // July to September, then October and November's row with the print.
        assert_eq!(f.data.bars[0].volume, 0.0 + 1.0 + 2.0);
        assert_eq!(f.data.bars[1].close, 5.0);
    }

    #[test]
    fn yahoo_session_grid_ignores_rows_without_a_price() {
        // Yahoo sends null prices for some minutes, here the first three.
        let ts: Vec<i64> = (0..30).map(|i| NOV4_OPEN + i * 60).collect();
        let mut body = yahoo_body(&ts, "AAPL", "EQUITY", -18_000);
        for i in 0..3 {
            body["chart"]["result"][0]["indicators"]["quote"][0]["close"][i] =
                serde_json::Value::Null;
        }
        let f = parse_yahoo_chart(&body, "7m", 100, None, &win("1m", 0)).unwrap();
        let d = dates(&f);
        assert_eq!(d[0], NOV4_OPEN, "the first bar sits on the open");
        assert_eq!(d[1], NOV4_OPEN + 420);
        assert!(d.iter().all(|t| (t - NOV4_OPEN) % 420 == 0), "{d:?}");
        // So do nulls in the middle of the session.
        let mut body = yahoo_body(&ts, "AAPL", "EQUITY", -18_000);
        for i in 8..16 {
            body["chart"]["result"][0]["indicators"]["quote"][0]["close"][i] =
                serde_json::Value::Null;
        }
        let f = parse_yahoo_chart(&body, "7m", 100, None, &win("1m", 0)).unwrap();
        let d = dates(&f);
        assert!(d.iter().all(|t| (t - NOV4_OPEN) % 420 == 0), "{d:?}");
    }

    #[test]
    fn yahoo_two_week_and_two_month_bars_survive_a_dst_change() {
        let ts = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp();
        // Yahoo's stamps: local midnight in the offset of that date. Today's
        // gmtoffset is EST, so the EDT stamps read as 23:00 the evening before.
        let weeks: Vec<i64> = [
            "2026-10-12T04:00:00Z",
            "2026-10-19T04:00:00Z",
            "2026-10-26T04:00:00Z",
            "2026-11-02T05:00:00Z",
        ]
        .iter()
        .map(|s| ts(s))
        .collect();
        let body = yahoo_body(&weeks, "AAPL", "EQUITY", -18_000);
        let f = parse_yahoo_chart(&body, "1wk", 100, None, &win("1wk", 0)).unwrap();
        assert_eq!(
            dates(&f),
            vec![
                ts("2026-10-12T00:00:00Z"),
                ts("2026-10-19T00:00:00Z"),
                ts("2026-10-26T00:00:00Z"),
                ts("2026-11-02T00:00:00Z"),
            ],
            "weekly bars keep their Monday"
        );
        let f = parse_yahoo_chart(&body, "2wk", 100, None, &win("1wk", 0)).unwrap();
        let d = dates(&f);
        let two = Interval::parse("2wk").unwrap();
        assert!(d.iter().all(|t| two.grid_start(*t) == *t), "{d:?}");
        assert_eq!(f.data.bars.iter().map(|b| b.volume).sum::<f64>(), 6.0);

        let months: Vec<i64> = [
            "2026-09-01T04:00:00Z",
            "2026-10-01T04:00:00Z",
            "2026-11-01T04:00:00Z",
        ]
        .iter()
        .map(|s| ts(s))
        .collect();
        let body = yahoo_body(&months, "AAPL", "EQUITY", -18_000);
        let f = parse_yahoo_chart(&body, "1mo", 100, None, &win("1mo", 0)).unwrap();
        assert_eq!(dates(&f)[2], ts("2026-11-01T00:00:00Z"), "not Oct 31");
        let f = parse_yahoo_chart(&body, "2mo", 100, None, &win("1mo", 0)).unwrap();
        assert_eq!(
            dates(&f),
            vec![ts("2026-09-01T00:00:00Z"), ts("2026-11-01T00:00:00Z")]
        );
    }

    #[test]
    fn yahoo_2h_6h_12h_equity_buckets_anchor_at_session_open() {
        // Nov 3 has 7 hourly bars (09:30..15:30 ET); Nov 4 the forming 3.
        let run = |tf| {
            let f = parse_yahoo_chart(&fixture(), tf, 100, None, &win("1h", 0)).unwrap();
            (dates(&f), f)
        };
        let (d, f) = run("2h");
        assert_eq!(
            d,
            vec![
                NOV3_OPEN,
                NOV3_OPEN + 2 * H,
                NOV3_OPEN + 4 * H,
                NOV3_OPEN + 6 * H, // the session's last bar stands alone
                NOV4_OPEN,
                NOV4_OPEN + 2 * H,
            ]
        );
        assert_eq!(f.data.bars[0].volume, 1000.0 + 1001.0);
        assert_eq!(f.data.bars[3].volume, 1006.0);

        let (d, f) = run("6h");
        assert_eq!(d, vec![NOV3_OPEN, NOV3_OPEN + 6 * H, NOV4_OPEN]);
        assert_eq!(f.data.bars[0].volume, (1000..1006).sum::<i32>() as f64);

        // 12h: one bar per session; the forming one keeps the live close.
        let (d, f) = run("12h");
        assert_eq!(d, vec![NOV3_OPEN, NOV4_OPEN]);
        assert_eq!(f.data.bars[0].volume, (1000..1007).sum::<i32>() as f64);
        assert_eq!(f.data.bars[1].close, 209.0);

        // A window starting mid-session drops that day, as for 4h.
        let f = parse_yahoo_chart(&fixture(), "2h", 100, None, &win("1h", NOV3_OPEN + H)).unwrap();
        assert_eq!(dates(&f), vec![NOV4_OPEN, NOV4_OPEN + 2 * H]);
    }

    /// Yahoo body with distinct OHLCV per timestamp: o = i, h = i + 5,
    /// l = 100 - i, c = i + 0.5, v = 10.
    fn ohlc_body(ts: &[i64], symbol: &str, instrument: &str) -> serde_json::Value {
        let col =
            |f: &dyn Fn(f64) -> f64| -> Vec<f64> { (0..ts.len()).map(|i| f(i as f64)).collect() };
        serde_json::json!({ "chart": { "result": [{
            "meta": { "symbol": symbol, "instrumentType": instrument, "gmtoffset": 0 },
            "timestamp": ts,
            "indicators": { "quote": [{
                "open": col(&|i| i), "high": col(&|i| i + 5.0), "low": col(&|i| 100.0 - i),
                "close": col(&|i| i + 0.5), "volume": col(&|_| 10.0),
            }] }
        }]}})
    }

    fn ohlcv(b: &Bar) -> (f64, f64, f64, f64, f64) {
        (b.open, b.high, b.low, b.close, b.volume)
    }

    #[test]
    fn yahoo_3m_aggregates_one_minute_bars() {
        let base = 1_700_006_400; // multiple of 3m
        assert_eq!(base % 180, 0);
        // Seven minutes starting one minute into the first bucket.
        let ts: Vec<i64> = (0..7).map(|i| base + 60 + i * 60).collect();
        let body = ohlc_body(&ts, "BTC-USD", "CRYPTOCURRENCY");
        let f = parse_yahoo_chart(&body, "3m", 100, None, &win("1m", base)).unwrap();
        // Buckets dated by their grid start: 2, 3 and 2 bars; the last is forming.
        assert_eq!(dates(&f), vec![base, base + 180, base + 360]);
        let b = &f.data.bars;
        assert_eq!(ohlcv(&b[0]), (0.0, 6.0, 99.0, 1.5, 20.0));
        assert_eq!(ohlcv(&b[1]), (2.0, 9.0, 96.0, 4.5, 30.0));
        assert_eq!(ohlcv(&b[2]), (5.0, 11.0, 94.0, 6.5, 20.0));
        // A window starting inside a bucket drops it.
        let f = parse_yahoo_chart(&body, "3m", 100, None, &win("1m", base + 1)).unwrap();
        assert_eq!(dates(&f), vec![base + 180, base + 360]);
    }

    #[test]
    fn yahoo_3m_snaps_the_live_bar_before_aggregating() {
        let base = 1_700_006_400;
        // The last print at base + 125 s belongs to the 1m bar at base + 120,
        // which falls in the first 3m bucket.
        let ts = vec![base, base + 60, base + 125];
        let body = ohlc_body(&ts, "BTC-USD", "CRYPTOCURRENCY");
        let f = parse_yahoo_chart(&body, "3m", 100, None, &win("1m", 0)).unwrap();
        assert_eq!(dates(&f), vec![base]);
        assert_eq!(f.data.bars[0].close, 2.5);
        assert_eq!(f.data.bars[0].volume, 30.0);
    }

    #[test]
    fn yahoo_3d_aggregates_daily_bars() {
        // UTC-midnight bars bucket on a 3-day grid counted from the epoch.
        let day = |d: i64| d * DAY;
        let d0 = 20_000 - 20_000 % 3 + 1; // Binance's 3d klines open at day % 3 == 1
        let ts: Vec<i64> = (d0..d0 + 8).map(day).collect();
        let body = ohlc_body(&ts, "BTC-USD", "CRYPTOCURRENCY");
        let f = parse_yahoo_chart(&body, "3d", 100, None, &win("1d", 0)).unwrap();
        assert_eq!(dates(&f), vec![day(d0), day(d0 + 3), day(d0 + 6)]);
        let b = &f.data.bars;
        assert_eq!(ohlcv(&b[0]), (0.0, 7.0, 98.0, 2.5, 30.0));
        // Partial last bucket (2 of 3 days).
        assert_eq!(ohlcv(&b[2]), (6.0, 12.0, 93.0, 7.5, 20.0));
        // A window starting inside the first bucket drops it.
        let f = parse_yahoo_chart(&body, "3d", 100, None, &win("1d", day(d0) + 1)).unwrap();
        assert_eq!(dates(&f), vec![day(d0 + 3), day(d0 + 6)]);
        // limit keeps the newest bars.
        let f = parse_yahoo_chart(&body, "3d", 1, None, &win("1d", 0)).unwrap();
        assert_eq!(dates(&f), vec![day(d0 + 6)]);
    }

    #[test]
    fn yahoo_futures_buckets_do_not_span_the_daily_break() {
        // ES=F-like: hourly bars 18:00..16:00 with a 17:00-18:00 break
        // (exchange time = UTC here), two sessions.
        let base = 1_700_006_400; // midnight UTC
        let s1 = base + 18 * H;
        let s2 = s1 + 24 * H;
        let ts: Vec<i64> = (0..23)
            .map(|i| s1 + i * H)
            .chain((0..23).map(|i| s2 + i * H))
            .collect();
        let body = ohlc_body(&ts, "ES=F", "FUTURE");
        let run = |tf| dates(&parse_yahoo_chart(&body, tf, 100, None, &win("1h", 0)).unwrap());
        assert_eq!(run("12h"), vec![s1, s1 + 12 * H, s2, s2 + 12 * H], "12h");
        assert_eq!(
            run("6h"),
            vec![
                s1,
                s1 + 6 * H,
                s1 + 12 * H,
                s1 + 18 * H,
                s2,
                s2 + 6 * H,
                s2 + 12 * H,
                s2 + 18 * H
            ]
        );
        let four: Vec<i64> = (0..6)
            .map(|k| s1 + 4 * k * H)
            .chain((0..6).map(|k| s2 + 4 * k * H))
            .collect();
        assert_eq!(run("4h"), four);
        // The 16:00 bucket holds only the 16:00 session's last 3 bars.
        let f = parse_yahoo_chart(&body, "4h", 100, None, &win("1h", 0)).unwrap();
        assert_eq!(f.data.bars[5].volume, 30.0);
    }

    #[test]
    fn snap_live_bar_merges_the_phantom_print_after_the_close() {
        let open = NOV4_OPEN;
        let close = open + 23_400; // 16:00 ET
        let mut bars = vec![
            bar_at(close - 120, 1.0),
            bar_at(close - 60, 2.0),
            bar_at(close, 3.0),
        ];
        bars[2].volume = 0.0;
        snap_live_bar(&mut bars, 60, Some(open), Some(close));
        assert_eq!(bars.len(), 2);
        assert_eq!(bar_ts(&bars[1]), Some(close - 60));
        assert_eq!(bars[1].close, 3.0);
        // Before the session open (next session's period): same.
        let mut bars = vec![bar_at(open - 120, 1.0), bar_at(open - 60, 2.0)];
        bars[1].volume = 0.0;
        snap_live_bar(&mut bars, 60, Some(open), Some(close));
        assert_eq!(bars.len(), 1);
        // A print with volume, or inside the session, is kept.
        let mut bars = vec![bar_at(close - 120, 1.0), bar_at(close, 3.0)];
        snap_live_bar(&mut bars, 60, Some(open), Some(close));
        assert_eq!(bars.len(), 2);
        let mut bars = vec![bar_at(open + 60, 1.0), bar_at(open + 120, 3.0)];
        bars[1].volume = 0.0;
        snap_live_bar(&mut bars, 60, Some(open), Some(close));
        assert_eq!(bars.len(), 2);
    }

    #[test]
    fn yahoo_first_session_is_kept_when_the_window_starts_before_the_open() {
        // Window starts at 08:30 ET on Nov 3, an hour before the open.
        let f = parse_yahoo_chart(&fixture(), "2h", 100, None, &win("1h", NOV3_OPEN - H)).unwrap();
        assert_eq!(dates(&f)[0], NOV3_OPEN);
        // 3m is a plain epoch grid: only the cut first bucket goes.
        let base = 1_700_006_400;
        let ts: Vec<i64> = (0..6).map(|i| base + 60 + i * 60).collect();
        let body = ohlc_body(&ts, "AAPL", "EQUITY");
        let f = parse_yahoo_chart(&body, "3m", 100, None, &win("1m", base + 60)).unwrap();
        assert_eq!(dates(&f), vec![base + 180, base + 360]);
    }

    #[test]
    fn yahoo_has_more_follows_lookback_not_count() {
        let now = 1_750_000_000;
        // 1m: 2000 bars exceed one 7-day window (~1950 session minutes), but
        // the window was not clamped by max age, so more history exists.
        let w = yahoo_window("1m", 2000, None, now);
        assert!(!w.age_clamped);
        let ts: Vec<i64> = (0..1950).rev().map(|i| now - 60 - i * 60).collect();
        let body = yahoo_body(&ts, "AAPL", "EQUITY", -18_000);
        let f = parse_yahoo_chart(&body, "1m", 2000, None, &w).unwrap();
        assert_eq!(f.data.len(), 1950);
        assert!(f.has_more);

        // Clamped by Yahoo's max age: nothing older is reachable.
        let w = yahoo_window("5m", 100_000, None, now);
        assert!(w.age_clamped);
        let f = parse_yahoo_chart(&body, "5m", 100_000, None, &w).unwrap();
        assert!(!f.has_more);

        // firstTradeDate inside the window: history starts there.
        let f = parse_yahoo_chart(&fixture(), "1h", 100, None, &win("1h", 0)).unwrap();
        assert!(!f.has_more);
        let f = parse_yahoo_chart(&fixture(), "1h", 100, None, &win("1h", NOV3_OPEN)).unwrap();
        assert!(f.has_more);
    }

    // --- Quotes (fixtures) ---

    const BINANCE_TICKERS: &str = include_str!("../tests/fixtures/binance_ticker_24hr.json");
    const AAPL_QUOTE: &str = include_str!("../tests/fixtures/yahoo_aapl_quote.json");

    #[test]
    fn binance_ticker_url_is_an_encoded_json_array() {
        assert_eq!(
            binance_ticker_url(&["BTCUSDT", "ETHUSDT"]),
            "https://api.binance.com/api/v3/ticker/24hr?symbols=%5B%22BTCUSDT%22,%22ETHUSDT%22%5D"
        );
    }

    #[test]
    fn binance_tickers_parse_in_requested_order() {
        let body: serde_json::Value = serde_json::from_str(BINANCE_TICKERS).unwrap();
        let q = parse_binance_tickers(&body, &["ETHUSDT", "BTCUSDT", "SOLUSDT"]);
        let eth = q[0].as_ref().unwrap();
        assert_eq!(eth.source, "binance");
        assert_eq!(eth.last, 2436.74);
        assert_eq!(eth.prev_close, 2288.98);
        let btc = q[1].as_ref().unwrap();
        assert_eq!(btc.last, 80689.85);
        // Rolling 24 h: the reference is openPrice, matching priceChange.
        assert_eq!(btc.prev_close, 77782.0);
        assert!((btc.last - btc.prev_close - 2907.85).abs() < 1e-6);
        assert_eq!(btc.high, Some(81196.01));
        assert_eq!(btc.low, Some(75736.38));
        assert_eq!(btc.volume, Some(21753.13912));
        assert!(q[2].as_ref().unwrap_err().contains("no ticker"));
    }

    #[test]
    fn binance_tickers_reject_non_array_and_bad_numbers() {
        let err = serde_json::json!({ "code": -1121, "msg": "Invalid symbol." });
        assert!(parse_binance_tickers(&err, &["BTCUSDT"])[0].is_err());
        let bad = serde_json::json!([{ "symbol": "BTCUSDT", "lastPrice": "x", "openPrice": "1" }]);
        assert!(parse_binance_tickers(&bad, &["BTCUSDT"])[0]
            .as_ref()
            .unwrap_err()
            .contains("lastPrice"));
    }

    #[test]
    fn yahoo_quote_uses_previous_session_close() {
        let body: serde_json::Value = serde_json::from_str(AAPL_QUOTE).unwrap();
        let q = parse_yahoo_quote(&body).unwrap();
        assert_eq!(q.source, "yahoo");
        assert_eq!(q.last, 349.51);
        assert_eq!(q.prev_close, 331.9);
        assert_eq!(q.high, Some(354.47));
        assert_eq!(q.low, Some(340.99));
        assert_eq!(q.volume, Some(88_958_176.0));
        // Matches Yahoo's own regularMarketChangePercent (5.306).
        let pct = (q.last - q.prev_close) / q.prev_close * 100.0;
        assert!((pct - 5.306).abs() < 1e-3);
    }

    #[test]
    fn yahoo_quote_reports_upstream_error() {
        let body = serde_json::json!({
            "chart": { "result": null, "error": { "code": "Not Found", "description": "No data found, symbol may be delisted" } }
        });
        assert_eq!(
            parse_yahoo_quote(&body).unwrap_err(),
            "Yahoo: No data found, symbol may be delisted"
        );
        let no_price = serde_json::json!({ "chart": { "result": [{ "meta": { "chartPreviousClose": 1.0 } }] } });
        assert!(parse_yahoo_quote(&no_price)
            .unwrap_err()
            .contains("regularMarketPrice"));
    }

    #[test]
    fn sample_quote_is_current_daily_bar_vs_previous() {
        let now = 1_793_800_000_123;
        let q = sample_quote("BTCUSDT", now).unwrap();
        let bars = data::sample_ohlcv("BTCUSDT", "1d", 2, None, now)
            .unwrap()
            .bars;
        assert_eq!(q.source, "sample");
        assert_eq!(q.prev_close, bars[0].close);
        assert_eq!(q.last, bars[1].close);
        assert_eq!(q.high, Some(bars[1].high));
        // Deterministic for a given instant, moves with the wall clock.
        assert_eq!(sample_quote("BTCUSDT", now).unwrap(), q);
        assert_ne!(sample_quote("BTCUSDT", now + 1_000).unwrap().last, q.last);
        assert_ne!(sample_quote("AAPL", now).unwrap().last, q.last);
    }

    #[test]
    fn fetch_quotes_keeps_order_and_isolates_failures() {
        let syms = vec!["AAPL".to_string(), "BTCUSDT".to_string()];
        let q = fetch_quotes(DataSource::Sample, &syms);
        assert_eq!(q.len(), 2);
        assert!(q.iter().all(|r| r.is_ok()));
        let q = fetch_quotes(DataSource::Unavailable, &syms);
        assert!(q.iter().all(|r| r.is_err()));
    }

    #[test]
    fn par_map_preserves_order_with_bounded_workers() {
        let items: Vec<String> = (0..40).map(|i| i.to_string()).collect();
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        let (running, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let out = par_map(&refs, |s| {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(2));
            running.fetch_sub(1, Ordering::SeqCst);
            s.parse::<usize>().unwrap()
        });
        assert_eq!(out, (0..40).collect::<Vec<_>>());
        assert!(peak.load(Ordering::SeqCst) <= QUOTE_CONCURRENCY);
        assert!(par_map(&[], |s| s.len()).is_empty());
    }

    // --- Quote cache, bad-symbol memory, Binance fan-out ---

    fn q(last: f64) -> Quote {
        Quote {
            last,
            prev_close: 1.0,
            high: None,
            low: None,
            volume: None,
            source: "binance",
        }
    }

    fn syms(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Fake upstreams that count calls and record which Binance symbols were
    /// requested. NOPEUSDT is unknown to "Binance"; quotes carry `now` so a
    /// refetch is visible.
    struct Fakes {
        binance_calls: Mutex<Vec<Vec<String>>>,
        yahoo_calls: AtomicUsize,
    }

    impl Fakes {
        fn new() -> Self {
            Fakes {
                binance_calls: Mutex::new(Vec::new()),
                yahoo_calls: AtomicUsize::new(0),
            }
        }

        fn run(
            &self,
            cache: &Mutex<QuoteCache>,
            symbols: &[String],
            now: i64,
        ) -> Vec<Result<Quote, String>> {
            cached_quotes(
                cache,
                symbols,
                now,
                |s: &[&str]| {
                    self.binance_calls
                        .lock()
                        .unwrap()
                        .push(s.iter().map(|x| x.to_string()).collect());
                    s.iter()
                        .map(|x| match *x {
                            "NOPEUSDT" => Err((true, "Binance: Invalid symbol.".to_string())),
                            _ => Ok(q(now as f64)),
                        })
                        .collect()
                },
                |_| {
                    self.yahoo_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(q(now as f64))
                },
            )
        }
    }

    #[test]
    fn quote_cache_serves_fresh_results_and_refetches_after_ttl() {
        let cache = Mutex::new(QuoteCache::default());
        let f = Fakes::new();
        let s = syms(&["BTCUSDT", "AAPL"]);
        let t0 = 1_000_000;

        let first = f.run(&cache, &s, t0);
        assert_eq!(first[0].as_ref().unwrap().last, t0 as f64);
        assert_eq!(first[1].as_ref().unwrap().last, t0 as f64);
        assert_eq!(f.binance_calls.lock().unwrap().len(), 1);
        assert_eq!(f.yahoo_calls.load(Ordering::SeqCst), 1);

        // Within the TTL: no upstream calls, same values.
        assert_eq!(f.run(&cache, &s, t0 + QUOTE_TTL_MS - 1), first);
        assert_eq!(f.binance_calls.lock().unwrap().len(), 1);
        assert_eq!(f.yahoo_calls.load(Ordering::SeqCst), 1);

        // At the TTL: refetched.
        let later = f.run(&cache, &s, t0 + QUOTE_TTL_MS);
        assert_eq!(later[0].as_ref().unwrap().last, (t0 + QUOTE_TTL_MS) as f64);
        assert_eq!(f.binance_calls.lock().unwrap().len(), 2);
        assert_eq!(f.yahoo_calls.load(Ordering::SeqCst), 2);

        // A cached symbol mixed with a new one only fetches the new one.
        f.run(
            &cache,
            &syms(&["BTCUSDT", "ETHUSDT"]),
            t0 + QUOTE_TTL_MS + 1,
        );
        assert_eq!(f.binance_calls.lock().unwrap()[2], vec!["ETHUSDT"]);
    }

    #[test]
    fn unknown_binance_symbol_is_remembered_and_kept_out_of_batches() {
        let cache = Mutex::new(QuoteCache::default());
        let f = Fakes::new();
        let s = syms(&["BTCUSDT", "NOPEUSDT"]);
        let t0 = 5_000_000;

        let r = f.run(&cache, &s, t0);
        assert!(r[0].is_ok());
        assert_eq!(r[1].as_ref().unwrap_err(), "Binance: Invalid symbol.");

        // Quotes expired, bad-symbol memory not: only BTCUSDT is requested.
        let r = f.run(&cache, &s, t0 + 60_000);
        assert_eq!(f.binance_calls.lock().unwrap()[1], vec!["BTCUSDT"]);
        assert_eq!(r[1].as_ref().unwrap_err(), "Binance: Invalid symbol.");

        // After the memory expires the symbol is tried again.
        f.run(&cache, &s, t0 + BAD_SYMBOL_TTL_MS);
        assert_eq!(
            f.binance_calls.lock().unwrap()[2],
            vec!["BTCUSDT", "NOPEUSDT"]
        );
    }

    #[test]
    fn binance_unknown_symbol_only_for_400_with_symbol_codes() {
        let code = |c: i64| serde_json::json!({ "code": c, "msg": "x" });
        assert!(binance_unknown_symbol(400, &code(-1121)));
        assert!(binance_unknown_symbol(400, &code(-1100)));
        assert!(!binance_unknown_symbol(400, &code(-1102)));
        assert!(!binance_unknown_symbol(429, &code(-1003)));
        assert!(!binance_unknown_symbol(418, &code(-1003)));
        assert!(!binance_unknown_symbol(400, &serde_json::json!({})));
    }

    #[test]
    fn binance_fans_out_per_symbol_only_when_a_symbol_is_unknown() {
        let body: serde_json::Value = serde_json::from_str(BINANCE_TICKERS).unwrap();
        let entry = |sym: &str| {
            serde_json::Value::Array(
                body.as_array()
                    .unwrap()
                    .iter()
                    .filter(|e| e["symbol"] == sym)
                    .cloned()
                    .collect(),
            )
        };

        // Rate-limited batch: one request, every symbol fails, no fan-out.
        let calls = AtomicUsize::new(0);
        let r = fetch_binance_quotes(&["BTCUSDT", "ETHUSDT"], |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err((false, "Binance API returned status 429".into()))
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(r.iter().all(|x| matches!(x, Err((false, _)))));

        // Unknown symbol in the batch: retried per symbol.
        let calls = AtomicUsize::new(0);
        let r = fetch_binance_quotes(&["BTCUSDT", "NOPEUSDT"], |url| {
            calls.fetch_add(1, Ordering::SeqCst);
            if url.contains("NOPEUSDT") {
                Err((true, "Binance: Invalid symbol.".into()))
            } else {
                Ok(entry("BTCUSDT"))
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(r[0].as_ref().unwrap().last, 80689.85);
        assert!(matches!(&r[1], Err((true, _))));
    }

    #[test]
    fn symbols_become_one_encoded_path_segment() {
        assert_eq!(
            yahoo_quote_url("AAPL").unwrap(),
            "https://query1.finance.yahoo.com/v8/finance/chart/AAPL?range=1d&interval=1d"
        );
        assert!(yahoo_quote_url("^GSPC")
            .unwrap()
            .contains("/chart/%5EGSPC?"));
        assert!(yahoo_quote_url("EURUSD=X")
            .unwrap()
            .contains("/chart/EURUSD%3DX?"));
        // Slashes and query characters cannot leave the segment.
        assert!(yahoo_quote_url("../../X")
            .unwrap()
            .contains("/chart/..%2F..%2FX?"));
        assert!(yahoo_quote_url("A?b#c")
            .unwrap()
            .contains("/chart/A%3Fb%23c?"));
        for dots in ["", ".", ".."] {
            assert!(yahoo_quote_url(dots).is_err(), "{dots:?}");
        }
        let w = yahoo_window("1d", 10, None, 1_750_000_000);
        assert!(yahoo_chart_url("../v7/x", &w)
            .unwrap()
            .contains("/chart/..%2Fv7%2Fx?"));
        assert!(yahoo_chart_url("..", &w).is_err());
        assert!(binance_ticker_url(&["A\"B"]).contains("%22A%22B%22"));
    }

    fn daily_dates(
        symbol: &str,
        gmtoffset: i64,
        open_tod: i64,
        days: &[i64],
        end: Option<i64>,
    ) -> Vec<i64> {
        // Session-open stamps on the exchange-local days `days` (days since epoch).
        let ts: Vec<i64> = days
            .iter()
            .map(|d| d * DAY + open_tod - gmtoffset)
            .collect();
        let body = yahoo_body(&ts, symbol, "EQUITY", gmtoffset);
        let f = parse_yahoo_chart(&body, "1d", 100, end, &win("1d", 0)).unwrap();
        dates(&f)
    }

    #[test]
    fn yahoo_date_bars_are_stamped_at_the_exchange_local_date() {
        let days = [20_000, 20_001, 20_002];
        let want: Vec<i64> = days.iter().map(|d| d * DAY).collect();
        // ASX in AEDT (+11h) opens 10:00 local = 23:00 UTC the day before.
        assert_eq!(
            daily_dates("BHP.AX", 11 * 3_600, 10 * 3_600, &days, None),
            want
        );
        // NZX in NZDT (+13h) opens 10:00 local = 21:00 UTC the day before.
        assert_eq!(
            daily_dates("AIR.NZ", 13 * 3_600, 10 * 3_600, &days, None),
            want
        );
        // NYSE (EST, -5h) opens 09:30 local = 14:30 UTC the same day.
        assert_eq!(
            daily_dates("AAPL", -5 * 3_600, 9 * 3_600 + 1_800, &days, None),
            want
        );
    }

    #[test]
    fn yahoo_date_bar_history_pages_do_not_repeat_the_boundary_bar() {
        let days = [20_000, 20_001, 20_002];
        // The client pages with the oldest bar's stamp as end_time.
        let end = Some(20_002 * DAY * 1000);
        assert_eq!(
            daily_dates("BHP.AX", 11 * 3_600, 10 * 3_600, &days, end),
            vec![20_000 * DAY, 20_001 * DAY]
        );
        assert_eq!(
            daily_dates("AAPL", -5 * 3_600, 9 * 3_600 + 1_800, &days, end),
            vec![20_000 * DAY, 20_001 * DAY]
        );
    }

    #[test]
    fn date_intervals() {
        let is_date = |i| Interval::parse(i).is_ok_and(|iv| iv.is_date_bar());
        for i in [
            "1d", "2D", "3d", "5d", "1wk", "1w", "2W", "1mo", "1M", "24h",
        ] {
            assert!(is_date(i), "{i}");
        }
        for i in ["1m", "7m", "15m", "1h", "4h", "36h", "", "d"] {
            assert!(!is_date(i), "{i}");
        }
    }
}
