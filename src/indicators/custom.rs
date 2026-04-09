use crate::data::OhlcvData;
use crate::indicator::*;
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
