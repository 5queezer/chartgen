//! An honest look at the pattern detectors, without any network.
//!
//! Two sources. A random walk with gaps and wicks has no structure at all: every
//! pattern on it is a chance formation, so its count is a false positive rate and
//! its directional hit rate should sit near 50%. The e2e sample source is a smooth
//! wave, not a market: its counts show what the screenshots show, and its hit rates
//! only show that a wave keeps going, so they are not printed as an edge.
//! Prints what guides/tradingview-parity.md quotes.
//!
//! Usage: `cargo run --example pattern_stats`

use chartgen::data::{sample_ohlcv, Bar};
use chartgen::indicator::{Bias, PatternStatus};
use chartgen::indicators::candle_patterns::{self, TrendRule, PATTERNS};
use chartgen::indicators::chart_patterns::{self, Settings, KINDS};
use std::collections::BTreeMap;

const SYMBOLS: &[&str] = &["BTCUSDT", "ETHUSDT", "SOLUSDT", "AAPL", "MSFT", "EURUSD"];
const INTERVALS: &[&str] = &["15m", "1h", "4h", "1d"];
const BARS: usize = 5_000;
/// Far enough back that the forming bar plays no part.
const NOW_MS: i64 = 1_750_000_000_000;
const HORIZON: usize = 10;

fn sample_series() -> Vec<Vec<Bar>> {
    let mut out = vec![];
    for s in SYMBOLS {
        for iv in INTERVALS {
            out.push(
                sample_ohlcv(s, iv, BARS, None, NOW_MS)
                    .expect("sample")
                    .bars,
            );
        }
    }
    out
}

/// A small LCG, so a run is repeatable without a dependency.
struct Rng(u64);
impl Rng {
    fn unit(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f64 + 0.5) / (1u64 << 31) as f64
    }
    fn normal(&mut self) -> f64 {
        (-2.0 * self.unit().ln()).sqrt() * (2.0 * std::f64::consts::PI * self.unit()).cos()
    }
}

/// `n` bars of a random walk: 1% a bar, a gap between bars on a fifth of them when
/// `gaps` (a session market), wicks of the same size. No drift, no memory.
fn random_walk(seed: u64, n: usize, gaps: bool) -> Vec<Bar> {
    let mut r = Rng(seed);
    let mut close = 100.0_f64;
    (0..n)
        .map(|i| {
            let gap = if gaps && r.unit() < 0.2 {
                0.005 * r.normal()
            } else {
                0.0
            };
            let open = close * (1.0 + gap);
            close = open * (1.0 + 0.01 * r.normal());
            let high = open.max(close) * (1.0 + 0.005 * r.normal().abs());
            let low = open.min(close) * (1.0 - 0.005 * r.normal().abs());
            Bar {
                open,
                high,
                low,
                close,
                volume: 1.0,
                date: i.to_string(),
            }
        })
        .collect()
}

fn walks() -> Vec<Vec<Bar>> {
    (0..24)
        .map(|k| random_walk(0x5EED + k as u64 * 7919, BARS, k % 2 == 1))
        .collect()
}

/// True when the close `HORIZON` bars on is on the pattern's side of the close at
/// `at`; `None` where there are too few bars or the pattern has no side.
fn followed(bars: &[Bar], at: usize, bias: Bias) -> Option<bool> {
    let later = bars.get(at + HORIZON)?;
    match bias {
        Bias::Bullish => Some(later.close > bars[at].close),
        Bias::Bearish => Some(later.close < bars[at].close),
        Bias::Neutral => None,
    }
}

fn candles(title: &str, all: &[Vec<Bar>]) {
    let total: usize = all.iter().map(Vec::len).sum();
    let mut rows: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
    for bars in all {
        for h in candle_patterns::detect(bars, "all", TrendRule::Sma50) {
            let d = &PATTERNS[h.def];
            let r = rows.entry(d.key).or_default();
            r.0 += 1;
            if let Some(f) = followed(bars, h.bar, d.bias) {
                r.1 += 1;
                r.2 += usize::from(f);
            }
        }
    }
    println!("Candlestick patterns on {total} bars of {title}, trend rule SMA 50");
    println!(
        "{:<26}{:>10}{:>16}",
        "pattern", "per 1000", "right after 10"
    );
    let mut sum = 0usize;
    let (mut judged, mut right) = (0usize, 0usize);
    for d in PATTERNS {
        let (n, j, r) = rows.get(d.key).copied().unwrap_or_default();
        sum += n;
        judged += j;
        right += r;
        let hit = if j > 0 {
            format!("{:.1}%", 100.0 * r as f64 / j as f64)
        } else {
            "-".into()
        };
        println!(
            "{:<26}{:>10.2}{:>16}",
            d.key,
            1000.0 * n as f64 / total as f64,
            hit
        );
    }
    println!(
        "all: {:.1} labels per 1000 bars; directional ones right after {HORIZON} bars: {:.1}% of {judged}\n",
        1000.0 * sum as f64 / total as f64,
        100.0 * right as f64 / judged.max(1) as f64
    );
}

