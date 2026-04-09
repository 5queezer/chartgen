pub mod cipher_b;
pub mod macd;
pub mod rsi;
pub mod wavetrend;

use crate::indicator::Indicator;

/// Get indicator by name.
pub fn by_name(name: &str) -> Option<Box<dyn Indicator>> {
    match name {
        "macd" => Some(Box::new(macd::Macd::default())),
        "wavetrend" => Some(Box::new(wavetrend::WaveTrend::default())),
        "rsi" => Some(Box::new(rsi::Rsi::default())),
        "cipher_b" => Some(Box::new(cipher_b::CipherB::default())),
        _ => None,
    }
}

pub fn available() -> &'static [&'static str] {
    &["macd", "wavetrend", "rsi", "cipher_b"]
}
