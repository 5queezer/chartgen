use crate::data::{Bar, OhlcvData};
use crate::indicator::{rgba, Bars as BarsSeries, Dot, Fill, HLine, Indicator, Line, PanelResult};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How often one futures series is asked for. A push group recomputes up to 4 times
/// a second, and these endpoints have limits of their own that the weight header does
/// not show (fundingRate: 500 requests per 5 minutes per IP).
const SERIES_REFRESH: Duration = Duration::from_secs(15);
/// How long a series is served after its last good read, when a call fails or the gate
/// refuses it, so the panel keeps its shape and the chart does not reload. Longer than
/// the gate's default 60 s cooldown plus one `SERIES_REFRESH`.
const SERIES_KEEP: Duration = Duration::from_secs(90);

/// How long a caller with nothing to serve waits for a read already in flight: as long
/// as that read's HTTP timeout. Chart requests and pushes run on the blocking pool;
/// `--trade`'s engine computes on its task, which these HTTP calls already block.
const SERIES_WAIT: Duration = Duration::from_secs(15);

struct HeldSeries<T> {
    /// When the current refresh was claimed, before its fetch.
    tried: Instant,
    good: Option<(Instant, Arc<Vec<T>>)>,
    error: String,
    fetching: bool,
}

type SeriesKey = (String, String);

/// Futures series by symbol and period.
struct SeriesCache<T> {
    held: Mutex<HashMap<SeriesKey, HeldSeries<T>>>,
    fetched: Condvar,
}

impl<T> Default for SeriesCache<T> {
    fn default() -> Self {
        Self {
            held: Mutex::default(),
            fetched: Condvar::new(),
        }
    }
}

/// The turn to refresh a series. Ends however the fetch does, a panic included, so
/// nobody waits on it for longer than it runs.
struct SeriesClaim<'a, T> {
    cache: &'a SeriesCache<T>,
    key: SeriesKey,
}

impl<T> Drop for SeriesClaim<'_, T> {
    fn drop(&mut self) {
        if let Some(h) = self.cache.lock().get_mut(&self.key) {
            h.fetching = false;
        }
        self.cache.fetched.notify_all();
    }
}

impl<T> SeriesCache<T> {
    fn lock(&self) -> MutexGuard<'_, HashMap<SeriesKey, HeldSeries<T>>> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The series for `symbol` and `period`: `fetch`ed at most once per
    /// `SERIES_REFRESH`, whether that worked or not, and otherwise the last good one
    /// while it lasts.
    fn get(
        &self,
        symbol: &str,
        period: &str,
        now: Instant,
        fetch: impl FnOnce() -> Result<Vec<T>, String>,
    ) -> Result<Arc<Vec<T>>, String> {
        self.get_waiting(symbol, period, now, SERIES_WAIT, fetch)
    }

    /// `get`, with its wait as an argument. The refresh is claimed under the lock
    /// before the fetch, so of the callers at an expiry only one sends a request. The
    /// others get the last good series at once. With none held, as on a first read,
    /// they wait up to `wait` for that request, so a chart loading at the same moment
    /// is not left with a blank panel, and a failed read gives them its error.
    fn get_waiting(
        &self,
        symbol: &str,
        period: &str,
        now: Instant,
        wait: Duration,
        fetch: impl FnOnce() -> Result<Vec<T>, String>,
    ) -> Result<Arc<Vec<T>>, String> {
        self.get_extending(symbol, period, now, wait, |_| fetch())
    }

    /// `get_waiting`, where `fetch` is given the series it replaces (the last good
    /// one, however old) so it can read only what is new.
    fn get_extending(
        &self,
        symbol: &str,
        period: &str,
        now: Instant,
        wait: Duration,
        fetch: impl FnOnce(Option<&[T]>) -> Result<Vec<T>, String>,
    ) -> Result<Arc<Vec<T>>, String> {
        let key = (symbol.to_uppercase(), period.to_string());
        let lasting = |held: &HeldSeries<T>| {
            held.good
                .as_ref()
                .filter(|(at, _)| now.saturating_duration_since(*at) < SERIES_KEEP)
                .map(|(_, series)| series.clone())
        };
        let (claim, before) = {
            let mut held = self.lock();
            held.retain(|_, h| now.saturating_duration_since(h.tried) < SERIES_KEEP);
            let give_up = Instant::now() + wait;
            // A claim holds for one refresh, so a fetch that hangs cannot stick.
            while let Some(h) = held
                .get(&key)
                .filter(|h| now.saturating_duration_since(h.tried) < SERIES_REFRESH)
            {
                if let Some(series) = lasting(h) {
                    return Ok(series);
                }
                if !h.fetching {
                    return Err(h.error.clone());
                }
                let left = give_up.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(SERIES_LOADING.into());
                }
                held = self
                    .fetched
                    .wait_timeout(held, left)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            let entry = held.entry(key.clone()).or_insert_with(|| HeldSeries {
                tried: now,
                good: None,
                error: SERIES_LOADING.into(),
                fetching: false,
            });
            entry.tried = now;
            entry.fetching = true;
            let before = entry.good.as_ref().map(|(_, series)| series.clone());
            (
                SeriesClaim {
                    cache: self,
                    key: key.clone(),
                },
                before,
            )
        };
        let result = fetch(before.as_deref().map(Vec::as_slice));
        let mut held = self.lock();
        let entry = held.entry(key).or_insert_with(|| HeldSeries {
            tried: now,
            good: None,
            error: SERIES_LOADING.into(),
            fetching: true,
        });
        let answer = match result {
            Ok(series) => {
                let series = Arc::new(series);
                // A fetch that outlasted its refresh must not replace a newer one.
                if entry.good.as_ref().is_none_or(|(at, _)| *at <= now) {
                    entry.good = Some((now, series.clone()));
                }
                Ok(series)
            }
            Err(e) => {
                entry.error = e.clone();
                lasting(entry).ok_or(e)
            }
        };
        drop(held);
        drop(claim);
        answer
    }
}

/// Says a series has not been read yet, or a read is taking too long.
const SERIES_LOADING: &str = "futures data is not loaded yet; try again shortly";

static FUNDING: LazyLock<SeriesCache<(i64, f64)>> = LazyLock::new(SeriesCache::default);
static OPEN_INTEREST: LazyLock<SeriesCache<(i64, f64)>> = LazyLock::new(SeriesCache::default);
static LONG_SHORT: LazyLock<SeriesCache<(i64, f64, f64)>> = LazyLock::new(SeriesCache::default);

fn http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap()
}

// ---- Helpers ----

/// Align external time series to bar timestamps.
/// `external`: Vec<(timestamp_ms, value)> sorted by time.
/// Bar dates are epoch seconds as strings.
/// `max_gap_ms`: maximum allowed gap between bar and data point.
fn align_to_bars(external: &[(i64, f64)], bars: &[Bar], max_gap_ms: i64) -> Vec<f64> {
    let n = bars.len();
    let mut out = vec![f64::NAN; n];
    if external.is_empty() {
        return out;
    }

    let mut ext_idx = 0;
    for (i, bar) in bars.iter().enumerate() {
        let bar_ms = bar.date.parse::<i64>().unwrap_or(0) * 1000;
        while ext_idx + 1 < external.len()
            && (external[ext_idx + 1].0 - bar_ms).abs() <= (external[ext_idx].0 - bar_ms).abs()
        {
            ext_idx += 1;
        }
        let gap = (external[ext_idx].0 - bar_ms).abs();
        if gap <= max_gap_ms {
            out[i] = external[ext_idx].1;
        }
    }
    out
}

