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
    /// The line breaks where its trend flips (Supertrend, SAR): it is NaN by
    /// design while the other trend is on, so its legend and `last_value`
    /// read the last bar. Other lines read their last finite value.
    pub breaks: bool,
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

/// Which way a detected pattern points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bias {
    Bullish,
    Bearish,
    Neutral,
}

impl Bias {
    pub fn as_str(self) -> &'static str {
        match self {
            Bias::Bullish => "bullish",
            Bias::Bearish => "bearish",
            Bias::Neutral => "neutral",
        }
    }

    /// TradingView's pattern colors: blue bullish, red bearish, grey neutral.
    pub fn color(self) -> RGBAColor {
        match self {
            Bias::Bullish => rgba(0x2962FF, 1.0),
            Bias::Bearish => rgba(0xF23645, 1.0),
            Bias::Neutral => rgba(0x787B86, 1.0),
        }
    }
}

/// A text label on one bar (candlestick patterns, ADR-0046).
#[derive(Clone, Debug)]
pub struct BarLabel {
    pub x: usize,
    /// The bar's high (`above`) or low (not `above`): the label sits past it.
    pub y: f64,
    pub above: bool,
    /// Short text on the chart, e.g. "H".
    pub text: String,
    /// Stable id, e.g. "hammer".
    pub key: String,
    /// Display name, e.g. "Hammer".
    pub name: String,
    pub bias: Bias,
    pub tooltip: String,
    /// On the forming bar: can change or vanish until the bar closes.
    pub tentative: bool,
}

/// Where a detected chart pattern stands. See ADR-0046.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternStatus {
    Forming,
    Confirmed,
    Failed,
}

impl PatternStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PatternStatus::Forming => "forming",
            PatternStatus::Confirmed => "confirmed",
            PatternStatus::Failed => "failed",
        }
    }
}

/// One straight stroke of a pattern's outline. Bar indices are signed: a
/// response cut to its newest bars still names points before its first bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub x1: i64,
    pub y1: f64,
    pub x2: i64,
    pub y2: f64,
    pub dashed: bool,
}

/// A pattern drawn over several bars, re-detected as a whole on every
/// response: a newer response replaces all of them (ADR-0046).
#[derive(Clone, Debug)]
pub struct PatternShape {
    pub key: String,
    pub name: String,
    pub bias: Bias,
    pub status: PatternStatus,
    pub strokes: Vec<Stroke>,
    pub label_x: i64,
    pub label_y: f64,
    pub label_above: bool,
    pub tooltip: String,
    /// Measured-move target, when the pattern has one.
    pub target: Option<f64>,
    /// A close past this price fails the pattern.
    pub invalidation: Option<f64>,
    /// Bar whose close confirmed the pattern.
    pub confirmed_x: Option<i64>,
    /// When it formed and resolved, as bar open times; `None` without bar times.
    pub times: Option<PatternTimes>,
    /// Height in price: the measured move's distance past the broken line.
    pub height: Option<f64>,
}

/// A pattern's bars as open times in ms since epoch, which stay put while the
/// window slides and its indices do not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PatternTimes {
    /// First and last bar of the pattern before any breakout.
    pub start: i64,
    pub end: i64,
    /// First bar with its last pivot known.
    pub known: i64,
    pub confirmed: Option<i64>,
    pub failed: Option<i64>,
}

/// A flat level (`lines[line]`) from bar `x` to the last bar. The whole level
/// moves when its anchors do, so a newer response rewrites it on every bar
/// instead of only the bars it covers (ADR-0046).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelLine {
    pub line: usize,
    pub x: i64,
    pub y: f64,
}

