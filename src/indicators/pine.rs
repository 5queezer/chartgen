//! Shared Pine Script v5 semantics for the TradingView-parity indicators.
//!
//! NaN rules (Pine `na`):
//! - Recursive smoothers (`ema_n` / `rma_n`) seed with the SMA of the first
//!   `n` valid values, then keep their state across a NaN (or infinite) input:
//!   that bar outputs NaN and the recursion continues from the previous state.
//! - Windowed helpers (`sum_n`, `sma_n`, `wma_n`, `highest`, `stdev`, ...)
//!   output NaN while any NaN (or infinite) value lies inside the window.
//! - Division by zero yields NaN (`div`), never inf.
//!
//! Warmup values are NaN (serialized as null), never 0.

use crate::data::OhlcvData;
use crate::indicator::{rgba, Bars, Line};

// ---- Series helpers ----

/// Source series as selected by the TradingView `source` input.
pub fn source_series(data: &OhlcvData, src: &str) -> Vec<f64> {
    data.bars
        .iter()
        .map(|b| match src {
            "open" => b.open,
            "high" => b.high,
            "low" => b.low,
            "hl2" => (b.high + b.low) / 2.0,
            "hlc3" => (b.high + b.low + b.close) / 3.0,
            "ohlc4" => (b.open + b.high + b.low + b.close) / 4.0,
            "hlcc4" => (b.high + b.low + 2.0 * b.close) / 4.0,
            _ => b.close,
        })
        .collect()
}

/// Apply `f` to every full window of `n` values; NaN if the window holds a NaN.
pub fn windowed(x: &[f64], n: usize, f: impl Fn(&[f64]) -> f64) -> Vec<f64> {
    let mut out = vec![f64::NAN; x.len()];
    if n == 0 {
        return out;
    }
    for i in (n - 1)..x.len() {
        let w = &x[i + 1 - n..=i];
        if w.iter().all(|v| v.is_finite()) {
            out[i] = f(w);
        }
    }
    out
}

/// Running-sum window total, NaN until `n` consecutive finite values exist (O(n)).
pub fn sum_n(x: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; x.len()];
    if n == 0 {
        return out;
    }
    let (mut sum, mut bad) = (0.0, 0usize);
    for i in 0..x.len() {
        if x[i].is_finite() {
            sum += x[i];
        } else {
            bad += 1;
        }
        if i >= n {
            let old = x[i - n];
            if old.is_finite() {
                sum -= old;
            } else {
                bad -= 1;
            }
        }
        if i + 1 >= n && bad == 0 {
            out[i] = sum;
        }
    }
    out
}

/// ta.sma: running sum, NaN until `n` consecutive valid values exist.
pub fn sma_n(x: &[f64], n: usize) -> Vec<f64> {
    sum_n(x, n).into_iter().map(|s| s / n as f64).collect()
}

/// Exponential smoothing seeded with the SMA (ta.ema uses alpha = 2/(n+1),
/// ta.rma uses alpha = 1/n).
pub fn smoothed(x: &[f64], n: usize, alpha: f64) -> Vec<f64> {
    let mut out = vec![f64::NAN; x.len()];
    let seed = sma_n(x, n);
    let mut prev = f64::NAN;
    for i in 0..x.len() {
        if prev.is_nan() {
            prev = seed[i];
            out[i] = prev;
        } else if x[i].is_finite() {
            prev = alpha * x[i] + (1.0 - alpha) * prev;
            out[i] = prev;
        }
    }
    out
}

pub fn ema_n(x: &[f64], n: usize) -> Vec<f64> {
    smoothed(x, n, 2.0 / (n as f64 + 1.0))
}

pub fn rma_n(x: &[f64], n: usize) -> Vec<f64> {
    smoothed(x, n, 1.0 / n.max(1) as f64)
}

/// ta.wma: linear weights, newest bar weighs `n`.
pub fn wma_n(x: &[f64], n: usize) -> Vec<f64> {
    let norm = (n * (n + 1)) as f64 / 2.0;
    windowed(x, n, |w| {
        w.iter()
            .enumerate()
            .map(|(i, v)| v * (i + 1) as f64)
            .sum::<f64>()
            / norm
    })
}

/// Least-squares line through the window (x = 0..n-1, oldest first);
/// returns (intercept, slope).
pub fn linfit(w: &[f64]) -> (f64, f64) {
    let n = w.len() as f64;
    let (mut sx, mut sy, mut sxy, mut sxx) = (0.0, 0.0, 0.0, 0.0);
    for (i, &y) in w.iter().enumerate() {
        let x = i as f64;
        sx += x;
        sy += y;
        sxy += x * y;
        sxx += x * x;
    }
    let den = n * sxx - sx * sx;
    let slope = if den == 0.0 {
        0.0
    } else {
        (n * sxy - sx * sy) / den
    };
    ((sy - slope * sx) / n, slope)
}

