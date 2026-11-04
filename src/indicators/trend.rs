//! Moving-average, volume and trend indicators mirroring TradingView's
//! built-ins (Pine v5 semantics). Warmup values are NaN.

use super::pine::{
    line, linfit, ma_by_type, rma_n, shift, sma_n, source_series, true_range, volumes, windowed,
};
use crate::data::OhlcvData;
use crate::indicator::{rgba, Bars, DivLine, Fill, Indicator, PanelResult};
use serde_json::{json, Value};

pub const MA_TYPES: &[&str] = &[
    "sma", "ema", "wma", "hma", "vwma", "rma", "smma", "alma", "dema", "tema", "lsma",
];
pub const SOURCES: &[&str] = &[
    "close", "open", "high", "low", "hl2", "hlc3", "ohlc4", "hlcc4",
];

fn get_usize(p: &Value, k: &str) -> Option<usize> {
    p.get(k).and_then(|v| v.as_u64()).map(|v| v as usize)
}

fn get_f64(p: &Value, k: &str) -> Option<f64> {
    p.get(k).and_then(|v| v.as_f64())
}

fn get_str(p: &Value, k: &str, allowed: &[&str]) -> Option<String> {
    p.get(k)
        .and_then(|v| v.as_str())
        .filter(|s| allowed.contains(s))
        .map(String::from)
}

fn type_param(default: &str) -> Value {
    json!({"name": "type", "type": "string", "default": default, "enum": MA_TYPES,
           "description": "Moving average type"})
}

fn source_param() -> Value {
    json!({"name": "source", "type": "string", "default": "close", "enum": SOURCES,
           "description": "Price source"})
}

// ---- volume ----

pub struct Volume {
    ma_length: usize,
}

impl Default for Volume {
    fn default() -> Self {
        Self { ma_length: 20 }
    }
}

impl Indicator for Volume {
    fn name(&self) -> &str {
        "Volume"
    }
    fn description(&self) -> &str {
        "Volume histogram colored by candle direction, with optional volume MA (TradingView Volume)"
    }
    fn params(&self) -> Value {
        json!([{"name": "ma_length", "type": "integer", "default": 20, "min": 0, "max": 10000,
                "description": "Volume SMA length (0 = off)"}])
    }
    fn configure(&mut self, params: &Value) {
        if let Some(v) = get_usize(params, "ma_length") {
            self.ma_length = v;
        }
    }
    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let vol = volumes(data);
        let colors = data
            .bars
            .iter()
            .map(|b| {
                rgba(
                    if b.close >= b.open {
                        0x26a69a
                    } else {
                        0xef5350
                    },
                    0.5,
                )
            })
            .collect();
        let mut lines = vec![];
        if self.ma_length > 0 {
            lines.push(line(sma_n(&vol, self.ma_length), 0x2962FF, 1, "Volume MA"));
        }
        PanelResult {
            lines,
            bars: vec![Bars {
                y: vol,
                colors,
                bottom: 0.0,
            }],
            label: "Volume".into(),
            ..Default::default()
        }
    }
}

// ---- ma / vwma / hma ----

pub struct Ma {
    kind: String,
    /// Fixed-type alias (vwma, hma): `type` is not configurable.
    fixed: bool,
    length: usize,
    source: String,
    offset: i64,
}

impl Default for Ma {
    fn default() -> Self {
        Self {
            kind: "sma".into(),
            fixed: false,
            length: 9,
            source: "close".into(),
            offset: 0,
        }
    }
}

impl Ma {
    pub fn fixed(kind: &str) -> Self {
        Self {
            kind: kind.into(),
            fixed: true,
            ..Default::default()
        }
    }
}

