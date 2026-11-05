//! Classic TradingView built-ins (EMA, BB, KC, ATR, OBV, CCI, ROC, MFI,
//! Stoch, %R, DC, CMF) computed with Pine v5 semantics via `pine.rs`.
//! The file name predates the port off the `ta` crate (ADR-0018).

use super::pine::{
    atr, dev, div, ema_n, is_zero, mfi, roc, sma_n, source_series, stdev, sum_n, zip,
};
use crate::data::OhlcvData;
use crate::indicator::*;
use serde_json::{json, Value};

// ---- EMA Stack (overlay) ----

pub struct EmaStack;

impl Indicator for EmaStack {
    fn name(&self) -> &str {
        "EMA Stack"
    }

    fn description(&self) -> &str {
        "EMA Stack — 4 exponential moving averages (8/21/50/200)"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let closes = data.closes();
        let ema8 = ema(&closes, 8);
        let ema21 = ema(&closes, 21);
        let ema50 = ema(&closes, 50);
        let ema200 = ema(&closes, 200);

        PanelResult {
            lines: vec![
                Line {
                    y: ema8,
                    color: rgba(0xFFEB3B, 0.9),
                    width: 1,
                    label: Some("EMA 8".into()),
                },
                Line {
                    y: ema21,
                    color: rgba(0xFF9800, 0.9),
                    width: 1,
                    label: Some("EMA 21".into()),
                },
                Line {
                    y: ema50,
                    color: rgba(0x2196F3, 0.9),
                    width: 1,
                    label: Some("EMA 50".into()),
                },
                Line {
                    y: ema200,
                    color: rgba(0xF44336, 0.9),
                    width: 2,
                    label: Some("EMA 200".into()),
                },
            ],
            is_overlay: true,
            label: "EMA".into(),
            ..Default::default()
        }
    }
}

// ---- Bollinger Bands (overlay) ----

pub struct BollingerBandsInd {
    pub period: usize,
    pub std_dev: f64,
}

impl Default for BollingerBandsInd {
    fn default() -> Self {
        Self {
            period: 20,
            std_dev: 2.0,
        }
    }
}

impl Indicator for BollingerBandsInd {
    fn name(&self) -> &str {
        "Bollinger Bands"
    }

    fn description(&self) -> &str {
        "Bollinger Bands — volatility bands around SMA"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "period", "type": "integer", "min": 1, "max": 10000, "default": 20},
            {"name": "std_dev", "type": "number", "min": 0, "max": 100, "default": 2.0}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("period").and_then(|v| v.as_u64()) {
            self.period = v as usize;
        }
        if let Some(v) = params.get("std_dev").and_then(|v| v.as_f64()) {
            self.std_dev = v;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        // TV: basis = ta.sma(close, n), dev = mult * ta.stdev(close, n) (population).
        let closes = data.closes();
        let middle = sma_n(&closes, self.period);
        let sd = stdev(&closes, self.period);
        let upper = zip(&middle, &sd, |m, d| m + self.std_dev * d);
        let lower = zip(&middle, &sd, |m, d| m - self.std_dev * d);

        PanelResult {
            lines: vec![
                Line {
                    y: upper.clone(),
                    color: rgba(0x787B86, 0.7),
                    width: 1,
                    label: Some("BB Upper".into()),
                },
                Line {
                    y: middle,
                    color: rgba(0x2196F3, 0.8),
                    width: 1,
                    label: Some("BB Mid".into()),
                },
                Line {
                    y: lower.clone(),
                    color: rgba(0x787B86, 0.7),
                    width: 1,
                    label: Some("BB Lower".into()),
                },
            ],
            fills: vec![Fill {
                y1: upper,
                y2: lower,
                color: rgba(0x2196F3, 0.1),
                color_below: None,
            }],
            is_overlay: true,
            label: "BB".into(),
            ..Default::default()
        }
    }
}

// ---- Keltner Channels (overlay) ----

pub struct KeltnerChannels {
    pub period: usize,
    pub multiplier: f64,
    pub atr_length: usize,
}

