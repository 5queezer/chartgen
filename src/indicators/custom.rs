use crate::data::OhlcvData;
use crate::indicator::*;
use serde_json::{json, Value};
use ta::indicators::AverageTrueRange as TaAtr;
use ta::{DataItem, Next};

fn make_data_item(bar: &crate::data::Bar) -> DataItem {
    DataItem::builder()
        .open(bar.open)
        .high(bar.high)
        .low(bar.low)
        .close(bar.close)
        .volume(bar.volume)
        .build()
        .unwrap()
}

// ---- VWAP (overlay) ----

pub struct Vwap;

impl Indicator for Vwap {
    fn name(&self) -> &str {
        "VWAP"
    }

    fn description(&self) -> &str {
        "Volume Weighted Average Price — intraday anchor"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut vals = vec![f64::NAN; n];
        let mut cum_tp_vol = 0.0;
        let mut cum_vol = 0.0;

        for (i, bar) in data.bars.iter().enumerate() {
            let tp = (bar.high + bar.low + bar.close) / 3.0;
            cum_tp_vol += tp * bar.volume;
            cum_vol += bar.volume;
            if cum_vol > 0.0 {
                vals[i] = cum_tp_vol / cum_vol;
            }
        }

        PanelResult {
            lines: vec![Line {
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
        json!([{"name": "period", "type": "integer", "default": 14}])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("period").and_then(|v| v.as_u64()) {
            self.period = v as usize;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let period = self.period;
        let mut adx_vals = vec![f64::NAN; n];
        let mut plus_di_vals = vec![f64::NAN; n];
        let mut minus_di_vals = vec![f64::NAN; n];

        if n < 2 {
            return PanelResult {
                label: "ADX".into(),
                ..Default::default()
            };
        }

        let p = period as f64;

        // Compute raw TR, +DM, -DM
        let mut tr = vec![0.0; n];
        let mut plus_dm = vec![0.0; n];
        let mut minus_dm = vec![0.0; n];

        for i in 1..n {
            let high = data.bars[i].high;
            let low = data.bars[i].low;
            let prev_high = data.bars[i - 1].high;
            let prev_low = data.bars[i - 1].low;
            let prev_close = data.bars[i - 1].close;

            tr[i] = (high - low)
                .max((high - prev_close).abs())
                .max((low - prev_close).abs());

            let up_move = high - prev_high;
            let down_move = prev_low - low;

            if up_move > down_move && up_move > 0.0 {
                plus_dm[i] = up_move;
            }
            if down_move > up_move && down_move > 0.0 {
                minus_dm[i] = down_move;
            }
        }

        // Wilder's smoothing for TR, +DM, -DM
        let mut smooth_tr = 0.0;
        let mut smooth_plus_dm = 0.0;
        let mut smooth_minus_dm = 0.0;

        // Initialize with sum of first `period` values (starting from index 1)
        if n < period + 1 {
            return PanelResult {
                label: "ADX".into(),
                ..Default::default()
            };
        }

        for i in 1..=period {
            smooth_tr += tr[i];
            smooth_plus_dm += plus_dm[i];
            smooth_minus_dm += minus_dm[i];
        }

        let mut dx_vals = vec![f64::NAN; n];

        // First DI values at index `period`
        if smooth_tr > 0.0 {
            plus_di_vals[period] = 100.0 * smooth_plus_dm / smooth_tr;
            minus_di_vals[period] = 100.0 * smooth_minus_dm / smooth_tr;
            let di_sum = plus_di_vals[period] + minus_di_vals[period];
            if di_sum > 0.0 {
                dx_vals[period] =
                    100.0 * (plus_di_vals[period] - minus_di_vals[period]).abs() / di_sum;
            }
        }

        for i in (period + 1)..n {
            smooth_tr = smooth_tr - (smooth_tr / p) + tr[i];
            smooth_plus_dm = smooth_plus_dm - (smooth_plus_dm / p) + plus_dm[i];
            smooth_minus_dm = smooth_minus_dm - (smooth_minus_dm / p) + minus_dm[i];

            if smooth_tr > 0.0 {
                plus_di_vals[i] = 100.0 * smooth_plus_dm / smooth_tr;
                minus_di_vals[i] = 100.0 * smooth_minus_dm / smooth_tr;
                let di_sum = plus_di_vals[i] + minus_di_vals[i];
                if di_sum > 0.0 {
                    dx_vals[i] = 100.0 * (plus_di_vals[i] - minus_di_vals[i]).abs() / di_sum;
                }
            }
        }

        // ADX = Wilder's smoothing of DX
        // First ADX value at index 2*period
        if n > 2 * period {
            let mut adx_sum = 0.0;
            let mut count = 0;
            for val in &dx_vals[period..=(2 * period)] {
                if !val.is_nan() {
                    adx_sum += val;
                    count += 1;
                }
            }
            if count > 0 {
                let mut adx = adx_sum / count as f64;
                adx_vals[2 * period] = adx;

                for i in (2 * period + 1)..n {
                    if !dx_vals[i].is_nan() {
                        adx = (adx * (p - 1.0) + dx_vals[i]) / p;
                        adx_vals[i] = adx;
                    }
                }
            }
        }

        PanelResult {
            lines: vec![
                Line {
                    y: adx_vals,
                    color: rgba(0xFFFFFF, 0.9),
                    width: 2,
                    label: Some("ADX".into()),
                },
                Line {
                    y: plus_di_vals,
                    color: rgba(0x26a69a, 0.9),
                    width: 1,
                    label: Some("+DI".into()),
                },
                Line {
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
        "Supertrend — ATR-based trend follower"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "period", "type": "integer", "default": 10},
            {"name": "multiplier", "type": "number", "default": 3.0}
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

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut dots = Vec::new();

        if n < 2 {
            return PanelResult {
                is_overlay: true,
                label: "Supertrend".into(),
                ..Default::default()
            };
        }

        // Compute ATR using ta-rs
        let mut atr_ind = TaAtr::new(self.period).unwrap();
        let mut atr_vals = vec![f64::NAN; n];
        for (i, bar) in data.bars.iter().enumerate() {
            let di = make_data_item(bar);
            atr_vals[i] = atr_ind.next(&di);
        }

        let mut final_upper = vec![f64::NAN; n];
        let mut final_lower = vec![f64::NAN; n];
        // true = UP, false = DOWN
        let mut trend = vec![true; n];
        let mut st_val = vec![f64::NAN; n];

        for i in 0..n {
            if atr_vals[i].is_nan() {
                continue;
            }

            let hl2 = (data.bars[i].high + data.bars[i].low) / 2.0;
            let basic_upper = hl2 + self.multiplier * atr_vals[i];
            let basic_lower = hl2 - self.multiplier * atr_vals[i];

            if i == 0 || final_upper[i - 1].is_nan() {
                final_upper[i] = basic_upper;
                final_lower[i] = basic_lower;
                trend[i] = true;
            } else {
                let prev_close = data.bars[i - 1].close;

                final_upper[i] =
                    if basic_upper < final_upper[i - 1] || prev_close > final_upper[i - 1] {
                        basic_upper
                    } else {
                        final_upper[i - 1]
                    };

                final_lower[i] =
                    if basic_lower > final_lower[i - 1] || prev_close < final_lower[i - 1] {
                        basic_lower
                    } else {
                        final_lower[i - 1]
                    };

                let close = data.bars[i].close;
                if close > final_upper[i] {
                    trend[i] = true;
                } else if close < final_lower[i] {
                    trend[i] = false;
                } else {
                    trend[i] = trend[i - 1];
                }
            }

            let val = if trend[i] {
                final_lower[i]
            } else {
                final_upper[i]
            };
            st_val[i] = val;

            let color = if trend[i] {
                rgba(0x26a69a, 0.9)
            } else {
                rgba(0xef5350, 0.9)
            };
            dots.push(Dot {
                x: i,
                y: val,
                color,
                size: 2,
            });
        }

        PanelResult {
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
            {"name": "af_start", "type": "number", "default": 0.02},
            {"name": "af_step", "type": "number", "default": 0.02},
            {"name": "af_max", "type": "number", "default": 0.2}
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

        if n < 2 {
            return PanelResult {
                is_overlay: true,
                label: "SAR".into(),
                ..Default::default()
            };
        }

        // true = UP (SAR below price), false = DOWN (SAR above price)
        let mut trend_up = true;
        let mut sar = data.bars[0].low;
        let mut ep = data.bars[0].high;
        let mut af = self.af_start;

        // First bar
        dots.push(Dot {
            x: 0,
            y: sar,
            color: rgba(0x26a69a, 0.9),
            size: 2,
        });

        for i in 1..n {
            let high = data.bars[i].high;
            let low = data.bars[i].low;

            let mut new_sar = sar + af * (ep - sar);

            if trend_up {
                // Clamp SAR to be at or below previous lows
                new_sar = new_sar.min(data.bars[i - 1].low);
                if i >= 2 {
                    new_sar = new_sar.min(data.bars[i - 2].low);
                }

                if high > ep {
                    ep = high;
                    af = (af + self.af_step).min(self.af_max);
                }

                if low < new_sar {
                    // Reverse to DOWN
                    trend_up = false;
                    new_sar = ep;
                    ep = low;
                    af = self.af_start;
                }
            } else {
                // Clamp SAR to be at or above previous highs
                new_sar = new_sar.max(data.bars[i - 1].high);
                if i >= 2 {
                    new_sar = new_sar.max(data.bars[i - 2].high);
                }

                if low < ep {
                    ep = low;
                    af = (af + self.af_step).min(self.af_max);
                }

                if high > new_sar {
                    // Reverse to UP
                    trend_up = true;
                    new_sar = ep;
                    ep = high;
                    af = self.af_start;
                }
            }

            sar = new_sar;

            let color = if trend_up {
                rgba(0x26a69a, 0.9)
            } else {
                rgba(0xef5350, 0.9)
            };
            dots.push(Dot {
                x: i,
                y: sar,
                color,
                size: 2,
            });
        }

        PanelResult {
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
        Self { period: 20 }
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
        json!([{"name": "period", "type": "integer", "default": 20}])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("period").and_then(|v| v.as_u64()) {
            self.period = v as usize;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut vals = vec![f64::NAN; n];

        if n < 2 {
            return PanelResult {
                label: "HV".into(),
                ..Default::default()
            };
        }

        // Compute log returns
        let mut log_returns = vec![f64::NAN; n];
        for (i, pair) in data.bars.windows(2).enumerate() {
            if pair[1].close > 0.0 && pair[0].close > 0.0 {
                log_returns[i + 1] = (pair[1].close / pair[0].close).ln();
            }
        }

        let period = self.period;
        // Rolling window standard deviation, annualized
        for i in period..n {
            let window = &log_returns[i + 1 - period..=i];
            let valid: Vec<f64> = window.iter().copied().filter(|r| !r.is_nan()).collect();
            if valid.len() < 2 {
                continue;
            }
            let count = valid.len() as f64;
            let mean: f64 = valid.iter().sum::<f64>() / count;
            let var: f64 = valid.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / count;
            vals[i] = var.sqrt() * (252.0_f64).sqrt() * 100.0;
        }

        PanelResult {
            lines: vec![Line {
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
}

impl Default for VwapBands {
    fn default() -> Self {
        Self { std_dev: 2.0 }
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
        json!([{"name": "std_dev", "type": "number", "default": 2.0}])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("std_dev").and_then(|v| v.as_f64()) {
            self.std_dev = v;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut vwap_vals = vec![f64::NAN; n];
        let mut upper = vec![f64::NAN; n];
        let mut lower = vec![f64::NAN; n];

        let mut cum_tp_vol = 0.0;
        let mut cum_vol = 0.0;
        let mut cum_tp2_vol = 0.0;

        for (i, bar) in data.bars.iter().enumerate() {
            let tp = (bar.high + bar.low + bar.close) / 3.0;
            cum_tp_vol += tp * bar.volume;
            cum_vol += bar.volume;
            cum_tp2_vol += tp * tp * bar.volume;
            if cum_vol > 0.0 {
                let vwap = cum_tp_vol / cum_vol;
                let variance = (cum_tp2_vol / cum_vol - vwap * vwap).max(0.0);
                let std = variance.sqrt();
                vwap_vals[i] = vwap;
                upper[i] = vwap + self.std_dev * std;
                lower[i] = vwap - self.std_dev * std;
            }
        }

        PanelResult {
            lines: vec![
                Line {
                    y: upper.clone(),
                    color: rgba(0x787B86, 0.5),
                    width: 1,
                    label: Some("VWAP Upper".into()),
                },
                Line {
                    y: vwap_vals,
                    color: rgba(0xFF9800, 0.9),
                    width: 2,
                    label: Some("VWAP".into()),
                },
                Line {
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
        let mut dots = Vec::with_capacity(n);

        if n == 0 {
            return PanelResult {
                is_overlay: true,
                label: "HA".into(),
                ..Default::default()
            };
        }

        let mut ha_open = vec![0.0_f64; n];
        let mut ha_close = vec![0.0_f64; n];

        // First bar
        ha_close[0] =
            (data.bars[0].open + data.bars[0].high + data.bars[0].low + data.bars[0].close) / 4.0;
        ha_open[0] = (data.bars[0].open + data.bars[0].close) / 2.0;

        for i in 1..n {
            let bar = &data.bars[i];
            ha_close[i] = (bar.open + bar.high + bar.low + bar.close) / 4.0;
            ha_open[i] = (ha_open[i - 1] + ha_close[i - 1]) / 2.0;
        }

        for i in 0..n {
            let color = if ha_close[i] >= ha_open[i] {
                rgba(0x26a69a, 0.9) // bullish green
            } else {
                rgba(0xef5350, 0.9) // bearish red
            };
            dots.push(Dot {
                x: i,
                y: ha_close[i],
                color,
                size: 3,
            });
        }

        PanelResult {
            dots,
            is_overlay: true,
            label: "HA".into(),
            ..Default::default()
        }
    }
}

// ---- Pivot Points (overlay) ----

pub struct PivotPoints;

impl Indicator for PivotPoints {
    fn name(&self) -> &str {
        "Pivot"
    }

    fn description(&self) -> &str {
        "Pivot Points — support/resistance from prior bar"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut pivot = vec![f64::NAN; n];
        let mut r1 = vec![f64::NAN; n];
        let mut s1 = vec![f64::NAN; n];
        let mut r2 = vec![f64::NAN; n];
        let mut s2 = vec![f64::NAN; n];

        for i in 1..n {
            let prev = &data.bars[i - 1];
            let p = (prev.high + prev.low + prev.close) / 3.0;
            pivot[i] = p;
            r1[i] = 2.0 * p - prev.low;
            s1[i] = 2.0 * p - prev.high;
            r2[i] = p + (prev.high - prev.low);
            s2[i] = p - (prev.high - prev.low);
        }

        PanelResult {
            lines: vec![
                Line {
                    y: pivot,
                    color: rgba(0xFFEB3B, 0.6),
                    width: 1,
                    label: Some("Pivot".into()),
                },
                Line {
                    y: r1,
                    color: rgba(0xef5350, 0.4),
                    width: 1,
                    label: Some("R1".into()),
                },
                Line {
                    y: s1,
                    color: rgba(0x26a69a, 0.4),
                    width: 1,
                    label: Some("S1".into()),
                },
                Line {
                    y: r2,
                    color: rgba(0xef5350, 0.3),
                    width: 1,
                    label: Some("R2".into()),
                },
                Line {
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

impl Indicator for VolumeProfile {
    fn name(&self) -> &str {
        "VPVR"
    }

    fn description(&self) -> &str {
        "Volume Profile Visible Range — horizontal volume histogram with POC/VAH/VAL"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "bins", "type": "integer", "default": 24},
            {"name": "side", "type": "string", "default": "left", "description": "left or right"},
            {"name": "range_bars", "type": "integer", "default": null, "description": "only compute over last N bars (null = all)"},
            {"name": "split_up_down", "type": "boolean", "default": false, "description": "separate up/down volume colors"},
            {"name": "color_up", "type": "string", "default": "#26a69a", "description": "hex color for up-volume"},
            {"name": "color_down", "type": "string", "default": "#ef5350", "description": "hex color for down-volume"},
            {"name": "opacity", "type": "number", "default": 0.35, "description": "overall opacity multiplier (0.0-1.0)"}
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
            let try_low = if va_low_idx > 0 {
                volume_bins[va_low_idx - 1]
            } else {
                0.0
            };
            let try_high = if va_high_idx < bins - 1 {
                volume_bins[va_high_idx + 1]
            } else {
                0.0
            };
            if try_low >= try_high && va_low_idx > 0 {
                va_low_idx -= 1;
                va_vol += volume_bins[va_low_idx];
            } else if va_high_idx < bins - 1 {
                va_high_idx += 1;
                va_vol += volume_bins[va_high_idx];
            } else if va_low_idx > 0 {
                va_low_idx -= 1;
                va_vol += volume_bins[va_low_idx];
            } else {
                break;
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

/// Weighted Moving Average.
fn wma(data: &[f64], period: usize) -> Vec<f64> {
    let n = data.len();
    let mut out = vec![f64::NAN; n];
    if n == 0 || period == 0 {
        return out;
    }
    let denom: f64 = (1..=period).sum::<usize>() as f64;
    for i in (period - 1)..n {
        let mut sum = 0.0;
        let mut all_valid = true;
        for j in 0..period {
            let v = data[i + j + 1 - period];
            if v.is_nan() {
                all_valid = false;
                break;
            }
            sum += v * (j + 1) as f64;
        }
        if all_valid {
            out[i] = sum / denom;
        }
    }
    out
}

/// Hull Moving Average: WMA(2*WMA(src, len/2) - WMA(src, len), sqrt(len)).
fn hma(data: &[f64], period: usize) -> Vec<f64> {
    let n = data.len();
    if n == 0 || period < 2 {
        return vec![f64::NAN; n];
    }
    let half = period / 2;
    let sqrt_len = (period as f64).sqrt().round() as usize;

    let wma_half = wma(data, half.max(1));
    let wma_full = wma(data, period);

    // diff = 2 * wma_half - wma_full
    let mut diff = vec![f64::NAN; n];
    for i in 0..n {
        if !wma_half[i].is_nan() && !wma_full[i].is_nan() {
            diff[i] = 2.0 * wma_half[i] - wma_full[i];
        }
    }
    wma(&diff, sqrt_len.max(1))
}

/// Rolling standard deviation over `period` bars (population stdev).
fn rolling_stdev(data: &[f64], period: usize) -> Vec<f64> {
    let n = data.len();
    let mut out = vec![f64::NAN; n];
    if n == 0 || period == 0 {
        return out;
    }
    for i in (period - 1)..n {
        let window = &data[i + 1 - period..=i];
        let valid: Vec<f64> = window.iter().copied().filter(|v| !v.is_nan()).collect();
        if valid.len() < 2 {
            continue;
        }
        let cnt = valid.len() as f64;
        let mean = valid.iter().sum::<f64>() / cnt;
        let var = valid.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / cnt;
        out[i] = var.sqrt();
    }
    out
}

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
            {"name": "vzo_length", "type": "integer", "default": 70},
            {"name": "k", "type": "number", "default": 0.06},
            {"name": "sig_length", "type": "integer", "default": 10}
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

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        if n < 3 {
            return PanelResult {
                label: "KVF".into(),
                ..Default::default()
            };
        }

        let closes = data.closes();
        let volumes: Vec<f64> = data.bars.iter().map(|b| b.volume).collect();

        // Volume direction: positive if close > close[i-2], else negative
        let mut vol_dir = vec![f64::NAN; n];
        vol_dir[0] = volumes[0];
        vol_dir[1] = volumes[1];
        for i in 2..n {
            vol_dir[i] = if closes[i] > closes[i - 2] {
                volumes[i]
            } else {
                -volumes[i]
            };
        }

        // VZO
        let vzo_volume = hma(&vol_dir, self.vzo_length);
        let total_volume = hma(&volumes, self.vzo_length);

        let mut vzo = vec![f64::NAN; n];
        for i in 0..n {
            if !vzo_volume[i].is_nan() && !total_volume[i].is_nan() && total_volume[i].abs() > 1e-30
            {
                vzo[i] = vzo_volume[i] / total_volume[i];
            }
        }

        // Z-score normalize VZO
        let vzo_std = rolling_stdev(&vzo, 200);
        for i in 0..n {
            if !vzo[i].is_nan() && !vzo_std[i].is_nan() && vzo_std[i] > 1e-30 {
                vzo[i] /= vzo_std[i];
            }
        }

        // Kalman filter — Pine Script initializes M_n = 0.0, nz(M_n[1]) = 0
        let mut m_n = vec![f64::NAN; n];
        let mut prev_m = 0.0_f64; // Pine: series float M_n = 0.0
        for i in 0..n {
            if vzo[i].is_nan() {
                m_n[i] = prev_m; // carry forward like Pine's nz()
                continue;
            }
            let a_n = vzo[i];
            prev_m = self.k * a_n + (1.0 - self.k) * prev_m;
            m_n[i] = prev_m;
        }

        // Signal line = SMA(m_n, sig_length)
        let signal = sma(&m_n, self.sig_length);

        // Volume histogram background
        let vol_std = rolling_stdev(&volumes, 200);
        let mut vol_norm = vec![f64::NAN; n];
        for i in 0..n {
            if !vol_std[i].is_nan() && vol_std[i] > 1e-30 {
                let v = (volumes[i] / vol_std[i]) / 10.0;
                vol_norm[i] = if v > 0.8 { 0.0 } else { v };
            }
        }

        // Build dots for Kalman line (colored per bar)
        let mut dots = Vec::new();
        let cyan = 0x00BCD4;
        let red = 0xFF1744;

        for i in 0..n {
            if m_n[i].is_nan() || signal[i].is_nan() {
                continue;
            }
            let bullish = m_n[i] > signal[i];
            let color = if bullish {
                rgba(cyan, 0.9)
            } else {
                rgba(red, 0.9)
            };
            dots.push(Dot {
                x: i,
                y: m_n[i],
                color,
                size: 2,
            });
            // Signal dot (fainter)
            let sig_color = if bullish {
                rgba(cyan, 0.4)
            } else {
                rgba(red, 0.4)
            };
            dots.push(Dot {
                x: i,
                y: signal[i],
                color: sig_color,
                size: 1,
            });
        }

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
                let (size, color) = if prev_m < -self.ob_os_zone {
                    (6, rgba(cyan, 1.0)) // oversold buy
                } else if in_neutral {
                    (4, rgba(cyan, 1.0)) // local buy
                } else {
                    continue;
                };
                dots.push(Dot {
                    x: i - 1,
                    y: prev_m,
                    color,
                    size,
                });
            } else if cross_under {
                let (size, color) = if prev_m > self.ob_os_zone {
                    (6, rgba(red, 1.0)) // overbought sell
                } else if in_neutral {
                    (4, rgba(red, 1.0)) // local sell
                } else {
                    continue;
                };
                dots.push(Dot {
                    x: i - 1,
                    y: prev_m,
                    color,
                    size,
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
            lines: vec![],
            fills: vec![
                Fill {
                    y1: bull_m,
                    y2: bull_s,
                    color: rgba(cyan, 0.15),
                },
                Fill {
                    y1: bear_m,
                    y2: bear_s,
                    color: rgba(red, 0.15),
                },
                Fill {
                    y1: top,
                    y2: ob_line,
                    color: rgba(red, 0.1),
                },
                Fill {
                    y1: os_line,
                    y2: bottom,
                    color: rgba(cyan, 0.1),
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
        }
    }
}

// ---- Ichimoku Cloud (overlay) ----

pub struct Ichimoku {
    pub tenkan: usize,
    pub kijun: usize,
    pub senkou_b: usize,
}

impl Default for Ichimoku {
    fn default() -> Self {
        Self {
            tenkan: 9,
            kijun: 26,
            senkou_b: 52,
        }
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
            {"name": "tenkan", "type": "integer", "default": 9},
            {"name": "kijun", "type": "integer", "default": 26},
            {"name": "senkou_b", "type": "integer", "default": 52}
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
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let highs = data.highs();
        let lows = data.lows();
        let n = data.len();

        let hh_tenkan = highest(&highs, self.tenkan);
        let ll_tenkan = lowest(&lows, self.tenkan);
        let hh_kijun = highest(&highs, self.kijun);
        let ll_kijun = lowest(&lows, self.kijun);
        let hh_senkou_b = highest(&highs, self.senkou_b);
        let ll_senkou_b = lowest(&lows, self.senkou_b);

        let mut tenkan_sen = vec![f64::NAN; n];
        let mut kijun_sen = vec![f64::NAN; n];
        let mut senkou_a = vec![f64::NAN; n];
        let mut senkou_b = vec![f64::NAN; n];

        for i in 0..n {
            if !hh_tenkan[i].is_nan() && !ll_tenkan[i].is_nan() {
                tenkan_sen[i] = (hh_tenkan[i] + ll_tenkan[i]) / 2.0;
            }
            if !hh_kijun[i].is_nan() && !ll_kijun[i].is_nan() {
                kijun_sen[i] = (hh_kijun[i] + ll_kijun[i]) / 2.0;
            }
            if !tenkan_sen[i].is_nan() && !kijun_sen[i].is_nan() {
                senkou_a[i] = (tenkan_sen[i] + kijun_sen[i]) / 2.0;
            }
            if !hh_senkou_b[i].is_nan() && !ll_senkou_b[i].is_nan() {
                senkou_b[i] = (hh_senkou_b[i] + ll_senkou_b[i]) / 2.0;
            }
        }

        PanelResult {
            lines: vec![
                Line {
                    y: tenkan_sen,
                    color: rgba(0x0095ff, 0.9),
                    width: 1,
                    label: Some("Tenkan".into()),
                },
                Line {
                    y: kijun_sen,
                    color: rgba(0xff0044, 0.9),
                    width: 1,
                    label: Some("Kijun".into()),
                },
                Line {
                    y: senkou_a.clone(),
                    color: rgba(0x26a69a, 0.5),
                    width: 1,
                    label: Some("Senkou A".into()),
                },
                Line {
                    y: senkou_b.clone(),
                    color: rgba(0xef5350, 0.5),
                    width: 1,
                    label: Some("Senkou B".into()),
                },
            ],
            fills: vec![Fill {
                y1: senkou_a,
                y2: senkou_b,
                color: rgba(0x2196F3, 0.08),
            }],
            is_overlay: true,
            label: "Ichimoku".into(),
            ..Default::default()
        }
    }
}
