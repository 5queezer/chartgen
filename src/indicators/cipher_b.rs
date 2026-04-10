use super::rsi::compute_rsi;
use crate::data::OhlcvData;
use crate::indicator::*;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// All configurable parameters for Cipher B.
#[derive(Clone, Debug)]
pub struct CipherBConfig {
    // WaveTrend
    pub wt_show: bool,
    pub wt_buy_show: bool,
    pub wt_gold_show: bool,
    pub wt_sell_show: bool,
    pub wt_div_show: bool,
    pub vwap_show: bool,
    pub wt_channel_len: usize,
    pub wt_average_len: usize,
    pub wt_ma_len: usize,

    // OB/OS levels
    pub ob_level: f64,
    pub ob_level2: f64,
    pub ob_level3: f64,
    pub os_level: f64,
    pub os_level2: f64,
    pub os_level3: f64,

    // WT Divergence settings
    pub wt_show_div: bool,
    pub wt_show_hidden_div: bool,
    pub show_hidden_div_nl: bool,
    pub wt_div_ob_level: f64,
    pub wt_div_os_level: f64,
    pub wt_div_ob_level_add_show: bool,
    pub wt_div_ob_level_add: f64,
    pub wt_div_os_level_add: f64,

    // RSI+MFI
    pub rsi_mfi_show: bool,
    pub rsi_mfi_period: usize,
    pub rsi_mfi_multiplier: f64,
    pub rsi_mfi_pos_y: f64,

    // RSI
    pub rsi_show: bool,
    pub rsi_len: usize,
    pub rsi_oversold: f64,
    pub rsi_overbought: f64,
    pub rsi_show_div: bool,
    pub rsi_show_hidden_div: bool,
    pub rsi_div_ob_level: f64,
    pub rsi_div_os_level: f64,

    // Stochastic RSI
    pub stoch_show: bool,
    pub stoch_use_log: bool,
    pub stoch_avg: bool,
    pub stoch_len: usize,
    pub stoch_rsi_len: usize,
    pub stoch_k_smooth: usize,
    pub stoch_d_smooth: usize,
    pub stoch_show_div: bool,
    pub stoch_show_hidden_div: bool,

    // Dot mode
    pub dot_mode: String, // "classic" (existing behavior) or "strict" (TV-like filtering)

    // Schaff Trend Cycle
    pub tc_show: bool,
    pub tc_length: usize,
    pub tc_fast_length: usize,
    pub tc_slow_length: usize,
    pub tc_factor: f64,
}

impl Default for CipherBConfig {
    fn default() -> Self {
        Self {
            wt_show: true,
            wt_buy_show: true,
            wt_gold_show: true,
            wt_sell_show: true,
            wt_div_show: true,
            vwap_show: true,
            wt_channel_len: 9,
            wt_average_len: 12,
            wt_ma_len: 3,

            ob_level: 53.0,
            ob_level2: 60.0,
            ob_level3: 100.0,
            os_level: -53.0,
            os_level2: -60.0,
            os_level3: -75.0,

            wt_show_div: true,
            wt_show_hidden_div: false,
            show_hidden_div_nl: true,
            wt_div_ob_level: 45.0,
            wt_div_os_level: -65.0,
            wt_div_ob_level_add_show: true,
            wt_div_ob_level_add: 15.0,
            wt_div_os_level_add: -40.0,

            rsi_mfi_show: true,
            rsi_mfi_period: 60,
            rsi_mfi_multiplier: 150.0,
            rsi_mfi_pos_y: 2.5,

            rsi_show: true,
            rsi_len: 14,
            rsi_oversold: 30.0,
            rsi_overbought: 60.0,
            rsi_show_div: false,
            rsi_show_hidden_div: false,
            rsi_div_ob_level: 60.0,
            rsi_div_os_level: 30.0,

            stoch_show: true,
            stoch_use_log: true,
            stoch_avg: false,
            stoch_len: 14,
            stoch_rsi_len: 14,
            stoch_k_smooth: 3,
            stoch_d_smooth: 3,
            stoch_show_div: false,
            stoch_show_hidden_div: false,

            dot_mode: "strict".to_string(),

            tc_show: false,
            tc_length: 10,
            tc_fast_length: 23,
            tc_slow_length: 50,
            tc_factor: 0.5,
        }
    }
}

