use crate::data::OhlcvData;
use crate::indicator::*;
use crate::orderflow::value_area_grows_down;
use crate::trading::signals;
use serde_json::{json, Value};

// ---- VWAP (overlay) ----

/// TradingView "Anchor Period" choices supported here.
const VWAP_ANCHORS: &[&str] = &["session", "week", "month", "none"];

fn anchor_param() -> Value {
    json!({"name": "anchor", "type": "string", "default": "session", "enum": VWAP_ANCHORS,
           "description": "Reset period: UTC day (session), ISO week, calendar month, or never"})
}

fn configure_anchor(anchor: &mut String, params: &Value) {
    if let Some(v) = params
        .get("anchor")
        .and_then(|v| v.as_str())
        .filter(|v| VWAP_ANCHORS.contains(v))
    {
        *anchor = v.to_string();
    }
}

/// Pine `ta.vwap(hlc3, isNewPeriod, 1)`: volume-weighted mean of hlc3 and its
/// volume-weighted population stdev, both reset on the first bar of every
/// anchor period. Periods are UTC calendar days / ISO weeks / months of the
/// bar times; on daily and higher charts "session" therefore resets on every
/// bar (TradingView's `timeframe.change("D")` is true on each daily bar).
/// Without bar timestamps the sums never reset (anchor "none").
fn anchored_vwap(data: &OhlcvData, anchor: &str) -> (Vec<f64>, Vec<f64>) {
    use super::pine::{bar_times, period_key, Period};
    let period = match anchor {
        "session" => Some(Period::Day),
        "week" => Some(Period::Week),
        "month" => Some(Period::Month),
        _ => None,
    };
    let times = bar_times(data);
    let n = data.len();
    let (mut vwap, mut sd) = (vec![f64::NAN; n], vec![f64::NAN; n]);
    let (mut sv, mut sv_src, mut sv_src2) = (0.0, 0.0, 0.0);
    let mut prev_key = None;
    for (i, bar) in data.bars.iter().enumerate() {
        let key = match (&times, period) {
            (Some(t), Some(p)) => Some(period_key(t[i], p)),
            _ => None,
        };
        if i == 0 || key != prev_key {
            (sv, sv_src, sv_src2) = (0.0, 0.0, 0.0);
        }
        prev_key = key;
        let tp = (bar.high + bar.low + bar.close) / 3.0;
        sv += bar.volume;
        sv_src += tp * bar.volume;
        sv_src2 += tp * tp * bar.volume;
        if sv > 0.0 {
            let m = sv_src / sv;
            vwap[i] = m;
            sd[i] = (sv_src2 / sv - m * m).max(0.0).sqrt();
        }
    }
    (vwap, sd)
}

pub struct Vwap {
    pub anchor: String,
}

impl Default for Vwap {
    fn default() -> Self {
        Self {
            anchor: "session".into(),
        }
    }
}

impl Indicator for Vwap {
    fn name(&self) -> &str {
        "VWAP"
    }

    fn description(&self) -> &str {
        "Volume Weighted Average Price — anchored to the session, week or month"
    }

    fn params(&self) -> Value {
        json!([anchor_param()])
    }

    fn configure(&mut self, params: &Value) {
        configure_anchor(&mut self.anchor, params);
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let (vals, _) = anchored_vwap(data, &self.anchor);

        PanelResult {
            lines: vec![Line {
                breaks: false,
                y: vals,
                color: rgba(0xFF9800, 0.9),
                width: 2,
                label: Some("VWAP".into()),
            }],
            is_overlay: true,
            label: "VWAP".into(),
            ..Default::default()
        }
    }
}

// ---- ADX (panel) ----

pub struct Adx {
    pub period: usize,
}

impl Default for Adx {
    fn default() -> Self {
        Self { period: 14 }
    }
}

impl Indicator for Adx {
    fn name(&self) -> &str {
        "ADX"
    }

    fn description(&self) -> &str {
        "Average Directional Index — trend strength with +DI/-DI"
    }

    fn params(&self) -> Value {
        json!([{"name": "period", "type": "integer", "min": 1, "max": 10000, "default": 14}])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("period").and_then(|v| v.as_u64()) {
            self.period = v as usize;
        }
    }

    fn warmup_bars(&self) -> usize {
        // DI smooths with ta.rma, then ADX smooths DX with it again.
        2 * super::pine::ma_warmup("rma", self.period)
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        use super::pine::{change_n, div, fixnan, rma_n, true_range, zip};
        // TradingView ADX / DMI (DI length = ADX smoothing = period):
        // plusDM / minusDM from ta.change(high) / -ta.change(low),
        // DI = fixnan(100 * ta.rma(DM, n) / ta.rma(ta.tr, n)),
        // ADX = 100 * ta.rma(|+DI - -DI| / (sum == 0 ? 1 : sum), n).
        let period = self.period;
        let up = change_n(&data.highs(), 1);
        let down: Vec<f64> = change_n(&data.lows(), 1).iter().map(|c| -c).collect();
        let dm = |a: f64, b: f64| {
            if a.is_nan() {
                f64::NAN
            } else if a > b && a > 0.0 {
                a
            } else {
                0.0
            }
        };
        let plus_dm = zip(&up, &down, dm);
        let minus_dm = zip(&down, &up, dm);
        // ta.tr (handle_na = false): na on bar 0, where close[1] is na.
        let mut tr = true_range(data);
        if let Some(t) = tr.first_mut() {
            *t = f64::NAN;
        }
        let trur = rma_n(&tr, period);
        let di = |dm: &[f64]| {
            let scaled: Vec<f64> = rma_n(dm, period).iter().map(|v| 100.0 * v).collect();
            fixnan(&div(&scaled, &trur))
        };
        let plus_di_vals = di(&plus_dm);
        let minus_di_vals = di(&minus_dm);
        let dx = zip(&plus_di_vals, &minus_di_vals, |p, m| {
            let sum = p + m;
            (p - m).abs() / if sum == 0.0 { 1.0 } else { sum }
        });
        let adx_vals: Vec<f64> = rma_n(&dx, period).iter().map(|v| 100.0 * v).collect();

        PanelResult {
            lines: vec![
                Line {
                    breaks: false,
                    y: adx_vals,
                    color: rgba(0xFFFFFF, 0.9),
                    width: 2,
                    label: Some("ADX".into()),
                },
                Line {
                    breaks: false,
                    y: plus_di_vals,
                    color: rgba(0x26a69a, 0.9),
                    width: 1,
                    label: Some("+DI".into()),
                },
                Line {
                    breaks: false,
                    y: minus_di_vals,
                    color: rgba(0xef5350, 0.9),
                    width: 1,
                    label: Some("-DI".into()),
                },
            ],
            hlines: vec![HLine {
                y: 25.0,
                color: rgba(0x787B86, 0.5),
            }],
            y_range: Some((0.0, 100.0)),
            label: "ADX".into(),
            ..Default::default()
        }
    }
}

// ---- Supertrend (overlay) ----

const UP_COLOR: u32 = 0x26a69a;
const DOWN_COLOR: u32 = 0xef5350;

/// An up- and a down-trend line, each NaN while the other trend is on, like
/// two TradingView plots with plot.style_linebr: a flip breaks the line
/// instead of drawing a connector between the two levels.
fn trend_lines(up: Vec<f64>, down: Vec<f64>, labels: [&str; 2]) -> Vec<Line> {
    [(up, UP_COLOR, labels[0]), (down, DOWN_COLOR, labels[1])]
        .into_iter()
        .map(|(y, color, label)| Line {
            y,
            color: rgba(color, 0.9),
            width: 1,
            label: Some(label.into()),
            breaks: true,
        })
        .collect()
}

pub struct Supertrend {
    pub period: usize,
    pub multiplier: f64,
}

impl Default for Supertrend {
    fn default() -> Self {
        Self {
            period: 10,
            multiplier: 3.0,
        }
    }
}

impl Indicator for Supertrend {
    fn name(&self) -> &str {
        "Supertrend"
    }

    fn description(&self) -> &str {
        "Supertrend — ATR-based trend follower (Up Trend / Down Trend lines that break at a flip)"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "period", "type": "integer", "min": 1, "max": 10000, "default": 10},
            {"name": "multiplier", "type": "number", "min": 0, "max": 1000, "default": 3.0}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("period").and_then(|v| v.as_u64()) {
            self.period = v as usize;
        }
        if let Some(v) = params.get("multiplier").and_then(|v| v.as_f64()) {
            self.multiplier = v;
        }
    }

    fn warmup_bars(&self) -> usize {
        super::pine::ma_warmup("rma", self.period)
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut dots = Vec::new();

        // ta.supertrend: ATR is ta.atr (RMA of the true range).
        let atr_vals = super::pine::atr(data, self.period);

        // Verbatim port of TradingView's `ta.supertrend` reference code; the
        // bands use nz() of the previous bar (0 before the first value).
        let b = &data.bars;
        let nz = |v: f64| if v.is_nan() { 0.0 } else { v };
        let mut final_upper = vec![f64::NAN; n];
        let mut final_lower = vec![f64::NAN; n];
        // true = UP (direction -1), false = DOWN (direction 1)
        let mut trend = vec![false; n];
        let mut st_val = vec![f64::NAN; n];
        // TradingView plots the active band as "Up Trend" or "Down Trend"
        // with plot.style_linebr: the other one is na, so a flip breaks the
        // line instead of drawing a vertical connector.
        let mut up = vec![f64::NAN; n];
        let mut down = vec![f64::NAN; n];

        for i in 0..n {
            let hl2 = (b[i].high + b[i].low) / 2.0;
            let mut upper = hl2 + self.multiplier * atr_vals[i];
            let mut lower = hl2 - self.multiplier * atr_vals[i];
            let (prev_upper, prev_lower, prev_st, prev_close, prev_atr) = if i == 0 {
                (0.0, 0.0, f64::NAN, f64::NAN, f64::NAN)
            } else {
                (
                    nz(final_upper[i - 1]),
                    nz(final_lower[i - 1]),
                    st_val[i - 1],
                    b[i - 1].close,
                    atr_vals[i - 1],
                )
            };
            // Comparisons with NaN are false, as with Pine's na.
            if !(lower > prev_lower || prev_close < prev_lower) {
                lower = prev_lower;
            }
            if !(upper < prev_upper || prev_close > prev_upper) {
                upper = prev_upper;
            }
            final_upper[i] = upper;
            final_lower[i] = lower;

            let close = b[i].close;
            trend[i] = if prev_atr.is_nan() {
                false
            } else if prev_st == prev_upper {
                close > upper
            } else {
                close >= lower
            };
            st_val[i] = if trend[i] { lower } else { upper };

            // Not drawn while the ATR warms up, nor on bar 0, where the
            // missing close[1] makes Pine's nz() band 0.
            if atr_vals[i].is_nan() || i == 0 {
                continue;
            }
            let val = st_val[i];
            let (color, label) = if trend[i] {
                up[i] = val;
                (UP_COLOR, signals::FLIP_UP)
            } else {
                down[i] = val;
                (DOWN_COLOR, signals::FLIP_DOWN)
            };
            // A dot marks only a flip (signal event).
            if !atr_vals[i - 1].is_nan() && trend[i] != trend[i - 1] {
                dots.push(Dot {
                    x: i,
                    y: val,
                    color: rgba(color, 0.9),
                    size: 2,
                    label: Some(label.to_string()),
                });
            }
        }

        PanelResult {
            lines: trend_lines(up, down, ["Up Trend", "Down Trend"]),
            dots,
            is_overlay: true,
            label: "Supertrend".into(),
            ..Default::default()
        }
    }
}

// ---- Parabolic SAR (overlay) ----

pub struct ParabolicSar {
    pub af_start: f64,
    pub af_step: f64,
    pub af_max: f64,
}

impl Default for ParabolicSar {
    fn default() -> Self {
        Self {
            af_start: 0.02,
            af_step: 0.02,
            af_max: 0.2,
        }
    }
}

impl Indicator for ParabolicSar {
    fn name(&self) -> &str {
        "Parabolic SAR"
    }