impl Indicator for Ma {
    fn name(&self) -> &str {
        "MA"
    }
    fn description(&self) -> &str {
        if !self.fixed {
            "Moving Average (sma, ema, wma, hma, vwma, rma, alma, dema, tema, lsma)"
        } else if self.kind == "vwma" {
            "Volume Weighted Moving Average (ta.vwma)"
        } else {
            "Hull Moving Average (ta.hma)"
        }
    }
    fn params(&self) -> Value {
        let mut v = vec![];
        if !self.fixed {
            v.push(type_param("sma"));
        }
        v.push(
            json!({"name": "length", "type": "integer", "default": 9, "min": 1, "max": 10000,
                      "description": "MA length"}),
        );
        v.push(source_param());
        v.push(
            json!({"name": "offset", "type": "integer", "default": 0, "min": -5000, "max": 5000,
                      "description": "Shift the line by this many bars (negative = left). Values shifted past the last bar are dropped"}),
        );
        Value::Array(v)
    }
    fn configure(&mut self, p: &Value) {
        if !self.fixed {
            if let Some(s) = get_str(p, "type", MA_TYPES) {
                self.kind = s;
            }
        }
        if let Some(v) = get_usize(p, "length") {
            self.length = v;
        }
        if let Some(s) = get_str(p, "source", SOURCES) {
            self.source = s;
        }
        if let Some(v) = p.get("offset").and_then(|v| v.as_i64()) {
            self.offset = v;
        }
    }
    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let src = source_series(data, &self.source);
        let y = shift(
            &ma_by_type(&self.kind, &src, &volumes(data), self.length),
            self.offset,
        );
        let label = format!("{} {}", self.kind.to_uppercase(), self.length);
        PanelResult {
            lines: vec![line(y, 0x2962FF, 2, &label)],
            label,
            is_overlay: true,
            ..Default::default()
        }
    }
}

// ---- ma_ribbon ----

pub struct MaRibbon {
    kind: String,
    lengths: [usize; 4],
    source: String,
}

impl Default for MaRibbon {
    fn default() -> Self {
        Self {
            kind: "sma".into(),
            lengths: [20, 50, 100, 200],
            source: "close".into(),
        }
    }
}

impl Indicator for MaRibbon {
    fn name(&self) -> &str {
        "MA Ribbon"
    }
    fn description(&self) -> &str {
        "Moving Average Ribbon — four moving averages (20/50/100/200 by default)"
    }
    fn params(&self) -> Value {
        let mut v = vec![type_param("sma"), source_param()];
        for (i, d) in [20, 50, 100, 200].iter().enumerate() {
            v.push(
                json!({"name": format!("length{}", i + 1), "type": "integer", "default": d,
                          "min": 1, "max": 10000, "description": format!("MA {} length", i + 1)}),
            );
        }
        Value::Array(v)
    }
    fn configure(&mut self, p: &Value) {
        if let Some(s) = get_str(p, "type", MA_TYPES) {
            self.kind = s;
        }
        if let Some(s) = get_str(p, "source", SOURCES) {
            self.source = s;
        }
        for i in 0..4 {
            if let Some(v) = get_usize(p, &format!("length{}", i + 1)) {
                self.lengths[i] = v;
            }
        }
    }
    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let src = source_series(data, &self.source);
        let vol = volumes(data);
        let colors = [0xF23645, 0xFF9800, 0x4CAF50, 0x2962FF];
        let lines = self
            .lengths
            .iter()
            .zip(colors)
            .map(|(&n, c)| {
                let label = format!("{} {}", self.kind.to_uppercase(), n);
                line(ma_by_type(&self.kind, &src, &vol, n), c, 1, &label)
            })
            .collect();
        PanelResult {
            lines,
            label: "MA Ribbon".into(),
            is_overlay: true,
            ..Default::default()
        }
    }
}

// ---- envelopes ----

pub struct Envelopes {
    kind: String,
    length: usize,
    percent: f64,
    source: String,
}

impl Default for Envelopes {
    fn default() -> Self {
        Self {
            kind: "sma".into(),
            length: 20,
            percent: 10.0,
            source: "close".into(),
        }
    }
}

impl Indicator for Envelopes {
    fn name(&self) -> &str {
        "Envelopes"
    }
    fn description(&self) -> &str {
        "Envelopes — moving-average basis with upper/lower bands at a fixed percentage"
    }
    fn params(&self) -> Value {
        json!([type_param("sma"),
            {"name": "length", "type": "integer", "default": 20, "min": 1, "max": 10000,
             "description": "Basis MA length"},
            {"name": "percent", "type": "number", "default": 10.0, "min": 0, "max": 100,
             "description": "Band distance in percent of the basis"},
            source_param()])
    }
    fn configure(&mut self, p: &Value) {
        if let Some(s) = get_str(p, "type", MA_TYPES) {
            self.kind = s;
        }
        if let Some(v) = get_usize(p, "length") {
            self.length = v;
        }
        if let Some(v) = get_f64(p, "percent") {
            self.percent = v;
        }
        if let Some(s) = get_str(p, "source", SOURCES) {
            self.source = s;
        }
    }
    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let src = source_series(data, &self.source);
        let basis = ma_by_type(&self.kind, &src, &volumes(data), self.length);
        let k = self.percent / 100.0;
        let upper: Vec<f64> = basis.iter().map(|b| b * (1.0 + k)).collect();
        let lower: Vec<f64> = basis.iter().map(|b| b * (1.0 - k)).collect();
        PanelResult {
            lines: vec![
                line(upper.clone(), 0x2962FF, 1, "Env Upper"),
                line(basis, 0xFF6D00, 1, "Env Basis"),
                line(lower.clone(), 0x2962FF, 1, "Env Lower"),
            ],
            fills: vec![Fill {
                y1: upper,
                y2: lower,
                color: rgba(0x2962FF, 0.1),
                color_below: None,
            }],
            label: format!(
                "Envelopes {} {} {}%",
                self.kind.to_uppercase(),
                self.length,
                self.percent
            ),
            is_overlay: true,
            ..Default::default()
        }
    }
}

