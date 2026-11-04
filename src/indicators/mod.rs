pub mod cipher_b;
pub mod custom;
pub mod external;
pub mod macd;
pub mod oscillators;
pub mod rsi;
pub mod rsi_combo;
pub mod ta_bridge;
pub mod trend;
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
        "volume_profile" | "vp" | "vpvr" => Some(Box::new(custom::VolumeProfile::default())),
        "session_vp" | "svp" | "session_volume_profile" => {
            Some(Box::new(custom::SessionVolumeProfile::default()))
        }
        "hvn_lvn" | "vp_nodes" | "volume_nodes" => Some(Box::new(custom::HvnLvn::default())),
        "naked_poc" | "npoc" => Some(Box::new(custom::NakedPoc::default())),
        "tpo" | "market_profile" => Some(Box::new(custom::Tpo::default())),
        "kalman" | "kalman_volume" | "kvf" => Some(Box::new(custom::KalmanVolume::default())),
        "rsi_mfi_stoch" | "rsi_combo" | "combo" => {
            Some(Box::new(rsi_combo::RsiMfiStochCombo::default()))
        }
        "cvd" => Some(Box::new(external::Cvd)),
        "funding" | "funding_rate" => Some(Box::new(external::FundingRate)),
        "oi" | "open_interest" => Some(Box::new(external::OpenInterestHist)),
        "long_short" | "ls_ratio" => Some(Box::new(external::LongShortRatio)),
        "fear_greed" | "fng" => Some(Box::new(external::FearGreed)),
        "volume" => Some(Box::new(trend::Volume::default())),
        "ma" => Some(Box::new(trend::Ma::default())),
        "vwma" => Some(Box::new(trend::Ma::fixed("vwma"))),
        "hma" => Some(Box::new(trend::Ma::fixed("hma"))),
        "ma_ribbon" => Some(Box::new(trend::MaRibbon::default())),
        "envelopes" => Some(Box::new(trend::Envelopes::default())),
        "linreg_channel" => Some(Box::new(trend::LinRegChannel::default())),
        "aroon" => Some(Box::new(trend::Aroon::default())),
        "vortex" => Some(Box::new(trend::Vortex::default())),
        "chande_kroll_stop" => Some(Box::new(trend::ChandeKrollStop::default())),
        "zigzag" => Some(Box::new(trend::ZigZag::default())),
        n if oscillators::Kind::from_name(n).is_some() => {
            oscillators::by_name(n).map(|o| Box::new(o) as Box<dyn Indicator>)
        }
        _ => None,
    }
}

/// Check `params` against the indicator's declared schema (`Indicator::params`).
/// Only declared params are inspected; `null` means "use the default".
/// Integers must be whole numbers, floats must be finite, and both must respect
/// the optional `min`/`max` of the schema entry.
pub fn validate_params(name: &str, ind: &dyn Indicator, params: &Value) -> Result<(), String> {
    let Some(obj) = params.as_object() else {
        return Ok(());
    };
    let schema = ind.params();
    for spec in schema.as_array().into_iter().flatten() {
        let Some(pname) = spec.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(v) = obj.get(pname).filter(|v| !v.is_null()) else {
            continue;
        };
        let bad = |why: &str| Err(format!("Invalid parameter for {name}: {pname} {why}"));
        let ty = spec.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if let (Some(allowed), Some(s)) = (spec.get("enum").and_then(|v| v.as_array()), v.as_str())
        {
            if !allowed.iter().any(|a| a.as_str() == Some(s)) {
                let list: Vec<&str> = allowed.iter().filter_map(|a| a.as_str()).collect();
                return bad(&format!("must be one of: {}", list.join(", ")));
            }
        }
        let num = match ty {
            "integer" => match (v.as_i64(), v.as_u64()) {
                (Some(i), _) => i as f64,
                (None, Some(u)) => u as f64,
                _ => return bad("must be an integer"),
            },
            "number" => match v.as_f64() {
                Some(f) if f.is_finite() => f,
                Some(_) => return bad("must be a finite number"),
                None => return bad("must be a number"),
            },
            "boolean" if !v.is_boolean() => return bad("must be a boolean"),
            "string" if !v.is_string() => return bad("must be a string"),
            _ => continue,
        };
        if let Some(min) = spec.get("min").and_then(|v| v.as_f64()) {
            if num < min {
                return bad(&format!("must be >= {min}"));
            }
        }
        if let Some(max) = spec.get("max").and_then(|v| v.as_f64()) {
            if num > max {
                return bad(&format!("must be <= {max}"));
            }
        }
    }
    Ok(())
}