    fn description(&self) -> &str {
        "Parabolic SAR — trailing stop / trend reversal"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "af_start", "type": "number", "min": 0, "max": 1, "default": 0.02},
            {"name": "af_step", "type": "number", "min": 0, "max": 1, "default": 0.02},
            {"name": "af_max", "type": "number", "min": 0, "max": 1, "default": 0.2}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("af_start").and_then(|v| v.as_f64()) {
            self.af_start = v;
        }
        if let Some(v) = params.get("af_step").and_then(|v| v.as_f64()) {
            self.af_step = v;
        }
        if let Some(v) = params.get("af_max").and_then(|v| v.as_f64()) {
            self.af_max = v;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut dots = Vec::new();
        let (mut up, mut down) = (vec![f64::NAN; n], vec![f64::NAN; n]);

        // Port of Pine's `ta.sar` reference implementation. Bar 0 is na; bar 1
        // seeds the trend from close vs close[1].
        let (start, inc, max) = (self.af_start, self.af_step, self.af_max);
        let b = &data.bars;
        // true = UP (SAR below price), false = DOWN (SAR above price)
        let mut trend_up = false;
        let mut sar = f64::NAN;
        let mut ep = f64::NAN;
        let mut af = start;

        for i in 1..n {
            let (high, low) = (b[i].high, b[i].low);
            let mut first_trend_bar = false;
            let mut flipped = false;
            if i == 1 {
                trend_up = b[1].close > b[0].close;
                if trend_up {
                    ep = high;
                    sar = b[0].low;
                } else {
                    ep = low;
                    sar = b[0].high;
                }
                first_trend_bar = true;
                af = start;
            }

            sar += af * (ep - sar);

            if trend_up {
                if sar > low {
                    // Reverse to DOWN
                    first_trend_bar = true;
                    flipped = true;
                    trend_up = false;
                    sar = high.max(ep);
                    ep = low;
                    af = start;
                }
            } else if sar < high {
                // Reverse to UP
                first_trend_bar = true;
                flipped = true;
                trend_up = true;
                sar = low.min(ep);
                ep = high;
                af = start;
            }

            if !first_trend_bar {
                if trend_up {
                    if high > ep {
                        ep = high;
                        af = (af + inc).min(max);
                    }
                } else if low < ep {
                    ep = low;
                    af = (af + inc).min(max);
                }
            }

            // Clamp SAR to the previous two lows (UP) or highs (DOWN)
            if trend_up {
                sar = sar.min(b[i - 1].low);
                if i > 1 {
                    sar = sar.min(b[i - 2].low);
                }
            } else {
                sar = sar.max(b[i - 1].high);
                if i > 1 {
                    sar = sar.max(b[i - 2].high);
                }
            }

            let (color, label) = if trend_up {
                up[i] = sar;
                (UP_COLOR, signals::FLIP_UP)
            } else {
                down[i] = sar;
                (DOWN_COLOR, signals::FLIP_DOWN)
            };
            // A dot marks only a flip bar (direction reversal = signal event).
            if flipped {
                dots.push(Dot {
                    x: i,
                    y: sar,
                    color: rgba(color, 0.9),
                    size: 2,
                    label: Some(label.to_string()),
                });
            }
        }

        PanelResult {
            // TradingView draws a cross per bar; two lines that break at a
            // flip draw the same points on the series wire and the web chart.
            lines: trend_lines(up, down, ["SAR Up", "SAR Down"]),
            dots,
            is_overlay: true,
            label: "SAR".into(),
            ..Default::default()
        }
    }
}

// ---- Accumulation/Distribution Line (panel) ----

pub struct AdLine;

impl Indicator for AdLine {
    fn name(&self) -> &str {
        "A/D"
    }

    fn description(&self) -> &str {
        "Accumulation/Distribution Line — volume flow"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut vals = vec![f64::NAN; n];
        let mut cum = 0.0;

        for (i, bar) in data.bars.iter().enumerate() {
            let range = bar.high - bar.low;
            let multiplier = if range > 0.0 {
                ((bar.close - bar.low) - (bar.high - bar.close)) / range
            } else {
                0.0
            };
            cum += multiplier * bar.volume;
            vals[i] = cum;
        }

        PanelResult {
            lines: vec![Line {
                breaks: false,
                y: vals,
                color: rgba(0x2196F3, 0.9),
                width: 2,
                label: Some("A/D".into()),
            }],
            label: "A/D".into(),
            ..Default::default()
        }
    }
}

// ---- Historical Volatility (panel) ----

pub struct HistVol {
    pub period: usize,
}

impl Default for HistVol {
    fn default() -> Self {
        // TradingView's default length.
        Self { period: 10 }
    }
}

impl Indicator for HistVol {
    fn name(&self) -> &str {
        "HV"
    }

    fn description(&self) -> &str {
        "Historical Volatility — annualized log-return std dev"
    }

    fn params(&self) -> Value {
        json!([{"name": "period", "type": "integer", "min": 1, "max": 10000, "default": 10}])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("period").and_then(|v| v.as_u64()) {
            self.period = v as usize;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        // TV: 100 * ta.stdev(math.log(close / close[1]), n) * math.sqrt(365 / per),
        // per = 1 for intraday and 1D charts, 7 otherwise.
        let closes = data.closes();
        let log_returns: Vec<f64> = (0..closes.len())
            .map(|i| {
                if i == 0 {
                    f64::NAN
                } else {
                    (closes[i] / closes[i - 1]).ln()
                }
            })
            .collect();
        // An unknown timeframe cannot be annualized: the line stays NaN.
        use super::pine::Timeframe;
        let per = match super::pine::timeframe(data) {
            Some(Timeframe::Intraday(_) | Timeframe::Daily(1)) => 1.0,
            Some(_) => 7.0,
            None => f64::NAN,
        };
        let scale = 100.0 * (365.0_f64 / per).sqrt();
        let vals: Vec<f64> = super::pine::stdev(&log_returns, self.period)
            .iter()
            .map(|v| v * scale)
            .collect();

        PanelResult {
            lines: vec![Line {
                breaks: false,
                y: vals,
                color: rgba(0xFF9800, 0.9),
                width: 2,
                label: Some("HV".into()),
            }],
            label: "HV".into(),
            ..Default::default()
        }
    }
}

// ---- VWAP Bands (overlay) ----

pub struct VwapBands {
    pub std_dev: f64,
    pub anchor: String,
}

impl Default for VwapBands {
    fn default() -> Self {
        // TradingView: band multiplier #1 = 1, anchor "Session".
        Self {
            std_dev: 1.0,
            anchor: "session".into(),
        }
    }
}

impl Indicator for VwapBands {
    fn name(&self) -> &str {
        "VWAP Bands"
    }

    fn description(&self) -> &str {
        "VWAP Bands — standard deviation bands around VWAP"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "std_dev", "type": "number", "min": 0, "max": 100, "default": 1.0},
            anchor_param()
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("std_dev").and_then(|v| v.as_f64()) {
            self.std_dev = v;
        }
        configure_anchor(&mut self.anchor, params);
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let (vwap_vals, sd) = anchored_vwap(data, &self.anchor);
        let upper = super::pine::zip(&vwap_vals, &sd, |v, d| v + self.std_dev * d);
        let lower = super::pine::zip(&vwap_vals, &sd, |v, d| v - self.std_dev * d);

        PanelResult {
            lines: vec![
                Line {
                    breaks: false,
                    y: upper.clone(),
                    color: rgba(0x787B86, 0.5),
                    width: 1,
                    label: Some("VWAP Upper".into()),
                },
                Line {
                    breaks: false,
                    y: vwap_vals,
                    color: rgba(0xFF9800, 0.9),
                    width: 2,
                    label: Some("VWAP".into()),
                },
                Line {
                    breaks: false,
                    y: lower.clone(),
                    color: rgba(0x787B86, 0.5),
                    width: 1,
                    label: Some("VWAP Lower".into()),
                },
            ],
            fills: vec![Fill {
                y1: upper,
                y2: lower,
                color: rgba(0xFF9800, 0.08),
                color_below: None,
            }],
            is_overlay: true,
            label: "VWAP Bands".into(),
            ..Default::default()
        }
    }
}

// ---- Heikin Ashi (overlay) ----

pub struct HeikinAshi;

impl Indicator for HeikinAshi {
    fn name(&self) -> &str {
        "HA"
    }

    fn description(&self) -> &str {
        "Heikin Ashi — smoothed trend candles"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut dots = Vec::new();

        let ha = data.heikin_ashi();
        let ha_open: Vec<f64> = ha.bars.iter().map(|b| b.open).collect();
        let ha_close: Vec<f64> = ha.bars.iter().map(|b| b.close).collect();

        for i in 0..n {
            let bullish = ha_close[i] >= ha_open[i];
            let color = if bullish {
                rgba(0x26a69a, 0.9) // bullish green
            } else {
                rgba(0xef5350, 0.9) // bearish red
            };
            // Label only color-flip bars (bullish/bearish transition = signal event).
            let label = if i > 0 {
                let prev_bullish = ha_close[i - 1] >= ha_open[i - 1];
                if bullish != prev_bullish {
                    if bullish {
                        Some(signals::FLIP_UP.to_string())
                    } else {
                        Some(signals::FLIP_DOWN.to_string())
                    }
                } else {
                    None
                }
            } else {
                None
            };
            if label.is_some() {
                dots.push(Dot {
                    x: i,
                    y: ha_close[i],
                    color,
                    size: 3,
                    label,
                });
            }
        }

        PanelResult {
            // The HA close as a line, so the series wire and the web chart
            // carry it; dots mark only the color flips.
            lines: vec![Line {
                breaks: false,
                y: ha_close,
                color: rgba(0x787B86, 0.9),
                width: 1,
                label: Some("HA Close".into()),
            }],
            dots,
            is_overlay: true,
            label: "HA".into(),
            ..Default::default()
        }
    }
}

// ---- Pivot Points (overlay) ----

pub struct PivotPoints;

/// TradingView "Pivot Points Standard" with Pivots Timeframe "Auto":
/// intraday charts with a multiplier <= 15 (minutes, or seconds on second
/// charts) use the day, other intraday charts the week, daily charts the
/// month, weekly and monthly charts the year.
fn pivot_period(tf: super::pine::Timeframe) -> super::pine::Period {
    use super::pine::{Period, Timeframe};
    match tf {
        Timeframe::Intraday(secs) => {
            let multiplier = if secs < 60 { secs } else { secs / 60 };
            if multiplier <= 15 {
                Period::Day
            } else {
                Period::Week
            }
        }
        Timeframe::Daily(_) => Period::Month,
        Timeframe::Weekly | Timeframe::Monthly => Period::Year,
    }
}

impl Indicator for PivotPoints {
    fn name(&self) -> &str {
        "Pivot"
    }