// ---------------------------------------------------------------------------
// Divergence detection
// ---------------------------------------------------------------------------

struct DivResult {
    _fractal_top: Vec<bool>,
    fractal_bot: Vec<bool>,
    bear_div: Vec<bool>,
    bull_div: Vec<bool>,
    bear_div_hidden: Vec<bool>,
    bull_div_hidden: Vec<bool>,
}

#[allow(clippy::too_many_arguments)]
fn find_divs(
    src: &[f64],
    highs: &[f64],
    lows: &[f64],
    top_limit: f64,
    bot_limit: f64,
    use_limits: bool,
    show_hidden: bool,
    hidden_no_limit: bool,
) -> DivResult {
    let n = src.len();
    let mut fractal_top = vec![false; n];
    let mut fractal_bot = vec![false; n];
    let mut bear_div = vec![false; n];
    let mut bull_div = vec![false; n];
    let mut bear_div_hidden = vec![false; n];
    let mut bull_div_hidden = vec![false; n];

    if n < 5 {
        return DivResult {
            _fractal_top: fractal_top,
            fractal_bot,
            bear_div,
            bull_div,
            bear_div_hidden,
            bull_div_hidden,
        };
    }

    // Detect fractals: center is at i-2, checked at bar i
    // f_top_fractal: src[i-4] < src[i-2] && src[i-3] < src[i-2] && src[i-2] > src[i-1] && src[i-2] > src[i]
    // f_bot_fractal: src[i-4] > src[i-2] && src[i-3] > src[i-2] && src[i-2] < src[i-1] && src[i-2] < src[i]
    for i in 4..n {
        if src[i].is_nan()
            || src[i - 1].is_nan()
            || src[i - 2].is_nan()
            || src[i - 3].is_nan()
            || src[i - 4].is_nan()
        {
            continue;
        }
        if src[i - 4] < src[i - 2]
            && src[i - 3] < src[i - 2]
            && src[i - 2] > src[i - 1]
            && src[i - 2] > src[i]
        {
            fractal_top[i] = true;
        }
        if src[i - 4] > src[i - 2]
            && src[i - 3] > src[i - 2]
            && src[i - 2] < src[i - 1]
            && src[i - 2] < src[i]
        {
            fractal_bot[i] = true;
        }
    }

    // Track previous fractal centers for comparison
    let mut prev_top_center: Option<usize> = None;
    let mut prev_bot_center: Option<usize> = None;

    for i in 4..n {
        let center = i - 2;

        if fractal_top[i] {
            if let Some(prev_c) = prev_top_center {
                let cur_src = src[center];
                let prev_src = src[prev_c];
                let cur_price = highs[center];
                let prev_price = highs[prev_c];

                // Regular bearish: price higher high, indicator lower high
                if (!use_limits || cur_src >= top_limit)
                    && cur_price > prev_price
                    && cur_src < prev_src
                {
                    bear_div[i] = true;
                }
                // Hidden bearish: price lower high, indicator higher high
                if show_hidden
                    && (hidden_no_limit || cur_src >= top_limit)
                    && cur_price < prev_price
                    && cur_src > prev_src
                {
                    bear_div_hidden[i] = true;
                }
            }
            prev_top_center = Some(center);
        }

        if fractal_bot[i] {
            if let Some(prev_c) = prev_bot_center {
                let cur_src = src[center];
                let prev_src = src[prev_c];
                let cur_price = lows[center];
                let prev_price = lows[prev_c];

                // Regular bullish: price lower low, indicator higher low
                if (!use_limits || cur_src <= bot_limit)
                    && cur_price < prev_price
                    && cur_src > prev_src
                {
                    bull_div[i] = true;
                }
                // Hidden bullish: price higher low, indicator lower low
                if show_hidden
                    && (hidden_no_limit || cur_src <= bot_limit)
                    && cur_price > prev_price
                    && cur_src < prev_src
                {
                    bull_div_hidden[i] = true;
                }
            }
            prev_bot_center = Some(center);
        }
    }

    DivResult {
        _fractal_top: fractal_top,
        fractal_bot,
        bear_div,
        bull_div,
        bear_div_hidden,
        bull_div_hidden,
    }
}

