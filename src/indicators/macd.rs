use crate::data::OhlcvData;
use crate::indicator::*;

pub struct Macd {
    pub fast: usize,
    pub slow: usize,
    pub signal: usize,
}

impl Default for Macd {
    fn default() -> Self { Self { fast: 12, slow: 26, signal: 9 } }
}

impl Indicator for Macd {
    fn name(&self) -> &str { "MACD" }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let closes = data.closes();
        let ema_f = ema(&closes, self.fast);
        let ema_s = ema(&closes, self.slow);

        let macd_line: Vec<f64> = ema_f.iter().zip(&ema_s)
            .map(|(f, s)| f - s).collect();
        let sig = ema(&macd_line, self.signal);
        let hist: Vec<f64> = macd_line.iter().zip(&sig)
            .map(|(m, s)| m - s).collect();

        let hist_colors: Vec<_> = hist.iter()
            .map(|v| if *v >= 0.0 { rgba(0x26a69a, 0.8) } else { rgba(0xef5350, 0.8) })
            .collect();

        PanelResult {
            lines: vec![
                Line { y: macd_line, color: rgba(0x2962ff, 1.0), width: 2, label: Some("MACD".into()) },
                Line { y: sig, color: rgba(0xff6d00, 1.0), width: 2, label: Some("Signal".into()) },
            ],
            bars: vec![Bars { y: hist, colors: hist_colors, bottom: 0.0 }],
            hlines: vec![HLine { y: 0.0, color: rgba(0x787b86, 0.5) }],
            label: "MACD".into(),
            ..Default::default()
        }
    }
}