impl Default for KeltnerChannels {
    fn default() -> Self {
        // TradingView defaults: length 20, multiplier 2, ATR length 10.
        Self {
            period: 20,
            multiplier: 2.0,
            atr_length: 10,
        }
    }
}

impl Indicator for KeltnerChannels {
    fn name(&self) -> &str {
        "Keltner Channels"
    }

    fn description(&self) -> &str {
        "Keltner Channels — ATR-based volatility bands"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "period", "type": "integer", "min": 1, "max": 10000, "default": 20},
            {"name": "multiplier", "type": "number", "min": 0, "max": 1000, "default": 2.0},
            {"name": "atr_length", "type": "integer", "min": 1, "max": 10000, "default": 10,
             "description": "ATR length for the band width"}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("period").and_then(|v| v.as_u64()) {
            self.period = v as usize;
        }
        if let Some(v) = params.get("multiplier").and_then(|v| v.as_f64()) {
            self.multiplier = v;
        }
        if let Some(v) = params.get("atr_length").and_then(|v| v.as_u64()) {
            self.atr_length = v as usize;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        // TV (Bands Style "Average True Range"):
        // ta.ema(close, period) +/- mult * ta.atr(atr_length).
        let middle = ema_n(&data.closes(), self.period);
        let range = atr(data, self.atr_length);
        let upper = zip(&middle, &range, |m, r| m + self.multiplier * r);
        let lower = zip(&middle, &range, |m, r| m - self.multiplier * r);

        PanelResult {
            lines: vec![
                Line {
                    y: upper.clone(),
                    color: rgba(0x787B86, 0.7),
                    width: 1,
                    label: Some("KC Upper".into()),
                },
                Line {
                    y: middle,
                    color: rgba(0x2196F3, 0.8),
                    width: 1,
                    label: Some("KC Mid".into()),
                },
                Line {
                    y: lower.clone(),
                    color: rgba(0x787B86, 0.7),
                    width: 1,
                    label: Some("KC Lower".into()),
                },
            ],
            fills: vec![Fill {
                y1: upper,
                y2: lower,
                color: rgba(0x2196F3, 0.1),
                color_below: None,
            }],
            is_overlay: true,
            label: "KC".into(),
            ..Default::default()
        }
    }
}

// ---- ATR panel ----

pub struct Atr {
    pub period: usize,
}

impl Default for Atr {
    fn default() -> Self {
        Self { period: 14 }
    }
}

impl Indicator for Atr {
    fn name(&self) -> &str {
        "ATR"
    }

    fn description(&self) -> &str {
        "Average True Range — volatility measure"
    }

    fn params(&self) -> Value {
        json!([{"name": "period", "type": "integer", "min": 1, "max": 10000, "default": 14}])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("period").and_then(|v| v.as_u64()) {
            self.period = v as usize;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        // TV: ta.rma(ta.tr(true), n).
        let vals = atr(data, self.period);

        PanelResult {
            lines: vec![Line {
                y: vals,
                color: rgba(0xFF9800, 0.9),
                width: 2,
                label: Some("ATR".into()),
            }],
            label: "ATR".into(),
            ..Default::default()
        }
    }
}

// ---- OBV panel ----

pub struct Obv;

impl Indicator for Obv {
    fn name(&self) -> &str {
        "OBV"
    }

    fn description(&self) -> &str {
        "On-Balance Volume — cumulative volume flow"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        // TV: ta.cum(math.sign(ta.change(close)) * volume). ta.cum skips na,
        // so bar 0 (no change yet) is 0.
        let mut cum = 0.0;
        let vals: Vec<f64> = (0..data.len())
            .map(|i| {
                if i > 0 {
                    let ch = data.bars[i].close - data.bars[i - 1].close;
                    let sign = if ch > 0.0 {
                        1.0
                    } else if ch < 0.0 {
                        -1.0
                    } else {
                        0.0
                    };
                    let flow = sign * data.bars[i].volume;
                    if flow.is_finite() {
                        cum += flow;
                    }
                }
                cum
            })
            .collect();

        PanelResult {
            lines: vec![Line {
                y: vals,
                color: rgba(0x2196F3, 0.9),
                width: 2,
                label: Some("OBV".into()),
            }],
            label: "OBV".into(),
            ..Default::default()
        }
    }
}

// ---- CCI panel ----

pub struct Cci {
    pub period: usize,
}

impl Default for Cci {
    fn default() -> Self {
        Self { period: 20 }
    }
}

impl Indicator for Cci {
    fn name(&self) -> &str {
        "CCI"
    }