/// The Binance futures data period for a chart interval: the longest that
/// fits in one bar.
fn map_interval(interval: &str) -> &str {
    const PERIODS: [(i64, &str); 9] = [
        (86_400, "1d"),
        (43_200, "12h"),
        (21_600, "6h"),
        (14_400, "4h"),
        (7_200, "2h"),
        (3_600, "1h"),
        (1_800, "30m"),
        (900, "15m"),
        (300, "5m"),
    ];
    let Some(secs) = crate::data::interval_secs(interval) else {
        return "4h";
    };
    PERIODS
        .iter()
        .find(|(p, _)| *p <= secs)
        .map_or("5m", |(_, name)| name)
}

fn is_crypto_symbol(sym: &str) -> bool {
    let s = sym.to_uppercase();
    let quotes = ["USDT", "BUSD", "BTC", "ETH", "BNB", "USDC", "FDUSD"];
    quotes.iter().any(|q| s.ends_with(q) && s.len() > q.len())
}

// ---- CVD ----

pub struct Cvd;

impl Indicator for Cvd {
    fn name(&self) -> &str {
        "cvd"
    }

    fn description(&self) -> &str {
        "Cumulative Volume Delta — buy vs sell pressure"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut cvd = vec![f64::NAN; n];
        let mut cumulative = 0.0;

        for (i, bar) in data.bars.iter().enumerate() {
            let range = bar.high - bar.low;
            let delta = if range > 0.0 {
                let buy_vol = bar.volume * (bar.close - bar.low) / range;
                let sell_vol = bar.volume * (bar.high - bar.close) / range;
                buy_vol - sell_vol
            } else {
                0.0
            };
            cumulative += delta;
            cvd[i] = cumulative;
        }

        PanelResult {
            lines: vec![Line {
                breaks: false,
                y: cvd,
                color: rgba(0x2196F3, 1.0),
                width: 1,
                label: Some("CVD".into()),
            }],
            hlines: vec![HLine {
                y: 0.0,
                color: rgba(0x888888, 0.5),
            }],
            label: "CVD".into(),
            is_overlay: false,
            ..Default::default()
        }
    }
}

// ---- Futures series: windows, paging, missing data ----

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Rows Binance returns per request: its cap for `fundingRate` without a `startTime`
/// (it ignores a larger `limit`, measured) and its maximum for the data endpoints.
const PAGE_ROWS: usize = 500;
/// Requests to read the history of one funding window the first time. At 8 hours a
/// row that is about 2.7 years; the older bars stay empty.
const FUNDING_MAX_PAGES: usize = 6;
/// The same for open interest and the long/short ratio. At a 5 minute period it is
/// 4000 rows, about 14 days; Binance keeps only 30 (`RETENTION_MS`).
const HISTORY_MAX_PAGES: usize = 8;
/// How far back Binance keeps `openInterestHist` and the long/short ratio. A request
/// reaching further, by `startTime`, is refused with code -1130.
const RETENTION_MS: i64 = 30 * DAY_MS;
/// Kept inside the retention, so a row that ages out while the request is on its way
/// is not the reason it fails.
const RETENTION_MARGIN_MS: i64 = 10 * 60 * 1000;
/// Funding is paid every 8 hours (some symbols every 4 or 1); a bar is matched to the
/// nearest payment within this.
const FUNDING_GAP_MS: i64 = 8 * HOUR_MS;

/// A row of a futures series: something at a time.
trait Row: Clone {
    fn at(&self) -> i64;
}

impl Row for (i64, f64) {
    fn at(&self) -> i64 {
        self.0
    }
}

impl Row for (i64, f64, f64) {
    fn at(&self) -> i64 {
        self.0
    }
}

/// Why a panel has no values. Said in the label, so the legend tells the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Missing {
    /// A stock, index or FX symbol: these series exist for Binance futures only.
    NotCrypto,
    /// Binance answered with no rows at all: a spot-only pair, or a futures market
    /// that did not exist yet in this window.
    NoMarket,
    /// The request failed or the gate refused it. The reason is in the log.
    Unavailable,
    /// Every bar is older than Binance keeps this series.
    Retention,
}

impl Missing {
    fn note(self) -> &'static str {
        match self {
            Missing::NotCrypto => "Binance futures only",
            Missing::NoMarket => "no futures market",
            Missing::Unavailable => "unavailable",
            Missing::Retention => "last 30 days only",
        }
    }
}

/// The panel label: the name, and the reason when there are no values.
fn label_for(name: &str, why: Option<Missing>) -> String {
    match why {
        Some(m) => format!("{name} ({})", m.note()),
        None => name.to_string(),
    }
}

/// The span of time a panel's bars need from a futures series.
struct Window {
    /// The first bar's open, less the gap alignment allows.
    from: i64,
    /// The last bar's open, plus that gap.
    to: i64,
    /// `from` rounded down, so a chart that gains a bar keeps its series.
    bucket: i64,
    /// The newest bar is still current, so the series has to keep up with it.
    live: bool,
}

impl Window {
    fn of(bars: &[Bar], gap_ms: i64, bucket_ms: i64, now: i64) -> Option<Window> {
        let ms = |b: &Bar| b.date.parse::<i64>().ok().map(|s| s * 1000);
        let from = ms(bars.first()?)? - gap_ms;
        let to = ms(bars.last()?)? + gap_ms;
        Some(Window {
            from,
            to,
            bucket: from.div_euclid(bucket_ms) * bucket_ms,
            live: to >= now,
        })
    }

    /// The cache key's period part. A window of older bars is its own series, so a
    /// page read while scrolling back is never served as the newest, and never
    /// replaces it.
    fn key(&self, period: &str) -> String {
        let end = if self.live { 'L' } else { 'H' };
        format!("{period}@{}{end}", self.bucket)
    }
}

/// One request: rows from `start` on, and up to `end`, both inclusive.
struct Query {
    start: Option<i64>,
    end: Option<i64>,
}

/// The rows a window needs, `page` being one request to Binance.
///
/// Binance returns the *latest* rows up to `end` (500 at most), so the history is
/// read newest page first, each one ending before the oldest row of the last, until
/// a page reaches `from` or `floor`, comes back empty (no older rows exist), or
/// `max_pages` is spent. With `prev`, the series held from the last read, only the
/// rows after its last one are asked for: one request, whatever the window.
fn collect<T: Row>(
    prev: Option<&[T]>,
    w: &Window,
    now: i64,
    floor: i64,
    max_pages: usize,
    page: &mut dyn FnMut(Query) -> Result<Vec<T>, String>,
) -> Result<Vec<T>, String> {
    // A window of older bars is read up to its own end, not to now.
    let end = (w.to < now).then_some(w.to);
    if let Some(prev) = prev.filter(|p| !p.is_empty()) {
        let last = prev[prev.len() - 1].at();
        if last >= w.to {
            return Ok(prev.to_vec());
        }
        let mut new = page(Query {
            start: Some(last + 1),
            end,
        })?;
        new.retain(|r| r.at() > last);
        new.sort_by_key(Row::at);
        let mut out = prev.to_vec();
        out.extend(new);
        return Ok(out);
    }
    let (mut chunks, mut end) = (Vec::new(), end);
    for _ in 0..max_pages {
        let mut rows = page(Query { start: None, end })?;
        rows.sort_by_key(Row::at);
        let Some(oldest) = rows.first().map(Row::at) else {
            break;
        };
        chunks.push(rows);
        if oldest <= w.from.max(floor) {
            break;
        }
        end = Some(oldest - 1);
    }
    let mut out: Vec<T> = chunks.into_iter().rev().flatten().collect();
    out.dedup_by_key(|r| r.at());
    Ok(out)
}

