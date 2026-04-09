use crate::data::{Bar, OhlcvData};
use crate::indicator::*;
use ta::indicators::{
    AverageTrueRange as TaAtr, BollingerBands as TaBB, CommodityChannelIndex as TaCci,
    ExponentialMovingAverage, KeltnerChannel as TaKC, MoneyFlowIndex as TaMfi,
    OnBalanceVolume as TaObv, RateOfChange as TaRoc, SlowStochastic as TaSlow,
};
use ta::{DataItem, Next};

fn make_data_item(bar: &Bar) -> DataItem {
    DataItem::builder()
        .open(bar.open)
        .high(bar.high)
        .low(bar.low)
        .close(bar.close)
        .volume(bar.volume)
        .build()
        .unwrap()
}

// ---- EMA Stack (overlay) ----

pub struct EmaStack;

impl Indicator for EmaStack {
    fn name(&self) -> &str {
        "EMA Stack"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let closes = data.closes();
        let ema8 = ema(&closes, 8);
        let ema21 = ema(&closes, 21);
        let ema50 = ema(&closes, 50);
        let ema200 = ema(&closes, 200);

        PanelResult {
            lines: vec![
                Line {
                    y: ema8,
                    color: rgba(0xFFEB3B, 0.9),
                    width: 1,
                    label: Some("EMA 8".into()),
                },
                Line {
                    y: ema21,
                    color: rgba(0xFF9800, 0.9),
                    width: 1,
                    label: Some("EMA 21".into()),
                },
                Line {
                    y: ema50,
                    color: rgba(0x2196F3, 0.9),
                    width: 1,
                    label: Some("EMA 50".into()),
                },
                Line {
                    y: ema200,
                    color: rgba(0xF44336, 0.9),
                    width: 2,
                    label: Some("EMA 200".into()),
                },
            ],
            is_overlay: true,
            label: "EMA".into(),
            ..Default::default()
        }
    }
}

// ---- Bollinger Bands (overlay) ----

pub struct BollingerBandsInd {
    pub period: usize,
    pub std_dev: f64,
}

impl Default for BollingerBandsInd {
    fn default() -> Self {
        Self {
            period: 20,
            std_dev: 2.0,
        }
    }
}

impl Indicator for BollingerBandsInd {
    fn name(&self) -> &str {
        "Bollinger Bands"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let closes = data.closes();
        let n = closes.len();
        let mut upper = vec![f64::NAN; n];
        let mut middle = vec![f64::NAN; n];
        let mut lower = vec![f64::NAN; n];

        let mut ind = TaBB::new(self.period, self.std_dev).unwrap();
        for (i, &c) in closes.iter().enumerate() {
            if c.is_nan() {
                continue;
            }
            let out = ind.next(c);
            upper[i] = out.upper;
            middle[i] = out.average;
            lower[i] = out.lower;
        }

        PanelResult {
            lines: vec![
                Line {
                    y: upper.clone(),
                    color: rgba(0x787B86, 0.7),
                    width: 1,
                    label: Some("BB Upper".into()),
                },
                Line {
                    y: middle,
                    color: rgba(0x2196F3, 0.8),
                    width: 1,
                    label: Some("BB Mid".into()),
                },
                Line {
                    y: lower.clone(),
                    color: rgba(0x787B86, 0.7),
                    width: 1,
                    label: Some("BB Lower".into()),
                },
            ],
            fills: vec![Fill {
                y1: upper,
                y2: lower,
                color: rgba(0x2196F3, 0.1),
            }],
            is_overlay: true,
            label: "BB".into(),
            ..Default::default()
        }
    }
}

// ---- Keltner Channels (overlay) ----

pub struct KeltnerChannels {
    pub period: usize,
    pub multiplier: f64,
}

impl Default for KeltnerChannels {
    fn default() -> Self {
        Self {
            period: 20,
            multiplier: 1.5,
        }
    }
}