    fn description(&self) -> &str {
        "Commodity Channel Index — cyclical momentum"
    }

    fn params(&self) -> Value {
        json!([{"name": "period", "type": "integer", "min": 1, "max": 10000, "default": 20}])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("period").and_then(|v| v.as_u64()) {
            self.period = v as usize;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        // TV: (hlc3 - ta.sma(hlc3, n)) / (0.015 * ta.dev(hlc3, n)).
        let src = source_series(data, "hlc3");
        let ma = sma_n(&src, self.period);
        let md: Vec<f64> = dev(&src, self.period).iter().map(|d| 0.015 * d).collect();
        let vals = div(&zip(&src, &ma, |s, m| s - m), &md);

        PanelResult {
            lines: vec![Line {
                y: vals,
                color: rgba(0x2196F3, 0.9),
                width: 2,
                label: Some("CCI".into()),
            }],
            hlines: vec![
                HLine {
                    y: 100.0,
                    color: rgba(0x787B86, 0.5),
                },
                HLine {
                    y: -100.0,
                    color: rgba(0x787B86, 0.5),
                },
            ],
            y_range: Some((-250.0, 250.0)),
            label: "CCI".into(),
            ..Default::default()
        }
    }
}

// ---- ROC panel ----

pub struct Roc {
    pub period: usize,
}

impl Default for Roc {
    fn default() -> Self {
        // TradingView's default length.
        Self { period: 9 }
    }
}

impl Indicator for Roc {
    fn name(&self) -> &str {
        "ROC"
    }

    fn description(&self) -> &str {
        "Rate of Change — momentum percentage"
    }

    fn params(&self) -> Value {
        json!([{"name": "period", "type": "integer", "min": 1, "max": 10000, "default": 9}])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("period").and_then(|v| v.as_u64()) {
            self.period = v as usize;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        // TV: 100 * (close - close[n]) / close[n].
        let vals = roc(&data.closes(), self.period);

        PanelResult {
            lines: vec![Line {
                y: vals,
                color: rgba(0x2196F3, 0.9),
                width: 2,
                label: Some("ROC".into()),
            }],
            hlines: vec![HLine {
                y: 0.0,
                color: rgba(0x787B86, 0.5),
            }],
            label: "ROC".into(),
            ..Default::default()
        }
    }
}

// ---- MFI panel ----

pub struct Mfi {
    pub period: usize,
}

impl Default for Mfi {
    fn default() -> Self {
        Self { period: 14 }
    }
}

impl Indicator for Mfi {
    fn name(&self) -> &str {
        "MFI"
    }

    fn description(&self) -> &str {
        "Money Flow Index — volume-weighted RSI"
    }

    fn params(&self) -> Value {
        json!([{"name": "period", "type": "integer", "min": 1, "max": 10000, "default": 14}])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("period").and_then(|v| v.as_u64()) {
            self.period = v as usize;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        // TV: ta.mfi(hlc3, n).
        let vals = mfi(
            &source_series(data, "hlc3"),
            &super::pine::volumes(data),
            self.period,
        );

        PanelResult {
            lines: vec![Line {
                y: vals,
                color: rgba(0x2196F3, 0.9),
                width: 2,
                label: Some("MFI".into()),
            }],
            hlines: vec![
                HLine {
                    y: 80.0,
                    color: rgba(0xe13e3e, 0.3),
                },
                HLine {
                    y: 20.0,
                    color: rgba(0x3ee145, 0.3),
                },
            ],
            y_range: Some((0.0, 100.0)),
            label: "MFI".into(),
            ..Default::default()
        }
    }
}

// ---- Slow Stochastic panel ----

pub struct Stochastic {
    pub period: usize,
    pub k_smooth: usize,
    pub d_smooth: usize,
}

impl Default for Stochastic {
    fn default() -> Self {
        // TradingView defaults: %K length 14, %K smoothing 1, %D smoothing 3.
        Self {
            period: 14,
            k_smooth: 1,
            d_smooth: 3,
        }
    }
}

impl Indicator for Stochastic {
    fn name(&self) -> &str {
        "Stochastic"
    }