// ---- linreg_channel ----

pub struct LinRegChannel {
    length: usize,
    upper_dev: f64,
    lower_dev: f64,
    source: String,
}

impl Default for LinRegChannel {
    fn default() -> Self {
        Self {
            length: 100,
            upper_dev: 2.0,
            lower_dev: 2.0,
            source: "close".into(),
        }
    }
}

impl Indicator for LinRegChannel {
    fn name(&self) -> &str {
        "Linear Regression Channel"
    }
    fn description(&self) -> &str {
        "Linear Regression Channel — regression line over the last N bars with standard-deviation bands"
    }
    fn params(&self) -> Value {
        json!([{"name": "length", "type": "integer", "default": 100, "min": 2, "max": 10000,
                "description": "Number of bars in the regression (uses fewer if less data)"},
               {"name": "upper_dev", "type": "number", "default": 2.0, "min": 0, "max": 100,
                "description": "Upper deviation multiplier"},
               {"name": "lower_dev", "type": "number", "default": 2.0, "min": 0, "max": 100,
                "description": "Lower deviation multiplier"},
               source_param()])
    }
    fn configure(&mut self, p: &Value) {
        if let Some(v) = get_usize(p, "length") {
            self.length = v;
        }
        if let Some(v) = get_f64(p, "upper_dev") {
            self.upper_dev = v;
        }
        if let Some(v) = get_f64(p, "lower_dev") {
            self.lower_dev = v;
        }
        if let Some(s) = get_str(p, "source", SOURCES) {
            self.source = s;
        }
    }
    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let src = source_series(data, &self.source);
        let n = src.len();
        let (mut base, mut up, mut lo) = (vec![f64::NAN; n], vec![f64::NAN; n], vec![f64::NAN; n]);
        let len = self.length.min(n);
        let w = &src[n - len..];
        if len >= 2 && w.iter().all(|v| !v.is_nan()) {
            let (a, b) = linfit(w);
            let fit: Vec<f64> = (0..len).map(|i| a + b * i as f64).collect();
            let sd = (w
                .iter()
                .zip(&fit)
                .map(|(y, f)| (y - f).powi(2))
                .sum::<f64>()
                / (len - 1).max(1) as f64)
                .sqrt();
            for i in 0..len {
                base[n - len + i] = fit[i];
                up[n - len + i] = fit[i] + self.upper_dev * sd;
                lo[n - len + i] = fit[i] - self.lower_dev * sd;
            }
        }
        PanelResult {
            lines: vec![
                line(up.clone(), 0x2962FF, 1, "LR Upper"),
                line(base, 0xFF6D00, 1, "LR Base"),
                line(lo.clone(), 0x2962FF, 1, "LR Lower"),
            ],
            fills: vec![Fill {
                y1: up,
                y2: lo,
                color: rgba(0x2962FF, 0.1),
                color_below: None,
            }],
            label: format!("LinReg Channel {}", self.length),
            is_overlay: true,
            ..Default::default()
        }
    }
}

// ---- aroon ----

pub struct Aroon {
    length: usize,
}

impl Default for Aroon {
    fn default() -> Self {
        Self { length: 14 }
    }
}