/// Something an alert can wait for: `x` is the bar of its newest occurrence.
#[derive(Clone, Debug, PartialEq)]
pub struct IndicatorEvent {
    pub key: String,
    pub name: String,
    pub x: Option<i64>,
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
    pub labels: Vec<BarLabel>,
    pub patterns: Vec<PatternShape>,
    pub levels: Vec<LevelLine>,
    pub events: Vec<IndicatorEvent>,
    pub y_range: Option<(f64, f64)>,
    pub label: String,
    pub is_overlay: bool,
    /// Its lines hold levels: a change of value is a jump between two of them, not a
    /// slope, so nothing is drawn across it.
    pub level_lines: bool,
    /// Lines and fill edges hold the previous bar until the next bar, then jump.
    pub steps: bool,
    /// Keep numeric lines for legends/alerts, but draw a dot at each finite value.
    pub point_lines: bool,
    /// Start a new line/fill run BEFORE each bar index, without dropping values.
    pub line_breaks: Vec<usize>,
    /// Semantic names parallel to `hlines` (profile POC/VAH/VAL and HVN/LVN).
    pub hline_labels: Vec<Option<String>>,
    /// The bars a range profile was read from, first and last: the image draws its
    /// `hbars` and `hlines` over them alone. None: the whole chart.
    pub span: Option<(usize, usize)>,
}

/// Values an indicator plots past the last bar (ADR-0033), e.g. the
/// Ichimoku cloud's leading spans. `(i, values)` in `lines` continues
/// `PanelResult::lines[i]` and `(i, y1, y2)` in `fills` continues
/// `PanelResult::fills[i]`; every vector holds `bars` values, one per bar
/// after the last one, at most `MAX_PROJECTION_BARS`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Projection {
    pub bars: usize,
    pub lines: Vec<(usize, Vec<f64>)>,
    pub fills: Vec<(usize, Vec<f64>, Vec<f64>)>,
}

/// Most bars a projection reaches: Pine's limit on a plot `offset`.
pub const MAX_PROJECTION_BARS: usize = 500;

/// A failed indicator computation (only user scripts fail, ADR-0025):
/// the pipeline `step` (`parse`, `static`, `presence`, `dry_run`,
/// `runtime`), the 1-based source line when known, and a message.
#[derive(Clone, Debug, PartialEq)]
pub struct IndicatorError {
    pub step: &'static str,
    pub line: Option<usize>,
    pub message: String,
}

impl IndicatorError {
    pub fn new(step: &'static str, line: Option<usize>, message: impl Into<String>) -> Self {
        Self {
            step,
            line,
            message: message.into(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "step": self.step, "line": self.line, "message": self.message })
    }
}

impl std::fmt::Display for IndicatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(l) => write!(f, "{} error at line {l}: {}", self.step, self.message),
            None => write!(f, "{} error: {}", self.step, self.message),
        }
    }
}

/// Trait every indicator implements.
pub trait Indicator {
    #[allow(dead_code)]
    fn name(&self) -> &str;
    fn compute(&self, data: &OhlcvData) -> PanelResult;

    /// `compute` for indicators that can fail (user scripts, ADR-0025): the
    /// error fails only this indicator, not the chart. Built-ins never fail.
    fn try_compute(&self, data: &OhlcvData) -> Result<PanelResult, IndicatorError> {
        Ok(self.compute(data))
    }

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

    /// Whether it reads anything but the bars it is given, such as a series fetched
    /// from an exchange. A replay refuses these: what they read is not cut at the
    /// replay's time (ADR-0059).
    fn reads_beyond_bars(&self) -> bool {
        false
    }

    /// Whether this is better computed from lower-timeframe bars (ADR-0048).
    fn wants_intrabars(&self) -> bool {
        false
    }

    /// The result over `chart`'s bars with its volume read from `intrabars`, which
    /// cover them. `None` when it has no such form.
    fn with_intrabars(&self, _chart: &OhlcvData, _intrabars: &OhlcvData) -> Option<PanelResult> {
        None
    }

    /// Bars of history the values need before they converge (an SMA-seeded
    /// EMA's seed weighs < 0.1% after them). A non-zero series `warmup` is
    /// raised to this, up to its cap (ADR-0032). 0: no extra need.
    fn warmup_bars(&self) -> usize {
        0
    }

    /// `warmup_bars` on a chart of `chart_secs`, for an indicator whose need is a span
    /// of time rather than of bars.
    fn warmup_bars_at(&self, _chart_secs: Option<i64>) -> usize {
        self.warmup_bars()
    }

    /// What `compute` over `data` plots past the last bar (ADR-0033);
    /// `None` for indicators that plot nothing there.
    fn projection(&self, _data: &OhlcvData) -> Option<Projection> {
        None
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
