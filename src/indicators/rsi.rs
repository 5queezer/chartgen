use crate::data::OhlcvData;
use crate::indicator::*;
use ta::indicators::RelativeStrengthIndex;
use ta::Next;

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
    let mut rsi = RelativeStrengthIndex::new(length).unwrap();
    for (i, &c) in closes.iter().enumerate() {
        if c.is_nan() {
            continue;
        }
        out[i] = rsi.next(c);
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
