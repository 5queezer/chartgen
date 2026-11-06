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
        let key = (symbol.to_uppercase(), period.to_string());
        let lasting = |held: &HeldSeries<T>| {
            held.good
                .as_ref()
                .filter(|(at, _)| now.saturating_duration_since(*at) < SERIES_KEEP)
                .map(|(_, series)| series.clone())
        };
        let claim = {
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
            SeriesClaim {
                cache: self,
                key: key.clone(),
            }
        };
        let result = fetch();
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

// ---- Funding Rate ----

pub struct FundingRate;

fn fetch_funding_data(symbol: &str) -> Result<Vec<(i64, f64)>, String> {
    let client = http_client();
    let url = format!(
        "https://fapi.binance.com/fapi/v1/fundingRate?symbol={}&limit=1000",
        symbol.to_uppercase()
    );
    let gate = crate::binance_gate::futures();
    let sent = gate.admit()?;
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("Funding rate request failed: {}", e))?;
    gate.observe_response(sent, resp.status().as_u16(), resp.headers());
    if !resp.status().is_success() {
        return Err(format!(
            "Funding rate API returned status {}",
            resp.status()
        ));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| format!("Failed to parse funding rate response: {}", e))?;
    let arr = body
        .as_array()
        .ok_or("Funding rate response is not an array")?;

    let mut result = Vec::with_capacity(arr.len());
    for entry in arr {
        let ts = entry
            .get("fundingTime")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let rate: f64 = entry
            .get("fundingRate")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .parse()
            .unwrap_or(0.0);
        result.push((ts, rate));
    }
    result.sort_by_key(|&(ts, _)| ts);
    Ok(result)
}

impl Indicator for FundingRate {
    fn name(&self) -> &str {
        "funding_rate"
    }

    fn description(&self) -> &str {
        "Funding Rate — crypto perpetual futures funding (Binance)"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let sym = match data.symbol.as_deref() {
            Some(s) if is_crypto_symbol(s) => s,
            _ => {
                return PanelResult {
                    label: "Funding (no data)".into(),
                    ..Default::default()
                }
            }
        };

        let series = match FUNDING.get(sym, "", Instant::now(), || fetch_funding_data(sym)) {
            Ok(s) => s,
            Err(_) => {
                return PanelResult {
                    label: "Funding (no data)".into(),
                    ..Default::default()
                }
            }
        };

        // 8 hours max gap
        let max_gap_ms = 8 * 60 * 60 * 1000;
        let aligned = align_to_bars(&series, &data.bars, max_gap_ms);

        let n = aligned.len();
        let mut pct = vec![f64::NAN; n];
        let mut colors = Vec::with_capacity(n);
        for (i, &v) in aligned.iter().enumerate() {
            if v.is_nan() {
                pct[i] = 0.0;
                colors.push(rgba(0x888888, 0.7));
            } else {
                pct[i] = v * 100.0;
                if pct[i] >= 0.0 {
                    colors.push(rgba(0x26A69A, 0.7));
                } else {
                    colors.push(rgba(0xEF5350, 0.7));
                }
            }
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
            label: "Funding".into(),
            is_overlay: false,
            ..Default::default()
        }
    }
}

// ---- Open Interest ----

pub struct OpenInterestHist;

fn fetch_oi_data(symbol: &str, interval: &str) -> Result<Vec<(i64, f64)>, String> {
    let period = map_interval(interval);
    let client = http_client();
    let url = format!(
        "https://fapi.binance.com/futures/data/openInterestHist?symbol={}&period={}&limit=500",
        symbol.to_uppercase(),
        period
    );
    let gate = crate::binance_gate::futures();
    let sent = gate.admit()?;
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("OI request failed: {}", e))?;
    gate.observe_response(sent, resp.status().as_u16(), resp.headers());
    if !resp.status().is_success() {
        return Err(format!("OI API returned status {}", resp.status()));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| format!("Failed to parse OI response: {}", e))?;
    let arr = body.as_array().ok_or("OI response is not an array")?;

    let mut result = Vec::with_capacity(arr.len());
    for entry in arr {
        let ts = entry.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
        let val: f64 = entry
            .get("sumOpenInterestValue")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .parse()
            .unwrap_or(0.0);
        result.push((ts, val));
    }
    result.sort_by_key(|&(ts, _)| ts);
    Ok(result)
}

impl Indicator for OpenInterestHist {
    fn name(&self) -> &str {
        "open_interest"
    }