    fn description(&self) -> &str {
        "Pivot Points Standard (Traditional) — levels from the previous higher-timeframe period"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut pivot = vec![f64::NAN; n];
        let mut r1 = vec![f64::NAN; n];
        let mut s1 = vec![f64::NAN; n];
        let mut r2 = vec![f64::NAN; n];
        let mut s2 = vec![f64::NAN; n];

        // Bars are grouped into UTC calendar periods (see `pivot_period`);
        // every bar of a period gets the levels from the previous period's
        // high / low / close, so each level is a step line per period. Without
        // bar timestamps or a known timeframe the periods are unknown and the
        // lines stay NaN.
        let times = super::pine::bar_times(data);
        let tf = super::pine::timeframe(data);
        if let (Some(times), Some(tf)) = (times, tf) {
            let period = pivot_period(tf);
            let mut prev_hlc: Option<(f64, f64, f64)> = None;
            let mut cur = (f64::NEG_INFINITY, f64::INFINITY, f64::NAN);
            let mut cur_key = None;
            for (i, bar) in data.bars.iter().enumerate() {
                let key = super::pine::period_key(times[i], period);
                if cur_key != Some(key) {
                    if cur_key.is_some() {
                        prev_hlc = Some(cur);
                    }
                    cur = (f64::NEG_INFINITY, f64::INFINITY, f64::NAN);
                    cur_key = Some(key);
                }
                cur = (cur.0.max(bar.high), cur.1.min(bar.low), bar.close);
                if let Some((h, l, c)) = prev_hlc {
                    let p = (h + l + c) / 3.0;
                    pivot[i] = p;
                    r1[i] = 2.0 * p - l;
                    s1[i] = 2.0 * p - h;
                    r2[i] = p + (h - l);
                    s2[i] = p - (h - l);
                }
            }
        }

        PanelResult {
            lines: vec![
                Line {
                    breaks: false,
                    y: pivot,
                    color: rgba(0xFFEB3B, 0.6),
                    width: 1,
                    label: Some("Pivot".into()),
                },
                Line {
                    breaks: false,
                    y: r1,
                    color: rgba(0xef5350, 0.4),
                    width: 1,
                    label: Some("R1".into()),
                },
                Line {
                    breaks: false,
                    y: s1,
                    color: rgba(0x26a69a, 0.4),
                    width: 1,
                    label: Some("S1".into()),
                },
                Line {
                    breaks: false,
                    y: r2,
                    color: rgba(0xef5350, 0.3),
                    width: 1,
                    label: Some("R2".into()),
                },
                Line {
                    breaks: false,
                    y: s2,
                    color: rgba(0x26a69a, 0.3),
                    width: 1,
                    label: Some("S2".into()),
                },
            ],
            is_overlay: true,
            label: "Pivot".into(),
            ..Default::default()
        }
    }
}

// ---- Volume Profile (overlay) ----

#[derive(Clone)]
pub struct VolumeProfile {
    pub bins: usize,
    pub side: String,              // "left" or "right"
    pub range_bars: Option<usize>, // only compute over last N bars (None = all)
    pub split_up_down: bool,       // separate up/down volume colors
    pub color_up: u32,             // hex color for up-volume
    pub color_down: u32,           // hex color for down-volume
    pub opacity: f64,              // overall opacity multiplier
}

impl Default for VolumeProfile {
    fn default() -> Self {
        Self {
            bins: 24,
            side: "left".into(),
            range_bars: None,
            split_up_down: false,
            color_up: 0x26a69a,
            color_down: 0xef5350,
            opacity: 0.35,
        }
    }
}

/// `data` with only the bars opening in `[from, to)`.
fn bars_between(data: &OhlcvData, from: Option<i64>, to: Option<i64>) -> OhlcvData {
    let inside = |b: &&crate::data::Bar| {
        let t = b.date.parse::<i64>().unwrap_or(0);
        from.is_none_or(|f| t >= f) && to.is_none_or(|e| t < e)
    };
    OhlcvData {
        bars: data.bars.iter().filter(inside).cloned().collect(),
        symbol: data.symbol.clone(),
        interval: data.interval.clone(),
        session: data.session,
    }
}

impl Indicator for VolumeProfile {
    fn windowed_profile(&self) -> bool {
        true
    }

    fn wants_intrabars(&self) -> bool {
        true
    }

    fn with_intrabars(&self, chart: &OhlcvData, intrabars: &OhlcvData) -> Option<PanelResult> {
        // `range_bars` counts chart bars: find where they start in time.
        let from = self
            .range_bars
            .filter(|r| *r < chart.len())
            .and_then(|r| chart.bars[chart.len() - r].date.parse().ok());
        let profile = Self {
            range_bars: None,
            ..self.clone()
        };
        Some(profile.compute(&bars_between(intrabars, from, None)))
    }

    fn name(&self) -> &str {
        "VPVR"
    }

    fn description(&self) -> &str {
        "Volume Profile Visible Range — horizontal volume histogram with POC/VAH/VAL"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "bins", "type": "integer", "min": 1, "max": 1000, "default": 24},
            {"name": "side", "type": "string", "default": "left", "description": "left or right"},
            {"name": "range_bars", "type": "integer", "min": 1, "max": 100000, "default": null, "description": "only compute over last N bars (null = all)"},
            {"name": "split_up_down", "type": "boolean", "default": false, "description": "separate up/down volume colors"},
            {"name": "color_up", "type": "string", "default": "#26a69a", "description": "hex color for up-volume"},
            {"name": "color_down", "type": "string", "default": "#ef5350", "description": "hex color for down-volume"},
            {"name": "opacity", "type": "number", "min": 0, "max": 1, "default": 0.35, "description": "overall opacity multiplier (0.0-1.0)"}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("bins").and_then(|v| v.as_u64()) {
            self.bins = v as usize;
        }
        if let Some(v) = params.get("side").and_then(|v| v.as_str()) {
            self.side = v.to_string();
        }
        if let Some(v) = params.get("range_bars") {
            if v.is_null() {
                self.range_bars = None;
            } else if let Some(n) = v.as_u64() {
                self.range_bars = if n == 0 { None } else { Some(n as usize) };
            }
        }
        if let Some(v) = params.get("split_up_down").and_then(|v| v.as_bool()) {
            self.split_up_down = v;
        }
        if let Some(v) = params.get("color_up") {
            if let Some(n) = v.as_u64() {
                self.color_up = n as u32;
            } else if let Some(s) = v.as_str() {
                if let Some(hex) = s.strip_prefix('#') {
                    if hex.len() == 6 {
                        if let Ok(n) = u32::from_str_radix(hex, 16) {
                            self.color_up = n;
                        }
                    }
                }
            }
        }
        if let Some(v) = params.get("color_down") {
            if let Some(n) = v.as_u64() {
                self.color_down = n as u32;
            } else if let Some(s) = v.as_str() {
                if let Some(hex) = s.strip_prefix('#') {
                    if hex.len() == 6 {
                        if let Ok(n) = u32::from_str_radix(hex, 16) {
                            self.color_down = n;
                        }
                    }
                }
            }
        }
        if let Some(v) = params.get("opacity").and_then(|v| v.as_f64()) {
            self.opacity = v.clamp(0.0, 1.0);
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        if n == 0 {
            return PanelResult {
                is_overlay: true,
                label: "VPVR".into(),
                ..Default::default()
            };
        }

        // Slice to last N bars if range_bars is set
        let bars = if let Some(rb) = self.range_bars {
            let start = n.saturating_sub(rb);
            &data.bars[start..]
        } else {
            &data.bars[..]
        };

        if bars.is_empty() {
            return PanelResult {
                is_overlay: true,
                label: "VPVR".into(),
                ..Default::default()
            };
        }

        let mut price_min = f64::INFINITY;
        let mut price_max = f64::NEG_INFINITY;
        for bar in bars {
            if bar.low < price_min {
                price_min = bar.low;
            }
            if bar.high > price_max {
                price_max = bar.high;
            }
        }
        let price_range = price_max - price_min;
        if price_range <= 0.0 {
            return PanelResult {
                is_overlay: true,
                label: "VPVR".into(),
                ..Default::default()
            };
        }

        let bins = self.bins.max(1);
        let bin_size = price_range / bins as f64;
        let mut volume_bins = vec![0.0_f64; bins];
        let mut volume_bins_up = vec![0.0_f64; bins];
        let mut volume_bins_down = vec![0.0_f64; bins];
        let mut total_volume = 0.0;

        // Distribute volume proportionally across bins the bar spans
        for bar in bars {
            let bar_range = bar.high - bar.low;
            let is_up = bar.close >= bar.open;

            if bar_range <= 0.0 || bar.volume <= 0.0 {
                // Flat bar: assign all volume to close-price bin
                let idx = ((bar.close - price_min) / bin_size).floor() as usize;
                let idx = idx.min(bins - 1);
                volume_bins[idx] += bar.volume;
                if is_up {
                    volume_bins_up[idx] += bar.volume;
                } else {
                    volume_bins_down[idx] += bar.volume;
                }
            } else {
                let lo_bin = ((bar.low - price_min) / bin_size).floor() as usize;
                let hi_bin = ((bar.high - price_min) / bin_size).floor() as usize;
                let lo_bin = lo_bin.min(bins - 1);
                let hi_bin = hi_bin.min(bins - 1);
                if lo_bin == hi_bin {
                    volume_bins[lo_bin] += bar.volume;
                    if is_up {
                        volume_bins_up[lo_bin] += bar.volume;
                    } else {
                        volume_bins_down[lo_bin] += bar.volume;
                    }
                } else {
                    for (b, bin_vol) in volume_bins
                        .iter_mut()
                        .enumerate()
                        .take(hi_bin + 1)
                        .skip(lo_bin)
                    {
                        let bin_lo = price_min + b as f64 * bin_size;
                        let bin_hi = bin_lo + bin_size;
                        let overlap_lo = bar.low.max(bin_lo);
                        let overlap_hi = bar.high.min(bin_hi);
                        let fraction = (overlap_hi - overlap_lo) / bar_range;
                        let vol_portion = bar.volume * fraction.max(0.0);
                        *bin_vol += vol_portion;
                        if is_up {
                            volume_bins_up[b] += vol_portion;
                        } else {
                            volume_bins_down[b] += vol_portion;
                        }
                    }
                }
            }
            total_volume += bar.volume;
        }

        // POC (based on total volume)
        let mut poc_idx = 0;
        let mut max_vol = 0.0_f64;
        for (i, &v) in volume_bins.iter().enumerate() {
            if v > max_vol {
                max_vol = v;
                poc_idx = i;
            }
        }
        let poc_price = price_min + (poc_idx as f64 + 0.5) * bin_size;

        // Value Area: 70% of total volume expanding from POC
        let target_vol = total_volume * 0.70;
        let mut va_vol = volume_bins[poc_idx];
        let mut va_low_idx = poc_idx;
        let mut va_high_idx = poc_idx;

        while va_vol < target_vol && (va_low_idx > 0 || va_high_idx < bins - 1) {
            let below = va_low_idx.checked_sub(1).map(|i| volume_bins[i]);
            let above = volume_bins.get(va_high_idx + 1).copied();
            if value_area_grows_down(below, above, poc_idx - va_low_idx, va_high_idx - poc_idx) {
                va_low_idx -= 1;
                va_vol += volume_bins[va_low_idx];
            } else {
                va_high_idx += 1;
                va_vol += volume_bins[va_high_idx];
            }
        }

        let vah_price = price_min + (va_high_idx as f64 + 1.0) * bin_size;
        let val_price = price_min + va_low_idx as f64 * bin_size;

        // Build HBars
        let max_width = 0.25; // max 25% of chart width
        let opacity = self.opacity;
        let mut hbars = Vec::with_capacity(if self.split_up_down { bins * 2 } else { bins });

        for (i, &vol) in volume_bins.iter().enumerate() {
            if vol <= 0.0 {
                continue;
            }
            let center = price_min + (i as f64 + 0.5) * bin_size;

            if self.split_up_down {
                let up_vol = volume_bins_up[i];
                let down_vol = volume_bins_down[i];
                let up_width = if max_vol > 0.0 {
                    (up_vol / max_vol) * max_width
                } else {
                    0.0
                };
                let down_width = if max_vol > 0.0 {
                    (down_vol / max_vol) * max_width
                } else {
                    0.0
                };

                if up_vol > 0.0 {
                    hbars.push(HBar {
                        y: center,
                        height: bin_size * 0.9,
                        width: up_width,
                        offset: 0.0,
                        color: rgba(self.color_up, opacity),
                        left: self.side == "left",
                    });
                }
                if down_vol > 0.0 {
                    hbars.push(HBar {
                        y: center,
                        height: bin_size * 0.9,
                        width: down_width,
                        offset: up_width,
                        color: rgba(self.color_down, opacity),
                        left: self.side == "left",
                    });
                }
            } else {
                let width_frac = if max_vol > 0.0 {
                    (vol / max_vol) * max_width
                } else {
                    0.0
                };
                let color = if i == poc_idx {
                    rgba(0xFFD700, (opacity * 2.0).min(1.0))
                } else if i >= va_low_idx && i <= va_high_idx {
                    rgba(0x2962FF, (opacity * 1.14).min(1.0))
                } else {
                    rgba(0x2962FF, opacity * 0.57)
                };
                hbars.push(HBar {
                    y: center,
                    height: bin_size * 0.9,
                    width: width_frac,
                    offset: 0.0,
                    color,
                    left: self.side == "left",
                });
            }
        }

        // HLines for POC, VAH, VAL
        let poc_hline = HLine {
            y: poc_price,
            color: rgba(0xFFD700, 0.8),
        };
        let vah_hline = HLine {
            y: vah_price,
            color: rgba(0x787B86, 0.6),
        };
        let val_hline = HLine {
            y: val_price,
            color: rgba(0x787B86, 0.6),
        };

        PanelResult {
            hbars,
            hlines: vec![poc_hline, vah_hline, val_hline],
            is_overlay: true,
            label: "VPVR".into(),
            ..Default::default()
        }
    }
}

