use crate::data::OhlcvData;
use crate::indicator::*;

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
            .map(|((s, e), d)| if *d > 0.0 { (s - e) / (0.015 * d) } else { 0.0 })
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
                });
            } else if cross_down && wt2[i] >= self.ob {
                dots.push(Dot {
                    x: i,
                    y: wt2[i],
                    color: rgba(0xff0000, 1.0),
                    size: 6,
                });
            } else if cross_up {
                dots.push(Dot {
                    x: i,
                    y: wt2[i],
                    color: rgba(0x00e676, 0.7),
                    size: 3,
                });
            } else if cross_down {
                dots.push(Dot {
                    x: i,
                    y: wt2[i],
                    color: rgba(0xff5252, 0.7),
                    size: 3,
                });
            }
        }

        let vwap: Vec<f64> = wt1.iter().zip(&wt2).map(|(a, b)| a - b).collect();

        PanelResult {
            lines: vec![
                Line {
                    y: wt1.clone(),
                    color: rgba(0x4994ec, 0.8),
                    width: 2,
                    label: None,
                },
                Line {
                    y: wt2.clone(),
                    color: rgba(0x1f1559, 0.8),
                    width: 2,
                    label: None,
                },
            ],
            fills: vec![
                Fill {
                    y1: wt1,
                    y2: wt2,
                    color: rgba(0x4994ec, 0.25),
                },
                Fill {
                    y1: vwap,
                    y2: vec![0.0; n],
                    color: rgba(0xffffff, 0.15),
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