/// ta.linreg(src, n, 0): regression value at the newest bar of each window.
pub fn linreg_n(x: &[f64], n: usize) -> Vec<f64> {
    windowed(x, n, |w| {
        let (a, b) = linfit(w);
        a + b * (w.len() - 1) as f64
    })
}

/// ta.alma with offset 0.85, sigma 6.
pub fn alma_n(x: &[f64], n: usize) -> Vec<f64> {
    let m = 0.85 * (n as f64 - 1.0); // ta.alma floor=false
    let s = n as f64 / 6.0;
    let weights: Vec<f64> = (0..n)
        .map(|i| (-((i as f64 - m).powi(2)) / (2.0 * s * s)).exp())
        .collect();
    let norm: f64 = weights.iter().sum();
    windowed(x, n, |w| {
        w.iter().zip(&weights).map(|(v, k)| v * k).sum::<f64>() / norm
    })
}

/// ta.vwma: sma(src*volume, n) / sma(volume, n).
pub fn vwma_n(x: &[f64], vol: &[f64], n: usize) -> Vec<f64> {
    let xv: Vec<f64> = x.iter().zip(vol).map(|(a, b)| a * b).collect();
    let (num, den) = (sma_n(&xv, n), sma_n(vol, n));
    num.iter()
        .zip(&den)
        .map(|(a, b)| if *b == 0.0 { f64::NAN } else { a / b })
        .collect()
}

/// ta.hma: wma(2*wma(x, n/2) - wma(x, n), floor(sqrt(n))).
pub fn hma_n(x: &[f64], n: usize) -> Vec<f64> {
    let half = wma_n(x, (n / 2).max(1));
    let full = wma_n(x, n);
    let raw: Vec<f64> = half.iter().zip(&full).map(|(a, b)| 2.0 * a - b).collect();
    wma_n(&raw, ((n as f64).sqrt().floor() as usize).max(1))
}

/// Dispatch a moving average by TradingView type name.
pub fn ma_by_type(kind: &str, x: &[f64], vol: &[f64], n: usize) -> Vec<f64> {
    match kind {
        "ema" => ema_n(x, n),
        "wma" => wma_n(x, n),
        "hma" => hma_n(x, n),
        "vwma" => vwma_n(x, vol, n),
        "rma" | "smma" => rma_n(x, n),
        "alma" => alma_n(x, n),
        "dema" => {
            let e1 = ema_n(x, n);
            let e2 = ema_n(&e1, n);
            e1.iter().zip(&e2).map(|(a, b)| 2.0 * a - b).collect()
        }
        "tema" => {
            let e1 = ema_n(x, n);
            let e2 = ema_n(&e1, n);
            let e3 = ema_n(&e2, n);
            (0..x.len())
                .map(|i| 3.0 * e1[i] - 3.0 * e2[i] + e3[i])
                .collect()
        }
        "lsma" => linreg_n(x, n),
        _ => sma_n(x, n),
    }
}

/// Chart offset: the value computed at bar i is drawn at bar i + off.
pub fn shift(x: &[f64], off: i64) -> Vec<f64> {
    let mut out = vec![f64::NAN; x.len()];
    for (i, &v) in x.iter().enumerate() {
        let j = i as i64 + off;
        if j >= 0 && (j as usize) < x.len() {
            out[j as usize] = v;
        }
    }
    out
}

pub fn volumes(data: &OhlcvData) -> Vec<f64> {
    data.bars.iter().map(|b| b.volume).collect()
}

pub fn line(y: Vec<f64>, color: u32, width: u32, label: &str) -> Line {
    Line {
        y,
        color: rgba(color, 0.9),
        width,
        label: Some(label.into()),
    }
}

/// NaN-aware max/min: a NaN operand gives NaN (Pine `math.max` / `math.min`),
/// unlike `f64::max` which swallows it.
pub fn nan_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}
pub fn nan_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// True range; the first bar uses high - low (ta.tr(true)).
pub fn true_range(data: &OhlcvData) -> Vec<f64> {
    data.bars
        .iter()
        .enumerate()
        .map(|(i, b)| {
            if i == 0 {
                b.high - b.low
            } else {
                let pc = data.bars[i - 1].close;
                nan_max(
                    nan_max(b.high - b.low, (b.high - pc).abs()),
                    (b.low - pc).abs(),
                )
            }
        })
        .collect()
}