// ---- Kalman Volume Filter [ChartPrime] (panel) ----

pub struct KalmanVolume {
    pub vzo_length: usize,
    pub k: f64,
    pub sig_length: usize,
    pub ob_os_zone: f64,
}

impl Default for KalmanVolume {
    fn default() -> Self {
        Self {
            vzo_length: 70,
            k: 0.06,
            sig_length: 10,
            ob_os_zone: 0.6,
        }
    }
}

impl Indicator for KalmanVolume {
    fn name(&self) -> &str {
        "KVF"
    }

    fn description(&self) -> &str {
        "Kalman Volume Filter — smoothed volume zone oscillator"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "vzo_length", "type": "integer", "min": 1, "max": 10000, "default": 70},
            {"name": "k", "type": "number", "min": 0, "max": 1000, "default": 0.06},
            {"name": "sig_length", "type": "integer", "min": 1, "max": 10000, "default": 10}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("vzo_length").and_then(|v| v.as_u64()) {
            self.vzo_length = v as usize;
        }
        if let Some(v) = params.get("k").and_then(|v| v.as_f64()) {
            self.k = v;
        }
        if let Some(v) = params.get("sig_length").and_then(|v| v.as_u64()) {
            self.sig_length = v as usize;
        }
    }

    fn warmup_bars(&self) -> usize {
        // HMA, its 200-bar stdev, the Kalman smoothing (alpha k), the SMA.
        use super::pine::{decay_bars, ma_warmup};
        ma_warmup("hma", self.vzo_length)
            .saturating_add(200)
            .saturating_add(decay_bars(self.k))
            .saturating_add(self.sig_length)
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        use super::pine::{div, hma_n, stdev};
        let n = data.len();
        let closes = data.closes();
        let volumes: Vec<f64> = data.bars.iter().map(|b| b.volume).collect();

        // Volume direction: positive if close > close[i-2], else negative.
        // On bars 0-1 close[2] is na, the comparison is false -> negative.
        let vol_dir: Vec<f64> = (0..n)
            .map(|i| {
                if i >= 2 && closes[i] > closes[i - 2] {
                    volumes[i]
                } else {
                    -volumes[i]
                }
            })
            .collect();

        // VZO (ta.hma: floor(sqrt(len)) final WMA)
        let vzo_volume = hma_n(&vol_dir, self.vzo_length);
        let total_volume = hma_n(&volumes, self.vzo_length);

        let mut vzo_raw = vec![f64::NAN; n];
        for i in 0..n {
            if !vzo_volume[i].is_nan() && !total_volume[i].is_nan() && total_volume[i].abs() > 1e-30
            {
                vzo_raw[i] = vzo_volume[i] / total_volume[i];
            }
        }

        // Z-score normalize VZO: vzo / ta.stdev(vzo, 200), na until 200 valid bars
        let vzo = div(&vzo_raw, &stdev(&vzo_raw, 200));

        // Kalman filter — Pine Script initializes M_n = 0.0, nz(M_n[1]) = 0:
        // a na input gives a na M_n, and the next bar restarts from 0.
        let mut m_n = vec![f64::NAN; n];
        for i in 0..n {
            let prev_m = if i > 0 && !m_n[i - 1].is_nan() {
                m_n[i - 1]
            } else {
                0.0
            };
            m_n[i] = self.k * vzo[i] + (1.0 - self.k) * prev_m;
        }

        // Signal line = SMA(m_n, sig_length)
        let signal = sma(&m_n, self.sig_length);

        // Volume histogram background
        let vol_std = stdev(&volumes, 200);
        let mut vol_norm = vec![f64::NAN; n];
        for i in 0..n {
            if !vol_std[i].is_nan() && vol_std[i] > 1e-30 {
                let v = (volumes[i] / vol_std[i]) / 10.0;
                vol_norm[i] = if v > 0.8 { 0.0 } else { v };
            }
        }

        // Kalman line and its signal as lines (the series wire carries no
        // unlabeled dots); the bull/bear fills between them keep the colors.
        let mut dots = Vec::new();
        let cyan = 0x00BCD4;
        let red = 0xFF1744;
        let both = |i: usize| !m_n[i].is_nan() && !signal[i].is_nan();
        let kvf_lines = vec![
            Line {
                breaks: false,
                y: (0..n)
                    .map(|i| if both(i) { m_n[i] } else { f64::NAN })
                    .collect(),
                color: rgba(cyan, 0.9),
                width: 2,
                label: Some("KVF".into()),
            },
            Line {
                breaks: false,
                y: (0..n)
                    .map(|i| if both(i) { signal[i] } else { f64::NAN })
                    .collect(),
                color: rgba(red, 0.6),
                width: 1,
                label: Some("Signal".into()),
            },
        ];

        // Signal crossover/crossunder dots (at offset -1)
        for i in 2..n {
            if m_n[i].is_nan()
                || m_n[i - 1].is_nan()
                || signal[i].is_nan()
                || signal[i - 1].is_nan()
            {
                continue;
            }
            let cross_over = m_n[i - 1] <= signal[i - 1] && m_n[i] > signal[i];
            let cross_under = m_n[i - 1] >= signal[i - 1] && m_n[i] < signal[i];

            let prev_m = m_n[i - 1];
            let in_neutral = prev_m.abs() < self.ob_os_zone;

            if cross_over {
                let (size, color, label) = if prev_m < -self.ob_os_zone {
                    (6, rgba(cyan, 1.0), signals::BUY_OVERSOLD) // oversold buy
                } else if in_neutral {
                    (4, rgba(cyan, 1.0), signals::BUY) // local buy
                } else {
                    continue;
                };
                dots.push(Dot {
                    x: i - 1,
                    y: prev_m,
                    color,
                    size,
                    label: Some(label.to_string()),
                });
            } else if cross_under {
                let (size, color, label) = if prev_m > self.ob_os_zone {
                    (6, rgba(red, 1.0), signals::SELL_OVERBOUGHT) // overbought sell
                } else if in_neutral {
                    (4, rgba(red, 1.0), signals::SELL) // local sell
                } else {
                    continue;
                };
                dots.push(Dot {
                    x: i - 1,
                    y: prev_m,
                    color,
                    size,
                    label: Some(label.to_string()),
                });
            }
        }

        // Fills between m_n and signal (bullish / bearish)
        let mut bull_m = vec![f64::NAN; n];
        let mut bull_s = vec![f64::NAN; n];
        let mut bear_m = vec![f64::NAN; n];
        let mut bear_s = vec![f64::NAN; n];
        for i in 0..n {
            if m_n[i].is_nan() || signal[i].is_nan() {
                continue;
            }
            if m_n[i] > signal[i] {
                bull_m[i] = m_n[i];
                bull_s[i] = signal[i];
            } else {
                bear_m[i] = m_n[i];
                bear_s[i] = signal[i];
            }
        }

        // OB/OS zone fills
        let top = vec![2.0; n];
        let ob_line = vec![self.ob_os_zone; n];
        let os_line = vec![-self.ob_os_zone; n];
        let bottom = vec![-2.0; n];

        // Volume histogram bars (symmetric around zero)
        let mut pos_vol = vec![0.0; n];
        let mut neg_heights = vec![0.0; n];
        let mut bar_colors = Vec::with_capacity(n);
        let faint_teal = rgba(0x00BCD4, 0.15);
        for i in 0..n {
            let v = if vol_norm[i].is_nan() {
                0.0
            } else {
                vol_norm[i]
            };
            pos_vol[i] = v;
            neg_heights[i] = v; // height of negative bar (drawn from -v to 0)
            bar_colors.push(faint_teal);
        }

        // Negative bars: use negative y so bottom+y goes below zero
        let neg_y: Vec<f64> = neg_heights.iter().map(|v| -v).collect();

        PanelResult {
            lines: kvf_lines,
            fills: vec![
                Fill {
                    y1: bull_m,
                    y2: bull_s,
                    color: rgba(cyan, 0.15),
                    color_below: None,
                },
                Fill {
                    y1: bear_m,
                    y2: bear_s,
                    color: rgba(red, 0.15),
                    color_below: None,
                },
                Fill {
                    y1: top,
                    y2: ob_line,
                    color: rgba(red, 0.1),
                    color_below: None,
                },
                Fill {
                    y1: os_line,
                    y2: bottom,
                    color: rgba(cyan, 0.1),
                    color_below: None,
                },
            ],
            bars: vec![
                Bars {
                    y: pos_vol,
                    colors: bar_colors.clone(),
                    bottom: 0.0,
                },
                Bars {
                    y: neg_y,
                    colors: bar_colors,
                    bottom: 0.0,
                },
            ],
            dots,
            hlines: vec![HLine {
                y: 0.0,
                color: rgba(0x787B86, 0.4),
            }],
            hbars: vec![],
            divlines: vec![],
            y_range: Some((-2.5, 2.5)),
            label: "KVF".into(),
            is_overlay: false,
            ..Default::default()
        }
    }
}

// ---- Ichimoku Cloud (overlay) ----

pub struct Ichimoku {
    pub tenkan: usize,
    pub kijun: usize,
    pub senkou_b: usize,
    /// TradingView's "Lagging Span" displacement.
    pub displacement: usize,
}

impl Default for Ichimoku {
    fn default() -> Self {
        Self {
            tenkan: 9,
            kijun: 26,
            senkou_b: 52,
            displacement: 26,
        }
    }
}

impl Ichimoku {
    /// Bars the leading spans are drawn ahead: Pine's `displacement - 1`.
    fn lead(&self) -> usize {
        self.displacement.saturating_sub(1)
    }

    /// Tenkan, Kijun and the unshifted Senkou A / B of every bar.
    fn spans(&self, data: &OhlcvData) -> [Vec<f64>; 4] {
        let highs = data.highs();
        let lows = data.lows();
        let mid = |len: usize| -> Vec<f64> {
            let (hh, ll) = (highest(&highs, len), lowest(&lows, len));
            hh.iter().zip(&ll).map(|(h, l)| (h + l) / 2.0).collect()
        };
        let (tenkan, kijun) = (mid(self.tenkan), mid(self.kijun));
        let senkou_a = tenkan
            .iter()
            .zip(&kijun)
            .map(|(t, k)| (t + k) / 2.0)
            .collect();
        let senkou_b = mid(self.senkou_b);
        [tenkan, kijun, senkou_a, senkou_b]
    }
}

impl Indicator for Ichimoku {
    fn name(&self) -> &str {
        "Ichimoku"
    }

