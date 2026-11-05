//! Shared Pine Script v5 semantics for the TradingView-parity indicators.
//!
//! NaN rules (Pine `na`, ADR-0018):
//! - Windowed helpers (`sum_n`, `sma_n`, `wma_n`, `highest`, `lowest`,
//!   `stdev`, `dev`, ...) ignore NaN (or infinite) values, as the Pine
//!   reference documents for `ta.sma`, `ta.highest`, `ta.lowest`, `ta.stdev`
//!   and `math.sum`: they compute over the last `n` valid values and are NaN
//!   only until `n` valid values have been seen. A NaN input bar therefore
//!   still gets a value (from the previous `n` valid inputs).
//! - Recursive smoothers (`ema_n` / `rma_n`) seed with the SMA of the first
//!   `n` valid values. A NaN input keeps the state and outputs the previous
//!   value on that bar (no gap). This na behaviour is unverified against a
//!   live TradingView chart.
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

/// Apply `f` to the last `n` valid (finite) values at every bar, oldest
/// first; NaN values are skipped, and the result is NaN until `n` valid
/// values have been seen.
pub fn windowed(x: &[f64], n: usize, f: impl Fn(&[f64]) -> f64) -> Vec<f64> {
    let mut out = vec![f64::NAN; x.len()];
    if n == 0 {
        return out;
    }
    let mut w: std::collections::VecDeque<f64> = std::collections::VecDeque::with_capacity(n + 1);
    for i in 0..x.len() {
        if x[i].is_finite() {
            w.push_back(x[i]);
            if w.len() > n {
                w.pop_front();
            }
        }
        if w.len() == n {
            out[i] = f(w.make_contiguous());
        }
    }
    out
}

/// Total of the last `n` valid values (NaN skipped), NaN until `n` valid
/// values have been seen. O(len): a running sum that is re-summed exactly
/// every `n` updates so floating-point drift cannot build up, and that is
/// exactly 0 whenever every value in the window is 0.
pub fn sum_n(x: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; x.len()];
    if n == 0 {
        return out;
    }
    let mut w: std::collections::VecDeque<f64> = std::collections::VecDeque::with_capacity(n + 1);
    let (mut sum, mut nonzero, mut since_resum) = (0.0, 0usize, 0usize);
    for i in 0..x.len() {
        let v = x[i];
        if v.is_finite() {
            w.push_back(v);
            sum += v;
            nonzero += usize::from(v != 0.0);
            if w.len() > n {
                let old = w.pop_front().unwrap_or(0.0);
                sum -= old;
                nonzero -= usize::from(old != 0.0);
            }
            since_resum += 1;
            if since_resum >= n {
                sum = w.iter().sum();
                since_resum = 0;
            }
        }
        if w.len() == n {
            out[i] = if nonzero == 0 { 0.0 } else { sum };
        }
    }
    out
}

/// ta.sma over the last `n` valid values (see `sum_n`).
pub fn sma_n(x: &[f64], n: usize) -> Vec<f64> {
    sum_n(x, n).into_iter().map(|s| s / n as f64).collect()
}