impl Indicator for Aroon {
    fn name(&self) -> &str {
        "Aroon"
    }
    fn description(&self) -> &str {
        "Aroon Up/Down — bars since the highest high / lowest low over length+1 bars"
    }
    fn params(&self) -> Value {
        json!([{"name": "length", "type": "integer", "default": 14, "min": 1, "max": 10000,
                "description": "Aroon length"}])
    }
    fn configure(&mut self, p: &Value) {
        if let Some(v) = get_usize(p, "length") {
            self.length = v;
        }
    }
    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let l = self.length;
        let n = data.len();
        let (mut up, mut down) = (vec![f64::NAN; n], vec![f64::NAN; n]);
        // ta.highestbars / ta.lowestbars over l+1 bars; ties pick the newest bar.
        for i in l..n {
            let w = &data.bars[i - l..=i];
            let (mut hi, mut lo) = (0, 0);
            for (j, b) in w.iter().enumerate() {
                if b.high >= w[hi].high {
                    hi = j;
                }
                if b.low <= w[lo].low {
                    lo = j;
                }
            }
            up[i] = 100.0 * hi as f64 / l as f64;
            down[i] = 100.0 * lo as f64 / l as f64;
        }
        PanelResult {
            lines: vec![
                line(up, 0xFB8C00, 2, "Aroon Up"),
                line(down, 0x2962FF, 2, "Aroon Down"),
            ],
            y_range: Some((0.0, 100.0)),
            label: format!("Aroon {}", l),
            ..Default::default()
        }
    }
}

// ---- vortex ----

pub struct Vortex {
    length: usize,
}

impl Default for Vortex {
    fn default() -> Self {
        Self { length: 14 }
    }
}

impl Indicator for Vortex {
    fn name(&self) -> &str {
        "Vortex"
    }
    fn description(&self) -> &str {
        "Vortex Indicator — VI+ and VI- trend direction lines"
    }
    fn params(&self) -> Value {
        json!([{"name": "length", "type": "integer", "default": 14, "min": 2, "max": 10000,
                "description": "Vortex length"}])
    }
    fn configure(&mut self, p: &Value) {
        if let Some(v) = get_usize(p, "length") {
            self.length = v;
        }
    }
    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let (mut vmp, mut vmm) = (vec![f64::NAN; n], vec![f64::NAN; n]);
        for i in 1..n {
            vmp[i] = (data.bars[i].high - data.bars[i - 1].low).abs();
            vmm[i] = (data.bars[i].low - data.bars[i - 1].high).abs();
        }
        let (p, m, tr) = (
            sma_n(&vmp, self.length),
            sma_n(&vmm, self.length),
            sma_n(&true_range(data), self.length),
        );
        // Ratios of SMAs equal the ratios of sums in the Pine definition.
        let ratio = |a: &[f64]| -> Vec<f64> {
            a.iter()
                .zip(&tr)
                .map(|(x, t)| if *t == 0.0 { f64::NAN } else { x / t })
                .collect()
        };
        PanelResult {
            lines: vec![
                line(ratio(&p), 0x2962FF, 2, "VI+"),
                line(ratio(&m), 0xE91E63, 2, "VI-"),
            ],
            label: format!("Vortex {}", self.length),
            ..Default::default()
        }
    }
}

// ---- chande_kroll_stop ----

pub struct ChandeKrollStop {
    p: usize,
    x: f64,
    q: usize,
}

impl Default for ChandeKrollStop {
    fn default() -> Self {
        Self {
            p: 10,
            x: 1.0,
            q: 9,
        }
    }
}

impl Indicator for ChandeKrollStop {
    fn name(&self) -> &str {
        "Chande Kroll Stop"
    }
    fn description(&self) -> &str {
        "Chande Kroll Stop — ATR-based long and short trailing stops"
    }
    fn params(&self) -> Value {
        json!([{"name": "atr_length", "type": "integer", "default": 10, "min": 1, "max": 10000,
                "description": "ATR / first high-low stop length (p)"},
               {"name": "atr_mult", "type": "number", "default": 1.0, "min": 0, "max": 100,
                "description": "ATR multiplier (x)"},
               {"name": "stop_length", "type": "integer", "default": 9, "min": 1, "max": 10000,
                "description": "Stop smoothing length (q)"}])
    }
    fn configure(&mut self, p: &Value) {
        if let Some(v) = get_usize(p, "atr_length") {
            self.p = v;
        }
        if let Some(v) = get_f64(p, "atr_mult") {
            self.x = v;
        }
        if let Some(v) = get_usize(p, "stop_length") {
            self.q = v;
        }
    }
    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let atr = rma_n(&true_range(data), self.p);
        let max = |w: &[f64]| w.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let min = |w: &[f64]| w.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = windowed(&data.highs(), self.p, max);
        let lo = windowed(&data.lows(), self.p, min);
        let first_high: Vec<f64> = hi.iter().zip(&atr).map(|(h, a)| h - self.x * a).collect();
        let first_low: Vec<f64> = lo.iter().zip(&atr).map(|(l, a)| l + self.x * a).collect();
        let short = windowed(&first_high, self.q, max);
        let long = windowed(&first_low, self.q, min);
        PanelResult {
            lines: vec![
                line(long, 0x2962FF, 1, "Stop Long"),
                line(short, 0xFF6D00, 1, "Stop Short"),
            ],
            label: format!("Chande Kroll Stop {} {} {}", self.p, self.x, self.q),
            is_overlay: true,
            ..Default::default()
        }
    }
}

