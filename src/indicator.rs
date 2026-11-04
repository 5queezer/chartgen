use crate::data::OhlcvData;
use plotters::style::RGBAColor;
use serde_json::{json, Value};

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
    /// Optional color used where `y1 < y2` (e.g. a red Ichimoku cloud);
    /// `color` then applies where `y1 >= y2`. `None` uses `color` everywhere.
    pub color_below: Option<RGBAColor>,
}

impl Fill {
    /// Color for a bar with edges `y1`/`y2`.
    pub fn color_for(&self, y1: f64, y2: f64) -> RGBAColor {
        match self.color_below {
            Some(below) if y1 < y2 => below,
            _ => self.color,
        }
    }
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
    /// Optional semantic label for alert/signal matching (e.g. "buy", "rsi_oversold").
    /// `None` means the dot is purely decorative (color-coded markers).
    pub label: Option<String>,
}

/// Horizontal reference line.
#[derive(Clone)]
pub struct HLine {
    pub y: f64,
    pub color: RGBAColor,
}

/// Horizontal bar for VPVR-style overlays (drawn at a price level).
#[derive(Clone)]
pub struct HBar {
    pub y: f64,      // center price level
    pub height: f64, // bar height in price units
    pub width: f64,  // 0.0-1.0, fraction of chart width (proportional to volume)
    pub offset: f64, // offset from edge as fraction of chart width (0.0-1.0)
    pub color: RGBAColor,
    pub left: bool, // true = draw from left edge, false = draw from right edge
}

/// Diagonal line between two points (for divergence markers).
#[derive(Clone)]
pub struct DivLine {
    pub x1: usize, // start bar index
    pub y1: f64,   // start y value
    pub x2: usize, // end bar index
    pub y2: f64,   // end y value
    pub color: RGBAColor,
    pub dashed: bool, // true = dashed line
}

/// Output of an indicator computation.
#[derive(Clone, Default)]
pub struct PanelResult {
    pub lines: Vec<Line>,
    pub fills: Vec<Fill>,
    pub bars: Vec<Bars>,
    pub dots: Vec<Dot>,
    pub hlines: Vec<HLine>,
    pub hbars: Vec<HBar>,
    pub divlines: Vec<DivLine>,
    pub y_range: Option<(f64, f64)>,
    pub label: String,
    pub is_overlay: bool,
}

/// Trait every indicator implements.
pub trait Indicator {
    #[allow(dead_code)]
    fn name(&self) -> &str;
    fn compute(&self, data: &OhlcvData) -> PanelResult;

    /// Human-readable description for tool discovery.
    fn description(&self) -> &str {
        ""
    }

    /// Configurable parameters as JSON array: `[{"name": "length", "type": "integer", "default": 14, "description": "..."}, ...]`
    /// Empty array means no configurable params.
    fn params(&self) -> Value {
        json!([])
    }

    /// Apply custom configuration from JSON object. Called before compute().
    /// Only needs to handle the params declared by params().
    fn configure(&mut self, _params: &Value) {}

    /// True for profiles aggregated over the whole input window (volume /
    /// time profiles). Their `hbars` and `hlines` are profile-derived, so
    /// series output with `warmup` recomputes them over the visible bars
    /// only (ADR-0009).
    fn windowed_profile(&self) -> bool {
        false
    }
}

// ---- Helpers ----

pub fn rgba(hex: u32, alpha: f64) -> RGBAColor {
    let r = ((hex >> 16) & 0xFF) as u8;
    let g = ((hex >> 8) & 0xFF) as u8;
    let b = (hex & 0xFF) as u8;
    RGBAColor(r, g, b, alpha)
}

// The moving-average and window helpers below delegate to
// `indicators::pine`, the single source of Pine v5 semantics (SMA-seeded
// EMA, NaN warmup, NaN-aware windows). See ADR-0018.

/// Pine `ta.ema`: seeded with the SMA of the first `period` values.
pub fn ema(data: &[f64], period: usize) -> Vec<f64> {
    crate::indicators::pine::ema_n(data, period)
}

/// Pine `ta.sma`: NaN until `period` consecutive valid values exist.
pub fn sma(data: &[f64], period: usize) -> Vec<f64> {
    crate::indicators::pine::sma_n(data, period)
}

/// Rolling lowest value over `period` bars (Pine `ta.lowest`).
pub use crate::indicators::pine::lowest;

/// Rolling highest value over `period` bars (Pine `ta.highest`).
pub use crate::indicators::pine::highest;

/// Stochastic: 100 * (src - lowest(low, len)) / (highest(high, len) - lowest(low, len)).
/// For stoch RSI, src=high=low=rsi values. A zero range is NaN (Pine `ta.stoch`).
pub use crate::indicators::pine::stoch;

#[cfg(test)]
mod fill_tests {
    use super::*;

    #[test]
    fn fill_color_for_uses_below_color_only_where_y1_lt_y2() {
        let up = RGBAColor(0, 255, 0, 0.2);
        let down = RGBAColor(255, 0, 0, 0.2);
        let f = Fill {
            y1: vec![],
            y2: vec![],
            color: up,
            color_below: Some(down),
        };
        assert_eq!(f.color_for(2.0, 1.0), up);
        assert_eq!(f.color_for(1.0, 1.0), up);
        assert_eq!(f.color_for(1.0, 2.0), down);
        let single = Fill {
            color_below: None,
            ..f
        };
        assert_eq!(single.color_for(1.0, 2.0), up);
    }
}
