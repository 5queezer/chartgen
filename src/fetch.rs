use crate::data::{self, Bar, OhlcvData};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Binance klines REST caps `limit` at 1000 per request.
const BINANCE_MAX_LIMIT: usize = 1000;
/// Overall budget for one paginated Binance fetch.
const BINANCE_DEADLINE: Duration = Duration::from_secs(20);

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

/// Binance REST interval for a Yahoo-style timeframe the frontend may send
/// ("1wk" → "1w", "1mo" → "1M", "60m" → "1h"). Binance names pass through.
pub(crate) fn binance_interval(interval: &str) -> &str {
    match interval {
        "1wk" => "1w",
        "1mo" => "1M",
        "60m" => "1h",
        other => other,
    }
}

fn binance_klines_url(symbol: &str, interval: &str, limit: usize, end_ms: Option<i64>) -> String {
    let mut url = format!(
        "https://api.binance.com/api/v3/klines?symbol={}&interval={}&limit={}",
        symbol.to_uppercase(),
        binance_interval(interval),
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
    let deadline = Instant::now() + BINANCE_DEADLINE;
    let bars = paginate_backwards(limit, end_time_ms, deadline, |page_limit, end| {
        fetch_binance_page(&binance_klines_url(symbol, interval, page_limit, end))
    })?;

    if bars.is_empty() && end_time_ms.is_none() {
        return Err("Binance returned zero bars".into());
    }

    let has_more = bars.len() >= limit;
    Ok(Fetched {
        data: OhlcvData {
            bars,
            symbol: None,
            interval: None,
        },
        has_more,
    })
}

fn fetch_binance_page(url: &str) -> Result<Vec<Bar>, String> {
    let resp = http()
        .get(url)
        .send()
        .map_err(|e| format!("Binance request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Binance API returned status {}", resp.status()));
    }

    let body: serde_json::Value = resp
        .json()
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
    let window = yahoo_window(interval, limit, end_time_ms, now_ms() / 1000);

    if window.period1 >= window.period2 {
        // Entire window lies beyond Yahoo's intraday lookback.
        return match end_time_ms {
            Some(_) => Ok(Fetched {
                data: OhlcvData {
                    bars: Vec::new(),
                    symbol: None,
                    interval: None,
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
    let needs_aggregation = window.api_interval != interval
        && matches!(interval, "3m" | "2h" | "4h" | "6h" | "12h" | "3d");
    let empty = Fetched {
        data: OhlcvData {
            bars: Vec::new(),
            symbol: None,
            interval: None,
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
    let gmtoffset = meta_i64(&["gmtoffset"]).unwrap_or(0);
    // 24/7 instruments have no session to anchor 4h buckets on.
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

    let mut bars = Vec::new();
    #[allow(clippy::needless_range_loop)] // indexing 6 parallel arrays
    for i in 0..timestamps.len() {
        let ts = match timestamps[i].as_i64() {
            Some(v) => v,
            None => continue,
        };
        if end_time_ms.is_some_and(|e| ts.saturating_mul(1000) >= e) {
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
        snap_live_bar(&mut bars, step, session_start);
    }

    if needs_aggregation {
        let secs = data::interval_secs(interval).unwrap_or(DAY);
        bars = if interval == "3d" {
            aggregate_days(
                &bars,
                3,
                if all_day { 0 } else { gmtoffset },
                window.period1,
            )
        } else if all_day {
            aggregate_bars(&bars, secs, window.period1)
        } else {
            aggregate_sessions(&bars, secs, gmtoffset, window.period1)
        };
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

    Ok(Fetched {
        data: OhlcvData {
            bars,
            symbol: None,
            interval: None,
        },
        has_more,
    })
}

/// Yahoo stamps the live bar with the last trade time instead of its period
/// start. Snap it onto the `step` grid anchored at the current session open
/// (intraday/daily, `session_start`) or at the previous bar (weekly and up;
/// months by calendar), and merge it into the previous bar when both fall in
/// the same period. Volume merges with `max` so a repeated snapshot of the
/// forming bar is not double-counted.
fn snap_live_bar(bars: &mut Vec<Bar>, step: i64, session_start: Option<i64>) {
    let n = bars.len();
    let Some(ts) = bars.last().and_then(bar_ts) else {
        return;
    };
    let prev = n.checked_sub(2).and_then(|i| bar_ts(&bars[i]));
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

/// Yahoo interval that `interval` is fetched as. Yahoo has no native 3m, 2h,
/// 4h, 6h, 12h or 3d, so those are resampled from 1m, 1h and 1d bars.
fn yahoo_api_interval(interval: &str) -> &str {
    match interval {
        "3m" => "1m",
        "2h" | "4h" | "6h" | "12h" => "1h",
        "3d" => "1d",
        other => other,
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
    let calendar_factor = if step < DAY {
        6.0
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

/// Group consecutive bars into epoch-aligned `bucket_secs` buckets (1h → 4h)
/// for 24/7 instruments. Grid alignment keeps bucket boundaries identical
/// across overlapping requests, which history paging relies on. Buckets
/// starting before `min_start` (unix seconds) may be missing members and are
/// dropped.
fn aggregate_bars(bars: &[Bar], bucket_secs: i64, min_start: i64) -> Vec<Bar> {
    let bucket = |b: &Bar| bar_ts(b).map(|t| t.div_euclid(bucket_secs) * bucket_secs);
    bars.chunk_by(|a, b| bucket(a) == bucket(b))
        .filter(|chunk| bucket(&chunk[0]).is_some_and(|s| s >= min_start))
        .map(merge_bars)
        .collect()
}

/// Session-anchored buckets for exchange-traded symbols (TradingView style):
/// within each exchange-local trading day, buckets of `bucket_secs` start at
/// the day's first bar, and the day's last bucket may be shorter. Each
/// bucket's `t` is its first bar. `gmtoffset` (seconds) maps UTC to exchange
/// time; it is the current offset, but a DST shift of an hour never moves a
/// session bar across local midnight. The local day containing `min_start`
/// may be cut off by the window and is dropped.
fn aggregate_sessions(bars: &[Bar], bucket_secs: i64, gmtoffset: i64, min_start: i64) -> Vec<Bar> {
    let day = |t: i64| (t + gmtoffset).div_euclid(DAY);
    let first_full_day = day(min_start.saturating_sub(1)) + 1;
    let mut out = Vec::new();
    for session in bars.chunk_by(|a, b| bar_ts(a).map(day) == bar_ts(b).map(day)) {
        let Some(open) = bar_ts(&session[0]) else {
            continue;
        };
        if day(open) < first_full_day {
            continue;
        }
        let slot = |b: &Bar| bar_ts(b).map(|t| (t - open).div_euclid(bucket_secs));
        out.extend(session.chunk_by(|a, b| slot(a) == slot(b)).map(merge_bars));
    }
    out
}

/// Buckets of `n` consecutive calendar days (exchange-local, via `gmtoffset`
/// seconds), keyed by `local_day / n` so bucket boundaries stay identical
/// across overlapping requests. The bucket holding `min_start` may be missing
/// older members and is dropped; the last bucket may be partial (still forming).
fn aggregate_days(bars: &[Bar], n: i64, gmtoffset: i64, min_start: i64) -> Vec<Bar> {
    let key = |b: &Bar| bar_ts(b).map(|t| (t + gmtoffset).div_euclid(DAY).div_euclid(n));
    bars.chunk_by(|a, b| key(a) == key(b))
        .filter(|chunk| key(&chunk[0]).is_some_and(|k| k * n * DAY - gmtoffset >= min_start))
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
    let resp = http()
        .get(url)
        .send()
        .map_err(|e| (false, format!("Binance request failed: {e}")))?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().map_err(|e| {
        (
            false,
            format!("Failed to parse Binance response ({status}): {e}"),
        )
    })?;
    if status.is_success() {
        return Ok(body);
    }
    let msg = body.get("msg").and_then(|m| m.as_str()).map_or_else(
        || format!("Binance API returned status {status}"),
        |m| format!("Binance: {m}"),
    );
    Err((binance_unknown_symbol(status.as_u16(), &body), msg))
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
    fn binance_interval_maps_yahoo_names() {
        assert_eq!(binance_interval("1wk"), "1w");
        assert_eq!(binance_interval("1mo"), "1M");
        assert_eq!(binance_interval("60m"), "1h");
        for native in [
            "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h", "1d", "3d", "1w",
            "1M",
        ] {
            assert_eq!(binance_interval(native), native);
        }
        assert!(binance_klines_url("BTCUSDT", "1wk", 10, None).contains("&interval=1w&"));
        assert!(binance_klines_url("BTCUSDT", "1mo", 10, None).contains("&interval=1M&"));
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
        snap_live_bar(&mut bars, DAY, Some(NOV4_OPEN));
        assert_eq!(bar_ts(&bars[1]), Some(NOV4_OPEN));
        // Weekly: a mid-week print belongs to the previous (current-week) bar.
        let monday = 1_792_972_800; // 2026-10-26 00:00 UTC, a Monday
        let mut bars = vec![bar_at(monday, 1.0), bar_at(monday + 2 * DAY + 5 * H, 2.0)];
        snap_live_bar(&mut bars, 7 * DAY, None);
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].close, 2.0);
        // ... and a print in the next week starts a new Monday bar.
        let mut bars = vec![bar_at(monday, 1.0), bar_at(monday + 8 * DAY, 2.0)];
        snap_live_bar(&mut bars, 7 * DAY, None);
        assert_eq!(bar_ts(&bars[1]), Some(monday + 7 * DAY));
        // Monthly: snap to the 1st of the month at the previous bar's time of day.
        let oct1 = 1_790_827_200; // 2026-10-01 04:00 UTC
        let mut bars = vec![bar_at(oct1, 1.0), bar_at(NOV4_OPEN, 2.0)];
        snap_live_bar(&mut bars, 30 * DAY, None);
        assert_eq!(bar_ts(&bars[1]), Some(oct1 + 31 * DAY));
        // On-grid bars are left alone.
        let mut bars = vec![bar_at(NOV3_OPEN, 1.0), bar_at(NOV4_OPEN, 2.0)];
        snap_live_bar(&mut bars, DAY, Some(NOV4_OPEN));
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
                vec![base + H, base + 4 * H, base + 8 * H],
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
            ("60m", "60m"),
            ("1wk", "1wk"),
        ] {
            assert_eq!(yahoo_window(tf, 100, None, now).api_interval, api, "{tf}");
        }
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
        // Buckets (dated by their first bar): 2, 3 and 2 bars; the last is forming.
        assert_eq!(dates(&f), vec![base + 60, base + 180, base + 360]);
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
        let d0 = 20_000 - 20_000 % 3;
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
}