    fn description(&self) -> &str {
        "Slow Stochastic — overbought/oversold oscillator"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "period", "type": "integer", "min": 1, "max": 10000, "default": 14},
            {"name": "k_smooth", "type": "integer", "min": 1, "max": 10000, "default": 1},
            {"name": "d_smooth", "type": "integer", "min": 1, "max": 10000, "default": 3}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("period").and_then(|v| v.as_u64()) {
            self.period = v as usize;
        }
        if let Some(v) = params.get("k_smooth").and_then(|v| v.as_u64()) {
            self.k_smooth = v as usize;
        }
        if let Some(v) = params.get("d_smooth").and_then(|v| v.as_u64()) {
            self.d_smooth = v as usize;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        // TV: %K = ta.sma(ta.stoch(close, high, low, period), k_smooth).
        // %D is an SMA of %K with d_smooth period.
        let raw = stoch(&data.closes(), &data.highs(), &data.lows(), self.period);
        let k_vals = sma_n(&raw, self.k_smooth);
        let d_vals = sma_n(&k_vals, self.d_smooth);

        PanelResult {
            lines: vec![
                Line {
                    y: k_vals.clone(),
                    color: rgba(0x2196F3, 0.9),
                    width: 2,
                    label: Some("%K".into()),
                },
                Line {
                    y: d_vals.clone(),
                    color: rgba(0xFF9800, 0.9),
                    width: 2,
                    label: Some("%D".into()),
                },
            ],
            fills: vec![Fill {
                y1: k_vals,
                y2: d_vals,
                color: rgba(0x2196F3, 0.1),
                color_below: None,
            }],
            hlines: vec![
                HLine {
                    y: 80.0,
                    color: rgba(0xe13e3e, 0.3),
                },
                HLine {
                    y: 20.0,
                    color: rgba(0x3ee145, 0.3),
                },
            ],
            y_range: Some((0.0, 100.0)),
            label: "Stoch".into(),
            ..Default::default()
        }
    }
}

// ---- Williams %R panel (custom) ----

pub struct WilliamsR {
    pub period: usize,
}

impl Default for WilliamsR {
    fn default() -> Self {
        Self { period: 14 }
    }
}

impl Indicator for WilliamsR {
    fn name(&self) -> &str {
        "Williams %R"
    }

    fn description(&self) -> &str {
        "Williams %R — momentum reversal indicator"
    }

    fn params(&self) -> Value {
        json!([{"name": "period", "type": "integer", "min": 1, "max": 10000, "default": 14}])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("period").and_then(|v| v.as_u64()) {
            self.period = v as usize;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        // TV: 100 * (close - highest) / (highest - lowest); a zero range is NaN.
        let hh = highest(&data.highs(), self.period);
        let ll = lowest(&data.lows(), self.period);
        let vals = div(
            &zip(&data.closes(), &hh, |c, h| 100.0 * (c - h)),
            &zip(&hh, &ll, |h, l| h - l),
        );

        PanelResult {
            lines: vec![Line {
                y: vals,
                color: rgba(0x2196F3, 0.9),
                width: 2,
                label: Some("%R".into()),
            }],
            hlines: vec![
                HLine {
                    y: -20.0,
                    color: rgba(0xe13e3e, 0.3),
                },
                HLine {
                    y: -80.0,
                    color: rgba(0x3ee145, 0.3),
                },
            ],
            y_range: Some((-100.0, 0.0)),
            label: "W%R".into(),
            ..Default::default()
        }
    }
}

// ---- Donchian Channels (overlay, custom) ----

pub struct Donchian {
    pub period: usize,
}

impl Default for Donchian {
    fn default() -> Self {
        Self { period: 20 }
    }
}

impl Indicator for Donchian {
    fn name(&self) -> &str {
        "Donchian"
    }