/// Create an indicator by name and apply configuration params.
/// `Ok(None)` = unknown indicator; `Err` = a param failed validation.
pub fn by_name_configured(
    name: &str,
    params: &Value,
) -> Result<Option<Box<dyn Indicator>>, String> {
    let Some(mut ind) = by_name(name) else {
        return Ok(None);
    };
    validate_params(name, ind.as_ref(), params)?;
    ind.configure(params);
    Ok(Some(ind))
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
        "session_vp",
        "hvn_lvn",
        "naked_poc",
        "tpo",
        "kalman_volume",
        "rsi_mfi_stoch",
        "cvd",
        "funding",
        "oi",
        "long_short",
        "fear_greed",
        "volume",
        "ma",
        "vwma",
        "hma",
        "ma_ribbon",
        "envelopes",
        "linreg_channel",
        "aroon",
        "vortex",
        "chande_kroll_stop",
        "zigzag",
        "stoch_rsi",
        "awesome_oscillator",
        "momentum",
        "trix",
        "ultimate_oscillator",
        "ppo",
        "cmo",
        "dpo",
        "elder_force_index",
        "tsi",
        "kst",
        "coppock",
        "mass_index",
        "bb_percent_b",
        "bb_width",
        "choppiness",
        "fisher_transform",
        "ease_of_movement",
        "volume_oscillator",
        "balance_of_power",
        "rvi",
        "smi_ergodic",
        "connors_rsi",
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
        &["vp", "vpvr"],
        "overlay",
        custom::VolumeProfile::default()
    );
    reg!(
        "session_vp",
        &["svp", "session_volume_profile"],
        "overlay",
        custom::SessionVolumeProfile::default()
    );
    reg!(
        "hvn_lvn",
        &["vp_nodes", "volume_nodes"],
        "overlay",
        custom::HvnLvn::default()
    );
    reg!(
        "naked_poc",
        &["npoc"],
        "overlay",
        custom::NakedPoc::default()
    );
    reg!(
        "tpo",
        &["market_profile"],
        "overlay",
        custom::Tpo::default()
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
    reg!(
        "rsi_mfi_stoch",
        &["rsi_combo", "combo"],
        "panel",
        rsi_combo::RsiMfiStochCombo::default()
    );

    // Trend / moving averages / volume
    reg!("ma", &[], "overlay", trend::Ma::default());
    reg!("vwma", &[], "overlay", trend::Ma::fixed("vwma"));
    reg!("hma", &[], "overlay", trend::Ma::fixed("hma"));
    reg!("ma_ribbon", &[], "overlay", trend::MaRibbon::default());
    reg!("envelopes", &[], "overlay", trend::Envelopes::default());
    reg!(
        "linreg_channel",
        &[],
        "overlay",
        trend::LinRegChannel::default()
    );
    reg!(
        "chande_kroll_stop",
        &[],
        "overlay",
        trend::ChandeKrollStop::default()
    );
    reg!("zigzag", &[], "overlay", trend::ZigZag::default());
    reg!("volume", &[], "panel", trend::Volume::default());
    reg!("aroon", &[], "panel", trend::Aroon::default());
    reg!("vortex", &[], "panel", trend::Vortex::default());
    // TradingView-parity oscillators
    for k in oscillators::ALL {
        reg!(k.name(), k.aliases(), "panel", oscillators::Osc::new(k));
    }

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
