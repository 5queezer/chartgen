use crate::data::OhlcvData;
use crate::indicator::*;
use crate::indicators::rsi::compute_rsi;
use crate::trading::signals;
use serde_json::{json, Value};

pub struct RsiMfiStochCombo {
    pub rsi_length: usize,
    pub mfi_length: usize,
    pub stoch_k: usize,
    pub stoch_d: usize,
    pub stoch_smooth: usize,
    pub overbought: f64,
    pub oversold: f64,
    pub show_entry_dots: bool,
    pub show_trend_bars: bool,
}

impl Default for RsiMfiStochCombo {
    fn default() -> Self {
        Self {
            rsi_length: 14,
            mfi_length: 14,
            stoch_k: 14,
            stoch_d: 3,
            stoch_smooth: 3,
            overbought: 70.0,
            oversold: 30.0,
            show_entry_dots: true,
            show_trend_bars: true,
        }
    }
}

/// Compute the real Money Flow Index from OHLCV data (Pine `ta.mfi(hlc3, length)`).
fn compute_mfi(data: &OhlcvData, length: usize) -> Vec<f64> {
    use super::pine::{mfi, source_series, volumes};
    mfi(&source_series(data, "hlc3"), &volumes(data), length)
}

impl Indicator for RsiMfiStochCombo {
    fn name(&self) -> &str {
        "RSI/MFI/Stoch"
    }

