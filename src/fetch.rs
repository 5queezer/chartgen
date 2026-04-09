use crate::data::{Bar, OhlcvData};

pub fn fetch_binance(symbol: &str, interval: &str, limit: usize) -> Result<OhlcvData, String> {
    let client = reqwest::blocking::Client::new();
    let url = format!(
        "https://api.binance.com/api/v3/klines?symbol={}&interval={}&limit={}",
        symbol.to_uppercase(),
        interval,
        limit
    );

    let resp = client
        .get(&url)
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

    if bars.is_empty() {
        return Err("Binance returned zero bars".into());
    }

    Ok(OhlcvData {
        bars,
        symbol: None,
        interval: None,
    })
}

pub fn fetch_yahoo(symbol: &str, interval: &str, limit: usize) -> Result<OhlcvData, String> {
    let (api_interval, range) = yahoo_interval_range(interval);
    let needs_aggregation = interval == "4h" && api_interval == "1h";

    let client = reqwest::blocking::Client::new();
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&range={}",
        symbol, api_interval, range
    );

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

    let timestamps = result
        .get("timestamp")
        .and_then(|t| t.as_array())
        .ok_or("Yahoo response missing timestamps")?;

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
        bars = aggregate_bars(&bars, 4);
    }

    if bars.is_empty() {
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

fn yahoo_interval_range(interval: &str) -> (&str, &str) {
    match interval {
        "1m" | "2m" | "5m" | "15m" | "30m" => (interval, "5d"),
        "1h" | "60m" | "90m" => (interval, "1mo"),
        "4h" => ("1h", "6mo"),
        "1d" => ("1d", "1y"),
        "1wk" => ("1wk", "5y"),
        "1mo" => ("1mo", "max"),
        _ => (interval, "1y"),
    }
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

fn aggregate_bars(bars: &[Bar], period: usize) -> Vec<Bar> {
    bars.chunks(period)
        .filter(|chunk| !chunk.is_empty())
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