impl Indicator for KeltnerChannels {
    fn name(&self) -> &str {
        "Keltner Channels"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut upper = vec![f64::NAN; n];
        let mut middle = vec![f64::NAN; n];
        let mut lower = vec![f64::NAN; n];

        let mut ind = TaKC::new(self.period, self.multiplier).unwrap();
        for (i, bar) in data.bars.iter().enumerate() {
            let di = make_data_item(bar);
            let out = ind.next(&di);
            upper[i] = out.upper;
            middle[i] = out.average;
            lower[i] = out.lower;
        }

        PanelResult {
            lines: vec![
                Line {
                    y: upper.clone(),
                    color: rgba(0x787B86, 0.7),
                    width: 1,
                    label: Some("KC Upper".into()),
                },
                Line {
                    y: middle,
                    color: rgba(0x2196F3, 0.8),
                    width: 1,
                    label: Some("KC Mid".into()),
                },
                Line {
                    y: lower.clone(),
                    color: rgba(0x787B86, 0.7),
                    width: 1,
                    label: Some("KC Lower".into()),
                },
            ],
            fills: vec![Fill {
                y1: upper,
                y2: lower,
                color: rgba(0x2196F3, 0.1),
            }],
            is_overlay: true,
            label: "KC".into(),
            ..Default::default()
        }
    }
}

// ---- ATR panel ----

pub struct Atr {
    pub period: usize,
}

impl Default for Atr {
    fn default() -> Self {
        Self { period: 14 }
    }
}

impl Indicator for Atr {
    fn name(&self) -> &str {
        "ATR"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut vals = vec![f64::NAN; n];

        let mut ind = TaAtr::new(self.period).unwrap();
        for (i, bar) in data.bars.iter().enumerate() {
            let di = make_data_item(bar);
            vals[i] = ind.next(&di);
        }

        PanelResult {
            lines: vec![Line {
                y: vals,
                color: rgba(0xFF9800, 0.9),
                width: 2,
                label: Some("ATR".into()),
            }],
            label: "ATR".into(),
            ..Default::default()
        }
    }
}

// ---- OBV panel ----

pub struct Obv;

impl Indicator for Obv {
    fn name(&self) -> &str {
        "OBV"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut vals = vec![f64::NAN; n];

        let mut ind = TaObv::new();
        for (i, bar) in data.bars.iter().enumerate() {
            let di = make_data_item(bar);
            vals[i] = ind.next(&di);
        }

        PanelResult {
            lines: vec![Line {
                y: vals,
                color: rgba(0x2196F3, 0.9),
                width: 2,
                label: Some("OBV".into()),
            }],
            label: "OBV".into(),
            ..Default::default()
        }
    }
}

// ---- CCI panel ----

pub struct Cci {
    pub period: usize,
}

impl Default for Cci {
    fn default() -> Self {
        Self { period: 20 }
    }
}

impl Indicator for Cci {
    fn name(&self) -> &str {
        "CCI"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut vals = vec![f64::NAN; n];

        let mut ind = TaCci::new(self.period).unwrap();
        for (i, bar) in data.bars.iter().enumerate() {
            let di = make_data_item(bar);
            vals[i] = ind.next(&di);
        }

        PanelResult {
            lines: vec![Line {
                y: vals,
                color: rgba(0x2196F3, 0.9),
                width: 2,
                label: Some("CCI".into()),
            }],
            hlines: vec![
                HLine {
                    y: 100.0,
                    color: rgba(0x787B86, 0.5),
                },
                HLine {
                    y: -100.0,
                    color: rgba(0x787B86, 0.5),
                },
            ],
            y_range: Some((-250.0, 250.0)),
            label: "CCI".into(),
            ..Default::default()
        }
    }
}

// ---- ROC panel ----

pub struct Roc {
    pub period: usize,
}

impl Default for Roc {
    fn default() -> Self {
        Self { period: 14 }
    }
}