    fn description(&self) -> &str {
        "Open Interest — total outstanding futures contracts (Binance)"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let sym = match data.symbol.as_deref() {
            Some(s) if is_crypto_symbol(s) => s,
            _ => {
                return PanelResult {
                    label: "OI (no data)".into(),
                    ..Default::default()
                }
            }
        };
        let interval = data.interval.as_deref().unwrap_or("4h");

        let series = match OPEN_INTEREST.get(sym, map_interval(interval), Instant::now(), || {
            fetch_oi_data(sym, interval)
        }) {
            Ok(s) => s,
            Err(_) => {
                return PanelResult {
                    label: "OI (no data)".into(),
                    ..Default::default()
                }
            }
        };

        let max_gap_ms = interval_to_max_gap_ms(interval);
        let aligned = align_to_bars(&series, &data.bars, max_gap_ms);

        let n = aligned.len();
        let zeros = vec![0.0; n];

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
            label: "OI".into(),
            is_overlay: false,
            ..Default::default()
        }
    }
}

// ---- Long/Short Ratio ----

pub struct LongShortRatio;

fn fetch_ls_data(symbol: &str, interval: &str) -> Result<Vec<(i64, f64, f64)>, String> {
    let period = map_interval(interval);
    let client = http_client();
    let url = format!(
        "https://fapi.binance.com/futures/data/globalLongShortAccountRatio?symbol={}&period={}&limit=500",
        symbol.to_uppercase(),
        period
    );
    let gate = crate::binance_gate::futures();
    let sent = gate.admit()?;
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("L/S request failed: {}", e))?;
    gate.observe_response(sent, resp.status().as_u16(), resp.headers());
    if !resp.status().is_success() {
        return Err(format!("L/S API returned status {}", resp.status()));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| format!("Failed to parse L/S response: {}", e))?;
    let arr = body.as_array().ok_or("L/S response is not an array")?;

    let mut result = Vec::with_capacity(arr.len());
    for entry in arr {
        let ts = entry.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
        let long_acc: f64 = entry
            .get("longAccount")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .parse()
            .unwrap_or(0.0);
        let short_acc: f64 = entry
            .get("shortAccount")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .parse()
            .unwrap_or(0.0);
        result.push((ts, long_acc, short_acc));
    }
    result.sort_by_key(|&(ts, _, _)| ts);
    Ok(result)
}

impl Indicator for LongShortRatio {
    fn name(&self) -> &str {
        "long_short"
    }

    fn description(&self) -> &str {
        "Long/Short Ratio — account positioning (Binance)"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let sym = match data.symbol.as_deref() {
            Some(s) if is_crypto_symbol(s) => s,
            _ => {
                return PanelResult {
                    label: "L/S (no data)".into(),
                    ..Default::default()
                }
            }
        };
        let interval = data.interval.as_deref().unwrap_or("4h");

        let series = match LONG_SHORT.get(sym, map_interval(interval), Instant::now(), || {
            fetch_ls_data(sym, interval)
        }) {
            Ok(s) => s,
            Err(_) => {
                return PanelResult {
                    label: "L/S (no data)".into(),
                    ..Default::default()
                }
            }
        };

        // Convert to two separate series for align_to_bars
        let long_series: Vec<(i64, f64)> = series.iter().map(|&(ts, l, _)| (ts, l)).collect();
        let short_series: Vec<(i64, f64)> = series.iter().map(|&(ts, _, s)| (ts, s)).collect();

        let max_gap_ms = interval_to_max_gap_ms(interval);
        let long_aligned = align_to_bars(&long_series, &data.bars, max_gap_ms);
        let short_aligned = align_to_bars(&short_series, &data.bars, max_gap_ms);

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
            label: "L/S".into(),
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

    #[test]
    fn panels_read_their_series_through_the_cache() {
        let bars = vec![Bar {
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
            date: "1700000000".into(),
        }];
        let data = OhlcvData {
            bars,
            symbol: Some("TESTCACHEUSDT".into()),
            interval: Some("1h".into()),
            session: None,
        };
        let now = Instant::now();
        let at = 1_700_000_000_000;
        let held: Result<Vec<(i64, f64)>, String> = Ok(vec![(at, 0.5)]);
        FUNDING
            .get("TESTCACHEUSDT", "", now, || held.clone())
            .unwrap();
        OPEN_INTEREST
            .get("TESTCACHEUSDT", "1h", now, || held.clone())
            .unwrap();
        LONG_SHORT
            .get("TESTCACHEUSDT", "1h", now, || Ok(vec![(at, 0.5, 0.5)]))
            .unwrap();
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