    fn description(&self) -> &str {
        "RSI + Money Flow Index + Stochastic combo panel (all 0-100)"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "rsi_length", "type": "integer", "min": 1, "max": 10000, "default": 14},
            {"name": "mfi_length", "type": "integer", "min": 1, "max": 10000, "default": 14},
            {"name": "stoch_k", "type": "integer", "min": 1, "max": 10000, "default": 14},
            {"name": "stoch_d", "type": "integer", "min": 1, "max": 10000, "default": 3},
            {"name": "stoch_smooth", "type": "integer", "min": 1, "max": 10000, "default": 3},
            {"name": "overbought", "type": "number", "default": 70},
            {"name": "oversold", "type": "number", "default": 30},
            {"name": "show_entry_dots", "type": "boolean", "default": true},
            {"name": "show_trend_bars", "type": "boolean", "default": true}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("rsi_length").and_then(|v| v.as_u64()) {
            self.rsi_length = v as usize;
        }
        if let Some(v) = params.get("mfi_length").and_then(|v| v.as_u64()) {
            self.mfi_length = v as usize;
        }
        if let Some(v) = params.get("stoch_k").and_then(|v| v.as_u64()) {
            self.stoch_k = v as usize;
        }
        if let Some(v) = params.get("stoch_d").and_then(|v| v.as_u64()) {
            self.stoch_d = v as usize;
        }
        if let Some(v) = params.get("stoch_smooth").and_then(|v| v.as_u64()) {
            self.stoch_smooth = v as usize;
        }
        if let Some(v) = params.get("overbought").and_then(|v| v.as_f64()) {
            self.overbought = v;
        }
        if let Some(v) = params.get("oversold").and_then(|v| v.as_f64()) {
            self.oversold = v;
        }
        if let Some(v) = params.get("show_entry_dots").and_then(|v| v.as_bool()) {
            self.show_entry_dots = v;
        }
        if let Some(v) = params.get("show_trend_bars").and_then(|v| v.as_bool()) {
            self.show_trend_bars = v;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        if n < 2 {
            return PanelResult {
                label: "RSI/MFI/Stoch".into(),
                y_range: Some((0.0, 100.0)),
                ..Default::default()
            };
        }

        let closes = data.closes();
        let highs = data.highs();
        let lows = data.lows();

        // RSI
        let rsi = compute_rsi(&closes, self.rsi_length);

        // MFI (real Money Flow Index)
        let mfi = compute_mfi(data, self.mfi_length);

        // Stochastic
        let k_raw = stoch(&closes, &highs, &lows, self.stoch_k);
        let k_smooth = sma(&k_raw, self.stoch_smooth);
        let d_smooth = sma(&k_smooth, self.stoch_d);

        // Lines
        let mut lines = vec![
            Line {
                y: rsi.clone(),
                color: rgba(0x26a69a, 1.0),
                width: 2,
                label: Some("RSI".into()),
            },
            Line {
                y: mfi.clone(),
                color: rgba(0x2962FF, 1.0),
                width: 2,
                label: Some("MFI".into()),
            },
            Line {
                y: k_smooth.clone(),
                color: rgba(0xAB47BC, 1.0),
                width: 2,
                label: Some("Stoch K".into()),
            },
        ];

        // Optionally show Stoch D as a thinner reference line
        lines.push(Line {
            y: d_smooth.clone(),
            color: rgba(0xAB47BC, 0.4),
            width: 1,
            label: Some("Stoch D".into()),
        });

        // HLines
        let hlines = vec![
            HLine {
                y: self.overbought,
                color: rgba(0xff0000, 0.3),
            },
            HLine {
                y: 50.0,
                color: rgba(0x787B86, 0.3),
            },
            HLine {
                y: self.oversold,
                color: rgba(0x00e676, 0.3),
            },
        ];

        // Trend bars
        let mut bars_out = Vec::new();
        if self.show_trend_bars {
            let mut bar_colors = Vec::with_capacity(n);
            let bar_heights = vec![5.0; n];
            for i in 0..n {
                let r = rsi[i];
                let m = mfi[i];
                let k = k_smooth[i];
                if r.is_nan() || m.is_nan() || k.is_nan() {
                    bar_colors.push(rgba(0x000000, 0.0));
                } else if r > 50.0 && k > 50.0 && m > 50.0 {
                    bar_colors.push(rgba(0x26a69a, 0.15));
                } else if r < 50.0 && k < 50.0 && m < 50.0 {
                    bar_colors.push(rgba(0xef5350, 0.15));
                } else {
                    bar_colors.push(rgba(0x000000, 0.0));
                }
            }
            bars_out.push(Bars {
                y: bar_heights,
                colors: bar_colors,
                bottom: 0.0,
            });
        }

        // Entry dots
        let mut dots = Vec::new();
        if self.show_entry_dots {
            for i in 1..n {
                // RSI crosses up through oversold
                if !rsi[i].is_nan()
                    && !rsi[i - 1].is_nan()
                    && rsi[i - 1] < self.oversold
                    && rsi[i] >= self.oversold
                {
                    dots.push(Dot {
                        x: i,
                        y: rsi[i],
                        color: rgba(0x00e676, 1.0),
                        size: 5,
                        label: Some(signals::RSI_OVERSOLD.to_string()),
                    });
                }
                // RSI crosses down through overbought
                if !rsi[i].is_nan()
                    && !rsi[i - 1].is_nan()
                    && rsi[i - 1] > self.overbought
                    && rsi[i] <= self.overbought
                {
                    dots.push(Dot {
                        x: i,
                        y: rsi[i],
                        color: rgba(0xff0000, 1.0),
                        size: 5,
                        label: Some(signals::RSI_OVERBOUGHT.to_string()),
                    });
                }

                // Stoch K crosses up through D while K < 20
                if !k_smooth[i].is_nan()
                    && !k_smooth[i - 1].is_nan()
                    && !d_smooth[i].is_nan()
                    && !d_smooth[i - 1].is_nan()
                    && k_smooth[i - 1] < d_smooth[i - 1]
                    && k_smooth[i] >= d_smooth[i]
                    && k_smooth[i] < 20.0
                {
                    dots.push(Dot {
                        x: i,
                        y: k_smooth[i],
                        color: rgba(0x00e676, 1.0),
                        size: 5,
                        label: Some(signals::STOCH_OVERSOLD.to_string()),
                    });
                }
                // Stoch K crosses down through D while K > 80
                if !k_smooth[i].is_nan()
                    && !k_smooth[i - 1].is_nan()
                    && !d_smooth[i].is_nan()
                    && !d_smooth[i - 1].is_nan()
                    && k_smooth[i - 1] > d_smooth[i - 1]
                    && k_smooth[i] <= d_smooth[i]
                    && k_smooth[i] > 80.0
                {
                    dots.push(Dot {
                        x: i,
                        y: k_smooth[i],
                        color: rgba(0xff0000, 1.0),
                        size: 5,
                        label: Some(signals::STOCH_OVERBOUGHT.to_string()),
                    });
                }
            }
        }

        PanelResult {
            lines,
            hlines,
            bars: bars_out,
            dots,
            y_range: Some((0.0, 100.0)),
            label: "RSI/MFI/Stoch".into(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicators::pine::test_util::{closes, eq, NAN};

    #[test]
    fn combo_lines_follow_pine() {
        let mut ind = RsiMfiStochCombo::default();
        ind.configure(&json!({"rsi_length": 2, "mfi_length": 2, "stoch_k": 2,
                              "stoch_smooth": 2, "stoch_d": 2}));
        let r = ind.compute(&closes(&[1.0, 2.0, 1.0, 2.0, 3.0, 2.0]));
        // ta.rsi(2): RMA seeded with the SMA of the first 2 changes
        eq(&r.lines[0].y, &[NAN, NAN, 50.0, 75.0, 87.5, 43.75]);
        // ta.mfi(hlc3, 2): upper sums [_, 3, 2, 2, 5, 3], lower [_, 1, 1, 1, 0, 2]
        eq(
            &r.lines[1].y,
            &[NAN, 75.0, 200.0 / 3.0, 200.0 / 3.0, NAN, 60.0],
        );
        // raw stoch = [_, 100, 0, 100, 100, 0]; K = sma2, D = sma2(K)
        eq(&r.lines[2].y, &[NAN, NAN, 50.0, 50.0, 100.0, 50.0]);
        eq(&r.lines[3].y, &[NAN, NAN, NAN, 50.0, 75.0, 75.0]);
    }
}
