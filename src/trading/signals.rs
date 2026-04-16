//! Centralized signal label constants used by indicator-emitted dots.
//!
//! Indicators set `Dot.label = Some(LABEL.to_string())` on signal-bearing dots,
//! and alerts of type `IndicatorSignal { signal, .. }` match against the same
//! string. Defining each label as a `pub const &str` here gives compile-time
//! typo safety while keeping `String` at the rendering and MCP-API boundaries.
//!
//! When adding a new signal label, prefer reusing an existing const if the
//! semantic meaning is identical across indicators.

// ---------------------------------------------------------------------------
// shared (used by more than one indicator)
// ---------------------------------------------------------------------------

/// Generic buy signal (cipher_b WT crossover, custom flip-up cross).
pub const BUY: &str = "buy";
/// Generic sell signal (cipher_b WT crossunder, custom flip-down cross).
pub const SELL: &str = "sell";
/// RSI in oversold territory (cipher_b, rsi_combo).
pub const RSI_OVERSOLD: &str = "rsi_oversold";
/// RSI in overbought territory (cipher_b, rsi_combo).
pub const RSI_OVERBOUGHT: &str = "rsi_overbought";

// ---------------------------------------------------------------------------
// cipher_b
// ---------------------------------------------------------------------------

/// Smaller WT buy circle (lighter conviction).
pub const BUY_SMALL: &str = "buy_small";
/// Smaller WT sell circle (lighter conviction).
pub const SELL_SMALL: &str = "sell_small";
/// WaveTrend bullish divergence.
pub const WT_BULL_DIV: &str = "wt_bull_div";
/// WaveTrend bearish divergence.
pub const WT_BEAR_DIV: &str = "wt_bear_div";
/// RSI bullish divergence.
pub const RSI_BULL_DIV: &str = "rsi_bull_div";
/// RSI bearish divergence.
pub const RSI_BEAR_DIV: &str = "rsi_bear_div";
/// Stochastic bullish divergence.
pub const STOCH_BULL_DIV: &str = "stoch_bull_div";
/// Stochastic bearish divergence.
pub const STOCH_BEAR_DIV: &str = "stoch_bear_div";
/// Aggregated bullish divergence across detectors.
pub const ANY_BULL_DIV: &str = "any_bull_div";
/// Aggregated bearish divergence across detectors.
pub const ANY_BEAR_DIV: &str = "any_bear_div";
/// Cipher B "gold" buy confluence.
pub const GOLD_BUY: &str = "gold_buy";

// ---------------------------------------------------------------------------
// rsi_combo
// ---------------------------------------------------------------------------

/// Stochastic in oversold territory.
pub const STOCH_OVERSOLD: &str = "stoch_oversold";
/// Stochastic in overbought territory.
pub const STOCH_OVERBOUGHT: &str = "stoch_overbought";

// ---------------------------------------------------------------------------
// wavetrend
// ---------------------------------------------------------------------------

/// WT crossover while in oversold zone.
pub const CROSS_UP_OS: &str = "cross_up_os";
/// WT crossunder while in overbought zone.
pub const CROSS_DOWN_OB: &str = "cross_down_ob";
/// Plain WT crossover (outside OB/OS zones).
pub const CROSS_UP: &str = "cross_up";
/// Plain WT crossunder (outside OB/OS zones).
pub const CROSS_DOWN: &str = "cross_down";

// ---------------------------------------------------------------------------
// custom (supertrend / chandelier / heikin-ashi flips, momentum crosses)
// ---------------------------------------------------------------------------

/// Trend flip to up (supertrend, chandelier exit, heikin-ashi).
pub const FLIP_UP: &str = "flip_up";
/// Trend flip to down (supertrend, chandelier exit, heikin-ashi).
pub const FLIP_DOWN: &str = "flip_down";
/// Momentum crossover from oversold zone.
pub const BUY_OVERSOLD: &str = "buy_oversold";
/// Momentum crossunder from overbought zone.
pub const SELL_OVERBOUGHT: &str = "sell_overbought";

#[cfg(test)]
mod tests {
    use super::*;

    /// Canary against accidental desync between const name and value.
    /// Each const value should equal its name lowercased.
    #[test]
    fn const_values_match_const_names_lowercase() {
        // shared
        assert_eq!(BUY, "buy");
        assert_eq!(SELL, "sell");
        assert_eq!(RSI_OVERSOLD, "rsi_oversold");
        assert_eq!(RSI_OVERBOUGHT, "rsi_overbought");

        // cipher_b
        assert_eq!(BUY_SMALL, "buy_small");
        assert_eq!(SELL_SMALL, "sell_small");
        assert_eq!(WT_BULL_DIV, "wt_bull_div");
        assert_eq!(WT_BEAR_DIV, "wt_bear_div");
        assert_eq!(RSI_BULL_DIV, "rsi_bull_div");
        assert_eq!(RSI_BEAR_DIV, "rsi_bear_div");
        assert_eq!(STOCH_BULL_DIV, "stoch_bull_div");
        assert_eq!(STOCH_BEAR_DIV, "stoch_bear_div");
        assert_eq!(ANY_BULL_DIV, "any_bull_div");
        assert_eq!(ANY_BEAR_DIV, "any_bear_div");
        assert_eq!(GOLD_BUY, "gold_buy");

        // rsi_combo
        assert_eq!(STOCH_OVERSOLD, "stoch_oversold");
        assert_eq!(STOCH_OVERBOUGHT, "stoch_overbought");

        // wavetrend
        assert_eq!(CROSS_UP_OS, "cross_up_os");
        assert_eq!(CROSS_DOWN_OB, "cross_down_ob");
        assert_eq!(CROSS_UP, "cross_up");
        assert_eq!(CROSS_DOWN, "cross_down");

        // custom
        assert_eq!(FLIP_UP, "flip_up");
        assert_eq!(FLIP_DOWN, "flip_down");
        assert_eq!(BUY_OVERSOLD, "buy_oversold");
        assert_eq!(SELL_OVERBOUGHT, "sell_overbought");
    }
}
