use crate::data::{Bar, OhlcvData};
use crate::indicator::{rgba, Bars as BarsSeries, Dot, Fill, HLine, Indicator, Line, PanelResult};
use std::time::Duration;

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

fn map_interval(interval: &str) -> &str {
    match interval {
        "1m" | "5m" => "5m",
        "15m" => "15m",
        "30m" => "30m",
        "1h" | "60m" => "1h",
        "2h" => "2h",
        "4h" => "4h",
        "6h" => "6h",
        "12h" => "12h",
        "1d" | "1wk" | "1mo" => "1d",
        _ => "4h",
    }
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
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("Funding rate request failed: {}", e))?;
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

        let series = match fetch_funding_data(sym) {
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
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("OI request failed: {}", e))?;
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

        let series = match fetch_oi_data(sym, interval) {
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
                y: aligned.clone(),
                color: rgba(0x2196F3, 1.0),
                width: 1,
                label: Some("OI".into()),
            }],
            fills: vec![Fill {
                y1: aligned,
                y2: zeros,
                color: rgba(0x2196F3, 0.15),
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
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("L/S request failed: {}", e))?;
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

        let series = match fetch_ls_data(sym, interval) {
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
                    y: long_aligned.clone(),
                    color: rgba(0x26A69A, 1.0),
                    width: 1,
                    label: Some("Long".into()),
                },
                Line {
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
                });
            }
        }

        PanelResult {
            lines: vec![Line {
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
    let base_ms: i64 = match interval {
        "1m" => 60_000,
        "5m" => 5 * 60_000,
        "15m" => 15 * 60_000,
        "30m" => 30 * 60_000,
        "1h" | "60m" => 60 * 60_000,
        "2h" => 2 * 60 * 60_000,
        "4h" => 4 * 60 * 60_000,
        "6h" => 6 * 60 * 60_000,
        "12h" => 12 * 60 * 60_000,
        "1d" => 24 * 60 * 60_000,
        "1wk" => 7 * 24 * 60 * 60_000,
        "1mo" => 31 * 24 * 60 * 60_000,
        _ => 4 * 60 * 60_000,
    };
    // Allow 2x the interval as max gap
    base_ms * 2
}