    fn description(&self) -> &str {
        "Donchian Channels — breakout trading bands"
    }

    fn params(&self) -> Value {
        json!([{"name": "period", "type": "integer", "min": 1, "max": 10000, "default": 20}])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("period").and_then(|v| v.as_u64()) {
            self.period = v as usize;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let highs = data.highs();
        let lows = data.lows();
        let n = data.len();

        let upper = highest(&highs, self.period);
        let lower = lowest(&lows, self.period);

        let mut middle = vec![f64::NAN; n];
        for i in 0..n {
            if !upper[i].is_nan() && !lower[i].is_nan() {
                middle[i] = (upper[i] + lower[i]) / 2.0;
            }
        }

        PanelResult {
            lines: vec![
                Line {
                    y: upper.clone(),
                    color: rgba(0x787B86, 0.7),
                    width: 1,
                    label: Some("DC Upper".into()),
                },
                Line {
                    y: middle,
                    color: rgba(0x2196F3, 0.8),
                    width: 1,
                    label: Some("DC Mid".into()),
                },
                Line {
                    y: lower.clone(),
                    color: rgba(0x787B86, 0.7),
                    width: 1,
                    label: Some("DC Lower".into()),
                },
            ],
            fills: vec![Fill {
                y1: upper,
                y2: lower,
                color: rgba(0x2196F3, 0.1),
                color_below: None,
            }],
            is_overlay: true,
            label: "DC".into(),
            ..Default::default()
        }
    }
}

// ---- CMF panel (custom) ----

pub struct Cmf {
    pub period: usize,
}

impl Default for Cmf {
    fn default() -> Self {
        Self { period: 20 }
    }
}

impl Indicator for Cmf {
    fn name(&self) -> &str {
        "CMF"
    }

    fn description(&self) -> &str {
        "Chaikin Money Flow — buying/selling pressure"
    }