/// Pine `ta.atr`: RMA of the true range (ta.tr(true)).
pub fn atr(data: &OhlcvData, n: usize) -> Vec<f64> {
    rma_n(&true_range(data), n)
}

/// Pine `fixnan`: replace NaN with the last non-NaN value (leading NaNs stay).
pub fn fixnan(x: &[f64]) -> Vec<f64> {
    let mut last = f64::NAN;
    x.iter()
        .map(|&v| {
            if !v.is_nan() {
                last = v;
            }
            last
        })
        .collect()
}

// ---- Oscillator helpers ----

pub fn highest(x: &[f64], n: usize) -> Vec<f64> {
    windowed(x, n, |w| {
        w.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
    })
}
pub fn lowest(x: &[f64], n: usize) -> Vec<f64> {
    windowed(x, n, |w| w.iter().cloned().fold(f64::INFINITY, f64::min))
}
/// Population standard deviation (Pine `ta.stdev`, biased = true).
pub fn stdev(x: &[f64], n: usize) -> Vec<f64> {
    windowed(x, n, |w| {
        let m = w.iter().sum::<f64>() / w.len() as f64;
        (w.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / w.len() as f64).sqrt()
    })
}
/// Pine `ta.dev`: mean absolute deviation from the window's SMA.
pub fn dev(x: &[f64], n: usize) -> Vec<f64> {
    windowed(x, n, |w| {
        let m = w.iter().sum::<f64>() / w.len() as f64;
        w.iter().map(|v| (v - m).abs()).sum::<f64>() / w.len() as f64
    })
}
/// Pine `ta.mfi(src, n)` as written in the Pine reference:
/// `upper = sum(volume * (change(src) <= 0 ? 0 : src), n)`,
/// `lower = sum(volume * (change(src) >= 0 ? 0 : src), n)`,
/// `100 - 100 / (1 + upper / lower)`. On bar 0 the change is na, both
/// comparisons are false, so that bar's flow counts on both sides.
/// A zero `lower` is a division by zero -> NaN.
pub fn mfi(src: &[f64], vol: &[f64], n: usize) -> Vec<f64> {
    let ch = change_n(src, 1);
    let flow = |keep: fn(f64) -> bool| -> Vec<f64> {
        (0..src.len())
            .map(|i| if keep(ch[i]) { vol[i] * src[i] } else { 0.0 })
            .collect()
    };
    // A NaN change keeps the flow, matching Pine's false comparison with na.
    let upper = sum_n(&flow(|c| c > 0.0 || c.is_nan()), n);
    let lower = sum_n(&flow(|c| c < 0.0 || c.is_nan()), n);
    zip(&upper, &lower, |u, l| {
        if l == 0.0 || u.is_nan() || l.is_nan() {
            f64::NAN
        } else {
            100.0 - 100.0 / (1.0 + u / l)
        }
    })
}
/// Pine `ta.swma`: (x[3] + 2*x[2] + 2*x[1] + x[0]) / 6.
pub fn swma(x: &[f64]) -> Vec<f64> {
    windowed(x, 4, |w| (w[0] + 2.0 * w[1] + 2.0 * w[2] + w[3]) / 6.0)
}
/// Pine `math.max(x, 0)` / `-math.min(x, 0)`: na stays na.
pub fn gains_losses(ch: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let g = |f: fn(f64) -> f64| {
        ch.iter()
            .map(|&c| if c.is_nan() { c } else { f(c) })
            .collect()
    };
    (g(|c| c.max(0.0)), g(|c| (-c).max(0.0)))
}
/// `x - x[k]`
pub fn change_n(x: &[f64], k: usize) -> Vec<f64> {
    (0..x.len())
        .map(|i| if i >= k { x[i] - x[i - k] } else { f64::NAN })
        .collect()
}
/// Pine `ta.roc`: 100 * (x - x[k]) / x[k]; a zero base gives NaN, not inf.
pub fn roc(x: &[f64], k: usize) -> Vec<f64> {
    (0..x.len())
        .map(|i| {
            if i >= k && x[i - k] != 0.0 {
                100.0 * (x[i] - x[i - k]) / x[i - k]
            } else {
                f64::NAN
            }
        })
        .collect()
}
/// Pine `ta.rsi`: 100 when the average loss is 0, 0 when the average gain is 0.
pub fn rsi(x: &[f64], n: usize) -> Vec<f64> {
    let (up, dn) = gains_losses(&change_n(x, 1));
    let (u, d) = (rma_n(&up, n), rma_n(&dn, n));
    (0..x.len())
        .map(|i| {
            if u[i].is_nan() || d[i].is_nan() {
                f64::NAN
            } else if d[i] == 0.0 {
                100.0
            } else if u[i] == 0.0 {
                0.0
            } else {
                100.0 - 100.0 / (1.0 + u[i] / d[i])
            }
        })
        .collect()
}
/// Pine `ta.stoch(src, high, low, n)`; zero range -> NaN (division by zero).
pub fn stoch(src: &[f64], high: &[f64], low: &[f64], n: usize) -> Vec<f64> {
    let (hh, ll) = (highest(high, n), lowest(low, n));
    (0..src.len())
        .map(|i| {
            let r = hh[i] - ll[i];
            if r > 0.0 {
                100.0 * (src[i] - ll[i]) / r
            } else {
                f64::NAN
            }
        })
        .collect()
}
/// Pine `ta.percentrank`: share of the previous `n` values <= current.
pub fn percentrank(x: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; x.len()];
    for i in n..x.len() {
        let w = &x[i - n..i];
        if x[i].is_finite() && w.iter().all(|v| v.is_finite()) {
            out[i] = 100.0 * w.iter().filter(|&&v| v <= x[i]).count() as f64 / n as f64;
        }
    }
    out
}
pub fn zip(a: &[f64], b: &[f64], f: impl Fn(f64, f64) -> f64) -> Vec<f64> {
    a.iter().zip(b).map(|(&x, &y)| f(x, y)).collect()
}
/// `x / y` with a zero denominator -> NaN.
pub fn div(a: &[f64], b: &[f64]) -> Vec<f64> {
    zip(a, b, |x, y| if y == 0.0 { f64::NAN } else { x / y })
}
pub fn finite_or_nan(v: Vec<f64>) -> Vec<f64> {
    v.into_iter()
        .map(|x| if x.is_finite() { x } else { f64::NAN })
        .collect()
}
/// MACD-style histogram: dark when growing away from zero, light when fading.
pub fn hist(y: Vec<f64>) -> Bars {
    let colors = (0..y.len())
        .map(|i| {
            let pos = y[i] >= 0.0;
            let rising = i > 0 && y[i] > y[i - 1];
            match (pos, rising) {
                (true, true) => rgba(0x26a69a, 0.9),
                (true, false) => rgba(0xb2dfdb, 0.9),
                (false, false) => rgba(0xff5252, 0.9),
                (false, true) => rgba(0xffcdd2, 0.9),
            }
        })
        .collect();
    Bars {
        y,
        colors,
        bottom: 0.0,
    }
}