// ---- zigzag ----

pub struct ZigZag {
    deviation: f64,
    depth: usize,
}

impl Default for ZigZag {
    fn default() -> Self {
        Self {
            deviation: 5.0,
            depth: 10,
        }
    }
}

impl ZigZag {
    /// Confirmed zigzag pivots as (bar index, price, is_high).
    fn pivots(&self, data: &OhlcvData) -> Vec<(usize, f64, bool)> {
        let n = data.len();
        let d = self.depth;
        let mut out: Vec<(usize, f64, bool)> = vec![];
        if n < 2 * d + 1 {
            return out;
        }
        for i in d..n - d {
            let b = &data.bars[i];
            let left = &data.bars[i - d..i];
            let right = &data.bars[i + 1..=i + d];
            let is_h =
                left.iter().all(|x| b.high >= x.high) && right.iter().all(|x| b.high > x.high);
            let is_l = left.iter().all(|x| b.low <= x.low) && right.iter().all(|x| b.low < x.low);
            // An outside bar can be both; take the type opposite to the last pivot.
            let high = match (is_h, is_l) {
                (true, true) => out.last().map(|p| !p.2).unwrap_or(true),
                (true, false) => true,
                (false, true) => false,
                _ => continue,
            };
            let price = if high { b.high } else { b.low };
            match out.last_mut() {
                None => out.push((i, price, high)),
                Some(last) if last.2 == high => {
                    if (high && price > last.1) || (!high && price < last.1) {
                        *last = (i, price, high);
                    }
                }
                Some(last) => {
                    if last.1 == 0.0
                        || (price - last.1).abs() / last.1.abs() * 100.0 >= self.deviation
                    {
                        out.push((i, price, high));
                    }
                }
            }
        }
        out
    }
}

impl Indicator for ZigZag {
    fn name(&self) -> &str {
        "ZigZag"
    }
    fn description(&self) -> &str {
        "ZigZag — connects confirmed swing pivots that differ by at least the deviation %"
    }
    fn params(&self) -> Value {
        json!([{"name": "deviation", "type": "number", "default": 5.0, "min": 0, "max": 100,
                "description": "Minimum price change between pivots in percent"},
               {"name": "depth", "type": "integer", "default": 10, "min": 1, "max": 1000,
                "description": "Pivot legs: bars on each side of a swing high/low"}])
    }
    fn configure(&mut self, p: &Value) {
        if let Some(v) = get_f64(p, "deviation") {
            self.deviation = v;
        }
        if let Some(v) = get_usize(p, "depth") {
            self.depth = v;
        }
    }
    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let pv = self.pivots(data);
        let seg = |(x1, y1): (usize, f64), (x2, y2): (usize, f64)| DivLine {
            x1,
            y1,
            x2,
            y2,
            color: rgba(0x2962FF, 0.9),
            dashed: false,
        };
        let mut divlines: Vec<DivLine> = pv
            .windows(2)
            .map(|w| seg((w[0].0, w[0].1), (w[1].0, w[1].1)))
            .collect();
        // Live last leg: last pivot to the most extreme bar since it (newest on ties).
        if let Some(&(pi, py, was_high)) = pv.last() {
            let mut best: Option<(usize, f64)> = None;
            for (i, b) in data.bars.iter().enumerate().skip(pi + 1) {
                let v = if was_high { b.low } else { b.high };
                if best.is_none_or(|(_, bv)| if was_high { v <= bv } else { v >= bv }) {
                    best = Some((i, v));
                }
            }
            if let Some(end) = best {
                divlines.push(seg((pi, py), end));
            }
        }
        PanelResult {
            divlines,
            label: format!("ZigZag {}% {}", self.deviation, self.depth),
            is_overlay: true,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{sample_data, Bar};
    use crate::indicators::by_name_configured;
    use crate::indicators::pine::{alma_n, ema_n, hma_n, linreg_n, vwma_n, wma_n};

    const NAN: f64 = f64::NAN;

    fn close(a: &[f64], b: &[f64]) {
        assert_eq!(a.len(), b.len());
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            assert!(
                (x.is_nan() && y.is_nan()) || (x - y).abs() < 1e-9,
                "idx {i}: {x} vs {y} in {a:?} / {b:?}"
            );
        }
    }