    fn params(&self) -> Value {
        json!([{"name": "period", "type": "integer", "min": 1, "max": 10000, "default": 20}])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("period").and_then(|v| v.as_u64()) {
            self.period = v as usize;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();

        // Money Flow Multiplier: ((close - low) - (high - close)) / (high - low)
        // Money Flow Volume: multiplier * volume
        let mut mfv = vec![f64::NAN; n];
        let mut vol = vec![f64::NAN; n];

        for (i, bar) in data.bars.iter().enumerate() {
            let range = bar.high - bar.low;
            if range > 0.0 {
                let mult = ((bar.close - bar.low) - (bar.high - bar.close)) / range;
                mfv[i] = mult * bar.volume;
            } else {
                mfv[i] = 0.0;
            }
            vol[i] = bar.volume;
        }

        // CMF = sum(mfv, period) / sum(vol, period). |mfv| <= volume per bar, so
        // a numerator within float noise of the volume sum's scale is 0.
        let num = sum_n(&mfv, self.period);
        let den = sum_n(&vol, self.period);
        let num = zip(&num, &den, |m, d| if is_zero(m, d) { 0.0 } else { m });
        let vals = div(&num, &den);

        PanelResult {
            lines: vec![Line {
                y: vals,
                color: rgba(0x2196F3, 0.9),
                width: 2,
                label: Some("CMF".into()),
            }],
            hlines: vec![HLine {
                y: 0.0,
                color: rgba(0x787B86, 0.5),
            }],
            y_range: Some((-1.0, 1.0)),
            label: "CMF".into(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicators::pine::test_util::{bars, closes, eq, ohlc, NAN};

    fn ys(r: &PanelResult, i: usize) -> &[f64] {
        &r.lines[i].y
    }

    #[test]
    fn ema_stack_seeds_with_sma() {
        // EMA 8: 7 warmup NaNs, seed = SMA(1..=8) = 4.5, then 2/9*9 + 7/9*4.5 = 5.5.
        let d = closes(&(1..=9).map(f64::from).collect::<Vec<_>>());
        let r = EmaStack.compute(&d);
        let mut want = vec![NAN; 7];
        want.extend([4.5, 5.5]);
        eq(ys(&r, 0), &want);
        assert!(
            ys(&r, 1).iter().all(|v| v.is_nan()),
            "EMA 21 still warming up"
        );
    }

    #[test]
    fn bbands_sma_population_stdev() {
        let ind = BollingerBandsInd {
            period: 2,
            std_dev: 1.0,
        };
        let r = ind.compute(&closes(&[1.0, 3.0, 5.0, 5.0]));
        // basis = [_, 2, 4, 5], population stdev = [_, 1, 1, 0]
        eq(ys(&r, 0), &[NAN, 3.0, 5.0, 5.0]);
        eq(ys(&r, 1), &[NAN, 2.0, 4.0, 5.0]);
        eq(ys(&r, 2), &[NAN, 1.0, 3.0, 5.0]);
    }

    #[test]
    fn keltner_ema_basis_atr_bands() {
        let ind = KeltnerChannels {
            period: 2,
            multiplier: 1.0,
            atr_length: 2,
        };
        let r = ind.compute(&ohlc());
        // ema2(close) = [_, 1.5, 2/3*4 + 1/3*1.5, ...], atr2 = [_, 2, 2.5, 1.75]
        let mid = [NAN, 1.5, 19.0 / 6.0, 55.0 / 18.0];
        let atr = [NAN, 2.0, 2.5, 1.75];
        eq(&ys(&r, 1)[..4], &mid);
        let up: Vec<f64> = mid.iter().zip(&atr).map(|(m, a)| m + a).collect();
        let lo: Vec<f64> = mid.iter().zip(&atr).map(|(m, a)| m - a).collect();
        eq(&ys(&r, 0)[..4], &up);
        eq(&ys(&r, 2)[..4], &lo);
        // separate ATR length: atr3 = [_, _, 7/3, 17/9]
        let ind = KeltnerChannels {
            atr_length: 3,
            ..ind
        };
        let r = ind.compute(&ohlc());
        eq(
            &ys(&r, 0)[..4],
            &[NAN, NAN, 19.0 / 6.0 + 7.0 / 3.0, 55.0 / 18.0 + 17.0 / 9.0],
        );
    }

    #[test]
    fn tradingview_defaults() {
        let k = KeltnerChannels::default();
        assert_eq!((k.period, k.multiplier, k.atr_length), (20, 2.0, 10));
        let st = Stochastic::default();
        assert_eq!((st.period, st.k_smooth, st.d_smooth), (14, 1, 3));
        assert_eq!(Roc::default().period, 9);
    }

    #[test]
    fn atr_is_rma_of_true_range() {
        let r = Atr { period: 3 }.compute(&ohlc());
        // TR = [2, 2, 3, 1, ...]; seed = 7/3, then (7/3 * 2 + 1) / 3 = 17/9.
        eq(&ys(&r, 0)[..4], &[NAN, NAN, 7.0 / 3.0, 17.0 / 9.0]);
    }

    #[test]
    fn obv_cumulates_signed_volume() {
        let d = bars(&[
            (1.0, 1.0, 1.0, 1.0, 10.0),
            (2.0, 2.0, 2.0, 2.0, 20.0),
            (2.0, 2.0, 2.0, 2.0, 30.0),
            (1.0, 1.0, 1.0, 1.0, 40.0),
            (3.0, 3.0, 3.0, 3.0, 50.0),
        ]);
        eq(ys(&Obv.compute(&d), 0), &[0.0, 20.0, 20.0, -20.0, 30.0]);
    }

    #[test]
    fn cci_uses_mean_deviation() {
        let r = Cci { period: 3 }.compute(&closes(&[1.0, 2.0, 3.0, 2.0]));
        // idx 2: (3 - 2) / (0.015 * 2/3) = 100; idx 3: (2 - 7/3) / (0.015 * 4/9) = -50
        eq(ys(&r, 0), &[NAN, NAN, 100.0, -50.0]);
    }

    #[test]
    fn roc_has_nan_warmup() {
        let r = Roc { period: 2 }.compute(&closes(&[1.0, 2.0, 4.0, 2.0]));
        eq(ys(&r, 0), &[NAN, NAN, 300.0, 0.0]);
    }

    #[test]
    fn mfi_matches_pine_reference() {
        // hlc3 = close; flows (upper, lower) = (1, 1), (2, 0), (0, 1), (3, 0):
        // bar 0 has no change, so its flow counts on both sides.
        let r = Mfi { period: 2 }.compute(&closes(&[1.0, 2.0, 1.0, 3.0]));
        eq(ys(&r, 0), &[NAN, 75.0, 200.0 / 3.0, 75.0]);
    }

    #[test]
    fn stochastic_smooths_with_sma() {
        let ind = Stochastic {
            period: 2,
            k_smooth: 2,
            d_smooth: 2,
        };
        // raw %K = [_, 100, 100, 0, 100]
        let r = ind.compute(&closes(&[1.0, 2.0, 3.0, 2.0, 4.0]));
        eq(ys(&r, 0), &[NAN, NAN, 100.0, 50.0, 50.0]);
        eq(ys(&r, 1), &[NAN, NAN, NAN, 75.0, 50.0]);
    }

    #[test]
    fn williams_r_values_and_flat_range() {
        let ind = WilliamsR { period: 2 };
        let r = ind.compute(&closes(&[1.0, 2.0, 3.0, 2.0, 4.0]));
        eq(ys(&r, 0), &[NAN, 0.0, 0.0, -100.0, 0.0]);
        // zero range -> NaN (Pine division by zero), not 0
        let r = ind.compute(&closes(&[5.0, 5.0, 5.0]));
        assert!(ys(&r, 0).iter().all(|v| v.is_nan()));
    }

    #[test]
    fn donchian_channels() {
        let r = Donchian { period: 2 }.compute(&closes(&[1.0, 3.0, 2.0]));
        eq(ys(&r, 0), &[NAN, 3.0, 3.0]);
        eq(ys(&r, 1), &[NAN, 2.0, 2.5]);
        eq(ys(&r, 2), &[NAN, 1.0, 2.0]);
    }

    #[test]
    fn cmf_sum_ratio() {
        // ad = [10, -10, 20], volume = [10, 10, 20]
        let d = bars(&[
            (2.0, 2.0, 0.0, 2.0, 10.0),
            (0.0, 2.0, 0.0, 0.0, 10.0),
            (4.0, 4.0, 0.0, 4.0, 20.0),
        ]);
        eq(
            ys(&Cmf { period: 2 }.compute(&d), 0),
            &[NAN, 0.0, 10.0 / 30.0],
        );
    }

    #[test]
    fn stochastic_zero_range_leaves_no_gap() {
        // raw %K = [_, 100, NaN, NaN, 100]: the flat 2-bar windows are a
        // division by zero, which ta.sma skips, so %K carries 100.
        let ind = Stochastic {
            period: 2,
            k_smooth: 1,
            d_smooth: 2,
        };
        let r = ind.compute(&closes(&[1.0, 2.0, 2.0, 2.0, 3.0]));
        eq(ys(&r, 0), &[NAN, 100.0, 100.0, 100.0, 100.0]);
        eq(ys(&r, 1), &[NAN, NAN, 100.0, 100.0, 100.0]);
    }

    #[test]
    fn cmf_btc_scale_zero_flow_is_exactly_zero() {
        // 20 bars of large money flow, then 20 bars closing at mid-range:
        // the last window's flow is exactly 0, not float residue.
        let mut v = Vec::new();
        for i in 0..40 {
            let p = 65_000.0 + (i * 37 % 101) as f64 * 13.37;
            let close = if i < 20 {
                p + 40.0 * ((i % 3) as f64 - 1.0)
            } else {
                p
            };
            let vol = 2_345.678 + (i * 7919 % 9973) as f64 * 2.73;
            v.push((p, p + 75.5, p - 75.5, close, vol));
        }
        let y = Cmf { period: 20 }.compute(&bars(&v)).lines[0].y.clone();
        assert!(y[19].abs() > 1e-6);
        assert_eq!(y[39], 0.0);
    }
}