/// Fixture builders and a NaN-aware comparison shared by indicator unit tests.
#[cfg(test)]
pub(crate) mod test_util {
    use crate::data::{Bar, OhlcvData};

    pub const NAN: f64 = f64::NAN;

    /// Bars from (open, high, low, close, volume) tuples.
    pub fn bars(v: &[(f64, f64, f64, f64, f64)]) -> OhlcvData {
        OhlcvData {
            bars: v
                .iter()
                .map(|&(open, high, low, close, volume)| Bar {
                    open,
                    high,
                    low,
                    close,
                    volume,
                    date: String::new(),
                })
                .collect(),
            symbol: None,
            interval: None,
        }
    }

    /// Flat bars (open = high = low = close) with volume 1.
    pub fn closes(c: &[f64]) -> OhlcvData {
        bars(&c.iter().map(|&x| (x, x, x, x, 1.0)).collect::<Vec<_>>())
    }

    /// OHLC fixture for the ATR / Keltner / ADX / Supertrend / SAR tests.
    /// True range (ta.tr(true)): [2, 2, 3, 1, 2.5, 1.5, 3, 3].
    pub fn ohlc() -> OhlcvData {
        bars(&[
            (1.0, 2.0, 0.0, 1.0, 1.0),
            (1.0, 3.0, 1.0, 2.0, 1.0),
            (2.0, 5.0, 2.0, 4.0, 1.0),
            (4.0, 4.0, 3.0, 3.0, 1.0),
            (3.0, 3.5, 1.0, 1.5, 1.0),
            (1.5, 2.0, 0.5, 1.0, 1.0),
            (1.0, 4.0, 1.0, 3.5, 1.0),
            (3.5, 6.0, 3.0, 5.5, 1.0),
        ])
    }

    /// Element-wise comparison; NaN must match NaN.
    pub fn eq(got: &[f64], want: &[f64]) {
        assert_eq!(got.len(), want.len(), "{got:?} vs {want:?}");
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            if w.is_nan() {
                assert!(g.is_nan(), "idx {i}: got {g}, want NaN ({got:?})");
            } else {
                assert!((g - w).abs() < 1e-9, "idx {i}: got {g}, want {w} ({got:?})");
            }
        }
    }
}
