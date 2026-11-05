/// OHLCV bar and data container.

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Bar {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub date: String,
}

#[derive(Clone, Debug)]
pub struct OhlcvData {
    pub bars: Vec<Bar>,
    pub symbol: Option<String>,
    pub interval: Option<String>,
}

impl OhlcvData {
    /// Heikin Ashi transform: haClose = ohlc4, haOpen = (prev haOpen + prev
    /// haClose) / 2 seeded with (open + close) / 2, haHigh/haLow widened to
    /// contain haOpen/haClose. Volume and timestamps are kept.
    pub fn heikin_ashi(&self) -> OhlcvData {
        let mut prev: Option<(f64, f64)> = None;
        let bars = self
            .bars
            .iter()
            .map(|b| {
                let ha_close = (b.open + b.high + b.low + b.close) / 4.0;
                let ha_open = match prev {
                    Some((o, c)) => (o + c) / 2.0,
                    None => (b.open + b.close) / 2.0,
                };
                prev = Some((ha_open, ha_close));
                Bar {
                    open: ha_open,
                    high: b.high.max(ha_open).max(ha_close),
                    low: b.low.min(ha_open).min(ha_close),
                    close: ha_close,
                    volume: b.volume,
                    date: b.date.clone(),
                }
            })
            .collect();
        OhlcvData {
            bars,
            symbol: self.symbol.clone(),
            interval: self.interval.clone(),
        }
    }

    pub fn closes(&self) -> Vec<f64> {
        self.bars.iter().map(|b| b.close).collect()
    }
    pub fn highs(&self) -> Vec<f64> {
        self.bars.iter().map(|b| b.high).collect()
    }
    pub fn lows(&self) -> Vec<f64> {
        self.bars.iter().map(|b| b.low).collect()
    }
    pub fn len(&self) -> usize {
        self.bars.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }
}

