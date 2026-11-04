use crate::data::{self, Bar, OhlcvData};
use std::sync::OnceLock;
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
fn http() -> &'static reqwest::blocking::Client {
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
fn binance_interval(interval: &str) -> &str {
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

    let url = yahoo_chart_url(symbol, &window);

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
/// and aggregates 1h → 4h. Pure, so fixture JSON can drive it in tests.
fn parse_yahoo_chart(
    body: &serde_json::Value,
    interval: &str,
    limit: usize,
    end_time_ms: Option<i64>,
    window: &YahooWindow,
) -> Result<Fetched, String> {
    let needs_aggregation = interval == "4h" && window.api_interval == "1h";
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
        bars = if all_day {
            aggregate_bars(&bars, 4 * 3600, window.period1)
        } else {
            aggregate_sessions(&bars, 4 * 3600, gmtoffset, window.period1)
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

fn yahoo_window(
    interval: &str,
    limit: usize,
    end_time_ms: Option<i64>,
    now_s: i64,
) -> YahooWindow<'_> {
    // 4h is aggregated from 1h (Yahoo has no native 4h).
    let api_interval = if interval == "4h" { "1h" } else { interval };
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

fn yahoo_chart_url(symbol: &str, w: &YahooWindow) -> String {
    format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&period1={}&period2={}",
        symbol, w.api_interval, w.period1, w.period2
    )
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
        for native in ["1m", "5m", "15m", "30m", "1h", "4h", "1d", "1w", "1M"] {
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
            yahoo_chart_url("AAPL", &yahoo_window("1d", 200, None, now)),
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
}