fn charts(title: &str, all: &[Vec<Bar>]) {
    let s = Settings::default();
    let (mut bars_seen, mut rows): (usize, BTreeMap<&str, [usize; 6]>) = (0, BTreeMap::new());
    for bars in all {
        // Windows of 600 bars, as the indicator reads, side by side.
        for w in bars.chunks(chart_patterns::LOOKBACK) {
            bars_seen += w.len();
            for f in chart_patterns::detect(w, &s, "all") {
                let r = rows.entry(f.kind.key()).or_default();
                r[0] += 1;
                match f.status {
                    PatternStatus::Forming => r[1] += 1,
                    PatternStatus::Confirmed => r[2] += 1,
                    PatternStatus::Failed => r[3] += 1,
                }
                if let Some(at) = f.confirmed_at {
                    if let Some(ok) = followed(w, at, f.bias) {
                        r[4] += 1;
                        r[5] += usize::from(ok);
                    }
                }
            }
        }
    }
    println!(
        "Chart patterns on {bars_seen} bars of {title}, depth {} deviation {} tolerance {}",
        s.depth, s.deviation, s.tolerance
    );
    println!(
        "{:<28}{:>10}{:>10}{:>10}{:>10}{:>16}",
        "pattern", "per 1000", "forming", "confirmed", "failed", "right after 10"
    );
    let mut sum = [0usize; 6];
    for k in KINDS {
        let r = rows.get(k.key()).copied().unwrap_or_default();
        for (a, b) in sum.iter_mut().zip(r) {
            *a += b;
        }
        let hit = if r[4] > 0 {
            format!("{:.1}%", 100.0 * r[5] as f64 / r[4] as f64)
        } else {
            "-".into()
        };
        println!(
            "{:<28}{:>10.2}{:>10}{:>10}{:>10}{:>16}",
            k.key(),
            1000.0 * r[0] as f64 / bars_seen as f64,
            r[1],
            r[2],
            r[3],
            hit
        );
    }
    println!(
        "all: {:.2} per 1000 bars; right after {HORIZON} bars: {:.1}% of {} confirmations\n",
        1000.0 * sum[0] as f64 / bars_seen as f64,
        100.0 * sum[5] as f64 / sum[4].max(1) as f64,
        sum[4]
    );
}

/// How often a pattern, once confirmed, disappears or changes as bars arrive:
/// the honest test of "no silent repaint". The window slides like a live chart's.
fn stability(all: &[Vec<Bar>]) {
    let s = Settings::default();
    let (mut confirmed, mut vanished) = (0usize, 0usize);
    for bars in all.iter().step_by(3) {
        let bars = &bars[bars.len() - 2000..];
        let mut seen: BTreeMap<(usize, &str), bool> = BTreeMap::new();
        for end in (chart_patterns::LOOKBACK..bars.len()).step_by(5) {
            let from = end - chart_patterns::LOOKBACK;
            let now = chart_patterns::detect(&bars[from..end], &s, "all");
            let keys: Vec<(usize, &str, PatternStatus)> = now
                .iter()
                .filter_map(|f| f.confirmed_at.map(|at| (at + from, f.kind.key(), f.status)))
                .collect();
            for (at, key, _) in &keys {
                seen.entry((*at, key)).or_insert(true);
            }
            // Old ones scroll out of the window by design: only judge the newer half.
            for ((at, key), alive) in seen.iter_mut() {
                let inside = *at > from + chart_patterns::LOOKBACK / 2;
                let has = keys.iter().any(|(a, k, _)| a == at && k == key);
                if inside && !has && *alive {
                    *alive = false;
                }
            }
        }
        confirmed += seen.len();
        vanished += seen.values().filter(|alive| !**alive).count();
    }
    println!(
        "Stability: {confirmed} confirmed patterns over sliding windows, {vanished} later vanished from the newer half of the window ({:.1}%)\n",
        100.0 * vanished as f64 / confirmed.max(1) as f64
    );
}

fn main() {
    let walk = walks();
    candles("random walk", &walk);
    charts("random walk", &walk);
    stability(&walk);
    let sample = sample_series();
    candles("the sample source", &sample);
    charts("the sample source", &sample);
}