/// Exponential smoothing seeded with the SMA of the first `n` valid values
/// (ta.ema uses alpha = 2/(n+1), ta.rma uses alpha = 1/n). A NaN input keeps
/// the state and repeats the previous output.
pub fn smoothed(x: &[f64], n: usize, alpha: f64) -> Vec<f64> {
    let mut out = vec![f64::NAN; x.len()];
    let seed = sma_n(x, n);
    let mut prev = f64::NAN;
    for i in 0..x.len() {
        if prev.is_nan() {
            prev = seed[i];
        } else if x[i].is_finite() {
            prev = alpha * x[i] + (1.0 - alpha) * prev;
        }
        // A NaN input after the seed repeats the previous value.
        out[i] = prev;
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

/// Bars after which the start of an exponential smoothing with `alpha`
/// weighs < 0.1%. 0 for alpha outside (0, 1): no smoothing, or none decays.
/// Capped at `MAX_DECAY_BARS` so tiny alphas cannot overflow callers' sums.
pub fn decay_bars(alpha: f64) -> usize {
    const MAX_DECAY_BARS: f64 = 100_000.0;
    if !(alpha > 0.0 && alpha < 1.0) {
        return 0;
    }
    // ln_1p keeps precision where `1.0 - alpha` would round to 1.0.
    (1000f64.ln() / -(-alpha).ln_1p())
        .ceil()
        .min(MAX_DECAY_BARS) as usize
}

/// `Indicator::warmup_bars` of `ma_by_type(kind, .., n)`: the seed window
/// plus the bars after which the seed of an exponential kind weighs < 0.1%
/// (dema/tema chain two/three EMAs). Window kinds are exact after their
/// lookback (hma: the full WMA, then a sqrt-length one).
pub fn ma_warmup(kind: &str, n: usize) -> usize {
    let ema = || n + decay_bars(2.0 / (n as f64 + 1.0));
    match kind {
        "ema" => ema(),
        "dema" => 2 * ema(),
        "tema" => 3 * ema(),
        "rma" | "smma" => n + decay_bars(1.0 / n.max(1) as f64),
        "hma" => n + ((n as f64).sqrt().floor() as usize).max(1) - 1,
        _ => n,
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
        breaks: false,
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

// ---- Chart timeframe (Pine `timeframe.*`) and calendar periods ----

/// Bar open times in unix seconds; `None` unless every bar has one.
pub fn bar_times(data: &OhlcvData) -> Option<Vec<i64>> {
    data.bars
        .iter()
        .map(|b| b.date.parse::<i64>().ok())
        .collect()
}

/// Chart timeframe class, as Pine's `timeframe.isintraday` / `isdaily` / ...
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Timeframe {
    /// Bar length in seconds (below a day).
    Intraday(i64),
    /// Multiplier in days (1D, 2D, ...).
    Daily(i64),
    Weekly,
    Monthly,
}

/// The chart timeframe from `data.interval`, else inferred from the smallest
/// gap between bar times. `None` when neither is known.
pub fn timeframe(data: &OhlcvData) -> Option<Timeframe> {
    const DAY: i64 = 86_400;
    let secs = data
        .interval
        .as_deref()
        .and_then(crate::data::bar_interval_secs)
        .or_else(|| {
            let t = bar_times(data)?;
            t.windows(2).map(|w| w[1] - w[0]).filter(|d| *d > 0).min()
        })?;
    // Inferred gaps between daily stock bars shift by an hour across DST,
    // hence the tolerances.
    Some(if secs < 20 * 3_600 {
        Timeframe::Intraday(secs)
    } else if secs < 6 * DAY {
        Timeframe::Daily(((secs as f64 / DAY as f64).round() as i64).max(1))
    } else if secs < 25 * DAY {
        Timeframe::Weekly
    } else {
        Timeframe::Monthly
    })
}

/// A UTC calendar period, as used for VWAP anchors and pivot timeframes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Period {
    Day,
    /// ISO week (starts on Monday).
    Week,
    Month,
    Year,
}

/// Key of the UTC calendar period containing unix time `t`: two bars are in
/// the same period exactly when their keys are equal.
pub fn period_key(t: i64, p: Period) -> i64 {
    use chrono::Datelike;
    let Some(d) = chrono::DateTime::from_timestamp(t, 0).map(|dt| dt.date_naive()) else {
        return i64::MIN;
    };
    match p {
        Period::Day => d.num_days_from_ce() as i64,
        Period::Week => {
            let w = d.iso_week();
            w.year() as i64 * 100 + w.week() as i64
        }
        Period::Month => d.year() as i64 * 100 + d.month() as i64,
        Period::Year => d.year() as i64,
    }
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
/// A zero `lower` (see `is_zero`) is a division by zero -> NaN.
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
        if u.is_nan() || is_zero(l, u.abs() + l.abs()) {
            f64::NAN
        } else {
            100.0 - 100.0 / (1.0 + u / l)
        }
    })
}
/// `x` is zero up to floating-point noise relative to `scale`
/// (`|x| <= 1e-12 * scale`); an exact 0 always counts.
pub fn is_zero(x: f64, scale: f64) -> bool {
    x == 0.0 || x.abs() <= 1e-12 * scale.abs()
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
/// Pine `ta.crossover(a, b)`: `a > b` on this bar and `a <= b` on the
/// previous one. A comparison with na is false, so na never crosses.
pub fn crossover(a: &[f64], b: &[f64]) -> Vec<bool> {
    (0..a.len().min(b.len()))
        .map(|i| i > 0 && a[i] > b[i] && a[i - 1] <= b[i - 1])
        .collect()
}
/// Pine `ta.crossunder(a, b)`: `a < b` on this bar, `a >= b` on the previous.
pub fn crossunder(a: &[f64], b: &[f64]) -> Vec<bool> {
    (0..a.len().min(b.len()))
        .map(|i| i > 0 && a[i] < b[i] && a[i - 1] >= b[i - 1])
        .collect()
}
/// Pine `ta.pivothigh(src, left, right)`: on bar i, the value of bar
/// `i - right` when it is above the `left` bars before it and not below the
/// `right` bars after it; na otherwise (and while any bar of the window is
/// na). The tie rule is unverified against a live TradingView chart.
pub fn pivot_high(x: &[f64], left: usize, right: usize) -> Vec<f64> {
    pivot(x, left, right, |c, v| v < c, |c, v| v <= c)
}
/// Pine `ta.pivotlow(src, left, right)`: mirror of `pivot_high`.
pub fn pivot_low(x: &[f64], left: usize, right: usize) -> Vec<f64> {
    pivot(x, left, right, |c, v| v > c, |c, v| v >= c)
}
fn pivot(
    x: &[f64],
    left: usize,
    right: usize,
    before: fn(f64, f64) -> bool,
    after: fn(f64, f64) -> bool,
) -> Vec<f64> {
    let mut out = vec![f64::NAN; x.len()];
    for i in (left + right)..x.len() {
        let c = x[i - right];
        let w = &x[i - right - left..=i];
        if w.iter().any(|v| v.is_nan()) {
            continue;
        }
        if w[..left].iter().all(|&v| before(c, v)) && w[left + 1..].iter().all(|&v| after(c, v)) {
            out[i] = c;
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

#[cfg(test)]
mod tests {
    #[test]
    fn decay_bars_is_capped_and_survives_tiny_alphas() {
        use super::decay_bars;
        assert_eq!(decay_bars(1e-20), 100_000);
        assert_eq!(decay_bars(f64::MIN_POSITIVE), 100_000);
        assert_eq!(decay_bars(f64::NAN), 0);
        assert_eq!(decay_bars(-1.0), 0);
        assert_eq!(decay_bars(0.0), 0);
        assert_eq!(decay_bars(1.0), 0);
        assert_eq!(decay_bars(0.5), 10);
    }

    use super::test_util::{bars, eq, NAN};
    use super::*;

    #[test]
    fn windows_skip_nan_values() {
        // NaN only until n valid values; a NaN bar uses the last n valid ones.
        eq(&sma_n(&[1.0, NAN, 3.0, 5.0], 2), &[NAN, NAN, 2.0, 4.0]);
        eq(&sma_n(&[1.0, 3.0, NAN, 5.0], 2), &[NAN, 2.0, 2.0, 4.0]);
        eq(&sum_n(&[1.0, f64::INFINITY, 3.0], 2), &[NAN, NAN, 4.0]);
        eq(&highest(&[1.0, NAN, 3.0, 2.0], 2), &[NAN, NAN, 3.0, 3.0]);
        eq(&lowest(&[1.0, NAN, 3.0, 2.0], 2), &[NAN, NAN, 1.0, 2.0]);
        eq(&stdev(&[1.0, NAN, 3.0], 2), &[NAN, NAN, 1.0]);
        eq(&dev(&[1.0, NAN, 3.0], 2), &[NAN, NAN, 1.0]);
        eq(&wma_n(&[1.0, NAN, 4.0], 2), &[NAN, NAN, 3.0]);
    }

    #[test]
    fn smoothers_seed_on_valid_values_and_hold_across_nan() {
        // seed = SMA of the first 2 valid values, skipping the NaN
        eq(&ema_n(&[1.0, NAN, 3.0, 5.0], 2), &[NAN, NAN, 2.0, 4.0]);
        // after the seed a NaN input repeats the previous value
        eq(&ema_n(&[1.0, 3.0, NAN, 5.0], 2), &[NAN, 2.0, 2.0, 4.0]);
        eq(&rma_n(&[1.0, 3.0, NAN, 5.0], 2), &[NAN, 2.0, 2.0, 3.5]);
    }

    #[test]
    fn running_sum_is_exactly_zero_after_large_values_leave() {
        // 0.1 + 0.7 + 1e12 - 0.1 - 0.7 - 1e12 leaves float residue in a
        // plain running sum; an all-zero window must be exactly 0.
        // The exact re-sum runs at bars 2 and 5; bar 6 is the first
        // all-zero window and must be exactly 0.
        let x = [0.3, 0.1, 0.7, 1e12, 0.0, 0.0, 0.0, 0.25];
        let s = sum_n(&x, 3);
        assert!(s[..2].iter().all(|v| v.is_nan()));
        assert_eq!(s[6], 0.0);
        assert_eq!(s[7], 0.25);
        assert!((s[3] - (1e12 + 0.8)).abs() < 1e-3);
    }

    #[test]
    fn btc_scale_mfi_has_no_running_sum_drift() {
        // 40 mixed bars at BTC prices / volumes, then 15 rising bars: the
        // last 14-bar window has no negative flow, so `lower` is exactly 0
        // and ta.mfi divides by zero (NaN) instead of returning ~100.
        let mut v = Vec::new();
        let mut p = 64_123.37;
        for i in 0..55 {
            let step = if i < 40 {
                [431.17, -612.93, 287.41, -98.67, 503.29][i % 5]
            } else {
                12.5 + i as f64 * 0.37
            };
            p += step;
            let vol = 1_234.567_8 + (i * 7919 % 9973) as f64 * 4.21;
            v.push((p, p + 50.0, p - 50.0, p, vol));
        }
        let d = bars(&v);
        let src = source_series(&d, "hlc3");
        let vol = volumes(&d);
        let ch = change_n(&src, 1);
        let lower: Vec<f64> = (0..src.len())
            .map(|i| if ch[i] < 0.0 { vol[i] * src[i] } else { 0.0 })
            .collect();
        assert_eq!(sum_n(&lower, 14)[54], 0.0);
        let m = mfi(&src, &vol, 14);
        assert!(m[54].is_nan(), "lower == 0 -> NaN, got {}", m[54]);
        assert!(m[40].is_finite());
    }

    #[test]
    fn crosses_and_pivots() {
        let a = [1.0, 3.0, 2.0, NAN, 5.0];
        let b = [2.0, 2.0, 2.0, 2.0, 2.0];
        assert_eq!(crossover(&a, &b), [false, true, false, false, false]);
        assert_eq!(crossunder(&a, &b), [false, false, false, false, false]);
        let x = [1.0, 3.0, 2.0, 2.0, 1.0, 4.0];
        eq(&pivot_high(&x, 1, 1), &[NAN, NAN, 3.0, NAN, NAN, NAN]);
        eq(&pivot_low(&x, 1, 1), &[NAN, NAN, NAN, 2.0, NAN, 1.0]);
    }

    #[test]
    fn is_zero_is_relative() {
        assert!(is_zero(1.65e-8, 1.4e10));
        assert!(!is_zero(1.65e-8, 1.0));
        assert!(is_zero(0.0, 0.0));
    }
}