    fn description(&self) -> &str {
        "Ichimoku Cloud — trend, support/resistance, momentum"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "tenkan", "type": "integer", "min": 1, "max": 10000, "default": 9},
            {"name": "kijun", "type": "integer", "min": 1, "max": 10000, "default": 26},
            {"name": "senkou_b", "type": "integer", "min": 1, "max": 10000, "default": 52},
            {"name": "displacement", "type": "integer", "min": 1, "max": 10000, "default": 26,
             "description": "Leading spans are drawn displacement - 1 bars ahead, the lagging span (close) as many bars back"}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("tenkan").and_then(|v| v.as_u64()) {
            self.tenkan = v as usize;
        }
        if let Some(v) = params.get("kijun").and_then(|v| v.as_u64()) {
            self.kijun = v as usize;
        }
        if let Some(v) = params.get("senkou_b").and_then(|v| v.as_u64()) {
            self.senkou_b = v as usize;
        }
        if let Some(v) = params.get("displacement").and_then(|v| v.as_u64()) {
            self.displacement = v as usize;
        }
    }

    fn warmup_bars(&self) -> usize {
        // The leading spans at a bar come from the windows `lead` bars back.
        self.tenkan.max(self.kijun).max(self.senkou_b) + self.lead()
    }

    fn projection(&self, data: &OhlcvData) -> Option<Projection> {
        let off = self.lead();
        if off == 0 || data.bars.is_empty() {
            return None;
        }
        // Future bar k (index n + k) shows the spans of bar n + k - off.
        let [_, _, a, b] = self.spans(data);
        let n = data.len();
        let bars = off.min(MAX_PROJECTION_BARS);
        let tail = |v: &[f64]| -> Vec<f64> {
            (0..bars)
                .map(|k| (n + k).checked_sub(off).map_or(f64::NAN, |j| v[j]))
                .collect()
        };
        let (a, b) = (tail(&a), tail(&b));
        Some(Projection {
            bars,
            lines: vec![(2, a.clone()), (3, b.clone())],
            fills: vec![(0, a, b)],
        })
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let [tenkan_sen, kijun_sen, senkou_a, senkou_b] = self.spans(data);
        // TV plots the leading spans with `offset = displacement - 1`: bar i
        // shows the value computed displacement - 1 bars earlier. Values
        // past the last bar are `projection` (ADR-0033).
        // The lagging span is the close with `offset = -displacement + 1`.
        let off = self.lead() as i64;
        let senkou_a = super::pine::shift(&senkou_a, off);
        let senkou_b = super::pine::shift(&senkou_b, off);
        let lagging = super::pine::shift(&data.closes(), -off);

        PanelResult {
            lines: vec![
                Line {
                    breaks: false,
                    y: tenkan_sen,
                    color: rgba(0x0095ff, 0.9),
                    width: 1,
                    label: Some("Tenkan".into()),
                },
                Line {
                    breaks: false,
                    y: kijun_sen,
                    color: rgba(0xff0044, 0.9),
                    width: 1,
                    label: Some("Kijun".into()),
                },
                Line {
                    breaks: false,
                    y: senkou_a.clone(),
                    color: rgba(0x26a69a, 0.5),
                    width: 1,
                    label: Some("Senkou A".into()),
                },
                Line {
                    breaks: false,
                    y: senkou_b.clone(),
                    color: rgba(0xef5350, 0.5),
                    width: 1,
                    label: Some("Senkou B".into()),
                },
                Line {
                    breaks: false,
                    y: lagging,
                    color: rgba(0x43A047, 0.9),
                    width: 1,
                    label: Some("Lagging Span".into()),
                },
            ],
            fills: vec![Fill {
                y1: senkou_a,
                y2: senkou_b,
                // TradingView cloud: green where Senkou A >= B, red below.
                color: rgba(0x4CAF50, 0.2),
                color_below: Some(rgba(0xEF5350, 0.2)),
            }],
            is_overlay: true,
            label: "Ichimoku".into(),
            ..Default::default()
        }
    }
}

// ---- Shared profile helpers (used by Session VP, HVN/LVN, Naked POC, TPO) ----

struct ProfileResult {
    volume_bins: Vec<f64>,
    price_min: f64,
    bin_size: f64,
    bins: usize,
    poc_idx: usize,
    max_bin_volume: f64,
    #[allow(dead_code)]
    total_volume: f64,
    poc_price: f64,
    vah_price: f64,
    val_price: f64,
}

/// Build a price-binned profile from a slice of bars.
/// If `time_based`, each bar contributes 1.0 per bin touched (weighted by overlap);
/// otherwise weighted by bar volume. Returns None if bars are empty or price range is zero.
fn compute_profile(
    bars: &[crate::data::Bar],
    bins: usize,
    time_based: bool,
) -> Option<ProfileResult> {
    if bars.is_empty() || bins == 0 {
        return None;
    }
    let mut price_min = f64::INFINITY;
    let mut price_max = f64::NEG_INFINITY;
    for bar in bars {
        if bar.low < price_min {
            price_min = bar.low;
        }
        if bar.high > price_max {
            price_max = bar.high;
        }
    }
    let price_range = price_max - price_min;
    if !price_range.is_finite() || price_range <= 0.0 {
        return None;
    }
    let bin_size = price_range / bins as f64;
    let mut volume_bins = vec![0.0_f64; bins];
    let mut total = 0.0;

    for bar in bars {
        let weight = if time_based { 1.0 } else { bar.volume };
        if !weight.is_finite() || weight <= 0.0 {
            continue;
        }
        let bar_range = bar.high - bar.low;
        if bar_range <= 0.0 {
            let idx = ((bar.close - price_min) / bin_size).floor() as usize;
            let idx = idx.min(bins - 1);
            volume_bins[idx] += weight;
        } else {
            let lo_bin = ((bar.low - price_min) / bin_size).floor() as usize;
            let hi_bin = ((bar.high - price_min) / bin_size).floor() as usize;
            let lo_bin = lo_bin.min(bins - 1);
            let hi_bin = hi_bin.min(bins - 1);
            if lo_bin == hi_bin {
                volume_bins[lo_bin] += weight;
            } else {
                for (b, bin_vol) in volume_bins
                    .iter_mut()
                    .enumerate()
                    .take(hi_bin + 1)
                    .skip(lo_bin)
                {
                    let bin_lo = price_min + b as f64 * bin_size;
                    let bin_hi = bin_lo + bin_size;
                    let overlap_lo = bar.low.max(bin_lo);
                    let overlap_hi = bar.high.min(bin_hi);
                    let fraction = ((overlap_hi - overlap_lo) / bar_range).max(0.0);
                    *bin_vol += weight * fraction;
                }
            }
        }
        total += weight;
    }

    if total <= 0.0 {
        return None;
    }

    let mut poc_idx = 0;
    let mut max_vol = 0.0_f64;
    for (i, &v) in volume_bins.iter().enumerate() {
        if v > max_vol {
            max_vol = v;
            poc_idx = i;
        }
    }

    let target = total * 0.70;
    let mut va_vol = volume_bins[poc_idx];
    let mut lo = poc_idx;
    let mut hi = poc_idx;
    while va_vol < target && (lo > 0 || hi < bins - 1) {
        let below = lo.checked_sub(1).map(|i| volume_bins[i]);
        let above = volume_bins.get(hi + 1).copied();
        if value_area_grows_down(below, above, poc_idx - lo, hi - poc_idx) {
            lo -= 1;
            va_vol += volume_bins[lo];
        } else {
            hi += 1;
            va_vol += volume_bins[hi];
        }
    }

    Some(ProfileResult {
        volume_bins,
        price_min,
        bin_size,
        bins,
        poc_idx,
        max_bin_volume: max_vol,
        total_volume: total,
        poc_price: price_min + (poc_idx as f64 + 0.5) * bin_size,
        vah_price: price_min + (hi as f64 + 1.0) * bin_size,
        val_price: price_min + lo as f64 * bin_size,
    })
}

/// Partition bar indices into sessions. Honors `bars_per_session` if Some & > 0,
/// otherwise buckets by epoch-seconds date using `period_secs`. Fallback on parse
/// failure: put all bars in one session.
fn session_ranges(
    data: &OhlcvData,
    period_secs: i64,
    bars_per_session: Option<usize>,
) -> Vec<(usize, usize)> {
    let n = data.len();
    if n == 0 {
        return vec![];
    }
    if let Some(bps) = bars_per_session {
        if bps == 0 {
            return vec![(0, n)];
        }
        let mut out = Vec::new();
        let mut s = 0;
        while s < n {
            let e = (s + bps).min(n);
            out.push((s, e));
            s = e;
        }
        return out;
    }
    if period_secs <= 0 {
        return vec![(0, n)];
    }
    let mut out = Vec::new();
    let mut cur_bucket: Option<i64> = None;
    let mut start = 0;
    for i in 0..n {
        let ts = match data.bars[i].date.parse::<i64>() {
            Ok(v) => v,
            Err(_) => return vec![(0, n)],
        };
        let bucket = ts.div_euclid(period_secs);
        match cur_bucket {
            Some(b) if b == bucket => {}
            Some(_) => {
                out.push((start, i));
                start = i;
                cur_bucket = Some(bucket);
            }
            None => {
                cur_bucket = Some(bucket);
                start = i;
            }
        }
    }
    out.push((start, n));
    out
}

fn parse_period_secs(session: &str) -> i64 {
    match session.to_ascii_lowercase().as_str() {
        "weekly" | "week" | "1w" | "w" => 7 * 86400,
        "hourly" | "1h" | "h" => 3600,
        _ => 86400,
    }
}

// ---- Session Volume Profile (overlay) ----

/// Periodic volume profile: resets per session (daily/weekly/N-bar) and draws
/// per-session POC/VAH/VAL as one line each that holds a level per session. Its tails
/// recompute it from their own bars, so unlike the profiles that keep their load it
/// reads no finer bars: the load and the live part would disagree (ADR-0048).
pub struct SessionVolumeProfile {
    pub bins: usize,
    pub session: String,
    pub bars_per_session: Option<usize>,
    pub show_value_area: bool,
}

impl Default for SessionVolumeProfile {
    fn default() -> Self {
        Self {
            bins: 24,
            session: "daily".into(),
            bars_per_session: None,
            show_value_area: true,
        }
    }
}

impl Indicator for SessionVolumeProfile {
    fn name(&self) -> &str {
        "SVP"
    }

    fn description(&self) -> &str {
        "Session Volume Profile — per-session POC/VAH/VAL that reset each day/week"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "bins", "type": "integer", "min": 1, "max": 1000, "default": 24},
            {"name": "session", "type": "string", "default": "daily", "description": "daily, weekly, or hourly"},
            {"name": "bars_per_session", "type": "integer", "min": 1, "max": 100000, "default": null, "description": "override session by fixed bar count"},
            {"name": "show_value_area", "type": "boolean", "default": true, "description": "draw VAH/VAL in addition to POC"}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("bins").and_then(|v| v.as_u64()) {
            self.bins = v as usize;
        }
        if let Some(v) = params.get("session").and_then(|v| v.as_str()) {
            self.session = v.to_string();
        }
        if let Some(v) = params.get("bars_per_session") {
            if v.is_null() {
                self.bars_per_session = None;
            } else if let Some(n) = v.as_u64() {
                self.bars_per_session = if n == 0 { None } else { Some(n as usize) };
            }
        }
        if let Some(v) = params.get("show_value_area").and_then(|v| v.as_bool()) {
            self.show_value_area = v;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut out = PanelResult {
            is_overlay: true,
            label: "SVP".into(),
            // Each session holds its levels through its last bar; what is drawn stops
            // there rather than sloping to the next session's levels.
            level_lines: true,
            ..Default::default()
        };
        if n == 0 {
            return out;
        }

        let period_secs = parse_period_secs(&self.session);
        let ranges = session_ranges(data, period_secs, self.bars_per_session);
        let mut poc_line = vec![f64::NAN; n];
        let mut vah_line = vec![f64::NAN; n];
        let mut val_line = vec![f64::NAN; n];

        for (s, e) in ranges {
            if e <= s {
                continue;
            }
            let Some(prof) = compute_profile(&data.bars[s..e], self.bins.max(1), false) else {
                continue;
            };

            for i in s..e {
                poc_line[i] = prof.poc_price;
                vah_line[i] = prof.vah_price;
                val_line[i] = prof.val_price;
            }
        }

        // One line each, whatever the number of sessions in this window: a live tail
        // holds fewer of them than a load, and must keep the load's shape.
        out.lines.push(Line {
            breaks: false,
            y: poc_line,
            color: rgba(0xFFD700, 0.9),
            width: 2,
            label: Some("POC".into()),
        });
        if self.show_value_area {
            out.lines.push(Line {
                breaks: false,
                y: vah_line,
                color: rgba(0x787B86, 0.7),
                width: 1,
                label: Some("VAH".into()),
            });
            out.lines.push(Line {
                breaks: false,
                y: val_line,
                color: rgba(0x787B86, 0.7),
                width: 1,
                label: Some("VAL".into()),
            });
        }

        out
    }
}

