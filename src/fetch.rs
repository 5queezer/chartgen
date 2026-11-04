use crate::data::{self, Bar, OhlcvData};

/// Binance klines REST caps `limit` at 1000 per request.
const BINANCE_MAX_LIMIT: usize = 1000;

/// Where OHLCV data comes from. `Sample` (env `CHARTGEN_DATA_SOURCE=sample`)
/// never touches the network — see `data::sample_ohlcv`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataSource {
    Live,
    Sample,
}

impl DataSource {
    pub fn from_env() -> Self {
        match std::env::var("CHARTGEN_DATA_SOURCE").as_deref() {
            Ok("sample") => DataSource::Sample,
            _ => DataSource::Live,
        }
    }
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
) -> Result<OhlcvData, String> {
    match source {
        DataSource::Sample => data::sample_ohlcv(symbol, interval, limit, end_time_ms, now_ms()),
        DataSource::Live => match detect_source(symbol) {
            "binance" => fetch_binance(symbol, interval, limit, end_time_ms),
            _ => fetch_yahoo(symbol, interval, limit, end_time_ms),
        },
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Open time in ms of a bar whose `date` holds unix seconds.
fn bar_open_ms(bar: &Bar) -> Option<i64> {
    bar.date.parse::<i64>().ok().map(|s| s * 1000)
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
/// performs one request; a short page means history is exhausted.
fn paginate_backwards<F>(
    limit: usize,
    end_time_ms: Option<i64>,
    mut page: F,
) -> Result<Vec<Bar>, String>
where
    F: FnMut(usize, Option<i64>) -> Result<Vec<Bar>, String>,
{
    let mut out: Vec<Bar> = Vec::new();
    // Binance's endTime is inclusive; ours is exclusive.
    let mut end = end_time_ms.map(|e| e - 1);
    while out.len() < limit {
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
        end = Some(oldest - 1);
    }
    Ok(out)
}

pub fn fetch_binance(
    symbol: &str,
    interval: &str,
    limit: usize,
    end_time_ms: Option<i64>,
) -> Result<OhlcvData, String> {
    let client = reqwest::blocking::Client::new();
    let bars = paginate_backwards(limit, end_time_ms, |page_limit, end| {
        fetch_binance_page(
            &client,
            &binance_klines_url(symbol, interval, page_limit, end),
        )
    })?;

    if bars.is_empty() && end_time_ms.is_none() {
        return Err("Binance returned zero bars".into());
    }

    Ok(OhlcvData {
        bars,
        symbol: None,
        interval: None,
    })
}

fn fetch_binance_page(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<Bar>, String> {
    let resp = client
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
) -> Result<OhlcvData, String> {
    let window = yahoo_window(interval, limit, end_time_ms, now_ms() / 1000);
    let needs_aggregation = interval == "4h" && window.api_interval == "1h";
    let empty = OhlcvData {
        bars: Vec::new(),
        symbol: None,
        interval: None,
    };

    if window.period1 >= window.period2 {
        // Entire window lies beyond Yahoo's intraday lookback.
        return match end_time_ms {
            Some(_) => Ok(empty),
            None => Err("Requested window is outside Yahoo's history limit".into()),
        };
    }

    let client = reqwest::blocking::Client::new();
    let url = yahoo_chart_url(symbol, &window);

    let resp = client
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

    let result = body
        .get("chart")
        .and_then(|c| c.get("result"))
        .and_then(|r| r.as_array())
        .and_then(|a| a.first())
        .ok_or("Yahoo response missing chart.result[0]")?;

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
        if end_time_ms.is_some_and(|e| ts * 1000 >= e) {
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

    if needs_aggregation {
        bars = aggregate_bars(&bars, 4 * 3600, window.period1);
    }

    if bars.is_empty() && end_time_ms.is_none() {
        return Err("Yahoo returned zero bars".into());
    }

    // Take last N bars
    if bars.len() > limit {
        bars = bars.split_off(bars.len() - limit);
    }

    Ok(OhlcvData {
        bars,
        symbol: None,
        interval: None,
    })
}

/// Yahoo chart request window: unix-second `period1..period2` sized to hold
/// `limit` bars of `interval`.
#[derive(Debug, PartialEq, Eq)]
struct YahooWindow<'a> {
    api_interval: &'a str,
    period1: i64,
    period2: i64,
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

    let period2 = end_time_ms.map_or(now_s, |e| (e + 999).div_euclid(1000));
    let mut period1 = period2 - span;

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
        period1 = period1.max(now_s - a + 3_600);
    }

    YahooWindow {
        api_interval,
        period1,
        period2,
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

/// Group consecutive bars into epoch-aligned `bucket_secs` buckets (1h → 4h).
/// Grid alignment keeps bucket boundaries identical across overlapping
/// requests, which history paging relies on. Buckets starting before
/// `min_start` (unix seconds) may be missing members and are dropped.
fn aggregate_bars(bars: &[Bar], bucket_secs: i64, min_start: i64) -> Vec<Bar> {
    let bucket = |b: &Bar| {
        b.date
            .parse::<i64>()
            .ok()
            .map(|t| t.div_euclid(bucket_secs) * bucket_secs)
    };
    bars.chunk_by(|a, b| bucket(a) == bucket(b))
        .filter(|chunk| bucket(&chunk[0]).is_some_and(|s| s >= min_start))
        .map(|chunk| {
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
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR_MS: i64 = 3_600_000;

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
        let bars = paginate_backwards(2500, None, |limit, end| {
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
        let bars = paginate_backwards(5, Some(end), |limit, e| {
            first_end.get_or_insert(e);
            exchange(limit, e)
        })
        .unwrap();
        assert_eq!(first_end, Some(Some(end - 1)));
        assert_eq!(bars.len(), 5);
        assert_eq!(bar_open_ms(bars.last().unwrap()), Some(end - HOUR_MS));

        // Even if the exchange ignores endTime, nothing >= end_time leaks.
        let bars = paginate_backwards(5, Some(end), |limit, _| fake_exchange(0, last)(limit, None))
            .unwrap();
        assert!(bars.iter().all(|b| bar_open_ms(b).unwrap() < end));
    }

    #[test]
    fn paginate_stops_when_history_runs_out() {
        let first = 1_600_000_000_000 / HOUR_MS * HOUR_MS;
        let last = first + 1499 * HOUR_MS;
        let mut calls = 0;
        let mut exchange = fake_exchange(first, last);
        let bars = paginate_backwards(3000, None, |limit, end| {
            calls += 1;
            exchange(limit, end)
        })
        .unwrap();
        assert_eq!(bars.len(), 1500);
        assert_eq!(calls, 2);
        assert_eq!(bar_open_ms(&bars[0]), Some(first));

        // Paging past the listing date yields zero bars, not an error.
        let bars = paginate_backwards(10, Some(first), fake_exchange(first, last)).unwrap();
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
}