    /// Bars from (open, high, low, close, volume).
    fn bars(v: &[(f64, f64, f64, f64, f64)]) -> OhlcvData {
        OhlcvData {
            bars: v
                .iter()
                .enumerate()
                .map(|(i, &(open, high, low, close, volume))| Bar {
                    open,
                    high,
                    low,
                    close,
                    volume,
                    date: format!("D{i}"),
                })
                .collect(),
            symbol: None,
            interval: None,
        }
    }

    fn closes(c: &[f64]) -> OhlcvData {
        bars(&c.iter().map(|&c| (c, c, c, c, 1.0)).collect::<Vec<_>>())
    }

    fn lines(ind: &dyn Indicator, d: &OhlcvData) -> Vec<Vec<f64>> {
        ind.compute(d).lines.into_iter().map(|l| l.y).collect()
    }

    #[test]
    fn sma_wma_ema_rma() {
        close(
            &sma_n(&[1., 2., 3., 4., 5., 6.], 3),
            &[NAN, NAN, 2., 3., 4., 5.],
        );
        close(
            &wma_n(&[1., 2., 3., 4.], 3),
            &[NAN, NAN, 14. / 6., 20. / 6.],
        );
        close(&ema_n(&[1., 2., 3., 4., 5.], 3), &[NAN, NAN, 2., 3., 4.]);
        close(
            &rma_n(&[1., 2., 3., 4.], 3),
            &[NAN, NAN, 2., (2. * 2. + 4.) / 3.],
        );
    }

    #[test]
    fn hma_dema_tema_lsma_vwma_alma() {
        let x: Vec<f64> = (1..=10).map(|v| v as f64).collect();
        // hma of a linear series reproduces it; first valid index = n-1 + sqrt(n)-1.
        let h = hma_n(&x, 4);
        assert!(h[..4].iter().all(|v| v.is_nan()));
        assert!((h[4] - 5.0).abs() < 1e-9 && (h[9] - 10.0).abs() < 1e-9);

        let d = ma_by_type("dema", &[1., 2., 3., 4., 5.], &[], 2);
        close(&d, &[NAN, NAN, 3., 4., 5.]);
        let t = ma_by_type("tema", &[5.0; 12], &[], 3);
        assert_eq!(t.iter().take_while(|v| v.is_nan()).count(), 3 * 3 - 3);
        assert!((t[11] - 5.0).abs() < 1e-9);

        close(
            &linreg_n(&[1., 2., 4.], 3),
            &[NAN, NAN, 2.5 / 3. + 1.5 * 2.],
        );
        close(
            &vwma_n(&[1., 2., 3.], &[1., 1., 2.], 2),
            &[NAN, 1.5, 8. / 3.],
        );

        // m = 0.85 * 2 = 1.7 (no floor), s = 3 / 6 = 0.5
        let w: Vec<f64> = (0..3)
            .map(|i| (-((i as f64 - 1.7).powi(2)) / 0.5).exp())
            .collect();
        close(
            &alma_n(&[1., 2., 4.], 3),
            &[
                NAN,
                NAN,
                (w[0] + 2. * w[1] + 4. * w[2]) / w.iter().sum::<f64>(),
            ],
        );
    }

    #[test]
    fn sources_and_shift() {
        let d = bars(&[(1., 4., 0., 2., 1.)]);
        assert_eq!(source_series(&d, "hl2"), [2.0]);
        assert_eq!(source_series(&d, "hlc3"), [2.0]);
        assert_eq!(source_series(&d, "ohlc4"), [1.75]);
        assert_eq!(source_series(&d, "hlcc4"), [2.0]);
        close(&shift(&[1., 2., 3.], 1), &[NAN, 1., 2.]);
        close(&shift(&[1., 2., 3.], -1), &[2., 3., NAN]);
    }

