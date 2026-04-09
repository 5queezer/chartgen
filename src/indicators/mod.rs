pub mod cipher_b;
pub mod macd;
pub mod rsi;
pub mod ta_bridge;
pub mod wavetrend;

use crate::indicator::Indicator;

/// Get indicator by name.
pub fn by_name(name: &str) -> Option<Box<dyn Indicator>> {
    match name {
        "macd" => Some(Box::new(macd::Macd::default())),
        "wavetrend" => Some(Box::new(wavetrend::WaveTrend::default())),
        "rsi" => Some(Box::new(rsi::Rsi::default())),
        "cipher_b" => Some(Box::new(cipher_b::CipherB::default())),
        "ema_stack" | "ema" => Some(Box::new(ta_bridge::EmaStack)),
        "bbands" | "bollinger" => Some(Box::new(ta_bridge::BollingerBandsInd::default())),
        "keltner" => Some(Box::new(ta_bridge::KeltnerChannels::default())),
        "atr" => Some(Box::new(ta_bridge::Atr::default())),
        "obv" => Some(Box::new(ta_bridge::Obv)),
        "cci" => Some(Box::new(ta_bridge::Cci::default())),
        "roc" => Some(Box::new(ta_bridge::Roc::default())),
        "mfi" => Some(Box::new(ta_bridge::Mfi::default())),
        "stoch" | "stochastic" => Some(Box::new(ta_bridge::Stochastic::default())),
        "williams_r" | "willr" => Some(Box::new(ta_bridge::WilliamsR::default())),
        "donchian" => Some(Box::new(ta_bridge::Donchian::default())),
        "cmf" => Some(Box::new(ta_bridge::Cmf::default())),
        _ => None,
    }
}

pub fn available() -> &'static [&'static str] {
    &[
        "macd",
        "wavetrend",
        "rsi",
        "cipher_b",
        "ema_stack",
        "bbands",
        "keltner",
        "atr",
        "obv",
        "cci",
        "roc",
        "mfi",
        "stoch",
        "williams_r",
        "donchian",
        "cmf",
    ]
}