/// One request to a Binance futures endpoint, as its rows. Goes through the gate.
fn fetch_page(url: &str, q: &Query) -> Result<Vec<serde_json::Value>, String> {
    let mut url = format!("{url}&limit={PAGE_ROWS}");
    if let Some(start) = q.start {
        url += &format!("&startTime={start}");
    }
    if let Some(end) = q.end {
        url += &format!("&endTime={end}");
    }
    let gate = crate::binance_gate::futures();
    let sent = gate.admit()?;
    let resp = http_client()
        .get(&url)
        .send()
        .map_err(|e| format!("request failed: {e}"))?;
    gate.observe_response(sent, resp.status().as_u16(), resp.headers());
    if !resp.status().is_success() {
        return Err(format!("Binance returned status {}", resp.status()));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| format!("could not parse the response: {e}"))?;
    match body {
        serde_json::Value::Array(rows) => Ok(rows),
        _ => Err("the response is not an array".into()),
    }
}

/// A number Binance sends as a string.
fn number(row: &serde_json::Value, key: &str) -> Option<f64> {
    row.get(key)?.as_str()?.parse().ok()
}

/// `CHARTGEN_DATA_SOURCE=sample`: the futures series come from `sample_times`, not
/// from Binance, as the candles do. For E2E tests and demos.
fn sampling() -> bool {
    std::env::var("CHARTGEN_DATA_SOURCE").is_ok_and(|v| v == "sample")
}

/// The times of the rows Binance would return for `q` on a grid of `step_ms`: the
/// latest `PAGE_ROWS` up to now, and for the history series (`kept` is how far back)
/// no further back than Binance keeps them. A symbol ending in `FDUSD` has no futures
/// market (no rows), and one ending in `ERRUSDT` fails.
fn sample_times(
    symbol: &str,
    step_ms: i64,
    kept: Option<i64>,
    q: &Query,
) -> Result<Vec<i64>, String> {
    let sym = symbol.to_uppercase();
    if sym.ends_with("ERRUSDT") {
        return Err("sample source: this symbol always fails".into());
    }
    if sym.ends_with("FDUSD") {
        return Ok(vec![]);
    }
    let now = now_ms();
    let last = q.end.map_or(now, |e| e.min(now)).div_euclid(step_ms);
    let first = [q.start, kept.map(|k| now - k)]
        .into_iter()
        .flatten()
        .max()
        .map_or(i64::MIN, |t| t.div_euclid(step_ms) + 1);
    let first = first.max(last - PAGE_ROWS as i64 + 1);
    Ok((first..=last).map(|i| i * step_ms).collect())
}

/// A deterministic wave in `0..1` for a row time, so a sample chart has a shape.
fn sample_wave(ts: i64, step_ms: i64) -> f64 {
    ((ts / step_ms) as f64 * 0.35).sin() * 0.5 + 0.5
}

fn funding_page(symbol: &str, q: &Query) -> Result<Vec<(i64, f64)>, String> {
    if sampling() {
        let step = FUNDING_GAP_MS;
        return Ok(sample_times(symbol, step, None, q)?
            .into_iter()
            .map(|t| (t, (sample_wave(t, step) - 0.4) * 0.0004))
            .collect());
    }
    let url = format!(
        "https://fapi.binance.com/fapi/v1/fundingRate?symbol={}",
        symbol.to_uppercase()
    );
    Ok(fetch_page(&url, q)?
        .iter()
        .filter_map(|r| Some((r.get("fundingTime")?.as_i64()?, number(r, "fundingRate")?)))
        .collect())
}

fn period_ms(period: &str) -> i64 {
    crate::data::interval_secs(period).unwrap_or(300) * 1000
}

fn oi_page(symbol: &str, period: &str, q: &Query) -> Result<Vec<(i64, f64)>, String> {
    if sampling() {
        let step = period_ms(period);
        return Ok(sample_times(symbol, step, Some(RETENTION_MS), q)?
            .into_iter()
            .map(|t| (t, 2.0e9 + sample_wave(t, step) * 5.0e8))
            .collect());
    }
    let url = format!(
        "https://fapi.binance.com/futures/data/openInterestHist?symbol={}&period={period}",
        symbol.to_uppercase()
    );
    Ok(fetch_page(&url, q)?
        .iter()
        .filter_map(|r| {
            Some((
                r.get("timestamp")?.as_i64()?,
                number(r, "sumOpenInterestValue")?,
            ))
        })
        .collect())
}

fn ls_page(symbol: &str, period: &str, q: &Query) -> Result<Vec<(i64, f64, f64)>, String> {
    if sampling() {
        let step = period_ms(period);
        return Ok(sample_times(symbol, step, Some(RETENTION_MS), q)?
            .into_iter()
            .map(|t| {
                let long = 0.4 + sample_wave(t, step) * 0.2;
                (t, long, 1.0 - long)
            })
            .collect());
    }
    let url = format!(
        "https://fapi.binance.com/futures/data/globalLongShortAccountRatio?symbol={}&period={period}",
        symbol.to_uppercase()
    );
    Ok(fetch_page(&url, q)?
        .iter()
        .filter_map(|r| {
            Some((
                r.get("timestamp")?.as_i64()?,
                number(r, "longAccount")?,
                number(r, "shortAccount")?,
            ))
        })
        .collect())
}

/// The cached series for a window, `fetch`ed when it is due. A failure is logged here,
/// once per refresh (the cache calls `fetch` once per refresh), so the log explains a
/// panel that says "unavailable".
fn read<T: Row>(
    cache: &SeriesCache<T>,
    what: &str,
    symbol: &str,
    key: &str,
    fetch: impl FnOnce(Option<&[T]>) -> Result<Vec<T>, String>,
) -> Result<Arc<Vec<T>>, Missing> {
    let logged = |prev: Option<&[T]>| {
        fetch(prev).inspect_err(|e| eprintln!("[futures] {what} {symbol}: {e}"))
    };
    match cache.get_extending(symbol, key, Instant::now(), SERIES_WAIT, logged) {
        Ok(series) if series.is_empty() => Err(Missing::NoMarket),
        Ok(series) => Ok(series),
        Err(_) => Err(Missing::Unavailable),
    }
}

/// The symbol, when it can have futures data at all.
fn futures_symbol(data: &OhlcvData) -> Result<&str, Missing> {
    data.symbol
        .as_deref()
        .filter(|s| is_crypto_symbol(s))
        .ok_or(Missing::NotCrypto)
}

