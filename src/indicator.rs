use crate::data::OhlcvData;
use plotters::style::RGBAColor;

/// A line to draw on the panel.
#[derive(Clone)]
pub struct Line {
    pub y: Vec<f64>,
    pub color: RGBAColor,
    pub width: u32,
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
}

/// Trait every indicator implements.
pub trait Indicator {
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
    if data.is_empty() || period == 0 { return out; }
    let k = 2.0 / (period as f64 + 1.0);
    let mut val = data[0];
    out[0] = val;
    for i in 1..data.len() {
        if data[i].is_nan() { continue; }
        val = data[i] * k + val * (1.0 - k);
        out[i] = val;
    }
    out
}

pub fn sma(data: &[f64], period: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; data.len()];
    if data.len() < period { return out; }
    let mut sum: f64 = data[..period].iter().filter(|v| !v.is_nan()).sum();
    out[period - 1] = sum / period as f64;
    for i in period..data.len() {
        sum += data[i] - data[i - period];
        out[i] = sum / period as f64;
    }
    out
}
