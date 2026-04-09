use crate::data::OhlcvData;
use crate::indicator::*;

pub struct Rsi {
    pub length: usize,
    pub overbought: f64,
    pub oversold: f64,
}

impl Default for Rsi {
    fn default() -> Self {
        Self {
            length: 14,
            overbought: 70.0,
            oversold: 30.0,
        }
    }
}

pub fn compute_rsi(closes: &[f64], length: usize) -> Vec<f64> {
    let n = closes.len();
    let mut out = vec![f64::NAN; n];
    if n <= length {
        return out;
    }

    let mut avg_gain = 0.0;
    let mut avg_loss = 0.0;

    for i in 1..=length {
        let d = closes[i] - closes[i - 1];
        if d > 0.0 {
            avg_gain += d;
        } else {
            avg_loss -= d;
        }
    }
    avg_gain /= length as f64;
    avg_loss /= length as f64;

    if avg_loss > 0.0 {
        out[length] = 100.0 - 100.0 / (1.0 + avg_gain / avg_loss);
    } else {
        out[length] = 100.0;
    }

    for i in (length + 1)..n {
        let d = closes[i] - closes[i - 1];
        let (g, l) = if d > 0.0 { (d, 0.0) } else { (0.0, -d) };
        avg_gain = (avg_gain * (length as f64 - 1.0) + g) / length as f64;
        avg_loss = (avg_loss * (length as f64 - 1.0) + l) / length as f64;
        out[i] = if avg_loss > 0.0 {
            100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
        } else {
            100.0
        };
    }
    out
}

impl Indicator for Rsi {
    fn name(&self) -> &str {
        "RSI"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let rsi = compute_rsi(&data.closes(), self.length);

        PanelResult {
            lines: vec![Line {
                y: rsi,
                color: rgba(0xc33ee1, 0.8),
                width: 2,
                label: Some("RSI".into()),
            }],
            hlines: vec![
                HLine {
                    y: self.overbought,
                    color: rgba(0xe13e3e, 0.3),
                },
                HLine {
                    y: self.oversold,
                    color: rgba(0x3ee145, 0.3),
                },
                HLine {
                    y: 50.0,
                    color: rgba(0x787b86, 0.2),
                },
            ],
            y_range: Some((0.0, 100.0)),
            label: "RSI".into(),
            ..Default::default()
        }
    }
}