    #[test]
    fn ma_indicator_offset_and_validation() {
        let d = closes(&[1., 2., 3., 4., 5.]);
        let ind = by_name_configured("ma", &json!({"type": "ema", "length": 2, "offset": -1}))
            .unwrap()
            .unwrap();
        // ema2: NaN,1.5,2.5,3.5,4.5 shifted left by one.
        close(&lines(ind.as_ref(), &d)[0], &[1.5, 2.5, 3.5, 4.5, NAN]);
        assert!(by_name_configured("ma", &json!({"type": "foo"})).is_err());
        assert!(by_name_configured("ma", &json!({"source": "bar"})).is_err());
        assert!(by_name_configured("ma", &json!({"offset": -5001})).is_err());
        assert!(by_name_configured("ma", &json!({"length": 0})).is_err());
        // vwma / hma aliases have a fixed type.
        let v = by_name_configured("vwma", &json!({"type": "ema", "length": 2}))
            .unwrap()
            .unwrap();
        close(&lines(v.as_ref(), &d)[0], &[NAN, 1.5, 2.5, 3.5, 4.5]);
        assert!(!v
            .params()
            .to_string()
            .contains("\"type\":\"string\",\"default\":\"sma\""));
    }

    #[test]
    fn volume_colors_and_ma() {
        let d = bars(&[
            (1., 1., 1., 2., 10.),
            (2., 2., 1., 1., 20.),
            (1., 1., 1., 1., 30.),
        ]);
        let r = by_name_configured("volume", &json!({"ma_length": 2}))
            .unwrap()
            .unwrap()
            .compute(&d);
        assert_eq!(r.bars[0].y, [10., 20., 30.]);
        assert_eq!(r.bars[0].colors[0].0, 0x26);
        assert_eq!(r.bars[0].colors[1].0, 0xef);
        assert_eq!(r.bars[0].colors[2].0, 0x26);
        close(&r.lines[0].y, &[NAN, 15., 25.]);
        let off = by_name_configured("volume", &json!({"ma_length": 0}))
            .unwrap()
            .unwrap()
            .compute(&d);
        assert!(off.lines.is_empty());
    }

    #[test]
    fn envelopes_and_ribbon() {
        let d = closes(&[10., 20., 30.]);
        let r = by_name_configured("envelopes", &json!({"length": 2, "percent": 10}))
            .unwrap()
            .unwrap()
            .compute(&d);
        close(&r.lines[0].y, &[NAN, 16.5, 27.5]);
        close(&r.lines[1].y, &[NAN, 15., 25.]);
        close(&r.lines[2].y, &[NAN, 13.5, 22.5]);
        let rb = by_name_configured("ma_ribbon", &json!({"length1": 1, "length2": 2}))
            .unwrap()
            .unwrap()
            .compute(&d);
        assert_eq!(rb.lines.len(), 4);
        close(&rb.lines[0].y, &[10., 20., 30.]);
        close(&rb.lines[1].y, &[NAN, 15., 25.]);
        assert!(rb.lines[3].y.iter().all(|v| v.is_nan()));
    }

    #[test]
    fn linreg_channel_exact_line() {
        let d = closes(&[9., 1., 2., 3., 4.]);
        let r = by_name_configured("linreg_channel", &json!({"length": 4}))
            .unwrap()
            .unwrap()
            .compute(&d);
        close(&r.lines[1].y, &[NAN, 1., 2., 3., 4.]);
        close(&r.lines[0].y, &[NAN, 1., 2., 3., 4.]);
        // noisy: [1,3,2,4] -> slope 0.8, intercept 1.3 ; residual sd (divisor len-1) computed by hand
        let d = closes(&[1., 3., 2., 4.]);
        let r = by_name_configured("linreg_channel", &json!({"length": 4, "upper_dev": 1}))
            .unwrap()
            .unwrap()
            .compute(&d);
        let fit = [1.3, 2.1, 2.9, 3.7];
        let res = [-0.3, 0.9, -0.9, 0.3f64];
        let sd = (res.iter().map(|r| r * r).sum::<f64>() / 3.0).sqrt();
        close(&r.lines[1].y, &fit);
        assert!((r.lines[0].y[3] - (3.7 + sd)).abs() < 1e-9);
        assert!((r.lines[2].y[3] - (3.7 - 2.0 * sd)).abs() < 1e-9);
    }

