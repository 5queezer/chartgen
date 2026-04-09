use crate::data::OhlcvData;
use plotters::style::RGBAColor;
use ta::indicators::{ExponentialMovingAverage, SimpleMovingAverage};
use ta::Next;

/// A line to draw on the panel.
#[derive(Clone)]
pub struct Line {
    pub y: Vec<f64>,
    pub color: RGBAColor,
    pub width: u32,
    #[allow(dead_code)]
    pub label: Option<String>,
}

/// A filled region between two series.
#[derive(Clone)]
pub struct Fill {
    pub y1: Vec<f64>,
    pub y2: Vec<f64>,
    pub color: RGBAColor,
}

/// Histogram bars.
#[derive(Clone)]
pub struct Bars {
    pub y: Vec<f64>,
    pub colors: Vec<RGBAColor>,
    pub bottom: f64,
}

/// Scatter dots.
#[derive(Clone)]
pub struct Dot {
    pub x: usize,
    pub y: f64,
    pub color: RGBAColor,
    pub size: u32,
}

/// Horizontal reference line.
#[derive(Clone)]
pub struct HLine {
    pub y: f64,
    pub color: RGBAColor,
}

/// Output of an indicator computation.
#[derive(Clone, Default)]
pub struct PanelResult {
    pub lines: Vec<Line>,
    pub fills: Vec<Fill>,
    pub bars: Vec<Bars>,
    pub dots: Vec<Dot>,
    pub hlines: Vec<HLine>,
    pub y_range: Option<(f64, f64)>,
    pub label: String,
    pub is_overlay: bool,
}

/// Trait every indicator implements.
pub trait Indicator {
    #[allow(dead_code)]
    fn name(&self) -> &str;
    fn compute(&self, data: &OhlcvData) -> PanelResult;
}

// ---- Helpers ----

pub fn rgba(hex: u32, alpha: f64) -> RGBAColor {
    let r = ((hex >> 16) & 0xFF) as u8;
    let g = ((hex >> 8) & 0xFF) as u8;
    let b = (hex & 0xFF) as u8;
    RGBAColor(r, g, b, alpha)
}

pub fn ema(data: &[f64], period: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; data.len()];
    if data.is_empty() || period == 0 {
        return out;
    }
    let mut ind = ExponentialMovingAverage::new(period).unwrap();
    for (i, &v) in data.iter().enumerate() {
        if v.is_nan() {
            continue;
        }
        out[i] = ind.next(v);
    }
    out
}

pub fn sma(data: &[f64], period: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; data.len()];
    if data.is_empty() || period == 0 {
        return out;
    }
    let mut ind = SimpleMovingAverage::new(period).unwrap();
    for (i, &v) in data.iter().enumerate() {
        if v.is_nan() {
            continue;
        }
        out[i] = ind.next(v);
    }
    out
}

/// Rolling lowest value over `period` bars.
pub fn lowest(data: &[f64], period: usize) -> Vec<f64> {
    let n = data.len();
    let mut out = vec![f64::NAN; n];
    if n == 0 || period == 0 {
        return out;
    }
    for i in (period - 1)..n {
        let lo = data[(i + 1 - period)..=i]
            .iter()
            .filter(|v| !v.is_nan())
            .copied()
            .fold(f64::INFINITY, f64::min);
        out[i] = if lo.is_infinite() { f64::NAN } else { lo };
    }
    out
}

/// Rolling highest value over `period` bars.
pub fn highest(data: &[f64], period: usize) -> Vec<f64> {
    let n = data.len();
    let mut out = vec![f64::NAN; n];
    if n == 0 || period == 0 {
        return out;
    }
    for i in (period - 1)..n {
        let hi = data[(i + 1 - period)..=i]
            .iter()
            .filter(|v| !v.is_nan())
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        out[i] = if hi.is_infinite() { f64::NAN } else { hi };
    }
    out
}

/// Stochastic: 100 * (src - lowest(low, len)) / (highest(high, len) - lowest(low, len)).
/// For stoch RSI, src=high=low=rsi values.
pub fn stoch(src: &[f64], high: &[f64], low: &[f64], period: usize) -> Vec<f64> {
    let lo = lowest(low, period);
    let hi = highest(high, period);
    let n = src.len();
    let mut out = vec![f64::NAN; n];
    for i in 0..n {
        if src[i].is_nan() || lo[i].is_nan() || hi[i].is_nan() {
            continue;
        }
        let range = hi[i] - lo[i];
        out[i] = if range > 0.0 {
            100.0 * (src[i] - lo[i]) / range
        } else {
            0.0
        };
    }
    out
}