// ---- Fixed range and anchored volume profiles (overlay) ----

/// A time from an indicator parameter, unix seconds: a number of seconds, or
/// `YYYY-MM-DD` or `YYYY-MM-DD HH:MM` in UTC. Empty or unreadable is no limit.
fn parse_time_param(value: &Value) -> Option<i64> {
    if let Some(n) = value.as_i64() {
        return Some(n);
    }
    let text = value.as_str()?.trim();
    if let Ok(n) = text.parse::<i64>() {
        return Some(n);
    }
    let date_time = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M")
        .ok()
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
        })?;
    Some(date_time.and_utc().timestamp())
}

/// The profile of a time range of the loaded bars. `anchored` leaves out the end of
/// the range, so it runs to the newest bar: that is all that tells the two apart.
pub struct RangeVolumeProfile {
    pub profile: VolumeProfile,
    pub anchored: bool,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

impl RangeVolumeProfile {
    pub fn fixed() -> Self {
        Self {
            profile: VolumeProfile::default(),
            anchored: false,
            from: None,
            to: None,
        }
    }

    pub fn anchored() -> Self {
        Self {
            anchored: true,
            ..Self::fixed()
        }
    }

    fn short_name(&self) -> &'static str {
        if self.anchored {
            "AVP"
        } else {
            "FRVP"
        }
    }

    fn range_of(&self, data: &OhlcvData) -> PanelResult {
        let mut out = self
            .profile
            .compute(&bars_between(data, self.from, self.to));
        out.label = self.short_name().into();
        out
    }
}

impl Indicator for RangeVolumeProfile {
    fn windowed_profile(&self) -> bool {
        true
    }

    fn wants_intrabars(&self) -> bool {
        true
    }

    fn with_intrabars(&self, _chart: &OhlcvData, intrabars: &OhlcvData) -> Option<PanelResult> {
        Some(self.range_of(intrabars))
    }

    fn name(&self) -> &str {
        self.short_name()
    }

    fn description(&self) -> &str {
        if self.anchored {
            "Anchored Volume Profile: histogram with POC/VAH/VAL from an anchor time to the newest bar"
        } else {
            "Fixed Range Volume Profile: histogram with POC/VAH/VAL over a time range"
        }
    }

    fn params(&self) -> Value {
        let time =
            "YYYY-MM-DD, YYYY-MM-DD HH:MM in UTC, or unix seconds. Empty: the first loaded bar.";
        let mut params = vec![
            json!({"name": "bins", "type": "integer", "min": 1, "max": 1000, "default": 24}),
            json!({"name": "side", "type": "string", "default": "left", "description": "left or right"}),
            json!({"name": "opacity", "type": "number", "min": 0, "max": 1, "default": 0.35}),
        ];
        if self.anchored {
            params.push(
                json!({"name": "anchor", "type": "string", "default": "", "description": time}),
            );
        } else {
            params.push(
                json!({"name": "from", "type": "string", "default": "", "description": time}),
            );
            params.push(json!({"name": "to", "type": "string", "default": "", "description": "As from. Empty: the newest bar."}));
        }
        Value::Array(params)
    }

    fn configure(&mut self, params: &Value) {
        self.profile.configure(params);
        if let Some(v) = params.get("anchor").filter(|_| self.anchored) {
            self.from = parse_time_param(v);
        }
        if !self.anchored {
            if let Some(v) = params.get("from") {
                self.from = parse_time_param(v);
            }
            if let Some(v) = params.get("to") {
                self.to = parse_time_param(v);
            }
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        self.range_of(data)
    }
}

// ---- HVN / LVN Auto-Detection (overlay) ----

/// Detects High and Low Volume Nodes on the visible-range profile and marks
/// them as horizontal support/resistance lines.
pub struct HvnLvn {
    pub bins: usize,
    pub range_bars: Option<usize>,
    pub neighborhood: usize,
    pub min_prominence: f64,
    pub top_n: Option<usize>,
    pub show_hvn: bool,
    pub show_lvn: bool,
}

impl Default for HvnLvn {
    fn default() -> Self {
        Self {
            bins: 48,
            range_bars: None,
            neighborhood: 2,
            min_prominence: 0.10,
            top_n: None,
            show_hvn: true,
            show_lvn: true,
        }
    }
}

impl Indicator for HvnLvn {
    fn windowed_profile(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "HVN/LVN"
    }

    fn description(&self) -> &str {
        "High/Low Volume Nodes — auto-detected support/resistance from the volume profile"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "bins", "type": "integer", "min": 1, "max": 1000, "default": 48},
            {"name": "range_bars", "type": "integer", "min": 1, "max": 100000, "default": null, "description": "only scan the last N bars (null = all)"},
            {"name": "neighborhood", "type": "integer", "min": 1, "max": 10000, "default": 2, "description": "bins on each side to compare for local extrema"},
            {"name": "min_prominence", "type": "number", "min": 0, "max": 1, "default": 0.10, "description": "minimum relative prominence (0-1) vs max bin"},
            {"name": "top_n", "type": "integer", "min": 1, "max": 1000, "default": null, "description": "keep only the N strongest nodes of each kind"},
            {"name": "show_hvn", "type": "boolean", "default": true},
            {"name": "show_lvn", "type": "boolean", "default": true}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("bins").and_then(|v| v.as_u64()) {
            self.bins = v as usize;
        }
        if let Some(v) = params.get("range_bars") {
            if v.is_null() {
                self.range_bars = None;
            } else if let Some(n) = v.as_u64() {
                self.range_bars = if n == 0 { None } else { Some(n as usize) };
            }
        }
        if let Some(v) = params.get("neighborhood").and_then(|v| v.as_u64()) {
            self.neighborhood = v as usize;
        }
        if let Some(v) = params.get("min_prominence").and_then(|v| v.as_f64()) {
            self.min_prominence = v.clamp(0.0, 1.0);
        }
        if let Some(v) = params.get("top_n") {
            if v.is_null() {
                self.top_n = None;
            } else if let Some(n) = v.as_u64() {
                self.top_n = if n == 0 { None } else { Some(n as usize) };
            }
        }
        if let Some(v) = params.get("show_hvn").and_then(|v| v.as_bool()) {
            self.show_hvn = v;
        }
        if let Some(v) = params.get("show_lvn").and_then(|v| v.as_bool()) {
            self.show_lvn = v;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let mut out = PanelResult {
            is_overlay: true,
            label: "HVN/LVN".into(),
            ..Default::default()
        };
        let n = data.len();
        if n == 0 {
            return out;
        }
        let bars = if let Some(rb) = self.range_bars {
            &data.bars[n.saturating_sub(rb)..]
        } else {
            &data.bars[..]
        };
        let Some(prof) = compute_profile(bars, self.bins.max(3), false) else {
            return out;
        };

        let bins = prof.bins;
        let k = self.neighborhood.max(1);
        let min_abs = prof.max_bin_volume * self.min_prominence;

        // Each entry: (price, prominence). Prominence = |v - nearest-neighbor-extreme|.
        let mut hvns: Vec<(f64, f64)> = Vec::new();
        let mut lvns: Vec<(f64, f64)> = Vec::new();
        for i in 0..bins {
            let v = prof.volume_bins[i];
            let lo = i.saturating_sub(k);
            let hi = (i + k).min(bins - 1);
            let mut is_max = true;
            let mut is_min = true;
            let mut nbr_max = 0.0_f64;
            let mut nbr_min = f64::INFINITY;
            for j in lo..=hi {
                if j == i {
                    continue;
                }
                let nv = prof.volume_bins[j];
                if nv >= v {
                    is_max = false;
                }
                if nv <= v {
                    is_min = false;
                }
                if nv > nbr_max {
                    nbr_max = nv;
                }
                if nv < nbr_min {
                    nbr_min = nv;
                }
            }
            let price = prof.price_min + (i as f64 + 0.5) * prof.bin_size;
            if is_max && (v - nbr_max) >= min_abs && v > 0.0 {
                hvns.push((price, v - nbr_max));
            }
            if is_min
                && nbr_max.is_finite()
                && nbr_max > 0.0
                && (nbr_max - v) >= min_abs
                && v < prof.max_bin_volume
            {
                lvns.push((price, nbr_max - v));
            }
        }

        let by_prominence_desc = |a: &(f64, f64), b: &(f64, f64)| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        };
        hvns.sort_by(by_prominence_desc);
        lvns.sort_by(by_prominence_desc);
        if let Some(k) = self.top_n {
            hvns.truncate(k);
            lvns.truncate(k);
        }

        if self.show_hvn {
            for (price, _) in &hvns {
                out.hlines.push(HLine {
                    y: *price,
                    color: rgba(0x26a69a, 0.75),
                });
            }
        }
        if self.show_lvn {
            for (price, _) in &lvns {
                out.hlines.push(HLine {
                    y: *price,
                    color: rgba(0xef5350, 0.55),
                });
            }
        }
        out
    }
}

// ---- Naked POC (overlay) ----

/// Tracks per-session POCs and draws only those that have not been revisited
/// (price has not crossed the level since the session ended).
pub struct NakedPoc {
    pub bins: usize,
    pub session: String,
    pub bars_per_session: Option<usize>,
    pub max_sessions: Option<usize>,
    pub include_current: bool,
}

impl Default for NakedPoc {
    fn default() -> Self {
        Self {
            bins: 24,
            session: "daily".into(),
            bars_per_session: None,
            max_sessions: Some(20),
            include_current: true,
        }
    }
}

impl Indicator for NakedPoc {
    fn name(&self) -> &str {
        "nPOC"
    }

    fn description(&self) -> &str {
        "Naked POCs — unfilled point-of-control levels from previous sessions"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "bins", "type": "integer", "min": 1, "max": 1000, "default": 24},
            {"name": "session", "type": "string", "default": "daily", "description": "daily, weekly, hourly"},
            {"name": "bars_per_session", "type": "integer", "min": 1, "max": 100000, "default": null},
            {"name": "max_sessions", "type": "integer", "min": 1, "max": 1000, "default": 20, "description": "look back at most N sessions"},
            {"name": "include_current", "type": "boolean", "default": true, "description": "also draw the developing POC of the current session"}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("bins").and_then(|v| v.as_u64()) {
            self.bins = v as usize;
        }
        if let Some(v) = params.get("session").and_then(|v| v.as_str()) {
            self.session = v.to_string();
        }
        if let Some(v) = params.get("bars_per_session") {
            if v.is_null() {
                self.bars_per_session = None;
            } else if let Some(n) = v.as_u64() {
                self.bars_per_session = if n == 0 { None } else { Some(n as usize) };
            }
        }
        if let Some(v) = params.get("max_sessions") {
            if v.is_null() {
                self.max_sessions = None;
            } else if let Some(n) = v.as_u64() {
                self.max_sessions = if n == 0 { None } else { Some(n as usize) };
            }
        }
        if let Some(v) = params.get("include_current").and_then(|v| v.as_bool()) {
            self.include_current = v;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let mut out = PanelResult {
            is_overlay: true,
            label: "nPOC".into(),
            ..Default::default()
        };
        let n = data.len();
        if n == 0 {
            return out;
        }

        let period_secs = parse_period_secs(&self.session);
        let mut ranges = session_ranges(data, period_secs, self.bars_per_session);
        if let Some(max) = self.max_sessions {
            if ranges.len() > max {
                let drop = ranges.len() - max;
                ranges.drain(0..drop);
            }
        }

        let total = ranges.len();
        for (idx, (s, e)) in ranges.iter().enumerate() {
            if *e <= *s {
                continue;
            }
            let slice = &data.bars[*s..*e];
            let Some(prof) = compute_profile(slice, self.bins.max(1), false) else {
                continue;
            };
            let poc = prof.poc_price;
            let is_current = idx + 1 == total;

            if is_current {
                if !self.include_current {
                    continue;
                }
                let mut y = vec![f64::NAN; n];
                for slot in y.iter_mut().take(*e).skip(*s) {
                    *slot = poc;
                }
                out.lines.push(Line {
                    breaks: false,
                    y,
                    color: rgba(0xFFD700, 0.7),
                    width: 2,
                    label: Some("dPOC".into()),
                });
                continue;
            }

            // Naked check: has any bar after this session touched the POC?
            let filled = data.bars[*e..n]
                .iter()
                .any(|b| b.low <= poc && poc <= b.high);
            if filled {
                continue;
            }

            // Draw the naked POC only from session close onwards — it isn't
            // known before the source session ends.
            let mut y = vec![f64::NAN; n];
            for slot in y.iter_mut().skip(*e) {
                *slot = poc;
            }
            out.lines.push(Line {
                breaks: false,
                y,
                color: rgba(0xFF9800, 0.85),
                width: 2,
                label: Some("nPOC".into()),
            });
        }
        out
    }
}