/// A window of a history series (open interest, long/short ratio) and its period.
/// `Err(Retention)` when all of it is older than Binance keeps.
fn history_window(data: &OhlcvData, now: i64) -> Result<(Window, &str), Missing> {
    let interval = data.interval.as_deref().unwrap_or("4h");
    let period = map_interval(interval);
    let gap = interval_to_max_gap_ms(interval);
    let Some(w) = Window::of(&data.bars, gap, 64 * period_ms(period), now) else {
        return Err(Missing::NoMarket);
    };
    if w.to < now - RETENTION_MS {
        return Err(Missing::Retention);
    }
    Ok((w, period))
}

// ---- Funding Rate ----

pub struct FundingRate;

fn funding_window(data: &OhlcvData, now: i64) -> Option<Window> {
    Window::of(&data.bars, FUNDING_GAP_MS, DAY_MS, now)
}

fn funding_values(data: &OhlcvData) -> Result<Vec<f64>, Missing> {
    let sym = futures_symbol(data)?;
    let now = now_ms();
    let Some(w) = funding_window(data, now) else {
        return Ok(vec![f64::NAN; data.len()]);
    };
    let series = read(&FUNDING, "funding rate", sym, &w.key(""), |prev| {
        let mut page = |q: Query| funding_page(sym, &q);
        collect(prev, &w, now, i64::MIN, FUNDING_MAX_PAGES, &mut page)
    })?;
    Ok(align_to_bars(&series, &data.bars, FUNDING_GAP_MS))
}

impl Indicator for FundingRate {
    fn name(&self) -> &str {
        "funding_rate"
    }

    fn description(&self) -> &str {
        "Funding Rate — crypto perpetual futures funding (Binance futures)"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let (aligned, why) = match funding_values(data) {
            Ok(v) => (v, None),
            Err(m) => (vec![f64::NAN; data.len()], Some(m)),
        };

        // A bar with no funding stays empty (NaN), not zero: zero is a rate.
        let mut pct = Vec::with_capacity(aligned.len());
        let mut colors = Vec::with_capacity(aligned.len());
        for &v in &aligned {
            pct.push(v * 100.0);
            colors.push(match v {
                v if v.is_nan() => rgba(0x888888, 0.7),
                v if v >= 0.0 => rgba(0x26A69A, 0.7),
                _ => rgba(0xEF5350, 0.7),
            });
        }

        PanelResult {
            bars: vec![BarsSeries {
                y: pct,
                colors,
                bottom: 0.0,
            }],
            hlines: vec![HLine {
                y: 0.0,
                color: rgba(0x888888, 0.5),
            }],
            label: label_for("Funding", why),
            is_overlay: false,
            ..Default::default()
        }
    }
}

// ---- Open Interest ----

pub struct OpenInterestHist;

fn oi_values(data: &OhlcvData) -> Result<Vec<f64>, Missing> {
    let sym = futures_symbol(data)?;
    let now = now_ms();
    let (w, period) = history_window(data, now)?;
    let floor = now - RETENTION_MS + RETENTION_MARGIN_MS;
    let series = read(
        &OPEN_INTEREST,
        "open interest",
        sym,
        &w.key(period),
        |prev| {
            let mut page = |q: Query| oi_page(sym, period, &q);
            collect(prev, &w, now, floor, HISTORY_MAX_PAGES, &mut page)
        },
    )?;
    let gap = interval_to_max_gap_ms(data.interval.as_deref().unwrap_or("4h"));
    Ok(align_to_bars(&series, &data.bars, gap))
}

impl Indicator for OpenInterestHist {
    fn name(&self) -> &str {
        "open_interest"
    }

    fn description(&self) -> &str {
        "Open Interest — total outstanding futures contracts (Binance futures, last 30 days only)"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let (aligned, why) = match oi_values(data) {
            Ok(v) => (v, None),
            Err(m) => (vec![f64::NAN; data.len()], Some(m)),
        };
        let zeros = vec![0.0; aligned.len()];

        PanelResult {
            lines: vec![Line {
                breaks: false,
                y: aligned.clone(),
                color: rgba(0x2196F3, 1.0),
                width: 1,
                label: Some("OI".into()),
            }],
            fills: vec![Fill {
                y1: aligned,
                y2: zeros,
                color: rgba(0x2196F3, 0.15),
                color_below: None,
            }],
            label: label_for("OI", why),
            is_overlay: false,
            ..Default::default()
        }
    }
}

// ---- Long/Short Ratio ----

pub struct LongShortRatio;

/// The long and the short share per bar.
fn ls_values(data: &OhlcvData) -> Result<(Vec<f64>, Vec<f64>), Missing> {
    let sym = futures_symbol(data)?;
    let now = now_ms();
    let (w, period) = history_window(data, now)?;
    let floor = now - RETENTION_MS + RETENTION_MARGIN_MS;
    let series = read(
        &LONG_SHORT,
        "long/short ratio",
        sym,
        &w.key(period),
        |prev| {
            let mut page = |q: Query| ls_page(sym, period, &q);
            collect(prev, &w, now, floor, HISTORY_MAX_PAGES, &mut page)
        },
    )?;
    let long: Vec<(i64, f64)> = series.iter().map(|&(ts, l, _)| (ts, l)).collect();
    let short: Vec<(i64, f64)> = series.iter().map(|&(ts, _, s)| (ts, s)).collect();
    let gap = interval_to_max_gap_ms(data.interval.as_deref().unwrap_or("4h"));
    Ok((
        align_to_bars(&long, &data.bars, gap),
        align_to_bars(&short, &data.bars, gap),
    ))
}

impl Indicator for LongShortRatio {
    fn name(&self) -> &str {
        "long_short"
    }

    fn description(&self) -> &str {
        "Long/Short Ratio — account positioning (Binance futures, last 30 days only)"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let (long_aligned, short_aligned, why) = match ls_values(data) {
            Ok((l, s)) => (l, s, None),
            Err(m) => (
                vec![f64::NAN; data.len()],
                vec![f64::NAN; data.len()],
                Some(m),
            ),
        };

        PanelResult {
            lines: vec![
                Line {
                    breaks: false,
                    y: long_aligned.clone(),
                    color: rgba(0x26A69A, 1.0),
                    width: 1,
                    label: Some("Long".into()),
                },
                Line {
                    breaks: false,
                    y: short_aligned.clone(),
                    color: rgba(0xEF5350, 1.0),
                    width: 1,
                    label: Some("Short".into()),
                },
            ],
            fills: vec![Fill {
                y1: long_aligned,
                y2: short_aligned,
                color: rgba(0x26A69A, 0.1),
                color_below: None,
            }],
            hlines: vec![HLine {
                y: 0.5,
                color: rgba(0x888888, 0.5),
            }],
            y_range: Some((0.0, 1.0)),
            label: label_for("L/S", why),
            is_overlay: false,
            ..Default::default()
        }
    }
}

// ---- Fear & Greed Index ----

pub struct FearGreed;