// ---------------------------------------------------------------------------
// Schaff Trend Cycle
// ---------------------------------------------------------------------------

fn compute_schaff(
    closes: &[f64],
    length: usize,
    fast_len: usize,
    slow_len: usize,
    factor: f64,
) -> Vec<f64> {
    let n = closes.len();
    let ema1 = ema(closes, fast_len);
    let ema2 = ema(closes, slow_len);
    let macd_val: Vec<f64> = ema1
        .iter()
        .zip(&ema2)
        .map(|(a, b)| {
            if a.is_nan() || b.is_nan() {
                f64::NAN
            } else {
                a - b
            }
        })
        .collect();

    let alpha = lowest(&macd_val, length);
    let beta_raw = highest(&macd_val, length);
    let beta: Vec<f64> = beta_raw
        .iter()
        .zip(&alpha)
        .map(|(h, l)| {
            if h.is_nan() || l.is_nan() {
                f64::NAN
            } else {
                h - l
            }
        })
        .collect();

    // gamma
    let mut gamma = vec![f64::NAN; n];
    for i in 0..n {
        if macd_val[i].is_nan() || alpha[i].is_nan() || beta[i].is_nan() {
            continue;
        }
        if beta[i] > 0.0 {
            gamma[i] = (macd_val[i] - alpha[i]) / beta[i] * 100.0;
        } else if i > 0 && !gamma[i - 1].is_nan() {
            gamma[i] = gamma[i - 1];
        }
    }

    // delta: recursive smooth
    let mut delta = vec![f64::NAN; n];
    let mut started = false;
    for i in 0..n {
        if gamma[i].is_nan() {
            continue;
        }
        if !started {
            delta[i] = gamma[i];
            started = true;
        } else {
            let prev = if delta[i - 1].is_nan() {
                gamma[i]
            } else {
                delta[i - 1]
            };
            delta[i] = prev + factor * (gamma[i] - prev);
        }
    }

    let epsilon = lowest(&delta, length);
    let zeta_raw = highest(&delta, length);
    let zeta: Vec<f64> = zeta_raw
        .iter()
        .zip(&epsilon)
        .map(|(h, l)| {
            if h.is_nan() || l.is_nan() {
                f64::NAN
            } else {
                h - l
            }
        })
        .collect();

    // eta
    let mut eta = vec![f64::NAN; n];
    for i in 0..n {
        if delta[i].is_nan() || epsilon[i].is_nan() || zeta[i].is_nan() {
            continue;
        }
        if zeta[i] > 0.0 {
            eta[i] = (delta[i] - epsilon[i]) / zeta[i] * 100.0;
        } else if i > 0 && !eta[i - 1].is_nan() {
            eta[i] = eta[i - 1];
        }
    }

    // stc: recursive smooth
    let mut stc = vec![f64::NAN; n];
    started = false;
    for i in 0..n {
        if eta[i].is_nan() {
            continue;
        }
        if !started {
            stc[i] = eta[i];
            started = true;
        } else {
            let prev = if stc[i - 1].is_nan() {
                eta[i]
            } else {
                stc[i - 1]
            };
            stc[i] = prev + factor * (eta[i] - prev);
        }
    }

    stc
}

// ---------------------------------------------------------------------------
// CipherB
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct CipherB {
    pub config: CipherBConfig,
}

impl Indicator for CipherB {
    fn name(&self) -> &str {
        "Cipher B"
    }