impl Indicator for Roc {
    fn name(&self) -> &str {
        "ROC"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let closes = data.closes();
        let n = closes.len();
        let mut vals = vec![f64::NAN; n];

        let mut ind = TaRoc::new(self.period).unwrap();
        for (i, &c) in closes.iter().enumerate() {
            if c.is_nan() {
                continue;
            }
            vals[i] = ind.next(c);
        }

        PanelResult {
            lines: vec![Line {
                y: vals,
                color: rgba(0x2196F3, 0.9),
                width: 2,
                label: Some("ROC".into()),
            }],
            hlines: vec![HLine {
                y: 0.0,
                color: rgba(0x787B86, 0.5),
            }],
            label: "ROC".into(),
            ..Default::default()
        }
    }
}

// ---- MFI panel ----

pub struct Mfi {
    pub period: usize,
}

impl Default for Mfi {
    fn default() -> Self {
        Self { period: 14 }
    }
}

impl Indicator for Mfi {
    fn name(&self) -> &str {
        "MFI"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut vals = vec![f64::NAN; n];

        let mut ind = TaMfi::new(self.period).unwrap();
        for (i, bar) in data.bars.iter().enumerate() {
            let di = make_data_item(bar);
            vals[i] = ind.next(&di);
        }

        PanelResult {
            lines: vec![Line {
                y: vals,
                color: rgba(0x2196F3, 0.9),
                width: 2,
                label: Some("MFI".into()),
            }],
            hlines: vec![
                HLine {
                    y: 80.0,
                    color: rgba(0xe13e3e, 0.3),
                },
                HLine {
                    y: 20.0,
                    color: rgba(0x3ee145, 0.3),
                },
            ],
            y_range: Some((0.0, 100.0)),
            label: "MFI".into(),
            ..Default::default()
        }
    }
}

// ---- Slow Stochastic panel ----

pub struct Stochastic {
    pub period: usize,
    pub k_smooth: usize,
    pub d_smooth: usize,
}

impl Default for Stochastic {
    fn default() -> Self {
        Self {
            period: 14,
            k_smooth: 3,
            d_smooth: 3,
        }
    }
}

impl Indicator for Stochastic {
    fn name(&self) -> &str {
        "Stochastic"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();
        let mut k_vals = vec![f64::NAN; n];
        let mut d_vals = vec![f64::NAN; n];

        // SlowStochastic(period, k_smooth) gives %K.
        // %D is an EMA of %K with d_smooth period.
        let mut k_ind = TaSlow::new(self.period, self.k_smooth).unwrap();
        let mut d_ema = ExponentialMovingAverage::new(self.d_smooth).unwrap();

        for (i, bar) in data.bars.iter().enumerate() {
            let di = make_data_item(bar);
            let k = k_ind.next(&di);
            let d = d_ema.next(k);
            k_vals[i] = k;
            d_vals[i] = d;
        }

        PanelResult {
            lines: vec![
                Line {
                    y: k_vals.clone(),
                    color: rgba(0x2196F3, 0.9),
                    width: 2,
                    label: Some("%K".into()),
                },
                Line {
                    y: d_vals.clone(),
                    color: rgba(0xFF9800, 0.9),
                    width: 2,
                    label: Some("%D".into()),
                },
            ],
            fills: vec![Fill {
                y1: k_vals,
                y2: d_vals,
                color: rgba(0x2196F3, 0.1),
            }],
            hlines: vec![
                HLine {
                    y: 80.0,
                    color: rgba(0xe13e3e, 0.3),
                },
                HLine {
                    y: 20.0,
                    color: rgba(0x3ee145, 0.3),
                },
            ],
            y_range: Some((0.0, 100.0)),
            label: "Stoch".into(),
            ..Default::default()
        }
    }
}

// ---- Williams %R panel (custom) ----

pub struct WilliamsR {
    pub period: usize,
}

impl Default for WilliamsR {
    fn default() -> Self {
        Self { period: 14 }
    }
}