fn fetch_fear_greed_data() -> Result<Vec<(i64, f64)>, String> {
    let client = http_client();
    let url = "https://api.alternative.me/fng/?limit=365&format=json";
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("F&G request failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("F&G API returned status {}", resp.status()));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| format!("Failed to parse F&G response: {}", e))?;
    let arr = body
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or("F&G response missing data array")?;

    let mut result = Vec::with_capacity(arr.len());
    for entry in arr {
        let ts: i64 = entry
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .parse()
            .unwrap_or(0)
            * 1000; // seconds -> ms
        let val: f64 = entry
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .parse()
            .unwrap_or(0.0);
        result.push((ts, val));
    }
    result.sort_by_key(|&(ts, _)| ts);
    Ok(result)
}

fn fng_color(value: f64) -> plotters::style::RGBAColor {
    if value < 25.0 {
        rgba(0xEF5350, 1.0) // Extreme Fear
    } else if value < 45.0 {
        rgba(0xFF9800, 1.0) // Fear
    } else if value < 55.0 {
        rgba(0xFFEB3B, 1.0) // Neutral
    } else if value < 75.0 {
        rgba(0x66BB6A, 1.0) // Greed
    } else {
        rgba(0x26A69A, 1.0) // Extreme Greed
    }
}

impl Indicator for FearGreed {
    fn name(&self) -> &str {
        "fear_greed"
    }

    fn description(&self) -> &str {
        "Fear & Greed Index — market sentiment (alternative.me)"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let series = match fetch_fear_greed_data() {
            Ok(s) => s,
            Err(_) => {
                return PanelResult {
                    label: "F&G (no data)".into(),
                    ..Default::default()
                }
            }
        };

        // Daily data, max gap 24h
        let max_gap_ms = 24 * 60 * 60 * 1000;
        let aligned = align_to_bars(&series, &data.bars, max_gap_ms);

        let mut dots = Vec::new();
        for (i, &v) in aligned.iter().enumerate() {
            if !v.is_nan() {
                dots.push(Dot {
                    x: i,
                    y: v,
                    color: fng_color(v),
                    size: 2,
                    label: None,
                });
            }
        }

        PanelResult {
            lines: vec![Line {
                breaks: false,
                y: aligned,
                color: rgba(0x888888, 0.5),
                width: 1,
                label: Some("F&G".into()),
            }],
            dots,
            hlines: vec![
                HLine {
                    y: 25.0,
                    color: rgba(0x888888, 0.3),
                },
                HLine {
                    y: 50.0,
                    color: rgba(0x888888, 0.3),
                },
                HLine {
                    y: 75.0,
                    color: rgba(0x888888, 0.3),
                },
            ],
            y_range: Some((0.0, 100.0)),
            label: "F&G".into(),
            is_overlay: false,
            ..Default::default()
        }
    }
}

// ---- Interval to max gap helper ----

fn interval_to_max_gap_ms(interval: &str) -> i64 {
    let base_ms = crate::data::interval_secs(interval).unwrap_or(14_400) * 1000;
    // Allow 2x the interval as max gap
    base_ms * 2
}

#[cfg(test)]
mod interval_tests {
    use super::*;

    /// A cache read that counts its fetches.
    fn read(
        cache: &SeriesCache<i64>,
        calls: &mut usize,
        at: Instant,
        answer: Result<Vec<i64>, String>,
    ) -> Result<Vec<i64>, String> {
        cache
            .get("btcusdt", "1h", at, || {
                *calls += 1;
                answer
            })
            .map(|s| s.to_vec())
    }

    #[test]
    fn a_series_is_asked_for_once_per_refresh_however_often_it_is_recomputed() {
        let (cache, t0) = (SeriesCache::default(), Instant::now());
        let mut calls = 0;
        // Four recomputes a second, for most of the refresh: one call.
        for tick in 0..50 {
            let at = t0 + Duration::from_millis(250 * tick);
            let got = read(&cache, &mut calls, at, Ok(vec![1]));
            assert_eq!(got, Ok(vec![1]));
        }
        assert_eq!(calls, 1);
        // Then a new one, and its answer replaces the old.
        let got = read(&cache, &mut calls, t0 + SERIES_REFRESH, Ok(vec![2]));
        assert_eq!((got, calls), (Ok(vec![2]), 2));
        // Another symbol or period is its own series.
        let other = cache
            .get("ETHUSDT", "1h", t0, || {
                calls += 1;
                Ok(vec![3])
            })
            .unwrap();
        assert_eq!((other.to_vec(), calls), (vec![3], 3));
    }

    #[test]
    fn a_failed_call_serves_the_last_good_series_so_the_panel_keeps_its_shape() {
        let (cache, t0) = (SeriesCache::default(), Instant::now());
        let mut calls = 0;
        read(&cache, &mut calls, t0, Ok(vec![1])).unwrap();

        // Refused or failing past the refresh: still there, and not asked for on
        // every recompute.
        let refused = || Err("Binance rate limit: waiting 60s after HTTP 429".to_string());
        let at = t0 + SERIES_REFRESH;
        assert_eq!(read(&cache, &mut calls, at, refused()), Ok(vec![1]));
        assert_eq!(calls, 2);
        let soon = at + Duration::from_secs(1);
        assert_eq!(read(&cache, &mut calls, soon, refused()), Ok(vec![1]));
        assert_eq!(calls, 2);

        // Through the gate's default cooldown and the refresh after it.
        let during = t0 + Duration::from_secs(60) + SERIES_REFRESH;
        assert_eq!(read(&cache, &mut calls, during, refused()), Ok(vec![1]));

        // Past its life the error comes through, once per refresh too.
        let late = t0 + SERIES_KEEP + SERIES_REFRESH;
        let e = read(&cache, &mut calls, late, refused()).unwrap_err();
        assert!(e.contains("rate limit"), "{e}");
        let before = calls;
        let again = read(
            &cache,
            &mut calls,
            late + Duration::from_secs(1),
            Ok(vec![9]),
        );
        assert!(again.unwrap_err().contains("rate limit"));
        assert_eq!(calls, before);

        // The next good read is the series again.
        let healed = late + SERIES_REFRESH;
        assert_eq!(read(&cache, &mut calls, healed, Ok(vec![5])), Ok(vec![5]));
    }

    #[test]
    fn the_cache_forgets_series_nobody_asks_for() {
        let (cache, t0) = (SeriesCache::default(), Instant::now());
        let mut calls = 0;
        read(&cache, &mut calls, t0, Ok(vec![1])).unwrap();
        cache
            .get("ETHUSDT", "1h", t0 + SERIES_KEEP, || Ok(vec![2]))
            .unwrap();
        assert_eq!(cache.lock().len(), 1);
    }

    type SeriesRead = Result<Arc<Vec<i64>>, String>;

    /// A fetch held open on another thread: `started` says it is running, and sending
    /// on the returned channel lets it answer.
    fn in_flight<'s>(
        scope: &'s std::thread::Scope<'s, '_>,
        cache: &'s SeriesCache<i64>,
        at: Instant,
        answer: Result<Vec<i64>, String>,
    ) -> (
        std::sync::mpsc::Sender<()>,
        std::thread::ScopedJoinHandle<'s, SeriesRead>,
    ) {
        let (started_tx, started) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let leader = scope.spawn(move || {
            cache.get("btcusdt", "1h", at, move || {
                started_tx.send(()).unwrap();
                released.recv().unwrap();
                answer
            })
        });
        started.recv().unwrap();
        (release, leader)
    }

    fn no_fetch() -> Result<Vec<i64>, String> {
        panic!("only the caller that claimed the refresh may fetch")
    }

    #[test]
    fn callers_at_an_expiry_send_one_request_and_get_the_last_good_series() {
        let (cache, t0) = (SeriesCache::default(), Instant::now());
        let mut calls = 0;
        read(&cache, &mut calls, t0, Ok(vec![1])).unwrap();
        let at = t0 + SERIES_REFRESH;
        std::thread::scope(|s| {
            let (release, leader) = in_flight(s, &cache, at, Ok(vec![2]));
            // Another 64 push groups recompute while it runs: none of them fetches,
            // and none of them blocks.
            for i in 0..64 {
                let late = at + Duration::from_millis(i);
                let got = cache.get("btcusdt", "1h", late, no_fetch).unwrap();
                assert_eq!(*got, vec![1]);
            }
            release.send(()).unwrap();
            assert_eq!(*leader.join().unwrap().unwrap(), vec![2]);
        });
        let next = cache.get("btcusdt", "1h", at, no_fetch).unwrap();
        assert_eq!(*next, vec![2]);
    }

    #[test]
    fn a_first_read_racing_another_waits_for_it_rather_than_blanking() {
        let (cache, t0) = (SeriesCache::default(), Instant::now());
        std::thread::scope(|s| {
            let (release, leader) = in_flight(s, &cache, t0, Ok(vec![7]));
            let (done_tx, done) = std::sync::mpsc::channel();
            let cache = &cache;
            let late = s.spawn(move || {
                let got = cache.get("btcusdt", "1h", t0, no_fetch);
                done_tx.send(()).unwrap();
                got
            });
            // It waits, rather than answering with an empty panel.
            assert!(done.recv_timeout(Duration::from_millis(50)).is_err());
            release.send(()).unwrap();
            assert_eq!(*late.join().unwrap().unwrap(), vec![7]);
            assert_eq!(*leader.join().unwrap().unwrap(), vec![7]);
        });
    }

    #[test]
    fn a_first_read_gives_up_on_a_slow_fetch_with_a_clear_error() {
        let (cache, t0) = (SeriesCache::default(), Instant::now());
        std::thread::scope(|s| {
            let (release, leader) = in_flight(s, &cache, t0, Ok(vec![7]));
            let e = cache
                .get_waiting("btcusdt", "1h", t0, Duration::from_millis(5), no_fetch)
                .unwrap_err();
            assert_eq!(e, SERIES_LOADING);
            release.send(()).unwrap();
            leader.join().unwrap().unwrap();
        });
    }

    #[test]
    fn a_failed_read_tells_those_waiting_and_retries_after_the_refresh() {
        let (cache, t0) = (SeriesCache::default(), Instant::now());
        std::thread::scope(|s| {
            let (release, leader) = in_flight(s, &cache, t0, Err("HTTP 500".to_string()));
            let cache = &cache;
            let late = s.spawn(move || cache.get("btcusdt", "1h", t0, no_fetch));
            std::thread::sleep(Duration::from_millis(5));
            release.send(()).unwrap();
            assert_eq!(leader.join().unwrap().unwrap_err(), "HTTP 500");
            assert_eq!(late.join().unwrap().unwrap_err(), "HTTP 500");
        });
        assert!(!cache.lock().values().any(|h| h.fetching));
        // Inside the refresh it stays an error, and after it the series is read again.
        let soon = t0 + Duration::from_secs(1);
        assert!(cache.get("btcusdt", "1h", soon, no_fetch).is_err());
        let mut calls = 0;
        let healed = read(&cache, &mut calls, t0 + SERIES_REFRESH, Ok(vec![3]));
        assert_eq!((healed, calls), (Ok(vec![3]), 1));
    }

    #[test]
    fn a_fetch_that_panics_leaves_nothing_stuck() {
        let (cache, t0) = (SeriesCache::<i64>::default(), Instant::now());
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cache.get("btcusdt", "1h", t0, || panic!("the fetch blew up"))
        }));
        assert!(panicked.is_err());
        assert!(!cache.lock().values().any(|h| h.fetching));
        // Nobody waits on it: a clear error at once, and a new read after the refresh.
        let soon = t0 + Duration::from_secs(1);
        let e = cache
            .get_waiting("btcusdt", "1h", soon, Duration::from_secs(60), no_fetch)
            .unwrap_err();
        assert_eq!(e, SERIES_LOADING);
        let mut calls = 0;
        let healed = read(&cache, &mut calls, t0 + SERIES_REFRESH, Ok(vec![3]));
        assert_eq!((healed, calls), (Ok(vec![3]), 1));
    }

    #[test]
    fn a_fetch_that_hangs_holds_its_claim_for_one_refresh_only() {
        let (cache, t0) = (SeriesCache::default(), Instant::now());
        std::thread::scope(|s| {
            let (release, leader) = in_flight(s, &cache, t0, Ok(vec![1]));
            // A refresh later another caller may fetch, and what it reads stays.
            let mut calls = 0;
            let next = read(&cache, &mut calls, t0 + SERIES_REFRESH, Ok(vec![2]));
            assert_eq!((next, calls), (Ok(vec![2]), 1));
            // The slow one lands last, with older data: it must not replace it.
            release.send(()).unwrap();
            leader.join().unwrap().unwrap();
        });
        let held = cache
            .get("btcusdt", "1h", t0 + SERIES_REFRESH, no_fetch)
            .unwrap();
        assert_eq!(*held, vec![2]);
    }

    fn hourly(symbol: &str, first: i64, n: usize) -> OhlcvData {
        let bars = (0..n as i64)
            .map(|i| Bar {
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
                date: (first + i * 3600).to_string(),
            })
            .collect();
        OhlcvData {
            bars,
            symbol: Some(symbol.into()),
            interval: Some("1h".into()),
            session: None,
        }
    }

    /// Bars near now, so their window is the live one the panels read.
    fn recent(symbol: &str, n: usize) -> OhlcvData {
        let now = now_ms() / 1000;
        hourly(symbol, now - 3600 * n as i64, n)
    }

    /// Puts `rows` where the panels will read them, so no request is sent.
    fn seed<T: Row>(
        cache: &SeriesCache<T>,
        symbol: &str,
        key: String,
        rows: Result<Vec<T>, String>,
    ) {
        let _ = cache.get(symbol, &key, Instant::now(), || rows);
    }

    fn seed_all(data: &OhlcvData, at: i64, rows: Result<(), String>) {
        let now = now_ms();
        let sym = data.symbol.clone().unwrap();
        let fw = funding_window(data, now).unwrap();
        let (hw, period) = history_window(data, now).unwrap();
        let pick = |v: f64| rows.clone().map(|_| vec![(at, v)]);
        seed(&FUNDING, &sym, fw.key(""), pick(0.5));
        seed(&OPEN_INTEREST, &sym, hw.key(period), pick(0.5));
        let ls = rows.map(|_| vec![(at, 0.5, 0.5)]);
        seed(&LONG_SHORT, &sym, hw.key(period), ls);
    }

    fn shape(r: &PanelResult) -> (usize, usize, usize) {
        (r.lines.len(), r.bars.len(), r.fills.len())
    }

    #[test]
    fn panels_read_their_series_through_the_cache() {
        let data = recent("TESTCACHEUSDT", 1);
        let at = data.bars[0].date.parse::<i64>().unwrap() * 1000;
        seed_all(&data, at, Ok(()));
        // Served from the cache, with no network behind it: the values arrive.
        let funding = FundingRate.compute(&data);
        assert_eq!(funding.label, "Funding");
        assert_eq!(funding.bars[0].y, vec![50.0], "0.5 as a percentage");
        let oi = OpenInterestHist.compute(&data);
        assert_eq!(oi.label, "OI");
        assert_eq!(oi.lines[0].y, vec![0.5]);
        let long_short = LongShortRatio.compute(&data);
        assert_eq!(long_short.label, "L/S");
        assert!(long_short.lines.iter().any(|l| l.y[0].is_finite()));
    }

    #[test]
    fn a_bar_with_no_funding_is_empty_not_zero() {
        // Two bars a day apart: the one beside the payment has a value, the other none.
        let mut data = recent("TESTGAPUSDT", 1);
        let first = data.bars[0].clone();
        let far = Bar {
            date: (first.date.parse::<i64>().unwrap() - 86_400).to_string(),
            ..first.clone()
        };
        data.bars.insert(0, far);
        let at = first.date.parse::<i64>().unwrap() * 1000;
        let now = now_ms();
        let w = funding_window(&data, now).unwrap();
        seed(&FUNDING, "TESTGAPUSDT", w.key(""), Ok(vec![(at, 0.0001)]));
        let p = FundingRate.compute(&data);
        assert_eq!(p.label, "Funding");
        assert!(p.bars[0].y[0].is_nan(), "{:?}", p.bars[0].y);
        assert!((p.bars[0].y[1] - 0.01).abs() < 1e-9);
    }

    #[test]
    fn a_panel_says_why_it_has_no_values_and_keeps_its_shape() {
        let live = recent("SHAPEUSDT", 3);
        let at = live.bars[0].date.parse::<i64>().unwrap() * 1000;
        seed_all(&live, at, Ok(()));
        let with_data = [
            FundingRate.compute(&live),
            OpenInterestHist.compute(&live),
            LongShortRatio.compute(&live),
        ];

        // A stock: Binance futures only.
        let mut stock = recent("AAPL", 3);
        stock.interval = Some("1d".into());
        // A spot-only pair: Binance answers with no rows.
        let spot = recent("BTCFDUSD", 3);
        let now = now_ms();
        let sw = funding_window(&spot, now).unwrap();
        let (hw, period) = history_window(&spot, now).unwrap();
        seed(&FUNDING, "BTCFDUSD", sw.key(""), Ok(vec![]));
        seed(&OPEN_INTEREST, "BTCFDUSD", hw.key(period), Ok(vec![]));
        seed(&LONG_SHORT, "BTCFDUSD", hw.key(period), Ok(vec![]));
        // A failing request.
        let down = recent("DOWNUSDT", 3);
        seed_all(&down, 0, Err("Binance rate limit".into()));
        // Bars older than Binance keeps open interest.
        let old = hourly("OLDUSDT", 1_600_000_000, 3);
        let ow = funding_window(&old, now).unwrap();
        seed(&FUNDING, "OLDUSDT", ow.key(""), Ok(vec![]));

        let panels = |d: &OhlcvData| {
            [
                FundingRate.compute(d),
                OpenInterestHist.compute(d),
                LongShortRatio.compute(d),
            ]
        };
        for (d, note) in [
            (&stock, "Binance futures only"),
            (&spot, "no futures market"),
            (&down, "unavailable"),
        ] {
            for (p, name) in panels(d).iter().zip(["Funding", "OI", "L/S"]) {
                assert_eq!(p.label, format!("{name} ({note})"));
            }
        }
        for (p, name) in panels(&old)[1..].iter().zip(["OI", "L/S"]) {
            assert_eq!(p.label, format!("{name} (last 30 days only)"));
        }
        assert_eq!(panels(&old)[0].label, "Funding (no futures market)");

        // Same channels with or without values, and every bar empty rather than zero:
        // the web merges a tail into the series it has only when they match.
        for d in [&stock, &spot, &down, &old] {
            for (p, full) in panels(d).iter().zip(&with_data) {
                assert_eq!(shape(p), shape(full));
                let ys = p
                    .bars
                    .iter()
                    .map(|b| &b.y)
                    .chain(p.lines.iter().map(|l| &l.y));
                for y in ys {
                    assert_eq!(y.len(), d.len());
                    assert!(y.iter().all(|v| v.is_nan()), "{y:?}");
                }
            }
        }
    }

    // ---- Paging ----

    /// Binance as measured: the latest `PAGE_ROWS` rows between the two times.
    struct Fake {
        rows: Vec<(i64, f64)>,
        asked: Vec<(Option<i64>, Option<i64>)>,
    }

    impl Fake {
        fn page(&mut self, q: Query) -> Result<Vec<(i64, f64)>, String> {
            self.asked.push((q.start, q.end));
            let mut rows: Vec<_> = self
                .rows
                .iter()
                .filter(|r| q.start.is_none_or(|s| r.0 >= s) && q.end.is_none_or(|e| r.0 <= e))
                .copied()
                .collect();
            let skip = rows.len().saturating_sub(PAGE_ROWS);
            rows.drain(..skip);
            Ok(rows)
        }
    }

    const NOW: i64 = 1_800_000_000_000;

    fn window(from: i64, to: i64) -> Window {
        Window {
            from,
            to,
            bucket: from,
            live: to >= NOW,
        }
    }

    fn rows_every(step: i64, from: i64, to: i64) -> Vec<(i64, f64)> {
        (from / step..=to / step).map(|i| (i * step, 1.0)).collect()
    }

    #[test]
    fn history_is_read_newest_page_first_until_the_window_is_covered() {
        // 2000 hourly rows; the window wants the last 1200.
        let mut fake = Fake {
            rows: rows_every(HOUR_MS, NOW - 2000 * HOUR_MS, NOW),
            asked: vec![],
        };
        let w = window(NOW - 1200 * HOUR_MS, NOW + HOUR_MS);
        let got = collect(None, &w, NOW, i64::MIN, 8, &mut |q| fake.page(q)).unwrap();
        assert_eq!(fake.asked.len(), 3, "{:?}", fake.asked);
        assert_eq!(fake.asked[0], (None, None));
        assert!(got.first().unwrap().0 <= w.from);
        assert_eq!(got.last().unwrap().0, NOW);
        assert!(
            got.windows(2).all(|p| p[0].0 < p[1].0),
            "ascending, no repeats"
        );
    }

    #[test]
    fn the_requests_per_window_are_bounded() {
        let mut fake = Fake {
            rows: rows_every(HOUR_MS, 0, NOW),
            asked: vec![],
        };
        let w = window(0, NOW + HOUR_MS);
        let got = collect(None, &w, NOW, i64::MIN, 6, &mut |q| fake.page(q)).unwrap();
        assert_eq!(fake.asked.len(), 6);
        assert_eq!(got.len(), 6 * PAGE_ROWS);
        // The newest rows are the ones kept.
        assert_eq!(got.last().unwrap().0, NOW);
    }

    #[test]
    fn running_out_of_history_ends_with_an_empty_page_not_a_loop() {
        let mut fake = Fake {
            rows: rows_every(HOUR_MS, NOW - 700 * HOUR_MS, NOW),
            asked: vec![],
        };
        let w = window(0, NOW + HOUR_MS);
        let got = collect(None, &w, NOW, i64::MIN, 50, &mut |q| fake.page(q)).unwrap();
        assert_eq!((fake.asked.len(), got.len()), (3, 701));
    }

    #[test]
    fn retention_stops_the_reading_at_the_oldest_row_binance_keeps() {
        let floor = NOW - RETENTION_MS;
        let mut fake = Fake {
            rows: rows_every(HOUR_MS, floor, NOW),
            asked: vec![],
        };
        // The window reaches back a year; the first page past the floor is not asked for.
        let w = window(NOW - 365 * DAY_MS, NOW + HOUR_MS);
        collect(None, &w, NOW, floor, 50, &mut |q| fake.page(q)).unwrap();
        assert_eq!(fake.asked.len(), 2, "720 rows = 2 pages, then done");
    }

    #[test]
    fn a_window_of_older_bars_is_read_up_to_its_own_end() {
        let mut fake = Fake {
            rows: rows_every(HOUR_MS, NOW - 2000 * HOUR_MS, NOW),
            asked: vec![],
        };
        let to = NOW - 1000 * HOUR_MS;
        let w = window(to - 100 * HOUR_MS, to);
        let got = collect(None, &w, NOW, i64::MIN, 8, &mut |q| fake.page(q)).unwrap();
        assert_eq!(fake.asked, vec![(None, Some(to))]);
        assert_eq!(got.last().unwrap().0, to);
    }

    #[test]
    fn a_refresh_asks_only_for_what_is_new() {
        let mut fake = Fake {
            rows: rows_every(HOUR_MS, NOW - 300 * HOUR_MS, NOW),
            asked: vec![],
        };
        let w = window(NOW - 200 * HOUR_MS, NOW + HOUR_MS);
        let first = collect(None, &w, NOW, i64::MIN, 8, &mut |q| fake.page(q)).unwrap();
        fake.rows.push((NOW + HOUR_MS, 2.0));
        fake.asked.clear();
        let next = collect(Some(&first), &w, NOW + HOUR_MS, i64::MIN, 8, &mut |q| {
            fake.page(q)
        })
        .unwrap();
        assert_eq!(fake.asked, vec![(Some(NOW + 1), None)]);
        assert_eq!(next.len(), first.len() + 1);
        assert_eq!(next.last().unwrap(), &(NOW + HOUR_MS, 2.0));
        // A window already covered asks for nothing.
        fake.asked.clear();
        let old = window(NOW - 200 * HOUR_MS, NOW - 10 * HOUR_MS);
        let kept = collect(Some(&next), &old, NOW, i64::MIN, 8, &mut |q| fake.page(q)).unwrap();
        assert!(fake.asked.is_empty());
        assert_eq!(kept, next);
    }

    #[test]
    fn a_failing_page_fails_the_read_so_the_last_good_series_is_kept() {
        let w = window(0, NOW + HOUR_MS);
        let mut calls = 0;
        let got = collect::<(i64, f64)>(None, &w, NOW, i64::MIN, 8, &mut |_| {
            calls += 1;
            if calls == 1 {
                Ok(rows_every(HOUR_MS, NOW - 499 * HOUR_MS, NOW))
            } else {
                Err("HTTP 429".into())
            }
        });
        assert_eq!(got.unwrap_err(), "HTTP 429");
    }

    #[test]
    fn older_bars_are_their_own_series_and_never_the_newest_window() {
        let now = NOW;
        let bar = |ms: i64| Bar {
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
            date: (ms / 1000).to_string(),
        };
        let newest = Window::of(
            &[bar(now - 3 * HOUR_MS), bar(now)],
            FUNDING_GAP_MS,
            DAY_MS,
            now,
        );
        let older = Window::of(
            &[bar(now - 90 * DAY_MS), bar(now - 89 * DAY_MS)],
            FUNDING_GAP_MS,
            DAY_MS,
            now,
        );
        let (newest, older) = (newest.unwrap(), older.unwrap());
        assert!(newest.live && !older.live);
        assert_ne!(newest.key(""), older.key(""));
        assert_ne!(newest.key("1h"), newest.key("5m"));
        // The chart gaining a bar keeps its series; the next day starts another.
        let later = Window::of(
            &[bar(now - 2 * HOUR_MS), bar(now + HOUR_MS)],
            FUNDING_GAP_MS,
            DAY_MS,
            now + HOUR_MS,
        );
        assert_eq!(later.unwrap().key(""), newest.key(""));

        // And in the cache they do not touch.
        let cache = SeriesCache::<(i64, f64)>::default();
        let t = Instant::now();
        cache
            .get("BTCUSDT", &newest.key(""), t, || Ok(vec![(now, 1.0)]))
            .unwrap();
        cache
            .get("BTCUSDT", &older.key(""), t, || {
                Ok(vec![(now - 90 * DAY_MS, 2.0)])
            })
            .unwrap();
        let held = cache
            .get("BTCUSDT", &newest.key(""), t, no_fetch_rows)
            .unwrap();
        assert_eq!(*held, vec![(now, 1.0)]);
    }

    #[test]
    fn the_sample_source_answers_like_binance_without_a_request() {
        let q = |start, end| Query { start, end };
        let hour = HOUR_MS;
        let now = now_ms();
        // The latest page, on the grid, never in the future.
        let t = sample_times("BTCUSDT", hour, None, &q(None, None)).unwrap();
        assert_eq!(t.len(), PAGE_ROWS);
        assert!(t.last().unwrap() <= &now && t.iter().all(|x| x % hour == 0));
        // History series stop at the retention.
        let kept = sample_times("BTCUSDT", 300_000, Some(RETENTION_MS), &q(None, None)).unwrap();
        assert_eq!(kept.len(), PAGE_ROWS);
        let far = sample_times(
            "BTCUSDT",
            hour,
            Some(RETENTION_MS),
            &q(None, Some(now - 40 * DAY_MS)),
        );
        assert!(far.unwrap().is_empty());
        // Spot-only and failing symbols.
        assert!(sample_times("BTCFDUSD", hour, None, &q(None, None))
            .unwrap()
            .is_empty());
        assert!(sample_times("XERRUSDT", hour, None, &q(None, None)).is_err());
    }

    fn no_fetch_rows() -> Result<Vec<(i64, f64)>, String> {
        panic!("held already")
    }

    #[test]
    fn a_chart_interval_reads_the_longest_period_that_fits_in_a_bar() {
        for (tf, period) in [
            ("1m", "5m"),
            ("3m", "5m"),
            ("7m", "5m"),
            ("15m", "15m"),
            ("45m", "30m"),
            ("90m", "1h"),
            ("4h", "4h"),
            ("8h", "6h"),
            ("1d", "1d"),
            ("2D", "1d"),
            ("2W", "1d"),
            ("1mo", "1d"),
            ("banana", "4h"),
        ] {
            assert_eq!(map_interval(tf), period, "{tf}");
        }
        assert_eq!(interval_to_max_gap_ms("7m"), 840_000);
        assert_eq!(interval_to_max_gap_ms("2D"), 345_600_000);
    }
}