    fn description(&self) -> &str {
        "Market Cipher B — composite (WaveTrend + divergences + RSI + Stoch RSI + MFI)"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "dot_mode", "type": "string", "default": "strict", "description": "classic (WT cross only) or strict (with RSI+StochRSI filter)"},
            {"name": "wt_channel_length", "type": "integer", "default": 9},
            {"name": "wt_average_length", "type": "integer", "default": 12},
            {"name": "wt_oversold", "type": "number", "default": -53},
            {"name": "wt_overbought", "type": "number", "default": 53}
        ])
    }

    fn configure(&mut self, params: &Value) {
        if let Some(v) = params.get("dot_mode").and_then(|v| v.as_str()) {
            self.config.dot_mode = v.to_string();
        }
        if let Some(v) = params.get("wt_channel_length").and_then(|v| v.as_u64()) {
            self.config.wt_channel_len = v as usize;
        }
        if let Some(v) = params.get("wt_average_length").and_then(|v| v.as_u64()) {
            self.config.wt_average_len = v as usize;
        }
        if let Some(v) = params.get("wt_oversold").and_then(|v| v.as_f64()) {
            self.config.os_level = v;
        }
        if let Some(v) = params.get("wt_overbought").and_then(|v| v.as_f64()) {
            self.config.ob_level = v;
        }
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let cfg = &self.config;
        let n = data.len();
        if n < 5 {
            return PanelResult {
                label: "Cipher B".into(),
                y_range: Some((-105.0, 105.0)),
                ..Default::default()
            };
        }

        let closes = data.closes();
        let highs = data.highs();
        let lows = data.lows();

        let mut lines = Vec::new();
        let mut fills = Vec::new();
        let mut bars_out = Vec::new();
        let mut dots = Vec::new();
        let mut hlines = Vec::new();

        // ===================================================================
        // 1. WaveTrend
        // ===================================================================
        let hlc3: Vec<f64> = data
            .bars
            .iter()
            .map(|b| (b.high + b.low + b.close) / 3.0)
            .collect();

        let esa = ema(&hlc3, cfg.wt_channel_len);
        let diff: Vec<f64> = hlc3.iter().zip(&esa).map(|(s, e)| (s - e).abs()).collect();
        let de = ema(&diff, cfg.wt_channel_len);

        let ci: Vec<f64> = hlc3
            .iter()
            .zip(&esa)
            .zip(&de)
            .map(|((s, e), d)| if *d > 0.0 { (s - e) / (0.015 * d) } else { 0.0 })
            .collect();

        let wt1 = ema(&ci, cfg.wt_average_len);
        let wt2 = sma(&wt1, cfg.wt_ma_len);
        let vwap: Vec<f64> = wt1.iter().zip(&wt2).map(|(a, b)| a - b).collect();

        if cfg.wt_show {
            lines.push(Line {
                y: wt1.clone(),
                color: rgba(0x4994ec, 0.8),
                width: 2,
                label: Some("WT1".into()),
            });
            lines.push(Line {
                y: wt2.clone(),
                color: rgba(0x1f1559, 0.8),
                width: 2,
                label: Some("WT2".into()),
            });
            fills.push(Fill {
                y1: wt1.clone(),
                y2: wt2.clone(),
                color: rgba(0x4994ec, 0.25),
            });
        }
        if cfg.vwap_show {
            fills.push(Fill {
                y1: vwap.clone(),
                y2: vec![0.0; n],
                color: rgba(0xffffff, 0.15),
            });
        }

        // ===================================================================
        // 2. RSI
        // ===================================================================
        let rsi_vals = compute_rsi(&closes, cfg.rsi_len);
        if cfg.rsi_show {
            // RSI as line (base purple) + colored dots at OB/OS zones
            lines.push(Line {
                y: rsi_vals.clone(),
                color: rgba(0xc33ee1, 0.75),
                width: 2,
                label: Some("RSI".into()),
            });
            for (i, &rsi_v) in rsi_vals.iter().enumerate() {
                if rsi_v.is_nan() {
                    continue;
                }
                if rsi_v <= cfg.rsi_oversold {
                    dots.push(Dot {
                        x: i,
                        y: rsi_v,
                        color: rgba(0x3ee145, 0.8),
                        size: 2,
                    });
                } else if rsi_v >= cfg.rsi_overbought {
                    dots.push(Dot {
                        x: i,
                        y: rsi_v,
                        color: rgba(0xe13e3e, 0.8),
                        size: 2,
                    });
                }
            }
        }

        // ===================================================================
        // 3. MFI
        // ===================================================================
        if cfg.rsi_mfi_show {
            let mfi_raw: Vec<f64> = data
                .bars
                .iter()
                .map(|b| {
                    let range = b.high - b.low;
                    if range > 0.0 {
                        ((b.close - b.open) / range) * cfg.rsi_mfi_multiplier
                    } else {
                        0.0
                    }
                })
                .collect();

            let mfi_smooth = sma(&mfi_raw, cfg.rsi_mfi_period);
            let mfi_shifted: Vec<f64> = mfi_smooth
                .iter()
                .map(|v| {
                    if v.is_nan() {
                        f64::NAN
                    } else {
                        v - cfg.rsi_mfi_pos_y
                    }
                })
                .collect();

            let mfi_colors: Vec<plotters::style::RGBAColor> = mfi_shifted
                .iter()
                .map(|v| {
                    if v.is_nan() || *v > 0.0 {
                        rgba(0x3ee145, 0.25)
                    } else {
                        rgba(0xff3d2e, 0.25)
                    }
                })
                .collect();

            bars_out.push(Bars {
                y: vec![4.0; n],
                colors: mfi_colors,
                bottom: -95.0,
            });
        }

        // ===================================================================
        // 4. Stochastic RSI
        // ===================================================================
        let mut stoch_k = vec![f64::NAN; n];
        let mut stoch_d = vec![f64::NAN; n];
        let need_stoch = cfg.stoch_show
            || cfg.stoch_show_div
            || cfg.stoch_show_hidden_div
            || cfg.dot_mode == "strict";
        if need_stoch {
            let stoch_src: Vec<f64> = if cfg.stoch_use_log {
                closes.iter().map(|c| c.ln()).collect()
            } else {
                closes.clone()
            };
            let stoch_rsi = compute_rsi(&stoch_src, cfg.stoch_rsi_len);
            let k_raw = stoch(&stoch_rsi, &stoch_rsi, &stoch_rsi, cfg.stoch_len);
            let k_smoothed = sma(&k_raw, cfg.stoch_k_smooth);
            let d_smoothed = sma(&k_smoothed, cfg.stoch_d_smooth);

            if cfg.stoch_avg {
                for i in 0..n {
                    if !k_smoothed[i].is_nan() && !d_smoothed[i].is_nan() {
                        stoch_k[i] = (k_smoothed[i] + d_smoothed[i]) / 2.0;
                    } else {
                        stoch_k[i] = k_smoothed[i];
                    }
                    stoch_d[i] = d_smoothed[i];
                }
            } else {
                stoch_k = k_smoothed;
                stoch_d = d_smoothed;
            }

            if cfg.stoch_show {
                lines.push(Line {
                    y: stoch_k.clone(),
                    color: rgba(0x21baf3, 0.3),
                    width: 2,
                    label: Some("Stoch K".into()),
                });
                lines.push(Line {
                    y: stoch_d.clone(),
                    color: rgba(0x673ab7, 0.1),
                    width: 1,
                    label: Some("Stoch D".into()),
                });
                fills.push(Fill {
                    y1: stoch_k.clone(),
                    y2: stoch_d.clone(),
                    color: rgba(0x21baf3, 0.05),
                });
            }
        }

        // WT cross signals
        for i in 1..n {
            if wt1[i].is_nan() || wt2[i].is_nan() || wt1[i - 1].is_nan() || wt2[i - 1].is_nan() {
                continue;
            }
            let cross_up = wt1[i - 1] < wt2[i - 1] && wt1[i] >= wt2[i];
            let cross_down = wt1[i - 1] > wt2[i - 1] && wt1[i] <= wt2[i];

            let is_strict = cfg.dot_mode == "strict";

            if cross_up && cfg.wt_buy_show && wt2[i] <= cfg.os_level {
                if is_strict {
                    // Strict: require RSI oversold + Stoch RSI oversold
                    let rsi_ok = !rsi_vals[i].is_nan() && rsi_vals[i] < cfg.rsi_oversold;
                    let stoch_ok = !stoch_k[i].is_nan() && stoch_k[i] < 20.0;
                    if rsi_ok && stoch_ok {
                        dots.push(Dot {
                            x: i,
                            y: wt2[i],
                            color: rgba(0x3fff00, 1.0),
                            size: 6,
                        });
                    }
                } else {
                    dots.push(Dot {
                        x: i,
                        y: wt2[i],
                        color: rgba(0x3fff00, 1.0),
                        size: 6,
                    });
                }
            } else if cross_down && cfg.wt_sell_show && wt2[i] >= cfg.ob_level {
                if is_strict {
                    let rsi_ok = !rsi_vals[i].is_nan() && rsi_vals[i] > 70.0;
                    let stoch_ok = !stoch_k[i].is_nan() && stoch_k[i] > 80.0;
                    if rsi_ok && stoch_ok {
                        dots.push(Dot {
                            x: i,
                            y: wt2[i],
                            color: rgba(0xff0000, 1.0),
                            size: 6,
                        });
                    }
                } else {
                    dots.push(Dot {
                        x: i,
                        y: wt2[i],
                        color: rgba(0xff0000, 1.0),
                        size: 6,
                    });
                }
            } else if cross_up {
                if is_strict {
                    // Strict: only show small dot if RSI confirms
                    let rsi_ok = !rsi_vals[i].is_nan() && rsi_vals[i] < 40.0;
                    if rsi_ok {
                        dots.push(Dot {
                            x: i,
                            y: wt2[i],
                            color: rgba(0x00e676, 0.7),
                            size: 3,
                        });
                    }
                } else {
                    dots.push(Dot {
                        x: i,
                        y: wt2[i],
                        color: rgba(0x00e676, 0.7),
                        size: 3,
                    });
                }
            } else if cross_down {
                if is_strict {
                    let rsi_ok = !rsi_vals[i].is_nan() && rsi_vals[i] > 60.0;
                    if rsi_ok {
                        dots.push(Dot {
                            x: i,
                            y: wt2[i],
                            color: rgba(0xff5252, 0.7),
                            size: 3,
                        });
                    }
                } else {
                    dots.push(Dot {
                        x: i,
                        y: wt2[i],
                        color: rgba(0xff5252, 0.7),
                        size: 3,
                    });
                }
            }
        }

        // ===================================================================
        // 5. Schaff Trend Cycle
        // ===================================================================
        if cfg.tc_show {
            let stc = compute_schaff(
                &closes,
                cfg.tc_length,
                cfg.tc_fast_length,
                cfg.tc_slow_length,
                cfg.tc_factor,
            );
            lines.push(Line {
                y: stc,
                color: rgba(0x673ab7, 0.75),
                width: 2,
                label: Some("STC".into()),
            });
        }

        // ===================================================================
        // 6. Divergence detection
        // ===================================================================

        // WT divergences (primary)
        let mut wt_div_result: Option<DivResult> = None;
        if cfg.wt_show_div || cfg.wt_show_hidden_div {
            let dr = find_divs(
                &wt2,
                &highs,
                &lows,
                cfg.wt_div_ob_level,
                cfg.wt_div_os_level,
                true,
                cfg.wt_show_hidden_div,
                cfg.show_hidden_div_nl,
            );
            wt_div_result = Some(dr);
        }

        // WT divergences (2nd level, wider thresholds)
        let mut wt_div_add_result: Option<DivResult> = None;
        if cfg.wt_div_ob_level_add_show && (cfg.wt_show_div || cfg.wt_show_hidden_div) {
            let dr = find_divs(
                &wt2,
                &highs,
                &lows,
                cfg.wt_div_ob_level_add,
                cfg.wt_div_os_level_add,
                true,
                cfg.wt_show_hidden_div,
                cfg.show_hidden_div_nl,
            );
            wt_div_add_result = Some(dr);
        }

        // RSI divergences
        let mut rsi_div_result: Option<DivResult> = None;
        if cfg.rsi_show_div || cfg.rsi_show_hidden_div {
            let dr = find_divs(
                &rsi_vals,
                &highs,
                &lows,
                cfg.rsi_div_ob_level,
                cfg.rsi_div_os_level,
                true,
                cfg.rsi_show_hidden_div,
                cfg.show_hidden_div_nl,
            );
            rsi_div_result = Some(dr);
        }

        // Stoch divergences
        let mut stoch_div_result: Option<DivResult> = None;
        if cfg.stoch_show_div || cfg.stoch_show_hidden_div {
            let dr = find_divs(
                &stoch_k,
                &highs,
                &lows,
                cfg.wt_div_ob_level,
                cfg.wt_div_os_level,
                true,
                cfg.stoch_show_hidden_div,
                cfg.show_hidden_div_nl,
            );
            stoch_div_result = Some(dr);
        }

        // ===================================================================
        // 7. Signal dots
        // ===================================================================

        // Divergence dots
        for i in 4..n {
            let center = i - 2;
            let mut any_bull = false;
            let mut any_bear = false;

            // WT divergence markers (primary)
            if let Some(ref dr) = wt_div_result {
                if cfg.wt_show_div && dr.bull_div[i] {
                    dots.push(Dot {
                        x: center,
                        y: wt2[center],
                        color: rgba(0x00e676, 1.0),
                        size: 4,
                    });
                    any_bull = true;
                }
                if cfg.wt_show_div && dr.bear_div[i] {
                    dots.push(Dot {
                        x: center,
                        y: wt2[center],
                        color: rgba(0xe60000, 1.0),
                        size: 4,
                    });
                    any_bear = true;
                }
                if cfg.wt_show_hidden_div && dr.bull_div_hidden[i] {
                    any_bull = true;
                }
                if cfg.wt_show_hidden_div && dr.bear_div_hidden[i] {
                    any_bear = true;
                }
            }

            // WT divergence markers (2nd level — fainter color)
            if let Some(ref dr) = wt_div_add_result {
                if cfg.wt_show_div && dr.bull_div[i] {
                    dots.push(Dot {
                        x: center,
                        y: wt2[center],
                        color: rgba(0x00e676, 0.4),
                        size: 4,
                    });
                    any_bull = true;
                }
                if cfg.wt_show_div && dr.bear_div[i] {
                    dots.push(Dot {
                        x: center,
                        y: wt2[center],
                        color: rgba(0xe60000, 0.4),
                        size: 4,
                    });
                    any_bear = true;
                }
                if cfg.wt_show_hidden_div && dr.bull_div_hidden[i] {
                    any_bull = true;
                }
                if cfg.wt_show_hidden_div && dr.bear_div_hidden[i] {
                    any_bear = true;
                }
            }

            // RSI divergence markers
            if let Some(ref dr) = rsi_div_result {
                if cfg.rsi_show_div && dr.bull_div[i] {
                    if !rsi_vals[center].is_nan() {
                        dots.push(Dot {
                            x: center,
                            y: rsi_vals[center],
                            color: rgba(0x00e676, 1.0),
                            size: 4,
                        });
                    }
                    any_bull = true;
                }
                if cfg.rsi_show_div && dr.bear_div[i] {
                    if !rsi_vals[center].is_nan() {
                        dots.push(Dot {
                            x: center,
                            y: rsi_vals[center],
                            color: rgba(0xe60000, 1.0),
                            size: 4,
                        });
                    }
                    any_bear = true;
                }
                if cfg.rsi_show_hidden_div && dr.bull_div_hidden[i] {
                    any_bull = true;
                }
                if cfg.rsi_show_hidden_div && dr.bear_div_hidden[i] {
                    any_bear = true;
                }
            }

            // Stoch divergence markers
            if let Some(ref dr) = stoch_div_result {
                if cfg.stoch_show_div && dr.bull_div[i] {
                    if !stoch_k[center].is_nan() {
                        dots.push(Dot {
                            x: center,
                            y: stoch_k[center],
                            color: rgba(0x00e676, 1.0),
                            size: 4,
                        });
                    }
                    any_bull = true;
                }
                if cfg.stoch_show_div && dr.bear_div[i] {
                    if !stoch_k[center].is_nan() {
                        dots.push(Dot {
                            x: center,
                            y: stoch_k[center],
                            color: rgba(0xe60000, 1.0),
                            size: 4,
                        });
                    }
                    any_bear = true;
                }
                if cfg.stoch_show_hidden_div && dr.bull_div_hidden[i] {
                    any_bull = true;
                }
                if cfg.stoch_show_hidden_div && dr.bear_div_hidden[i] {
                    any_bear = true;
                }
            }

            // Combined divergence buy/sell dots (gated by wt_div_show)
            if cfg.wt_div_show {
                if any_bull {
                    dots.push(Dot {
                        x: center,
                        y: -95.0,
                        color: rgba(0x00e676, 1.0),
                        size: 5,
                    });
                }
                if any_bear {
                    dots.push(Dot {
                        x: center,
                        y: 95.0,
                        color: rgba(0xff0000, 1.0),
                        size: 5,
                    });
                }
            }
        }

        // Gold buy signal: complex condition
        if cfg.wt_gold_show {
            if let Some(ref wt_dr) = wt_div_result {
                // Find the most recent WT bot fractal value for gold buy detection
                let mut prev_wt_bot_val: Option<f64> = None;
                for i in 4..n {
                    if wt_dr.fractal_bot[i] {
                        let center = i - 2;
                        let cur_wt_bot_val = wt2[center];

                        // Check gold buy conditions:
                        // - wt_bull_div (or rsi_bull_div) triggered
                        // - previous WT fractal bot value <= os_level3
                        // - wt2 > os_level3
                        // - (prev_wt_low - wt2) <= -5
                        // - last RSI at fractal < 30
                        let has_wt_bull = cfg.wt_show_div && wt_dr.bull_div[i];
                        let has_rsi_bull = rsi_div_result
                            .as_ref()
                            .is_some_and(|rd| cfg.rsi_show_div && rd.bull_div[i]);

                        if has_wt_bull || has_rsi_bull {
                            if let Some(prev_val) = prev_wt_bot_val {
                                let rsi_at_center = rsi_vals[center];
                                if prev_val <= cfg.os_level3
                                    && wt2[i] > cfg.os_level3
                                    && (prev_val - wt2[i]) <= -5.0
                                    && !rsi_at_center.is_nan()
                                    && rsi_at_center < 30.0
                                {
                                    dots.push(Dot {
                                        x: i,
                                        y: wt2[i],
                                        color: rgba(0xe2a400, 1.0),
                                        size: 8,
                                    });
                                }
                            }
                        }

                        prev_wt_bot_val = Some(cur_wt_bot_val);
                    }
                }
            }
        }

        // ===================================================================
        // 8. HLines
        // ===================================================================
        hlines.push(HLine {
            y: cfg.ob_level2,
            color: rgba(0xffffff, 0.15),
        });
        hlines.push(HLine {
            y: cfg.ob_level3,
            color: rgba(0xffffff, 0.05),
        });
        hlines.push(HLine {
            y: cfg.os_level2,
            color: rgba(0xffffff, 0.15),
        });
        hlines.push(HLine {
            y: 0.0,
            color: rgba(0x787b86, 0.3),
        });

        PanelResult {
            lines,
            fills,
            bars: bars_out,
            dots,
            hlines,
            y_range: Some((-105.0, 105.0)),
            label: "Cipher B".into(),
            ..Default::default()
        }
    }
}
