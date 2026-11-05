use crate::data::OhlcvData;
use crate::indicator::*;
use serde_json::{json, Value};

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

/// Pine `ta.rsi`: Wilder RMA of gains/losses, seeded with their SMA.
pub fn compute_rsi(closes: &[f64], length: usize) -> Vec<f64> {
    super::pine::rsi(closes, length)
}

impl Indicator for Rsi {
    fn name(&self) -> &str {
        "RSI"
    }

    fn description(&self) -> &str {
        "Relative Strength Index — momentum oscillator (0-100)"
    }

    fn params(&self) -> Value {
        json!([{"name": "length", "type": "integer", "min": 1, "max": 10000, "default": 14, "description": "RSI period"}])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("length").and_then(|v| v.as_u64()) {
            self.length = v as usize;
        }
    }

    fn warmup_bars(&self) -> usize {
        // ta.rma of the changes, which start a bar in.
        super::pine::ma_warmup("rma", self.length) + 1
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicators::pine::test_util::{closes, eq, NAN};

    #[test]
    fn rsi14_wilder_rma_seeding() {
        // Wilder's / StockCharts' RSI(14) sample. The first 14 changes hold
        // gains of 3.34 and losses of 1.40, so the RMA seeds are 3.34/14 and
        // 0.1: RSI = 100 - 100 / (1 + 2.3857) = 70.464 at bar 14. Bar 15
        // (change -0.28): gain = 0.23857 * 13/14, loss = (1.3 + 0.28) / 14.
        let c = [
            44.34, 44.09, 44.15, 43.61, 44.33, 44.83, 45.10, 45.42, 45.84, 46.08, 45.89, 46.03,
            45.61, 46.28, 46.28, 46.00, 46.03, 46.41,
        ];
        let r = Rsi::default().compute(&closes(&c));
        let mut want = vec![NAN; 14];
        want.extend([
            100.0 - 100.0 / (1.0 + 3.34 / 1.40),
            100.0 - 100.0 / (1.0 + (3.34 / 14.0 * 13.0 / 14.0) / (1.58 / 14.0)),
            66.48094183471265,
            69.34685316290866,
        ]);
        eq(&r.lines[0].y, &want);
    }
}