    #[test]
    fn aroon_hand_values() {
        let d = bars(&[
            (1., 1., 5., 1., 1.),
            (1., 3., 4., 1., 1.),
            (1., 2., 3., 1., 1.),
            (1., 2., 4., 1., 1.),
            (1., 1., 5., 1., 1.),
        ]);
        let y = lines(
            by_name_configured("aroon", &json!({"length": 2}))
                .unwrap()
                .unwrap()
                .as_ref(),
            &d,
        );
        close(&y[0], &[NAN, NAN, 50., 0., 50.]);
        close(&y[1], &[NAN, NAN, 100., 50., 0.]);
    }

    #[test]
    fn vortex_hand_values() {
        let d = bars(&[
            (9., 10., 8., 9., 1.),
            (9., 12., 9., 11., 1.),
            (9., 11., 9., 10., 1.),
            (9., 13., 10., 12., 1.),
        ]);
        let y = lines(
            by_name_configured("vortex", &json!({"length": 2}))
                .unwrap()
                .unwrap()
                .as_ref(),
            &d,
        );
        assert!(y[0][..2].iter().all(|v| v.is_nan()));
        assert!((y[0][2] - 1.2).abs() < 1e-9);
        assert!((y[1][2] - 0.8).abs() < 1e-9);
    }

    #[test]
    fn chande_kroll_hand_values() {
        // low = i, high = i + 2, close = i + 1 -> TR = 2, ATR = 2.
        let v: Vec<_> = (0..6)
            .map(|i| {
                let i = i as f64;
                (i, i + 2., i, i + 1., 1.)
            })
            .collect();
        let y = lines(
            by_name_configured(
                "chande_kroll_stop",
                &json!({"atr_length": 2, "atr_mult": 0.5, "stop_length": 2}),
            )
            .unwrap()
            .unwrap()
            .as_ref(),
            &bars(&v),
        );
        // long = i - 1, short = i + 1 from index p + q - 2 = 2.
        close(&y[0], &[NAN, NAN, 1., 2., 3., 4.]);
        close(&y[1], &[NAN, NAN, 3., 4., 5., 6.]);
    }

    #[test]
    fn zigzag_segments() {
        let d = closes(&[10., 20., 10., 20., 10.]);
        let mk = |dev: f64| {
            by_name_configured("zigzag", &json!({"deviation": dev, "depth": 1}))
                .unwrap()
                .unwrap()
                .compute(&d)
        };
        let r = mk(5.0);
        // Pivots at 1 (H), 2 (L), 3 (H) plus the live leg 3 -> 4.
        assert_eq!(r.divlines.len(), 3);
        assert_eq!((r.divlines[0].x1, r.divlines[0].x2), (1, 2));
        assert_eq!((r.divlines[0].y1, r.divlines[0].y2), (20., 10.));
        let live = &r.divlines[2];
        assert_eq!((live.x1, live.y1, live.x2, live.y2), (3, 20., 4, 10.));
        // 50% swing is below the 60% threshold: one pivot survives, so only
        // its live leg to the newest lowest bar remains.
        let r = mk(60.0);
        assert_eq!(r.divlines.len(), 1);
        let l = &r.divlines[0];
        assert_eq!((l.x1, l.y1, l.x2, l.y2), (1, 20., 4, 10.));
    }

    #[test]
    fn new_indicators_never_panic_at_param_extremes() {
        let names = [
            "volume",
            "ma",
            "vwma",
            "hma",
            "ma_ribbon",
            "envelopes",
            "linreg_channel",
            "aroon",
            "vortex",
            "chande_kroll_stop",
            "zigzag",
        ];
        for name in names {
            let schema = crate::indicators::by_name(name).unwrap().params();
            for pick in ["min", "max"] {
                let mut p = serde_json::Map::new();
                for spec in schema.as_array().unwrap() {
                    if let Some(v) = spec.get(pick) {
                        p.insert(spec["name"].as_str().unwrap().into(), v.clone());
                    }
                    if let Some(e) = spec.get("enum").and_then(|e| e.as_array()) {
                        let idx = if pick == "min" { 0 } else { e.len() - 1 };
                        p.insert(spec["name"].as_str().unwrap().into(), e[idx].clone());
                    }
                }
                for n in [0, 1, 5, 60] {
                    let ind = by_name_configured(name, &Value::Object(p.clone()))
                        .unwrap_or_else(|e| panic!("{name} {pick}: {e}"))
                        .unwrap();
                    let r = ind.compute(&sample_data(n));
                    for l in &r.lines {
                        assert!(l.y.iter().all(|v| !v.is_infinite()), "{name}");
                    }
                }
            }
        }
    }
}
