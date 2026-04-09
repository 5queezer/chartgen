pub mod cipher_b;
pub mod custom;
pub mod external;
pub mod macd;
pub mod rsi;
pub mod ta_bridge;
pub mod wavetrend;

use crate::indicator::Indicator;
use serde_json::Value;

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
        "vwap" => Some(Box::new(custom::Vwap)),
        "adx" => Some(Box::new(custom::Adx::default())),
        "supertrend" => Some(Box::new(custom::Supertrend::default())),
        "sar" | "parabolic_sar" => Some(Box::new(custom::ParabolicSar::default())),
        "ichimoku" => Some(Box::new(custom::Ichimoku::default())),
        "ad" | "ad_line" => Some(Box::new(custom::AdLine)),
        "histvol" | "hv" => Some(Box::new(custom::HistVol::default())),
        "vwap_bands" => Some(Box::new(custom::VwapBands::default())),
        "heikin_ashi" | "ha" => Some(Box::new(custom::HeikinAshi)),
        "pivot" | "pivot_points" => Some(Box::new(custom::PivotPoints)),
        "volume_profile" | "vp" => Some(Box::new(custom::VolumeProfile::default())),
        "kalman" | "kalman_volume" | "kvf" => Some(Box::new(custom::KalmanVolume::default())),
        "cvd" => Some(Box::new(external::Cvd)),
        "funding" | "funding_rate" => Some(Box::new(external::FundingRate)),
        "oi" | "open_interest" => Some(Box::new(external::OpenInterestHist)),
        "long_short" | "ls_ratio" => Some(Box::new(external::LongShortRatio)),
        "fear_greed" | "fng" => Some(Box::new(external::FearGreed)),
        _ => None,
    }
}

/// Create an indicator by name and apply configuration params.
pub fn by_name_configured(name: &str, params: &Value) -> Option<Box<dyn Indicator>> {
    let mut ind = by_name(name)?;
    ind.configure(params);
    Some(ind)
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
        "vwap",
        "adx",
        "supertrend",
        "sar",
        "ichimoku",
        "ad",
        "histvol",
        "vwap_bands",
        "heikin_ashi",
        "pivot",
        "volume_profile",
        "kalman_volume",
        "cvd",
        "funding",
        "oi",
        "long_short",
        "fear_greed",
    ]
}

/// Indicator metadata for tool discovery.
pub struct IndicatorInfo {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: String,
    pub is_overlay: bool,
    pub category: &'static str,
    pub params: Value,
}

/// Returns metadata for all registered indicators.
pub fn registry() -> Vec<IndicatorInfo> {
    let mut entries = Vec::new();

    macro_rules! reg {
        ($name:expr, $aliases:expr, $cat:expr, $ind:expr) => {{
            let ind: Box<dyn Indicator> = Box::new($ind);
            entries.push(IndicatorInfo {
                name: $name,
                aliases: $aliases,
                description: ind.description().to_string(),
                is_overlay: $cat == "overlay",
                category: $cat,
                params: ind.params(),
            });
        }};
    }

    // Overlays
    reg!("ema_stack", &["ema"], "overlay", ta_bridge::EmaStack);
    reg!(
        "bbands",
        &["bollinger"],
        "overlay",
        ta_bridge::BollingerBandsInd::default()
    );
    reg!(
        "keltner",
        &[],
        "overlay",
        ta_bridge::KeltnerChannels::default()
    );
    reg!("donchian", &[], "overlay", ta_bridge::Donchian::default());
    reg!("vwap", &[], "overlay", custom::Vwap);
    reg!("vwap_bands", &[], "overlay", custom::VwapBands::default());
    reg!("supertrend", &[], "overlay", custom::Supertrend::default());
    reg!(
        "sar",
        &["parabolic_sar"],
        "overlay",
        custom::ParabolicSar::default()
    );
    reg!("ichimoku", &[], "overlay", custom::Ichimoku::default());
    reg!("heikin_ashi", &["ha"], "overlay", custom::HeikinAshi);
    reg!("pivot", &["pivot_points"], "overlay", custom::PivotPoints);
    reg!(
        "volume_profile",
        &["vp"],
        "overlay",
        custom::VolumeProfile::default()
    );

    // Panels
    reg!("cipher_b", &[], "panel", cipher_b::CipherB::default());
    reg!("macd", &[], "panel", macd::Macd::default());
    reg!("rsi", &[], "panel", rsi::Rsi::default());
    reg!("wavetrend", &[], "panel", wavetrend::WaveTrend::default());
    reg!(
        "stoch",
        &["stochastic"],
        "panel",
        ta_bridge::Stochastic::default()
    );
    reg!("atr", &[], "panel", ta_bridge::Atr::default());
    reg!("obv", &[], "panel", ta_bridge::Obv);
    reg!("cci", &[], "panel", ta_bridge::Cci::default());
    reg!("roc", &[], "panel", ta_bridge::Roc::default());
    reg!("mfi", &[], "panel", ta_bridge::Mfi::default());
    reg!(
        "williams_r",
        &["willr"],
        "panel",
        ta_bridge::WilliamsR::default()
    );
    reg!("cmf", &[], "panel", ta_bridge::Cmf::default());
    reg!("adx", &[], "panel", custom::Adx::default());
    reg!("ad", &["ad_line"], "panel", custom::AdLine);
    reg!("histvol", &["hv"], "panel", custom::HistVol::default());
    reg!(
        "kalman_volume",
        &["kalman", "kvf"],
        "panel",
        custom::KalmanVolume::default()
    );

    // External API
    reg!("cvd", &[], "external", external::Cvd);
    reg!(
        "funding",
        &["funding_rate"],
        "external",
        external::FundingRate
    );
    reg!(
        "oi",
        &["open_interest"],
        "external",
        external::OpenInterestHist
    );
    reg!(
        "long_short",
        &["ls_ratio"],
        "external",
        external::LongShortRatio
    );
    reg!("fear_greed", &["fng"], "external", external::FearGreed);

    entries
}
