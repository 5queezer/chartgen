use crate::data::OhlcvData;
use crate::indicator::*;
use plotters::style::RGBAColor;
use super::wavetrend::WaveTrend;

/// Cipher B = WaveTrend + MFI bar (composite).
pub struct CipherB;

impl Indicator for CipherB {
    fn name(&self) -> &str { "Cipher B" }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let mut result = WaveTrend::default().compute(data);

        // Add MFI bar at bottom
        let n = data.len();
        let mfi_raw: Vec<f64> = data.bars.iter().map(|b| {
            let range = b.high - b.low;
            if range > 0.0 { ((b.close - b.open) / range) * 150.0 } else { 0.0 }
        }).collect();

        let mfi_smooth = sma(&mfi_raw, 60);
        let mfi_colors: Vec<RGBAColor> = mfi_smooth.iter().map(|v| {
            if v.is_nan() || *v > 0.0 { rgba(0x26a69a, 0.6) } else { rgba(0xef5350, 0.6) }
        }).collect();

        result.bars.push(Bars {
            y: vec![4.0; n],
            colors: mfi_colors,
            bottom: -95.0,
        });

        result.label = "Cipher B".into();
        result
    }
}