/// Simple LCG PRNG (no external dep needed).
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_f64(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f64) / (u32::MAX as f64)
    }
    /// Approximate normal via Box-Muller.
    fn normal(&mut self, mean: f64, std: f64) -> f64 {
        let u1 = self.next_f64().max(1e-10);
        let u2 = self.next_f64();
        mean + std * (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

pub fn sample_data(n: usize) -> OhlcvData {
    let mut rng = Rng::new(42);
    let mut price = 30.0_f64;
    let mut bars = Vec::with_capacity(n);

    for i in 0..n {
        let o = price + rng.normal(0.0, 0.5);
        let c = o + rng.normal(-0.1, 1.0);
        let h = o.max(c) + rng.normal(0.0, 0.5).abs();
        let l = o.min(c) - rng.normal(0.0, 0.5).abs();
        let v = rng.normal(10.0, 0.8).exp();
        bars.push(Bar {
            open: o,
            high: h,
            low: l,
            close: c,
            volume: v,
            date: format!("D{}", i),
        });
        price = c;
    }
    OhlcvData {
        bars,
        symbol: None,
        interval: None,
    }
}

/// Length of a supported bar interval in seconds: the union of Binance and
/// Yahoo timeframe names ("1m", "4h", "1d", "1w"/"1wk", "1M"/"1mo", ...).
/// Months are approximated as 30 days. `None` for anything else, so callers
/// can reject the timeframe instead of doing arithmetic on it.
pub fn interval_secs(interval: &str) -> Option<i64> {
    const MIN: i64 = 60;
    const HOUR: i64 = 3_600;
    const DAY: i64 = 86_400;
    Some(match interval {
        "1m" => MIN,
        "2m" => 2 * MIN,
        "3m" => 3 * MIN,
        "5m" => 5 * MIN,
        "15m" => 15 * MIN,
        "30m" => 30 * MIN,
        "60m" | "1h" => HOUR,
        "90m" => 90 * MIN,
        "2h" => 2 * HOUR,
        "4h" => 4 * HOUR,
        "6h" => 6 * HOUR,
        "8h" => 8 * HOUR,
        "12h" => 12 * HOUR,
        "1d" => DAY,
        "3d" => 3 * DAY,
        "5d" => 5 * DAY,
        "1w" | "1wk" => 7 * DAY,
        "1M" | "1mo" => 30 * DAY,
        "3mo" => 90 * DAY,
        _ => return None,
    })
}

/// Bar length in seconds for indicator math (HV annualization, VWAP and
/// pivot anchoring). Unlike the strict `interval_secs` (which gates what the
/// data sources accept), this takes any multiple and any letter case:
/// "1D", "1W", "2w", "4H", "1y", "D". A capital `M` means months and a
/// lowercase `m` minutes (TradingView / Binance convention); "mo" is months
/// in any case. Months are 30 days, years 365. `None` when unrecognised:
/// callers treat that as an unknown timeframe, not as intraday.
pub fn bar_interval_secs(interval: &str) -> Option<i64> {
    const DAY: i64 = 86_400;
    let s = interval.trim();
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let k: i64 = if num.is_empty() { 1 } else { num.parse().ok()? };
    let unit_secs = match unit {
        "m" => 60,
        "M" => 30 * DAY,
        u => match u.to_ascii_lowercase().as_str() {
            "s" => 1,
            "min" => 60,
            "h" => 3_600,
            "d" => DAY,
            "w" | "wk" => 7 * DAY,
            "mo" => 30 * DAY,
            "y" => 365 * DAY,
            _ => return None,
        },
    };
    (k > 0).then(|| k.checked_mul(unit_secs)).flatten()
}

/// Offset of the bar grid from the epoch: weekly bars open on Monday
/// (1970-01-01 was a Thursday), everything else is epoch-aligned.
fn grid_offset_ms(step: i64) -> i64 {
    if step == 7 * 86_400 {
        4 * 86_400_000
    } else {
        0
    }
}

/// splitmix64 finalizer: a stateless hash so prices are a pure function of
/// (symbol, timestamp).
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Uniform in [0, 1) from a hash.
fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Close of the completed bar opening at `t` (unix seconds): three
/// sinusoids in bar units plus per-bar hash noise.
fn grid_close(seed: u64, t: i64, step: i64) -> f64 {
    use std::f64::consts::TAU;
    let x = t as f64 / step as f64;
    let phase = unit(mix(seed)) * TAU;
    let base = 50.0 + (seed % 200) as f64;
    let wave = 0.25 * (x / 600.0 * TAU + phase).sin()
        + 0.12 * (x / 150.0 * TAU + 2.0 * phase).sin()
        + 0.05 * (x / 37.0 * TAU + 3.0 * phase).sin();
    let noise = (unit(mix(seed ^ t as u64)) - 0.5) * 0.02;
    base * (1.0 + wave + noise)
}

/// Deterministic, network-free OHLCV source (`CHARTGEN_DATA_SOURCE=sample`).
///
/// Bars sit on the epoch-aligned `interval` grid (weeks start on Monday;
/// months are a fixed 30-day approximation). The newest bar is the last grid slot whose
/// open time is strictly before `end_time_ms`, capped at the bar forming at
/// `now_ms`. Every completed bar is a pure function of its open time and
/// `symbol`, so overlapping pages agree exactly. The forming bar moves
/// towards its final close by elapsed time and wobbles with wall-clock
/// seconds so a live poll observes change. No bars exist before the unix
/// epoch, so paging eventually runs out of history.
pub fn sample_ohlcv(
    symbol: &str,
    interval: &str,
    count: usize,
    end_time_ms: Option<i64>,
    now_ms: i64,
) -> Result<OhlcvData, String> {
    let step = interval_secs(interval).ok_or(format!("Unsupported interval: {interval}"))?;
    let step_ms = step * 1000;
    // FNV-1a: stable across Rust versions, unlike DefaultHasher.
    let seed = symbol.bytes().fold(0xCBF2_9CE4_8422_2325_u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01B3)
    });

    let off = grid_offset_ms(step);
    let slot = |ms: i64| {
        ms.saturating_sub(off)
            .div_euclid(step_ms)
            .saturating_mul(step_ms)
            .saturating_add(off)
    };
    let current_open_ms = slot(now_ms);
    let last_open_ms = match end_time_ms {
        Some(end) => slot(end.saturating_sub(1)).min(current_open_ms),
        None => current_open_ms,
    };
    let available = if last_open_ms < 0 {
        0
    } else {
        ((last_open_ms - off) / step_ms + 1) as usize
    };
    let n = count.min(available);

    let bars = (0..n)
        .map(|i| {
            let open_ms = last_open_ms - (n - 1 - i) as i64 * step_ms;
            let t = open_ms / 1000;
            let o = grid_close(seed, t - step, step);
            let c = grid_close(seed, t, step);
            let wick_hi = unit(mix(seed ^ t as u64 ^ 0x4849)) * 0.006;
            let wick_lo = unit(mix(seed ^ t as u64 ^ 0x4C4F)) * 0.006;
            let h = o.max(c) * (1.0 + wick_hi);
            let l = o.min(c) * (1.0 - wick_lo);
            let v = 1_000.0 * (1.0 + 4.0 * unit(mix(seed ^ t as u64 ^ 0x564F)));
            if open_ms != current_open_ms {
                return Bar {
                    open: o,
                    high: h,
                    low: l,
                    close: c,
                    volume: v,
                    date: t.to_string(),
                };
            }
            let frac = ((now_ms - open_ms) as f64 / step_ms as f64).clamp(0.0, 1.0);
            let wobble = o * 0.003 * (now_ms as f64 / 1000.0).floor().sin();
            let close = o + (c - o) * frac + wobble;
            Bar {
                open: o,
                high: (o.max(c) + (h - o.max(c)) * frac).max(close),
                low: (o.min(c) - (o.min(c) - l) * frac).min(close),
                close,
                volume: v * frac,
                date: t.to_string(),
            }
        })
        .collect();

    Ok(OhlcvData {
        bars,
        symbol: None,
        interval: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR_MS: i64 = 3_600_000;

    #[test]
    fn bar_interval_secs_is_case_insensitive_and_strict_about_unknowns() {
        const DAY: i64 = 86_400;
        assert_eq!(bar_interval_secs("1D"), Some(DAY));
        assert_eq!(bar_interval_secs("1d"), Some(DAY));
        assert_eq!(bar_interval_secs("D"), Some(DAY));
        assert_eq!(bar_interval_secs("1W"), Some(7 * DAY));
        assert_eq!(bar_interval_secs("2w"), Some(14 * DAY));
        assert_eq!(bar_interval_secs("1WK"), Some(7 * DAY));
        assert_eq!(bar_interval_secs("4H"), Some(14_400));
        assert_eq!(bar_interval_secs("15m"), Some(900));
        assert_eq!(bar_interval_secs("1M"), Some(30 * DAY));
        assert_eq!(bar_interval_secs("3MO"), Some(90 * DAY));
        assert_eq!(bar_interval_secs("1y"), Some(365 * DAY));
        assert_eq!(bar_interval_secs("0d"), None);
        assert_eq!(bar_interval_secs("1x"), None);
        assert_eq!(bar_interval_secs(""), None);
        assert_eq!(bar_interval_secs("99999999999999999999d"), None);
    }

    #[test]
    fn interval_secs_parses_binance_and_yahoo_forms() {
        assert_eq!(interval_secs("1m"), Some(60));
        assert_eq!(interval_secs("90m"), Some(5_400));
        assert_eq!(interval_secs("4h"), Some(14_400));
        assert_eq!(interval_secs("1d"), Some(86_400));
        assert_eq!(interval_secs("1w"), Some(604_800));
        assert_eq!(interval_secs("1wk"), Some(604_800));
        assert_eq!(interval_secs("1M"), Some(2_592_000));
        assert_eq!(interval_secs("1mo"), Some(2_592_000));
        assert_eq!(interval_secs("h"), None);
        assert_eq!(interval_secs("0h"), None);
        assert_eq!(interval_secs("1x"), None);
        // Absurd multiples are rejected rather than overflowing.
        assert_eq!(interval_secs("4611686018427387904m"), None);
        assert_eq!(interval_secs("7h"), None);
    }

    #[test]
    fn sample_ohlcv_aligns_to_grid_and_respects_end_time() {
        let now = 1_750_000_123_456;
        // end_time off the grid: last bar is the slot strictly before it.
        let end = 1_700_000_000_000 + 1_234;
        let d = sample_ohlcv("BTCUSDT", "4h", 50, Some(end), now).unwrap();
        assert_eq!(d.len(), 50);
        let ts: Vec<i64> = d.bars.iter().map(|b| b.date.parse().unwrap()).collect();
        for w in ts.windows(2) {
            assert_eq!(w[1] - w[0], 14_400);
        }
        assert!(ts.iter().all(|t| t % 14_400 == 0));
        assert!(ts.last().unwrap() * 1000 < end);
        assert!(ts.last().unwrap() * 1000 + 14_400_000 >= end);

        // end_time on the grid is exclusive.
        let on_grid = 1_700_006_400_000;
        assert_eq!(on_grid % (4 * HOUR_MS), 0);
        let d = sample_ohlcv("BTCUSDT", "4h", 3, Some(on_grid), now).unwrap();
        let last: i64 = d.bars.last().unwrap().date.parse().unwrap();
        assert_eq!(last * 1000, on_grid - 4 * HOUR_MS);
    }

    #[test]
    fn sample_ohlcv_serves_every_supported_interval() {
        let now = 1_750_000_123_456;
        for (tf, secs) in [
            ("3m", 180),
            ("2h", 7_200),
            ("6h", 21_600),
            ("12h", 43_200),
            ("3d", 259_200),
            ("1mo", 2_592_000),
        ] {
            let d = sample_ohlcv("BTCUSDT", tf, 20, None, now).unwrap();
            assert_eq!(d.len(), 20, "{tf}");
            let ts: Vec<i64> = d.bars.iter().map(|b| b.date.parse().unwrap()).collect();
            assert!(ts.windows(2).all(|w| w[1] - w[0] == secs), "{tf} spacing");
            assert!(ts.iter().all(|t| t % secs == 0), "{tf} grid");
        }
    }

    #[test]
    fn sample_ohlcv_overlapping_pages_agree() {
        let now = 1_750_000_000_000;
        let end_a = 1_700_000_000_000;
        let a = sample_ohlcv("ETHUSDT", "1h", 100, Some(end_a), now).unwrap();
        let b = sample_ohlcv("ETHUSDT", "1h", 100, Some(end_a - 40 * HOUR_MS), now).unwrap();
        let mut overlap = 0;
        for bb in &b.bars {
            if let Some(ab) = a.bars.iter().find(|x| x.date == bb.date) {
                overlap += 1;
                assert_eq!(
                    (ab.open, ab.high, ab.low, ab.close, ab.volume),
                    (bb.open, bb.high, bb.low, bb.close, bb.volume)
                );
            }
        }
        assert_eq!(overlap, 60);
        // Different symbols produce different prices.
        let c = sample_ohlcv("SOLUSDT", "1h", 100, Some(end_a), now).unwrap();
        assert_ne!(a.bars[0].close, c.bars[0].close);
        // Candles are continuous (open == previous close) and well-formed.
        for w in a.bars.windows(2) {
            assert_eq!(w[1].open, w[0].close);
            assert!(w[1].high >= w[1].open.max(w[1].close));
            assert!(w[1].low <= w[1].open.min(w[1].close));
        }
    }

    #[test]
    fn sample_ohlcv_forming_bar_changes_with_wall_clock() {
        let open = 1_700_000_000_000 / HOUR_MS * HOUR_MS;
        let a = sample_ohlcv("BTCUSDT", "1h", 3, None, open + 600_000).unwrap();
        let b = sample_ohlcv("BTCUSDT", "1h", 3, None, open + 601_000).unwrap();
        assert_eq!(a.bars.last().unwrap().date, (open / 1000).to_string());
        // Completed bars identical, forming bar moved.
        assert_eq!(a.bars[1].close, b.bars[1].close);
        assert_ne!(a.bars[2].close, b.bars[2].close);
        assert_eq!(a.bars[2].open, b.bars[2].open);
        for bar in [&a.bars[2], &b.bars[2]] {
            assert!(bar.high >= bar.open.max(bar.close));
            assert!(bar.low <= bar.open.min(bar.close));
        }
        // end_time in the future is capped at the forming bar.
        let c = sample_ohlcv(
            "BTCUSDT",
            "1h",
            3,
            Some(open + 10 * HOUR_MS),
            open + 600_000,
        )
        .unwrap();
        assert_eq!(c.bars.last().unwrap().date, a.bars[2].date);
    }

    #[test]
    fn sample_ohlcv_weekly_bars_open_on_monday() {
        let d = sample_ohlcv("X", "1wk", 10, None, 1_750_000_000_000).unwrap();
        assert_eq!(d.len(), 10);
        for b in &d.bars {
            let day = b.date.parse::<i64>().unwrap() / 86_400;
            // Day 0 (1970-01-01) was a Thursday, so Mondays are day % 7 == 4.
            assert_eq!(day % 7, 4, "weekly bar must open on Monday");
        }
        // The first weekly bar after the epoch is Monday 1970-01-05.
        let d = sample_ohlcv("X", "1w", 5, Some(15 * 86_400_000), 1_750_000_000_000).unwrap();
        assert_eq!(d.bars[0].date, (4 * 86_400).to_string());
        assert_eq!(d.len(), 2);
        // end_time = i64::MIN saturates instead of overflowing.
        assert!(sample_ohlcv("X", "1h", 5, Some(i64::MIN), 0)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn sample_ohlcv_stops_at_epoch() {
        let d = sample_ohlcv("X", "1h", 50, Some(10 * HOUR_MS), 1_750_000_000_000).unwrap();
        assert_eq!(d.len(), 10);
        assert_eq!(d.bars[0].date, "0");
        assert!(sample_ohlcv("X", "1h", 5, Some(0), 1_750_000_000_000)
            .unwrap()
            .is_empty());
        assert!(sample_ohlcv("X", "7x", 5, None, 0).is_err());
    }
}
