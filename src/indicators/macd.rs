use crate::data::OhlcvData;
use crate::indicator::*;
use ta::indicators::MovingAverageConvergenceDivergence;
use ta::Next;

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

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let closes = data.closes();
        let n = closes.len();
        let mut macd_line = vec![f64::NAN; n];
        let mut sig = vec![f64::NAN; n];
        let mut hist = vec![f64::NAN; n];

        let mut ind =
            MovingAverageConvergenceDivergence::new(self.fast, self.slow, self.signal).unwrap();
        for (i, &c) in closes.iter().enumerate() {
            if c.is_nan() {
                continue;
            }
            let out = ind.next(c);
            macd_line[i] = out.macd;
            sig[i] = out.signal;
            hist[i] = out.histogram;
        }

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