// ---- TPO / Market Profile (overlay) ----

/// Time Price Opportunity profile: like Volume Profile but each bar contributes
/// equal weight (count of time units at each price), not volume.
pub struct Tpo {
    pub bins: usize,
    pub side: String,
    pub range_bars: Option<usize>,
    pub opacity: f64,
}

impl Default for Tpo {
    fn default() -> Self {
        Self {
            bins: 32,
            side: "right".into(),
            range_bars: None,
            opacity: 0.35,
        }
    }
}

impl Indicator for Tpo {
    fn windowed_profile(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "TPO"
    }

    fn description(&self) -> &str {
        "Time Price Opportunity — time-based market profile with POC/VAH/VAL"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "bins", "type": "integer", "min": 1, "max": 1000, "default": 32},
            {"name": "side", "type": "string", "default": "right", "description": "left or right"},
            {"name": "range_bars", "type": "integer", "min": 1, "max": 100000, "default": null, "description": "only compute over last N bars"},
            {"name": "opacity", "type": "number", "min": 0, "max": 1, "default": 0.35, "description": "bar opacity (0.0-1.0)"}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("bins").and_then(|v| v.as_u64()) {
            self.bins = v as usize;
        }
        if let Some(v) = params.get("side").and_then(|v| v.as_str()) {
            self.side = v.to_string();
        }
        if let Some(v) = params.get("range_bars") {
            if v.is_null() {
                self.range_bars = None;
            } else if let Some(n) = v.as_u64() {
                self.range_bars = if n == 0 { None } else { Some(n as usize) };
            }
        }
        if let Some(v) = params.get("opacity").and_then(|v| v.as_f64()) {
            self.opacity = v.clamp(0.0, 1.0);
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let mut out = PanelResult {
            is_overlay: true,
            label: "TPO".into(),
            ..Default::default()
        };
        let n = data.len();
        if n == 0 {
            return out;
        }
        let bars = if let Some(rb) = self.range_bars {
            &data.bars[n.saturating_sub(rb)..]
        } else {
            &data.bars[..]
        };
        let Some(prof) = compute_profile(bars, self.bins.max(1), true) else {
            return out;
        };

        let max_width = 0.25;
        let left = self.side == "left";
        let va_lo_idx = ((prof.val_price - prof.price_min) / prof.bin_size).round() as usize;
        let va_hi_idx = (((prof.vah_price - prof.price_min) / prof.bin_size).round() as usize)
            .saturating_sub(1);

        for (i, &count) in prof.volume_bins.iter().enumerate() {
            if count <= 0.0 {
                continue;
            }
            let center = prof.price_min + (i as f64 + 0.5) * prof.bin_size;
            let width_frac = if prof.max_bin_volume > 0.0 {
                (count / prof.max_bin_volume) * max_width
            } else {
                0.0
            };
            let color = if i == prof.poc_idx {
                rgba(0xFFD700, (self.opacity * 2.0).min(1.0))
            } else if i >= va_lo_idx && i <= va_hi_idx {
                rgba(0x9C27B0, (self.opacity * 1.2).min(1.0))
            } else {
                rgba(0x9C27B0, self.opacity * 0.55)
            };
            out.hbars.push(HBar {
                y: center,
                height: prof.bin_size * 0.9,
                width: width_frac,
                offset: 0.0,
                color,
                left,
            });
        }

        out.hlines.push(HLine {
            y: prof.poc_price,
            color: rgba(0xFFD700, 0.8),
        });
        out.hlines.push(HLine {
            y: prof.vah_price,
            color: rgba(0x9C27B0, 0.5),
        });
        out.hlines.push(HLine {
            y: prof.val_price,
            color: rgba(0x9C27B0, 0.5),
        });

        out
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn kalman_volume_warmup_survives_extreme_k() {
        for k in [0.0, 1e-20, 1000.0] {
            let mut kv = KalmanVolume::default();
            kv.configure(&json!({ "k": k, "vzo_length": 10000, "sig_length": 10000 }));
            assert!(kv.warmup_bars() >= 20_200);
        }
    }

    use super::*;
    use crate::data::sample_data;
    use crate::indicators::pine::test_util::{bars, closes, eq, ohlc, NAN};

    fn flips(r: &PanelResult) -> Vec<(usize, &str)> {
        r.dots
            .iter()
            .filter_map(|d| d.label.as_deref().map(|l| (d.x, l)))
            .collect()
    }

    #[test]
    fn adx_matches_pine_dmi() {
        // +DM = [_, 1, 2, 0, 0, ...], -DM = [_, 0, 0, 0, 2, ...],
        // rma2(ta.tr) = [_, _, 2.5, 1.75, 2.125]: DI starts at bar 2.
        let r = Adx { period: 2 }.compute(&ohlc());
        eq(
            &r.lines[1].y[..5],
            &[NAN, NAN, 60.0, 300.0 / 7.0, 300.0 / 17.0],
        );
        eq(&r.lines[2].y[..5], &[NAN, NAN, 0.0, 0.0, 800.0 / 17.0]);
        // DX = [_, _, 1, 1, 5/11]; ADX = 100 * rma2(DX) seeds one bar later.
        eq(&r.lines[0].y[..5], &[NAN, NAN, NAN, 100.0, 800.0 / 11.0]);
    }

    #[test]
    fn supertrend_starts_down_and_flips_up() {
        let ind = Supertrend {
            period: 2,
            multiplier: 1.0,
        };
        let r = ind.compute(&ohlc());
        // ATR2 = [_, 2, 2.5, 1.75, 2.125, 1.8125, 2.40625, ...]; bar 1 is the
        // first ATR bar, where ta.supertrend's direction is down (upper band).
        eq(
            &st_ys(&r),
            &[NAN, 4.0, 4.0, 4.0, 4.0, 3.0625, 0.09375, 1.796875],
        );
        assert_eq!(flips(&r), vec![(6, signals::FLIP_UP)]);
        assert_eq!(r.dots.len(), 1, "a dot marks only a flip");
        assert_eq!(r.lines[0].label.as_deref(), Some("Up Trend"));
        assert_eq!(r.lines[1].label.as_deref(), Some("Down Trend"));
        assert_breaks_at_flips(&r);
        eq(&r.lines[0].y[5..], &[NAN, 0.09375, 1.796875]);
    }

    /// Supertrend / SAR per bar from its up and down trend lines, of which
    /// at most one has a value at any bar.
    fn st_ys(r: &PanelResult) -> Vec<f64> {
        let (up, down) = (&r.lines[0].y, &r.lines[1].y);
        up.iter()
            .zip(down)
            .map(|(&u, &d)| {
                assert!(u.is_nan() || d.is_nan(), "both trend lines at one bar");
                if u.is_nan() {
                    d
                } else {
                    u
                }
            })
            .collect()
    }

    /// Like TradingView's plot.style_linebr: at a flip bar the line of the
    /// new trend starts and the old one has no value (it ended a bar
    /// before), so nothing connects the two bands.
    fn assert_breaks_at_flips(r: &PanelResult) {
        for (x, label) in flips(r) {
            let (new, old) = if label == signals::FLIP_UP {
                (0, 1)
            } else {
                (1, 0)
            };
            assert!(r.lines[new].y[x].is_finite(), "flip at {x}");
            assert!(r.lines[old].y[x].is_nan(), "flip at {x}");
            assert!(r.lines[old].y[x - 1].is_finite(), "flip at {x}");
        }
    }

    #[test]
    fn supertrend_branches_on_previous_band_like_tradingview() {
        // multiplier 0: both bands are hl2, so `prevST == prevUpper` holds in
        // either direction and TradingView decides by `close > upper` alone.
        let ind = Supertrend {
            period: 2,
            multiplier: 0.0,
        };
        let r = ind.compute(&ohlc());
        eq(&st_ys(&r), &[NAN, 2.0, 3.5, 3.5, 2.25, 1.25, 2.5, 4.5]);
        assert_eq!(
            flips(&r),
            vec![
                (2, signals::FLIP_UP),
                (3, signals::FLIP_DOWN),
                (6, signals::FLIP_UP)
            ]
        );
        assert_breaks_at_flips(&r);
        // bar 0 is never drawn, even when the ATR is valid there
        let r = Supertrend {
            period: 1,
            multiplier: 1.0,
        }
        .compute(&ohlc());
        assert!(st_ys(&r)[0].is_nan() && st_ys(&r)[1].is_finite());
        // Too short a window still has both lines, so pages keep one shape.
        for bars in [0, 1] {
            let r = Supertrend::default().compute(&closes(&vec![1.0; bars]));
            assert!(r.lines.iter().all(|l| l.y.len() == bars));
            assert_eq!(st_ys(&r).len(), bars);
        }
    }

    #[test]
    fn sar_matches_pine_reference() {
        let r = ParabolicSar::default().compute(&ohlc());
        // Bar 0 is na; bar 1 seeds an uptrend at low[0] = 0.
        eq(&st_ys(&r), &[NAN, 0.0, 0.0, 0.2, 0.392, 5.0, 4.91, 0.5]);
        assert_eq!(
            flips(&r),
            vec![(5, signals::FLIP_DOWN), (7, signals::FLIP_UP)]
        );
        // SAR Up / SAR Down break at a flip; dots mark only the flips.
        assert_eq!(r.lines[0].label.as_deref(), Some("SAR Up"));
        assert_eq!(r.dots.len(), 2);
        assert_breaks_at_flips(&r);
        let r = ParabolicSar::default().compute(&closes(&[1.0]));
        assert!(r.lines.iter().all(|l| l.y.len() == 1 && l.y[0].is_nan()));
    }

    #[test]
    fn heikin_ashi_and_kvf_plot_lines() {
        // The series wire drops unlabeled dots, so these plot lines.
        let ha = HeikinAshi.compute(&ohlc());
        assert_eq!(ha.lines[0].label.as_deref(), Some("HA Close"));
        assert_eq!(ha.lines[0].y[0], (1.0 + 2.0 + 0.0 + 1.0) / 4.0);
        assert!(ha.dots.iter().all(|d| d.label.is_some()));
        let kvf = KalmanVolume::default().compute(&crate::data::sample_data(500));
        let labels: Vec<_> = kvf.lines.iter().map(|l| l.label.as_deref()).collect();
        assert_eq!(labels, [Some("KVF"), Some("Signal")]);
        assert!(kvf.lines.iter().all(|l| l.y[499].is_finite()));
        assert!(kvf.dots.iter().all(|d| d.label.is_some()));
        // Short windows keep one shape.
        for bars in 0..3 {
            let r = KalmanVolume::default().compute(&closes(&vec![1.0; bars]));
            assert_eq!(r.lines.len(), 2);
            assert_eq!(HeikinAshi.compute(&closes(&vec![1.0; bars])).lines.len(), 1);
        }
    }

    #[test]
    fn ichimoku_leads_spans_by_displacement() {
        let ind = Ichimoku {
            tenkan: 1,
            kijun: 2,
            senkou_b: 2,
            displacement: 26,
        };
        let r = ind.compute(&closes(&(1..=28).map(f64::from).collect::<Vec<_>>()));
        assert_eq!(r.lines[0].y[0], 1.0, "Tenkan is not shifted");
        // Senkou A at bar j = c_j - 0.25 (j >= 1), drawn 25 bars later.
        let mut a = vec![NAN; 26];
        a.extend([1.75, 2.75]);
        eq(&r.lines[2].y, &a);
        let mut b = vec![NAN; 26];
        b.extend([1.5, 2.5]);
        eq(&r.lines[3].y, &b);
        // Lagging span: close drawn 25 bars back -> bar 0 shows close[25] = 26.
        let mut lag = vec![26.0, 27.0, 28.0];
        lag.extend(vec![NAN; 25]);
        eq(&r.lines[4].y, &lag);
        // displacement 3: spans 2 bars ahead, lagging span 2 bars back
        let mut ind = ind;
        ind.configure(&json!({"displacement": 3}));
        let r = ind.compute(&closes(&[1.0, 2.0, 3.0, 4.0]));
        eq(&r.lines[2].y, &[NAN, NAN, NAN, 1.75]);
        eq(&r.lines[4].y, &[3.0, 4.0, NAN, NAN]);
    }

    #[test]
    fn ichimoku_projection_continues_the_shifted_spans() {
        // ADR-0033: the leading spans shifted over n + off bars are the
        // plotted values followed by the projection.
        let ind = Ichimoku::default();
        let data = sample_data(120);
        let (n, off) = (data.len(), 25);
        let r = ind.compute(&data);
        let p = ind.projection(&data).unwrap();
        assert_eq!(p.bars, off);
        let [_, _, a, b] = ind.spans(&data);
        let full = |v: &[f64]| {
            let mut ext = v.to_vec();
            ext.extend(vec![NAN; off]);
            crate::indicators::pine::shift(&ext, off as i64)
        };
        let (fa, fb) = (full(&a), full(&b));
        eq(&r.lines[2].y, &fa[..n]);
        eq(&r.lines[3].y, &fb[..n]);
        assert_eq!(p.lines.len(), 2);
        assert_eq!((p.lines[0].0, p.lines[1].0), (2, 3));
        eq(&p.lines[0].1, &fa[n..]);
        eq(&p.lines[1].1, &fb[n..]);
        assert_eq!(p.fills.len(), 1);
        assert_eq!(p.fills[0].0, 0);
        eq(&p.fills[0].1, &fa[n..]);
        eq(&p.fills[0].2, &fb[n..]);
        assert!(p.lines[0].1.iter().all(|v| v.is_finite()));
        // displacement 1 draws nothing ahead; fewer bars than the lead pad
        // the start with NaN.
        let mut d1 = Ichimoku::default();
        d1.configure(&json!({"displacement": 1}));
        assert!(d1.projection(&data).is_none());
        let short = Ichimoku {
            tenkan: 1,
            kijun: 1,
            senkou_b: 1,
            displacement: 4,
        };
        let p = short.projection(&closes(&[1.0, 2.0])).unwrap();
        eq(&p.lines[0].1, &[NAN, 1.0, 2.0]);
        // Pine's offset limit caps the projection at 500 bars.
        let mut far = Ichimoku::default();
        far.configure(&json!({"displacement": 10000}));
        let p = far.projection(&data).unwrap();
        assert_eq!(p.bars, MAX_PROJECTION_BARS);
        assert_eq!((p.lines[0].1.len(), p.fills[0].2.len()), (500, 500));
        assert!(
            p.lines[0].1.iter().all(|v| v.is_nan()),
            "120 bars reach no slot"
        );
    }

    #[test]
    fn histvol_annualizes_like_tradingview() {
        let e = std::f64::consts::E;
        let mut d = closes(&[1.0, e, e * e, e]);
        // log returns = [_, 1, 1, -1]; population stdev(2) = [_, _, 0, 1]
        let ind = HistVol { period: 2 };
        d.interval = Some("1d".into());
        eq(
            &ind.compute(&d).lines[0].y,
            &[NAN, NAN, 0.0, 100.0 * 365f64.sqrt()],
        );
        d.interval = Some("1w".into());
        let weekly = 100.0 * (365.0f64 / 7.0).sqrt();
        eq(&ind.compute(&d).lines[0].y, &[NAN, NAN, 0.0, weekly]);
        // any letter case / multiple; 1D and intraday use per = 1, 2D+ per = 7
        let daily = 100.0 * 365f64.sqrt();
        for (iv, want) in [
            ("1W", weekly),
            ("2w", weekly),
            ("1M", weekly),
            ("2D", weekly),
            ("1D", daily),
            ("4H", daily),
            ("15m", daily),
        ] {
            d.interval = Some(iv.into());
            eq(&ind.compute(&d).lines[0].y, &[NAN, NAN, 0.0, want]);
        }
        // unknown interval and no timestamps: unknown, not intraday
        d.interval = Some("1x".into());
        assert!(ind.compute(&d).lines[0].y.iter().all(|v| v.is_nan()));
        d.interval = None;
        assert!(ind.compute(&d).lines[0].y.iter().all(|v| v.is_nan()));
        // no interval, but weekly bar times: inferred as weekly
        for (i, b) in d.bars.iter_mut().enumerate() {
            b.date = (1_700_000_000 + i as i64 * 7 * 86_400).to_string();
        }
        eq(&ind.compute(&d).lines[0].y, &[NAN, NAN, 0.0, weekly]);
        assert_eq!(HistVol::default().period, 10);
    }

    #[test]
    fn kalman_volume_uses_pine_hma_and_stdev() {
        // ta.hma floors sqrt(3) to 1: hma3 = 2 * x - wma3.
        eq(
            &super::super::pine::hma_n(&[1.0, 2.0, 3.0, 4.0], 3),
            &[NAN, NAN, 11.0 / 3.0, 14.0 / 3.0],
        );
        let ind = KalmanVolume {
            vzo_length: 4,
            ..Default::default()
        };
        // hma4 is valid from bar 4, ta.stdev(vzo, 200) from bar 203, the
        // 10-bar signal from bar 212: no value (and no 0 placeholder) before.
        let r = ind.compute(&sample_data(300));
        for line in &r.lines {
            assert_eq!(line.y.iter().position(|y| y.is_finite()), Some(212));
            assert_ne!(line.y[212], 0.0);
        }
    }

    #[test]
    fn histvol_skips_invalid_log_returns() {
        // log returns = [_, 1, 1, -inf, inf]: the non-finite ones are skipped
        // by ta.stdev, so the window keeps using the last two valid returns.
        let e = std::f64::consts::E;
        let mut d = closes(&[1.0, e, e * e, 0.0, e]);
        d.interval = Some("1d".into());
        let r = HistVol { period: 2 }.compute(&d);
        eq(&r.lines[0].y, &[NAN, NAN, 0.0, 0.0, 0.0]);
    }

    /// Bars (high, low, close, volume) stamped with unix `times` and `interval`.
    fn timed(v: &[(f64, f64, f64, f64)], times: &[i64], interval: &str) -> OhlcvData {
        let mut d = bars(
            &v.iter()
                .map(|&(h, l, c, vol)| (c, h, l, c, vol))
                .collect::<Vec<_>>(),
        );
        for (b, t) in d.bars.iter_mut().zip(times) {
            b.date = t.to_string();
        }
        d.interval = Some(interval.into());
        d
    }

    fn flat(c: &[(f64, f64)]) -> Vec<(f64, f64, f64, f64)> {
        c.iter().map(|&(c, v)| (c, c, c, v)).collect()
    }

    #[test]
    fn vwap_session_resets_at_the_utc_day() {
        // 6h bars: Jan 1 12:00, 18:00, Jan 2 00:00, 06:00 (UTC)
        let t = [1_767_268_800, 1_767_290_400, 1_767_312_000, 1_767_333_600];
        let d = timed(
            &flat(&[(10.0, 1.0), (20.0, 3.0), (30.0, 1.0), (40.0, 1.0)]),
            &t,
            "6h",
        );
        let r = Vwap::default().compute(&d);
        eq(&r.lines[0].y, &[10.0, 17.5, 30.0, 35.0]);
        let mut none = Vwap::default();
        none.configure(&json!({"anchor": "none"}));
        eq(
            &none.compute(&d).lines[0].y,
            &[10.0, 17.5, 20.0, 140.0 / 6.0],
        );
        // bands: volume-weighted population stdev within the session
        let r = VwapBands::default().compute(&d);
        let sd1 = 18.75f64.sqrt();
        eq(&r.lines[0].y, &[10.0, 17.5 + sd1, 30.0, 40.0]);
        eq(&r.lines[2].y, &[10.0, 17.5 - sd1, 30.0, 30.0]);
        // without bar timestamps there is nothing to anchor to
        let r = Vwap::default().compute(&closes(&[10.0, 20.0]));
        eq(&r.lines[0].y, &[10.0, 15.0]);
    }

    #[test]
    fn vwap_week_and_month_anchors_on_daily_bars() {
        let v = flat(&[(10.0, 1.0), (20.0, 1.0), (30.0, 1.0)]);
        // Fri Jan 2, Mon Jan 5, Tue Jan 6 2026: a new ISO week on Monday
        let d = timed(&v, &[1_767_312_000, 1_767_571_200, 1_767_657_600], "1d");
        let mut w = Vwap::default();
        w.configure(&json!({"anchor": "week"}));
        eq(&w.compute(&d).lines[0].y, &[10.0, 20.0, 25.0]);
        // "session" on a daily chart resets on every bar (VWAP = hlc3)
        eq(&Vwap::default().compute(&d).lines[0].y, &[10.0, 20.0, 30.0]);
        // Jan 30, Jan 31, Feb 1: a new month on Feb 1
        let d = timed(&v, &[1_769_731_200, 1_769_817_600, 1_769_904_000], "1d");
        let mut m = Vwap::default();
        m.configure(&json!({"anchor": "month"}));
        eq(&m.compute(&d).lines[0].y, &[10.0, 15.0, 30.0]);
    }

    #[test]
    fn pivots_use_the_previous_higher_timeframe_period() {
        let ys =
            |r: &PanelResult| -> Vec<Vec<f64>> { r.lines.iter().map(|l| l.y.clone()).collect() };
        // 15m chart -> daily pivots. Jan 1: H 14, L 8, C 11 -> P 11,
        // R1 14, S1 8, R2 17, S2 5 for both Jan 2 bars.
        let v = [
            (12.0, 8.0, 10.0, 1.0),
            (14.0, 9.0, 11.0, 1.0),
            (20.0, 19.0, 19.5, 1.0),
            (21.0, 18.0, 20.0, 1.0),
        ];
        let t = [1_767_310_200, 1_767_311_100, 1_767_312_000, 1_767_312_900];
        let r = PivotPoints.compute(&timed(&v, &t, "15m"));
        let want = [11.0, 14.0, 8.0, 17.0, 5.0];
        for (line, w) in ys(&r).iter().zip(want) {
            eq(line, &[NAN, NAN, w, w]);
        }
        // 1h chart -> weekly pivots: Thu Jan 1 and Sun Jan 4 share an ISO
        // week, Mon Jan 5 starts the next one.
        let t = [1_767_268_800, 1_767_290_400, 1_767_567_600, 1_767_571_200];
        let r = PivotPoints.compute(&timed(&v, &t, "1h"));
        // week 1 = bars 0-2: H 20, L 8, C 19.5 -> P 47.5 / 3
        eq(&r.lines[0].y, &[NAN, NAN, NAN, 47.5 / 3.0]);
        // daily chart -> monthly pivots (Jan 29, 30, 31 | Feb 1)
        let t = [1_769_644_800, 1_769_731_200, 1_769_817_600, 1_769_904_000];
        let r = PivotPoints.compute(&timed(&v, &t, "1d"));
        eq(&r.lines[0].y, &[NAN, NAN, NAN, 47.5 / 3.0]);
        // weekly chart -> yearly pivots (Dec 15, 22, 29 2025 | Jan 5 2026)
        let t = [1_765_756_800, 1_766_361_600, 1_766_966_400, 1_767_571_200];
        let r = PivotPoints.compute(&timed(&v, &t, "1W"));
        eq(&r.lines[0].y, &[NAN, NAN, NAN, 47.5 / 3.0]);
        // no timestamps: the period is unknown
        let r = PivotPoints.compute(&closes(&[1.0, 2.0, 3.0]));
        assert!(r.lines.iter().all(|l| l.y.iter().all(|v| v.is_nan())));
    }
}
