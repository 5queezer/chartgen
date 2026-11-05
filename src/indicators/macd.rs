use crate::data::OhlcvData;
use crate::indicator::*;
use serde_json::{json, Value};

pub struct Macd {
    pub fast: usize,
    pub slow: usize,
    pub signal: usize,
}

impl Default for Macd {
    fn default() -> Self {
        Self {
            fast: 12,
            slow: 26,
            signal: 9,
        }
    }
}

impl Indicator for Macd {
    fn name(&self) -> &str {
        "MACD"
    }

    fn description(&self) -> &str {
        "Moving Average Convergence Divergence — trend momentum"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "fast", "type": "integer", "min": 1, "max": 10000, "default": 12},
            {"name": "slow", "type": "integer", "min": 1, "max": 10000, "default": 26},
            {"name": "signal", "type": "integer", "min": 1, "max": 10000, "default": 9}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("fast").and_then(|v| v.as_u64()) {
            self.fast = v as usize;
        }
        if let Some(v) = params.get("slow").and_then(|v| v.as_u64()) {
            self.slow = v as usize;
        }
        if let Some(v) = params.get("signal").and_then(|v| v.as_u64()) {
            self.signal = v as usize;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        // TV: macd = ta.ema(close, fast) - ta.ema(close, slow),
        // signal = ta.ema(macd, signal), hist = macd - signal.
        let closes = data.closes();
        let fast = ema(&closes, self.fast);
        let slow = ema(&closes, self.slow);
        let macd_line: Vec<f64> = fast.iter().zip(&slow).map(|(f, s)| f - s).collect();
        let sig = ema(&macd_line, self.signal);
        let hist: Vec<f64> = macd_line.iter().zip(&sig).map(|(m, s)| m - s).collect();

        let hist_colors: Vec<_> = hist
            .iter()
            .map(|v| {
                if *v >= 0.0 {
                    rgba(0x26a69a, 0.8)
                } else {
                    rgba(0xef5350, 0.8)
                }
            })
            .collect();

        PanelResult {
            lines: vec![
                Line {
                    y: macd_line,
                    color: rgba(0x2962ff, 1.0),
                    width: 2,
                    label: Some("MACD".into()),
                },
                Line {
                    y: sig,
                    color: rgba(0xff6d00, 1.0),
                    width: 2,
                    label: Some("Signal".into()),
                },
            ],
            bars: vec![Bars {
                y: hist,
                colors: hist_colors,
                bottom: 0.0,
            }],
            hlines: vec![HLine {
                y: 0.0,
                color: rgba(0x787b86, 0.5),
            }],
            label: "MACD".into(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicators::pine::test_util::{closes, eq, NAN};

    #[test]
    fn macd_uses_sma_seeded_emas() {
        let ind = Macd {
            fast: 2,
            slow: 3,
            signal: 2,
        };
        let r = ind.compute(&closes(&[1.0, 2.0, 4.0, 8.0, 16.0, 8.0]));
        // ema2 = [_, 1.5, 19/6, 115/18, ...], ema3 = [_, _, 7/3, 31/6, ...]
        let m = [NAN, NAN, 5.0 / 6.0, 11.0 / 9.0];
        eq(&r.lines[0].y[..4], &m);
        // signal = ema2(macd) seeds with SMA(5/6, 11/9) = 37/36 at bar 3
        let sig = r.lines[1].y.clone();
        eq(&sig[..4], &[NAN, NAN, NAN, 37.0 / 36.0]);
        eq(
            &r.bars[0].y[..4],
            &[NAN, NAN, NAN, 11.0 / 9.0 - 37.0 / 36.0],
        );
        // one bar after warmup: 2/3 * macd[4] + 1/3 * signal[3]
        assert!((sig[4] - (2.0 / 3.0 * r.lines[0].y[4] + 37.0 / 108.0)).abs() < 1e-9);
        assert!((r.lines[0].y[4] - 2.212962962962962).abs() < 1e-9);
    }
}