impl Indicator for WilliamsR {
    fn name(&self) -> &str {
        "Williams %R"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let closes = data.closes();
        let highs = data.highs();
        let lows = data.lows();
        let n = closes.len();

        let hh = highest(&highs, self.period);
        let ll = lowest(&lows, self.period);

        let mut vals = vec![f64::NAN; n];
        for i in 0..n {
            if hh[i].is_nan() || ll[i].is_nan() || closes[i].is_nan() {
                continue;
            }
            let range = hh[i] - ll[i];
            vals[i] = if range > 0.0 {
                (hh[i] - closes[i]) / range * -100.0
            } else {
                0.0
            };
        }

        PanelResult {
            lines: vec![Line {
                y: vals,
                color: rgba(0x2196F3, 0.9),
                width: 2,
                label: Some("%R".into()),
            }],
            hlines: vec![
                HLine {
                    y: -20.0,
                    color: rgba(0xe13e3e, 0.3),
                },
                HLine {
                    y: -80.0,
                    color: rgba(0x3ee145, 0.3),
                },
            ],
            y_range: Some((-100.0, 0.0)),
            label: "W%R".into(),
            ..Default::default()
        }
    }
}

// ---- Donchian Channels (overlay, custom) ----

pub struct Donchian {
    pub period: usize,
}

impl Default for Donchian {
    fn default() -> Self {
        Self { period: 20 }
    }
}

impl Indicator for Donchian {
    fn name(&self) -> &str {
        "Donchian"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let highs = data.highs();
        let lows = data.lows();
        let n = data.len();

        let upper = highest(&highs, self.period);
        let lower = lowest(&lows, self.period);

        let mut middle = vec![f64::NAN; n];
        for i in 0..n {
            if !upper[i].is_nan() && !lower[i].is_nan() {
                middle[i] = (upper[i] + lower[i]) / 2.0;
            }
        }

        PanelResult {
            lines: vec![
                Line {
                    y: upper.clone(),
                    color: rgba(0x787B86, 0.7),
                    width: 1,
                    label: Some("DC Upper".into()),
                },
                Line {
                    y: middle,
                    color: rgba(0x2196F3, 0.8),
                    width: 1,
                    label: Some("DC Mid".into()),
                },
                Line {
                    y: lower.clone(),
                    color: rgba(0x787B86, 0.7),
                    width: 1,
                    label: Some("DC Lower".into()),
                },
            ],
            fills: vec![Fill {
                y1: upper,
                y2: lower,
                color: rgba(0x2196F3, 0.1),
            }],
            is_overlay: true,
            label: "DC".into(),
            ..Default::default()
        }
    }
}

// ---- CMF panel (custom) ----

pub struct Cmf {
    pub period: usize,
}

impl Default for Cmf {
    fn default() -> Self {
        Self { period: 20 }
    }
}

impl Indicator for Cmf {
    fn name(&self) -> &str {
        "CMF"
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let n = data.len();

        // Money Flow Multiplier: ((close - low) - (high - close)) / (high - low)
        // Money Flow Volume: multiplier * volume
        let mut mfv = vec![f64::NAN; n];
        let mut vol = vec![f64::NAN; n];

        for (i, bar) in data.bars.iter().enumerate() {
            let range = bar.high - bar.low;
            if range > 0.0 {
                let mult = ((bar.close - bar.low) - (bar.high - bar.close)) / range;
                mfv[i] = mult * bar.volume;
            } else {
                mfv[i] = 0.0;
            }
            vol[i] = bar.volume;
        }

        // CMF = SMA(mfv, period) / SMA(vol, period)
        let mfv_sma = sma(&mfv, self.period);
        let vol_sma = sma(&vol, self.period);

        let mut vals = vec![f64::NAN; n];
        for i in 0..n {
            if !mfv_sma[i].is_nan() && !vol_sma[i].is_nan() && vol_sma[i] != 0.0 {
                vals[i] = mfv_sma[i] / vol_sma[i];
            }
        }

        PanelResult {
            lines: vec![Line {
                y: vals,
                color: rgba(0x2196F3, 0.9),
                width: 2,
                label: Some("CMF".into()),
            }],
            hlines: vec![HLine {
                y: 0.0,
                color: rgba(0x787B86, 0.5),
            }],
            y_range: Some((-1.0, 1.0)),
            label: "CMF".into(),
            ..Default::default()
        }
    }
}
