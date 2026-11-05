use crate::data::OhlcvData;
use crate::indicator::*;
use crate::trading::signals;
use serde_json::{json, Value};

pub struct WaveTrend {
    pub ch_len: usize,
    pub avg_len: usize,
    pub ma_len: usize,
    pub ob: f64,
    pub os: f64,
}

impl Default for WaveTrend {
    fn default() -> Self {
        Self {
            ch_len: 9,
            avg_len: 12,
            ma_len: 3,
            ob: 53.0,
            os: -53.0,
        }
    }
}

impl Indicator for WaveTrend {
    fn name(&self) -> &str {
        "WaveTrend"
    }

    fn description(&self) -> &str {
        "WaveTrend Oscillator — cycle-based momentum"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "ch_len", "type": "integer", "min": 1, "max": 10000, "default": 9},
            {"name": "avg_len", "type": "integer", "min": 1, "max": 10000, "default": 12}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("ch_len").and_then(|v| v.as_u64()) {
            self.ch_len = v as usize;
        }
        if let Some(v) = params.get("avg_len").and_then(|v| v.as_u64()) {
            self.avg_len = v as usize;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let hlc3: Vec<f64> = data
            .bars
            .iter()
            .map(|b| (b.high + b.low + b.close) / 3.0)
            .collect();

        let esa = ema(&hlc3, self.ch_len);
        let diff: Vec<f64> = hlc3.iter().zip(&esa).map(|(s, e)| (s - e).abs()).collect();
        let de = ema(&diff, self.ch_len);

        let ci: Vec<f64> = hlc3
            .iter()
            .zip(&esa)
            .zip(&de)
            // Pine: (ap - esa) / (0.015 * d); na or a zero d gives na.
            .map(|((s, e), d)| {
                if *d != 0.0 {
                    (s - e) / (0.015 * d)
                } else {
                    f64::NAN
                }
            })
            .collect();

        let wt1 = ema(&ci, self.avg_len);
        let wt2 = sma(&wt1, self.ma_len);

        // Signals
        let mut dots = Vec::new();
        for i in 1..n {
            if wt1[i].is_nan() || wt2[i].is_nan() || wt1[i - 1].is_nan() || wt2[i - 1].is_nan() {
                continue;
            }
            let cross_up = wt1[i - 1] < wt2[i - 1] && wt1[i] >= wt2[i];
            let cross_down = wt1[i - 1] > wt2[i - 1] && wt1[i] <= wt2[i];

            if cross_up && wt2[i] <= self.os {
                dots.push(Dot {
                    x: i,
                    y: wt2[i],
                    color: rgba(0x3fff00, 1.0),
                    size: 6,
                    label: Some(signals::CROSS_UP_OS.to_string()),
                });
            } else if cross_down && wt2[i] >= self.ob {
                dots.push(Dot {
                    x: i,
                    y: wt2[i],
                    color: rgba(0xff0000, 1.0),
                    size: 6,
                    label: Some(signals::CROSS_DOWN_OB.to_string()),
                });
            } else if cross_up {
                dots.push(Dot {
                    x: i,
                    y: wt2[i],
                    color: rgba(0x00e676, 0.7),
                    size: 3,
                    label: Some(signals::CROSS_UP.to_string()),
                });
            } else if cross_down {
                dots.push(Dot {
                    x: i,
                    y: wt2[i],
                    color: rgba(0xff5252, 0.7),
                    size: 3,
                    label: Some(signals::CROSS_DOWN.to_string()),
                });
            }
        }

        let vwap: Vec<f64> = wt1.iter().zip(&wt2).map(|(a, b)| a - b).collect();

        PanelResult {
            lines: vec![
                Line {
                    breaks: false,
                    y: wt1.clone(),
                    color: rgba(0x4994ec, 0.8),
                    width: 2,
                    label: Some("WT1".into()),
                },
                Line {
                    breaks: false,
                    y: wt2.clone(),
                    color: rgba(0x1f1559, 0.8),
                    width: 2,
                    label: Some("WT2".into()),
                },
            ],
            fills: vec![
                Fill {
                    y1: wt1,
                    y2: wt2,
                    color: rgba(0x4994ec, 0.25),
                    color_below: None,
                },
                Fill {
                    y1: vwap,
                    y2: vec![0.0; n],
                    color: rgba(0xffffff, 0.15),
                    color_below: None,
                },
            ],
            dots,
            hlines: vec![
                HLine {
                    y: self.ob,
                    color: rgba(0xffffff, 0.15),
                },
                HLine {
                    y: self.os,
                    color: rgba(0xffffff, 0.15),
                },
                HLine {
                    y: 0.0,
                    color: rgba(0x787b86, 0.3),
                },
            ],
            y_range: Some((-105.0, 105.0)),
            label: "WaveTrend".into(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicators::pine::test_util::{closes, eq, NAN};

    #[test]
    fn wavetrend_pine_semantics() {
        let ind = WaveTrend {
            ch_len: 2,
            avg_len: 2,
            ma_len: 2,
            ..Default::default()
        };
        // hlc3 = close. esa = ema2 = [_, 1.5, 19/6, 55/18, ...];
        // d = ema2(|ap - esa|) = [_, _, 2/3, 7/27, ...];
        // ci = [_, _, 250/3, -100/7, ...]; wt1 = ema2(ci) seeds at bar 3.
        let r = ind.compute(&closes(&[1.0, 2.0, 4.0, 3.0, 5.0, 4.0, 6.0]));
        let wt1_3 = (250.0 / 3.0 - 100.0 / 7.0) / 2.0;
        eq(&r.lines[0].y[..4], &[NAN, NAN, NAN, wt1_3]);
        assert!((r.lines[0].y[4] - 67.06349206349209).abs() < 1e-9);
        // wt2 = sma2(wt1)
        eq(
            &r.lines[1].y[..5],
            &[NAN, NAN, NAN, NAN, (wt1_3 + 67.06349206349209) / 2.0],
        );
    }

    #[test]
    fn wavetrend_flat_prices_are_nan_not_zero() {
        // d = 0 -> ci is a division by zero (na), so nothing is plotted.
        let r = WaveTrend::default().compute(&closes(&[5.0; 40]));
        assert!(r.lines[0].y.iter().all(|v| v.is_nan()));
        assert!(r.dots.is_empty());
    }
}
