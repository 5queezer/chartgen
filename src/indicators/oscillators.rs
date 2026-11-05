//! TradingView-parity oscillators (Pine Script v5 semantics).
//!
//! Conventions mirrored from Pine:
//! - `ta.sma` / `math.sum` / `ta.stdev` (population) are NaN until a full
//!   window of valid values exists; any NaN in the window yields NaN.
//! - `ta.ema` / `ta.rma` seed with the SMA of the first `len` valid values,
//!   then recurse with alpha = 2/(len+1) / 1/len.
//! - `ta.stoch` divides by (highest - lowest); a zero range yields NaN.
//! - `ta.rsi` returns 100 when the average loss is 0 and 0 when the average
//!   gain is 0.
//!
//! Warmup values are NaN (serialized as null), never 0.

use super::pine::{
    change_n, div, ema_n, finite_or_nan, gains_losses, highest, hist, line, lowest, ma_warmup,
    nan_max, nan_min, percentrank, roc, rsi, sma_n, stdev, stoch, sum_n, swma, true_range, wma_n,
    zip,
};
use crate::data::OhlcvData;
use crate::indicator::{rgba, Bars, HLine, Indicator, PanelResult};
use serde_json::{json, Value};

/// Connors RSI streak: consecutive up (+n) / down (-n) closes, 0 when equal.
/// Mirrors the TV script, including bar 0 (`s[1]` is na, so it counts as down).
fn updown(s: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; s.len()];
    for i in 0..s.len() {
        let prev = if i > 0 { out[i - 1] } else { 0.0 };
        let (eq, up) = if i > 0 {
            (s[i] == s[i - 1], s[i] > s[i - 1])
        } else {
            (false, false)
        };
        out[i] = if eq {
            0.0
        } else if up {
            if prev <= 0.0 {
                1.0
            } else {
                prev + 1.0
            }
        } else if prev >= 0.0 {
            -1.0
        } else {
            prev - 1.0
        };
    }
    out
}
/// Pine `ta.tsi` ratio: ema(ema(pc, long), short) / ema(ema(|pc|, long), short).
fn tsi_ratio(c: &[f64], short: usize, long: usize) -> Vec<f64> {
    let pc = change_n(c, 1);
    let ab: Vec<f64> = pc.iter().map(|v| v.abs()).collect();
    div(
        &ema_n(&ema_n(&pc, long), short),
        &ema_n(&ema_n(&ab, long), short),
    )
}

// ---- Indicator definitions ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    StochRsi,
    AwesomeOscillator,
    Momentum,
    Trix,
    UltimateOscillator,
    Ppo,
    Cmo,
    Dpo,
    ElderForceIndex,
    Tsi,
    Kst,
    Coppock,
    MassIndex,
    BbPercentB,
    BbWidth,
    Choppiness,
    FisherTransform,
    EaseOfMovement,
    VolumeOscillator,
    BalanceOfPower,
    Rvi,
    SmiErgodic,
    ConnorsRsi,
}

pub const ALL: [Kind; 23] = [
    Kind::StochRsi,
    Kind::AwesomeOscillator,
    Kind::Momentum,
    Kind::Trix,
    Kind::UltimateOscillator,
    Kind::Ppo,
    Kind::Cmo,
    Kind::Dpo,
    Kind::ElderForceIndex,
    Kind::Tsi,
    Kind::Kst,
    Kind::Coppock,
    Kind::MassIndex,
    Kind::BbPercentB,
    Kind::BbWidth,
    Kind::Choppiness,
    Kind::FisherTransform,
    Kind::EaseOfMovement,
    Kind::VolumeOscillator,
    Kind::BalanceOfPower,
    Kind::Rvi,
    Kind::SmiErgodic,
    Kind::ConnorsRsi,
];

pub const NAMES: [&str; 23] = [
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
];

/// (name, default, min, max, description). All params are integers except `mult`.
type P = (&'static str, f64, f64, f64, &'static str);
const L: f64 = 10000.0;

impl Kind {
    pub fn from_name(n: &str) -> Option<Kind> {
        NAMES
            .iter()
            .position(|&x| x == n)
            .or_else(|| ALL.iter().position(|k| k.aliases().contains(&n)))
            .map(|i| ALL[i])
    }

    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Kind::StochRsi => &["stochrsi"],
            Kind::AwesomeOscillator => &["ao"],
            Kind::Momentum => &["mom"],
            Kind::UltimateOscillator => &["uo"],
            Kind::Ppo => &["price_oscillator"],
            Kind::ElderForceIndex => &["efi"],
            Kind::Coppock => &["coppock_curve"],
            Kind::MassIndex => &["mi"],
            Kind::BbPercentB => &["percent_b", "bbp"],
            Kind::BbWidth => &["bbw", "bandwidth"],
            Kind::Choppiness => &["chop"],
            Kind::FisherTransform => &["fisher"],
            Kind::EaseOfMovement => &["eom"],
            Kind::VolumeOscillator => &["vol_osc"],
            Kind::BalanceOfPower => &["bop"],
            Kind::Rvi => &["relative_vigor_index"],
            Kind::SmiErgodic => &["smi"],
            Kind::ConnorsRsi => &["crsi", "connors"],
            _ => &[],
        }
    }

    pub fn name(self) -> &'static str {
        NAMES[ALL.iter().position(|&k| k == self).unwrap()]
    }

    fn title(self) -> &'static str {
        match self {
            Kind::StochRsi => "Stoch RSI",
            Kind::AwesomeOscillator => "AO",
            Kind::Momentum => "Momentum",
            Kind::Trix => "TRIX",
            Kind::UltimateOscillator => "UO",
            Kind::Ppo => "PPO",
            Kind::Cmo => "CMO",
            Kind::Dpo => "DPO",
            Kind::ElderForceIndex => "EFI",
            Kind::Tsi => "TSI",
            Kind::Kst => "KST",
            Kind::Coppock => "Coppock",
            Kind::MassIndex => "Mass Index",
            Kind::BbPercentB => "BB %B",
            Kind::BbWidth => "BBW",
            Kind::Choppiness => "CHOP",
            Kind::FisherTransform => "Fisher",
            Kind::EaseOfMovement => "EOM",
            Kind::VolumeOscillator => "Vol Osc",
            Kind::BalanceOfPower => "BOP",
            Kind::Rvi => "RVGI",
            Kind::SmiErgodic => "SMI Ergodic",
            Kind::ConnorsRsi => "CRSI",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Kind::StochRsi => {
                "Stochastic RSI (TradingView): ta.stoch of RSI, smoothed K and D (0-100)"
            }
            Kind::AwesomeOscillator => {
                "Awesome Oscillator: SMA(hl2,5) - SMA(hl2,34) histogram, green when rising"
            }
            Kind::Momentum => "Momentum: close - close[length]",
            Kind::Trix => "TRIX: 10000 * change of triple-smoothed EMA of log(close)",
            Kind::UltimateOscillator => {
                "Ultimate Oscillator (Larry Williams): weighted 7/14/28 buying-pressure ratios (0-100)"
            }
            Kind::Ppo => {
                "Price Oscillator (TradingView): (short MA - long MA) / long MA * 100, MA type ema or sma"
            }
            Kind::Cmo => "Chande Momentum Oscillator (-100 to 100)",
            Kind::Dpo => "Detrended Price Oscillator: close - SMA shifted back length/2+1 bars",
            Kind::ElderForceIndex => "Elder Force Index: EMA of change(close) * volume",
            Kind::Tsi => "True Strength Index: double-smoothed momentum ratio (x100) with signal",
            Kind::Kst => {
                "Know Sure Thing: weighted sum of four smoothed rates of change, with signal"
            }
            Kind::Coppock => "Coppock Curve: WMA of ROC(long) + ROC(short)",
            Kind::MassIndex => {
                "Mass Index: rolling sum of EMA(high-low,9) / EMA(EMA(high-low,9),9)"
            }
            Kind::BbPercentB => "Bollinger %B: (close - lower) / (upper - lower)",
            Kind::BbWidth => "Bollinger Bands Width: 100 * (upper - lower) / basis, with Highest Expansion and Lowest Contraction",
            Kind::Choppiness => "Choppiness Index (0-100): high = sideways, low = trending",
            Kind::FisherTransform => "Fisher Transform of hl2 with trigger line",
            Kind::EaseOfMovement => {
                "Ease of Movement: SMA of 10000 * change(hl2) * (high-low) / volume"
            }
            Kind::VolumeOscillator => {
                "Volume Oscillator: 100 * (EMA fast vol - EMA slow vol) / EMA slow vol"
            }
            Kind::BalanceOfPower => "Balance of Power: (close - open) / (high - low)",
            Kind::Rvi => {
                "Relative Vigor Index: SWMA(close-open) / SWMA(high-low) sums, with signal"
            }
            Kind::SmiErgodic => {
                "SMI Ergodic Indicator: ta.tsi ratio with signal and oscillator histogram"
            }
            Kind::ConnorsRsi => {
                "Connors RSI: average of RSI, streak RSI and percent rank of ROC(1) (0-100)"
            }
        }
    }

    fn spec(self) -> &'static [P] {
        match self {
            Kind::StochRsi => &[
                ("smooth_k", 3.0, 1.0, L, "%K smoothing (SMA)"),
                ("smooth_d", 3.0, 1.0, L, "%D smoothing (SMA of %K)"),
                ("rsi_length", 14.0, 1.0, L, "RSI period"),
                (
                    "stoch_length",
                    14.0,
                    1.0,
                    L,
                    "Stochastic lookback applied to RSI",
                ),
            ],
            Kind::AwesomeOscillator => &[
                ("fast", 5.0, 1.0, L, "Fast SMA of hl2"),
                ("slow", 34.0, 1.0, L, "Slow SMA of hl2"),
            ],
            Kind::Momentum => &[("length", 10.0, 1.0, L, "Bars back")],
            Kind::Trix => &[("length", 18.0, 1.0, L, "EMA length (applied three times)")],
            Kind::UltimateOscillator => &[
                ("fast", 7.0, 1.0, L, "Fast length"),
                ("middle", 14.0, 1.0, L, "Middle length"),
                ("slow", 28.0, 1.0, L, "Slow length"),
            ],
            Kind::Ppo => &[
                ("short", 10.0, 1.0, L, "Short moving-average length"),
                ("long", 21.0, 1.0, L, "Long moving-average length"),
            ],
            Kind::Cmo => &[("length", 9.0, 1.0, L, "Lookback")],
            Kind::Dpo => &[("length", 21.0, 1.0, L, "SMA length")],
            Kind::ElderForceIndex => &[("length", 13.0, 1.0, L, "EMA length")],
            Kind::Tsi => &[
                ("long", 25.0, 1.0, L, "Long (first) smoothing"),
                ("short", 13.0, 1.0, L, "Short (second) smoothing"),
                ("signal", 13.0, 1.0, L, "Signal EMA length"),
            ],
            Kind::Kst => &[
                ("roc1", 10.0, 1.0, L, "ROC length 1"),
                ("roc2", 15.0, 1.0, L, "ROC length 2"),
                ("roc3", 20.0, 1.0, L, "ROC length 3"),
                ("roc4", 30.0, 1.0, L, "ROC length 4"),
                ("sma1", 10.0, 1.0, L, "SMA length 1"),
                ("sma2", 10.0, 1.0, L, "SMA length 2"),
                ("sma3", 10.0, 1.0, L, "SMA length 3"),
                ("sma4", 15.0, 1.0, L, "SMA length 4"),
                ("signal", 9.0, 1.0, L, "Signal SMA length"),
            ],
            Kind::Coppock => &[
                ("wma_length", 10.0, 1.0, L, "WMA length"),
                ("long_roc", 14.0, 1.0, L, "Long ROC length"),
                ("short_roc", 11.0, 1.0, L, "Short ROC length"),
            ],
            Kind::MassIndex => &[("length", 10.0, 1.0, L, "Summation length")],
            Kind::BbPercentB => &[
                ("length", 20.0, 1.0, L, "SMA / stdev length"),
                ("mult", 2.0, 0.001, 50.0, "Standard deviation multiplier"),
            ],
            Kind::BbWidth => &[
                ("length", 20.0, 1.0, L, "SMA / stdev length"),
                ("mult", 2.0, 0.001, 50.0, "Standard deviation multiplier"),
                (
                    "highest_expansion",
                    125.0,
                    1.0,
                    L,
                    "Highest Expansion lookback",
                ),
                (
                    "lowest_contraction",
                    125.0,
                    1.0,
                    L,
                    "Lowest Contraction lookback",
                ),
            ],
            Kind::Choppiness => &[("length", 14.0, 2.0, L, "Lookback")],
            Kind::FisherTransform => &[("length", 9.0, 1.0, L, "Highest/lowest lookback of hl2")],
            Kind::EaseOfMovement => &[
                ("length", 14.0, 1.0, L, "SMA length"),
                ("divisor", 10000.0, 1.0, 1.0e9, "Scaling divisor"),
            ],
            Kind::VolumeOscillator => &[
                ("short", 5.0, 1.0, L, "Short volume EMA"),
                ("long", 10.0, 1.0, L, "Long volume EMA"),
            ],
            Kind::BalanceOfPower => &[],
            Kind::Rvi => &[("length", 10.0, 1.0, L, "Summation length")],
            Kind::SmiErgodic => &[
                ("long", 20.0, 1.0, L, "Long (first) smoothing"),
                ("short", 5.0, 1.0, L, "Short (second) smoothing"),
                ("signal", 5.0, 1.0, L, "Signal EMA length"),
            ],
            Kind::ConnorsRsi => &[
                ("rsi_length", 3.0, 1.0, L, "RSI length"),
                ("updown_length", 2.0, 1.0, L, "Streak RSI length"),
                ("roc_length", 100.0, 1.0, L, "Percent-rank lookback"),
            ],
        }
    }
}

pub struct Osc {
    pub kind: Kind,
    vals: Vec<f64>,
    /// Moving-average type for the Price Oscillator (`ema` | `sma`).
    ma_type: String,
}

impl Osc {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            vals: kind.spec().iter().map(|p| p.1).collect(),
            ma_type: "ema".into(),
        }
    }
    fn u(&self, i: usize) -> usize {
        self.vals[i] as usize
    }
}

fn hl(y: f64, alpha: f64) -> HLine {
    HLine {
        y,
        color: rgba(0x787b86, alpha),
    }
}
const BLUE: u32 = 0x2962ff;
const ORANGE: u32 = 0xff6d00;
const PURPLE: u32 = 0x7e57c2;

impl Indicator for Osc {
    fn name(&self) -> &str {
        self.kind.title()
    }

    fn description(&self) -> &str {
        self.kind.description()
    }

    fn params(&self) -> Value {
        let mut v: Vec<Value> = self
            .kind
            .spec()
            .iter()
                .map(|&(name, def, min, max, desc)| {
                    if name == "mult" {
                        json!({"name": name, "type": "number", "default": def, "min": min, "max": max, "description": desc})
                    } else {
                        json!({"name": name, "type": "integer", "default": def as i64, "min": min as i64, "max": max as i64, "description": desc})
                    }
                })
            .collect();
        if self.kind == Kind::Ppo {
            v.push(
                json!({"name": "type", "type": "string", "default": "ema", "enum": ["ema", "sma"],
                          "description": "Moving average type"}),
            );
        }
        Value::Array(v)
    }

    fn configure(&mut self, params: &Value) {
        if let Some(t) = params.get("type").and_then(|v| v.as_str()) {
            if self.kind == Kind::Ppo && (t == "ema" || t == "sma") {
                self.ma_type = t.into();
            }
        }
        for (i, &(name, _, min, max, _)) in self.kind.spec().iter().enumerate() {
            if let Some(v) = params.get(name).and_then(|v| v.as_f64()) {
                if v.is_finite() {
                    self.vals[i] = v.clamp(min, max);
                }
            }
        }
    }

    fn warmup_bars(&self) -> usize {
        let u = |i| self.u(i);
        let ema = |n| ma_warmup("ema", n);
        // Chained smoothings add up; a change or difference adds a bar.
        match self.kind {
            Kind::StochRsi => ma_warmup("rma", u(2)) + 1 + u(3) + u(0) + u(1),
            Kind::Trix => 3 * ema(u(0)) + 1,
            Kind::Ppo if self.ma_type == "ema" => ema(u(0).max(u(1))),
            Kind::ElderForceIndex => ema(u(0)) + 1,
            Kind::Tsi | Kind::SmiErgodic => ema(u(0)) + ema(u(1)) + ema(u(2)) + 1,
            Kind::VolumeOscillator => ema(u(0).max(u(1))),
            Kind::ConnorsRsi => (ma_warmup("rma", u(0).max(u(1))) + 1).max(u(2) + 1),
            _ => 0,
        }
    }

    fn compute(&self, d: &OhlcvData) -> PanelResult {
        let c = d.closes();
        let (h, l) = (d.highs(), d.lows());
        let hl2: Vec<f64> = zip(&h, &l, |a, b| (a + b) / 2.0);
        let vol: Vec<f64> = d.bars.iter().map(|b| b.volume).collect();
        let u = |i| self.u(i);
        let mut r = PanelResult {
            label: self.kind.title().into(),
            ..Default::default()
        };
        match self.kind {
            Kind::StochRsi => {
                let rsi = rsi(&c, u(2));
                let k = sma_n(&stoch(&rsi, &rsi, &rsi, u(3)), u(0));
                let dd = sma_n(&k, u(1));
                r.lines = vec![line(k, BLUE, 2, "%K"), line(dd, ORANGE, 2, "%D")];
                r.hlines = vec![hl(80.0, 0.4), hl(50.0, 0.2), hl(20.0, 0.4)];
                r.y_range = Some((0.0, 100.0));
            }
            Kind::AwesomeOscillator => {
                let ao = zip(&sma_n(&hl2, u(0)), &sma_n(&hl2, u(1)), |a, b| a - b);
                // TV: red when ao - ao[1] <= 0, green otherwise (na -> green).
                let colors = (0..ao.len())
                    .map(|i| {
                        if i > 0 && ao[i] - ao[i - 1] <= 0.0 {
                            rgba(0xf23645, 0.9)
                        } else {
                            rgba(0x089981, 0.9)
                        }
                    })
                    .collect();
                r.bars = vec![Bars {
                    y: ao,
                    colors,
                    bottom: 0.0,
                }];
                r.hlines = vec![hl(0.0, 0.5)];
            }
            Kind::Momentum => {
                r.lines = vec![line(change_n(&c, u(0)), BLUE, 2, "MOM")];
                r.hlines = vec![hl(0.0, 0.5)];
            }
            Kind::Trix => {
                let ln: Vec<f64> = c
                    .iter()
                    .map(|&v| if v > 0.0 { v.ln() } else { f64::NAN })
                    .collect();
                let e = ema_n(&ema_n(&ema_n(&ln, u(0)), u(0)), u(0));
                let y = change_n(&e, 1).iter().map(|v| v * 10000.0).collect();
                r.lines = vec![line(y, 0xf23645, 2, "TRIX")];
                r.hlines = vec![hl(0.0, 0.5)];
            }
            Kind::UltimateOscillator => {
                let n = c.len();
                let (mut bp, mut tr) = (vec![f64::NAN; n], vec![f64::NAN; n]);
                for i in 1..n {
                    let (hi, lo) = (nan_max(h[i], c[i - 1]), nan_min(l[i], c[i - 1]));
                    bp[i] = c[i] - lo;
                    tr[i] = hi - lo;
                }
                let avg = |len| div(&sum_n(&bp, len), &sum_n(&tr, len));
                let (a, b, s) = (avg(u(0)), avg(u(1)), avg(u(2)));
                let y = (0..n)
                    .map(|i| 100.0 * (4.0 * a[i] + 2.0 * b[i] + s[i]) / 7.0)
                    .collect();
                r.lines = vec![line(y, PURPLE, 2, "UO")];
                r.y_range = Some((0.0, 100.0));
            }
            Kind::Ppo => {
                let ma = |n| {
                    if self.ma_type == "sma" {
                        sma_n(&c, n)
                    } else {
                        ema_n(&c, n)
                    }
                };
                let long = ma(u(1));
                let po = zip(&ma(u(0)), &long, |s, l| (s - l) / l * 100.0);
                r.lines = vec![line(finite_or_nan(po), BLUE, 2, "PPO")];
                r.hlines = vec![hl(0.0, 0.5)];
            }
            Kind::Cmo => {
                let ch = change_n(&c, 1);
                let (up, dn) = gains_losses(&ch);
                let (s1, s2) = (sum_n(&up, u(0)), sum_n(&dn, u(0)));
                let y = div(
                    &zip(&s1, &s2, |a, b| 100.0 * (a - b)),
                    &zip(&s1, &s2, |a, b| a + b),
                );
                r.lines = vec![line(y, PURPLE, 2, "CMO")];
                r.hlines = vec![hl(0.0, 0.5)];
                r.y_range = Some((-100.0, 100.0));
            }
            Kind::Dpo => {
                // TV (not centered): close - sma(close, len)[len/2 + 1]
                let back = u(0) / 2 + 1;
                let ma = sma_n(&c, u(0));
                let y = (0..c.len())
                    .map(|i| {
                        if i >= back {
                            c[i] - ma[i - back]
                        } else {
                            f64::NAN
                        }
                    })
                    .collect();
                r.lines = vec![line(y, 0x43a047, 2, "DPO")];
                r.hlines = vec![hl(0.0, 0.5)];
            }
            Kind::ElderForceIndex => {
                let f = zip(&change_n(&c, 1), &vol, |a, b| a * b);
                r.lines = vec![line(ema_n(&f, u(0)), 0xf23645, 2, "EFI")];
                r.hlines = vec![hl(0.0, 0.5)];
            }
            Kind::Tsi => {
                let t: Vec<f64> = tsi_ratio(&c, u(1), u(0))
                    .iter()
                    .map(|v| v * 100.0)
                    .collect();
                let sig = ema_n(&t, u(2));
                r.lines = vec![line(t, BLUE, 2, "TSI"), line(sig, ORANGE, 2, "Signal")];
                r.hlines = vec![hl(0.0, 0.5)];
            }
            Kind::Kst => {
                let s: Vec<Vec<f64>> = (0..4).map(|k| sma_n(&roc(&c, u(k)), u(4 + k))).collect();
                let y: Vec<f64> = (0..c.len())
                    .map(|i| s[0][i] + 2.0 * s[1][i] + 3.0 * s[2][i] + 4.0 * s[3][i])
                    .collect();
                let sig = sma_n(&y, u(8));
                let (y, sig) = (finite_or_nan(y), finite_or_nan(sig));
                r.lines = vec![
                    line(y, 0x089981, 2, "KST"),
                    line(sig, 0xf23645, 2, "Signal"),
                ];
                r.hlines = vec![hl(0.0, 0.5)];
            }
            Kind::Coppock => {
                let s = zip(&roc(&c, u(1)), &roc(&c, u(2)), |a, b| a + b);
                r.lines = vec![line(finite_or_nan(wma_n(&s, u(0))), BLUE, 2, "Coppock")];
                r.hlines = vec![hl(0.0, 0.5)];
            }
            Kind::MassIndex => {
                let span = zip(&h, &l, |a, b| a - b);
                let e = ema_n(&span, 9);
                let y = sum_n(&div(&e, &ema_n(&e, 9)), u(0));
                r.lines = vec![line(y, BLUE, 2, "MI")];
            }
            Kind::BbPercentB | Kind::BbWidth => {
                let (basis, dev) = (sma_n(&c, u(0)), stdev(&c, u(0)));
                let m = self.vals[1];
                let up = zip(&basis, &dev, |b, s| b + m * s);
                let lo = zip(&basis, &dev, |b, s| b - m * s);
                if self.kind == Kind::BbPercentB {
                    let y = div(&zip(&c, &lo, |a, b| a - b), &zip(&up, &lo, |a, b| a - b));
                    r.lines = vec![line(y, 0x2196f3, 2, "%B")];
                    r.hlines = vec![hl(1.0, 0.4), hl(0.5, 0.2), hl(0.0, 0.4)];
                } else {
                    let y: Vec<f64> = div(&zip(&up, &lo, |a, b| a - b), &basis)
                        .iter()
                        .map(|v| v * 100.0)
                        .collect();
                    let (he, lc) = (highest(&y, u(2)), lowest(&y, u(3)));
                    r.lines = vec![
                        line(y, 0x2196f3, 2, "BBW"),
                        line(he, 0xf23645, 1, "Highest Expansion"),
                        line(lc, 0x089981, 1, "Lowest Contraction"),
                    ];
                }
            }
            Kind::Choppiness => {
                let n = u(0);
                let rng = zip(&highest(&h, n), &lowest(&l, n), |a, b| a - b);
                let ratio = div(&sum_n(&true_range(d), n), &rng);
                let y = ratio
                    .iter()
                    .map(|v| 100.0 * v.log10() / (n as f64).log10())
                    .collect();
                r.lines = vec![line(finite_or_nan(y), BLUE, 2, "CHOP")];
                r.hlines = vec![hl(61.8, 0.4), hl(38.2, 0.4)];
                r.y_range = Some((0.0, 100.0));
            }
            Kind::FisherTransform => {
                let (hi, lo) = (highest(&hl2, u(0)), lowest(&hl2, u(0)));
                let n = c.len();
                let (mut fish, mut value) = (vec![f64::NAN; n], vec![f64::NAN; n]);
                // Pine nz(x[1]): na -> 0
                let nz = |x: &[f64], i: usize| {
                    if i > 0 && !x[i - 1].is_nan() {
                        x[i - 1]
                    } else {
                        0.0
                    }
                };
                for i in 0..n {
                    let raw = 0.66 * ((hl2[i] - lo[i]) / (hi[i] - lo[i]).max(0.001) - 0.5)
                        + 0.67 * nz(&value, i);
                    value[i] = if raw > 0.99 {
                        0.999
                    } else if raw < -0.99 {
                        -0.999
                    } else {
                        raw
                    };
                    fish[i] = 0.5 * ((1.0 + value[i]) / (1.0 - value[i])).ln() + 0.5 * nz(&fish, i);
                }
                let trig = (0..n)
                    .map(|i| if i > 0 { fish[i - 1] } else { f64::NAN })
                    .collect();
                r.lines = vec![
                    line(fish, BLUE, 2, "Fisher"),
                    line(trig, ORANGE, 2, "Trigger"),
                ];
                r.hlines = vec![
                    hl(1.5, 0.3),
                    hl(0.75, 0.3),
                    hl(0.0, 0.5),
                    hl(-0.75, 0.3),
                    hl(-1.5, 0.3),
                ];
            }
            Kind::EaseOfMovement => {
                let rng = zip(&h, &l, |a, b| a - b);
                let raw = div(
                    &zip(&change_n(&hl2, 1), &rng, |a, b| self.vals[1] * a * b),
                    &vol,
                );
                r.lines = vec![line(sma_n(&raw, u(0)), 0x2196f3, 2, "EOM")];
                r.hlines = vec![hl(0.0, 0.5)];
            }
            Kind::VolumeOscillator => {
                let long = ema_n(&vol, u(1));
                let y = zip(&ema_n(&vol, u(0)), &long, |s, l| 100.0 * (s - l) / l);
                r.lines = vec![line(finite_or_nan(y), BLUE, 2, "Vol Osc")];
                r.hlines = vec![hl(0.0, 0.5)];
            }
            Kind::BalanceOfPower => {
                let num: Vec<f64> = d.bars.iter().map(|b| b.close - b.open).collect();
                r.lines = vec![line(div(&num, &zip(&h, &l, |a, b| a - b)), BLUE, 2, "BOP")];
                r.hlines = vec![hl(0.0, 0.5)];
            }
            Kind::Rvi => {
                let co: Vec<f64> = d.bars.iter().map(|b| b.close - b.open).collect();
                let num = sum_n(&swma(&co), u(0));
                let den = sum_n(&swma(&zip(&h, &l, |a, b| a - b)), u(0));
                let rvi = div(&num, &den);
                let sig = swma(&rvi);
                r.lines = vec![
                    line(rvi, 0x089981, 2, "RVGI"),
                    line(sig, 0xf23645, 2, "Signal"),
                ];
                r.hlines = vec![hl(0.0, 0.5)];
            }
            Kind::SmiErgodic => {
                let erg = tsi_ratio(&c, u(1), u(0));
                let sig = ema_n(&erg, u(2));
                let osc = zip(&erg, &sig, |a, b| a - b);
                r.lines = vec![line(erg, BLUE, 2, "SMI"), line(sig, ORANGE, 2, "Signal")];
                r.bars = vec![hist(osc)];
                r.hlines = vec![hl(0.0, 0.5)];
            }
            Kind::ConnorsRsi => {
                let r1 = rsi(&c, u(0));
                let ud = rsi(&updown(&c), u(1));
                let pr = percentrank(&roc(&c, 1), u(2));
                let y = (0..c.len())
                    .map(|i| (r1[i] + ud[i] + pr[i]) / 3.0)
                    .collect();
                r.lines = vec![line(y, BLUE, 2, "CRSI")];
                r.hlines = vec![hl(70.0, 0.4), hl(50.0, 0.2), hl(30.0, 0.4)];
                r.y_range = Some((0.0, 100.0));
            }
        }
        r
    }
}

/// Look up one of the oscillators in this module.
pub fn by_name(name: &str) -> Option<Osc> {
    Kind::from_name(name).map(Osc::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{sample_data, Bar};

    const NAN: f64 = f64::NAN;

    /// Bars from (open, high, low, close, volume) tuples.
    fn bars(v: &[(f64, f64, f64, f64, f64)]) -> OhlcvData {
        OhlcvData {
            bars: v
                .iter()
                .map(|&(open, high, low, close, volume)| Bar {
                    open,
                    high,
                    low,
                    close,
                    volume,
                    date: String::new(),
                })
                .collect(),
            symbol: None,
            interval: None,
        }
    }
    fn closes(c: &[f64]) -> OhlcvData {
        bars(&c.iter().map(|&x| (x, x, x, x, 1000.0)).collect::<Vec<_>>())
    }
    fn run(k: Kind, p: Value, d: &OhlcvData) -> PanelResult {
        let mut o = Osc::new(k);
        o.configure(&p);
        o.compute(d)
    }
    fn eq(got: &[f64], want: &[f64]) {
        assert_eq!(got.len(), want.len(), "{got:?} vs {want:?}");
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            if w.is_nan() {
                assert!(g.is_nan(), "idx {i}: got {g}, want NaN ({got:?})");
            } else {
                assert!((g - w).abs() < 1e-3, "idx {i}: got {g}, want {w} ({got:?})");
            }
        }
    }
    fn first_valid(y: &[f64]) -> usize {
        y.iter().position(|v| !v.is_nan()).unwrap()
    }

    #[test]
    fn helpers_match_pine() {
        // ta.rsi(len 2): rma seeds with SMA of the first 2 changes.
        eq(
            &rsi(&[1.0, 2.0, 1.0, 2.0, 3.0, 2.0], 2),
            &[NAN, NAN, 50.0, 75.0, 87.5, 43.75],
        );
        // ema seeds with SMA
        eq(&ema_n(&[1.0, 2.0, 3.0, 4.0], 2), &[NAN, 1.5, 2.5, 3.5]);
        eq(&wma_n(&[1.0, 2.0, 3.0], 3), &[NAN, NAN, 14.0 / 6.0]);
        eq(&stdev(&[1.0, 3.0], 2), &[NAN, 1.0]);
        eq(
            &updown(&[1.0, 2.0, 3.0, 3.0, 2.0, 1.0]),
            &[-1.0, 1.0, 2.0, 0.0, -1.0, -2.0],
        );
        // 2 of the previous 3 values (1,2,3) are <= 2
        eq(
            &percentrank(&[1.0, 2.0, 3.0, 2.0], 3),
            &[NAN, NAN, NAN, 200.0 / 3.0],
        );
        eq(&swma(&[0.0, 6.0, 0.0, 0.0]), &[NAN, NAN, NAN, 2.0]);
    }

    #[test]
    fn stoch_rsi_values_and_flat_range() {
        let p = json!({"rsi_length":2,"stoch_length":2,"smooth_k":1,"smooth_d":1});
        let r = run(
            Kind::StochRsi,
            p.clone(),
            &closes(&[1.0, 2.0, 1.0, 2.0, 3.0, 2.0]),
        );
        // rsi = [_, _, 50, 75, 87.5, 43.75]
        eq(&r.lines[0].y, &[NAN, NAN, NAN, 100.0, 100.0, 0.0]);
        // flat prices: range 0 -> NaN (Pine division by zero)
        let r = run(Kind::StochRsi, p, &closes(&[5.0; 8]));
        assert!(r.lines[0].y.iter().all(|v| v.is_nan()));
    }

    #[test]
    fn awesome_oscillator_values_and_colors() {
        let d = closes(&[1.0, 2.0, 4.0, 8.0, 16.0, 8.0]);
        let r = run(Kind::AwesomeOscillator, json!({"fast":2,"slow":3}), &d);
        // sma2 = 3, 6, 12, 12 ; sma3 = 7/3, 14/3, 28/3, 32/3 (idx 2..5)
        eq(
            &r.bars[0].y,
            &[NAN, NAN, 2.0 / 3.0, 4.0 / 3.0, 8.0 / 3.0, 4.0 / 3.0],
        );
        let cols = &r.bars[0].colors;
        assert_eq!(cols[3], rgba(0x089981, 0.9)); // rising -> green
        assert_eq!(cols[5], rgba(0xf23645, 0.9)); // falling -> red
    }

    #[test]
    fn momentum() {
        let r = run(
            Kind::Momentum,
            json!({"length":2}),
            &closes(&[1.0, 3.0, 6.0, 10.0, 15.0]),
        );
        eq(&r.lines[0].y, &[NAN, NAN, 5.0, 7.0, 9.0]);
    }

    #[test]
    fn trix_len1_is_log_change() {
        let r = run(Kind::Trix, json!({"length":1}), &closes(&[1.0, 2.0, 2.0]));
        eq(&r.lines[0].y, &[NAN, 10000.0 * 2f64.ln(), 0.0]);
    }

    #[test]
    fn ultimate_oscillator() {
        let d = bars(&[
            (9.0, 10.0, 8.0, 9.0, 1.0),
            (9.0, 12.0, 9.0, 11.0, 1.0),
            (11.0, 13.0, 10.0, 10.0, 1.0),
            (10.0, 12.0, 9.0, 12.0, 1.0),
        ]);
        let r = run(
            Kind::UltimateOscillator,
            json!({"fast":1,"middle":2,"slow":3}),
            &d,
        );
        // bp = [_,2,0,3], tr = [_,3,3,3]; avg = 1, 0.5, 5/9
        eq(
            &r.lines[0].y,
            &[NAN, NAN, NAN, 100.0 * (4.0 + 1.0 + 5.0 / 9.0) / 7.0],
        );
    }

    #[test]
    fn price_oscillator_ema_and_sma() {
        let d = closes(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let r = run(Kind::Ppo, json!({"short":2,"long":3}), &d);
        // ema2 = 1.5, 2.5, 3.5 ; ema3 = 2, 3 at idx 2,3 ; (s-l)/l*100
        eq(&r.lines[0].y[..4], &[NAN, NAN, 25.0, 100.0 / 6.0]);
        assert!(r.bars.is_empty() && r.lines.len() == 1);
        // sma2 = _,1.5,2.5,3.5 ; sma3 = _,_,2,3 -> idx2 (2.5-2)/2*100
        let r = run(Kind::Ppo, json!({"short":2,"long":3,"type":"sma"}), &d);
        eq(&r.lines[0].y[..4], &[NAN, NAN, 25.0, 100.0 * 0.5 / 3.0]);
    }

    #[test]
    fn non_finite_input_recovers() {
        // an inf close must not poison the recursive smoothers forever
        let mut c: Vec<f64> = (1..=40).map(|v| v as f64).collect();
        c[10] = f64::INFINITY;
        c[20] = 0.0; // ln(0) must be NaN in TRIX, not -inf
        for k in [Kind::Trix, Kind::Cmo, Kind::Tsi, Kind::Kst] {
            let y = run(k, json!({"length":2}), &closes(&c)).lines[0].y.clone();
            assert!(y.iter().all(|v| !v.is_infinite()), "{}", k.name());
        }
        let e = ema_n(&[1.0, 2.0, f64::INFINITY, 4.0, 5.0, 6.0], 2);
        assert!(e[5].is_finite(), "{e:?}");
    }

    #[test]
    fn ultimate_oscillator_nan_propagates() {
        let mut d = bars(&[(9.0, 10.0, 8.0, 9.0, 1.0); 6]);
        d.bars[2].high = f64::NAN;
        let r = run(
            Kind::UltimateOscillator,
            json!({"fast":1,"middle":2,"slow":3}),
            &d,
        );
        assert!(r.lines[0].y[2].is_nan());
        assert!(r.lines[0].y[3].is_nan());
    }

    #[test]
    fn roc_zero_base_is_nan() {
        eq(&roc(&[0.0, 5.0, 10.0], 1), &[NAN, NAN, 100.0]);
    }

    #[test]
    fn running_sum_is_linear_and_exact() {
        let x: Vec<f64> = (0..20000).map(|v| (v % 7) as f64).collect();
        let y = sum_n(&x, 10000);
        assert!(y[9998].is_nan());
        assert_eq!(y[9999], x[..10000].iter().sum::<f64>());
    }

    #[test]
    fn cmo() {
        let r = run(
            Kind::Cmo,
            json!({"length":3}),
            &closes(&[10.0, 11.0, 10.0, 12.0, 11.0, 13.0]),
        );
        eq(&r.lines[0].y, &[NAN, NAN, NAN, 50.0, 0.0, 60.0]);
    }

    #[test]
    fn dpo() {
        let r = run(
            Kind::Dpo,
            json!({"length":2}),
            &closes(&[1.0, 2.0, 4.0, 8.0, 16.0]),
        );
        // barsback = 2; sma2 = [_,1.5,3,6,12]; dpo = close - sma[i-2]
        eq(&r.lines[0].y, &[NAN, NAN, NAN, 6.5, 13.0]);
    }

    #[test]
    fn elder_force_index() {
        let d = bars(&[
            (0.0, 10.0, 10.0, 10.0, 50.0),
            (0.0, 12.0, 12.0, 12.0, 200.0),
            (0.0, 11.0, 11.0, 11.0, 300.0),
        ]);
        let r = run(Kind::ElderForceIndex, json!({"length":1}), &d);
        eq(&r.lines[0].y, &[NAN, 400.0, -300.0]);
    }

    #[test]
    fn tsi() {
        let p = json!({"long":2,"short":1,"signal":1});
        let r = run(Kind::Tsi, p, &closes(&[10.0, 12.0, 11.0, 14.0]));
        // pc = _,2,-1,3 ; ema2(pc)= .5, 13/6 ; ema2(|pc|)= 1.5, 2.5
        eq(
            &r.lines[0].y,
            &[NAN, NAN, 100.0 / 3.0, 100.0 * (13.0 / 6.0) / 2.5],
        );
    }

    #[test]
    fn kst() {
        let p = json!({"roc1":1,"roc2":1,"roc3":1,"roc4":1,"sma1":1,"sma2":1,"sma3":1,"sma4":1,"signal":1});
        let r = run(Kind::Kst, p, &closes(&[100.0, 110.0]));
        eq(&r.lines[0].y, &[NAN, 100.0]);
        eq(&r.lines[1].y, &[NAN, 100.0]);
    }

    #[test]
    fn coppock() {
        let p = json!({"wma_length":2,"long_roc":2,"short_roc":1});
        let r = run(Kind::Coppock, p, &closes(&[100.0, 110.0, 132.0, 165.0]));
        // idx2: 32 + 20 = 52 ; idx3: 50 + 25 = 75 ; wma2 = (75*2 + 52)/3
        eq(&r.lines[0].y, &[NAN, NAN, NAN, 202.0 / 3.0]);
    }

    #[test]
    fn mass_index_constant_range() {
        let d = bars(&[(1.0, 2.0, 1.0, 1.5, 1.0); 24]);
        let r = run(Kind::MassIndex, json!({"length":10}), &d);
        // first valid bar is 25, beyond 24 bars
        assert!(r.lines[0].y.iter().all(|v| v.is_nan()));
        let d = bars(&[(1.0, 2.0, 1.0, 1.5, 1.0); 30]);
        let r = run(Kind::MassIndex, json!({"length":10}), &d);
        assert_eq!(first_valid(&r.lines[0].y), 25);
        assert!((r.lines[0].y[25] - 10.0).abs() < 1e-9);
    }

    #[test]
    fn bollinger_percent_b_and_width() {
        let d = closes(&[1.0, 3.0, 2.0]);
        let p = json!({"length":2,"mult":2.0});
        // idx1: basis 2, dev 1 -> [0,4]; %B = 3/4 ; width = 100 * 4/2
        // idx2: basis 2.5, dev .5 -> [1.5,3.5]; %B = .25 ; width = 100 * 2/2.5
        eq(
            &run(Kind::BbPercentB, p.clone(), &d).lines[0].y,
            &[NAN, 0.75, 0.25],
        );
        let w = run(Kind::BbWidth, p, &d);
        eq(&w.lines[0].y, &[NAN, 200.0, 80.0]);
        assert_eq!(w.lines.len(), 3);
    }

    #[test]
    fn choppiness() {
        let d = bars(&[(9.0, 10.0, 8.0, 9.0, 1.0), (9.0, 12.0, 9.0, 11.0, 1.0)]);
        let r = run(Kind::Choppiness, json!({"length":2}), &d);
        // tr = 2, 3 ; range = 12 - 8 = 4 ; 100*log10(5/4)/log10(2)
        eq(&r.lines[0].y, &[NAN, 32.1928]);
    }

    #[test]
    fn fisher_transform() {
        let d = bars(&[
            (10.0, 10.0, 10.0, 10.0, 1.0),
            (12.0, 12.0, 12.0, 12.0, 1.0),
            (12.0, 12.0, 12.0, 12.0, 1.0),
        ]);
        let r = run(Kind::FisherTransform, json!({"length":2}), &d);
        // idx1: value = .66*(1-.5) = .33 ; fish = .5*ln(1.33/.67)
        let f1 = 0.5 * (1.33f64 / 0.67).ln();
        // idx2: window [12,12] -> range max(0,.001); value = .66*(-.5)+.67*.33
        let v2: f64 = 0.66 * (-0.5) + 0.67 * 0.33;
        let f2 = 0.5 * ((1.0 + v2) / (1.0 - v2)).ln() + 0.5 * f1;
        eq(&r.lines[0].y, &[NAN, f1, f2]);
        eq(&r.lines[1].y, &[NAN, NAN, f1]);
    }

    #[test]
    fn ease_of_movement() {
        let d = bars(&[
            (0.0, 11.0, 9.0, 10.0, 1000.0),
            (0.0, 13.0, 11.0, 12.0, 1000.0),
            (0.0, 13.0, 11.0, 12.0, 0.0),
        ]);
        let r = run(Kind::EaseOfMovement, json!({"length":1}), &d);
        // 10000 * 2 * 2 / 1000 = 40 ; zero volume -> NaN input, which
        // ta.sma ignores: the last valid value carries (no gap).
        eq(&r.lines[0].y, &[NAN, 40.0, 40.0]);
    }

    #[test]
    fn volume_oscillator() {
        let d = bars(&[(0.0, 1.0, 1.0, 1.0, 100.0), (0.0, 1.0, 1.0, 1.0, 200.0)]);
        let r = run(Kind::VolumeOscillator, json!({"short":1,"long":2}), &d);
        eq(&r.lines[0].y, &[NAN, 100.0 * 50.0 / 150.0]);
    }

    #[test]
    fn balance_of_power() {
        let d = bars(&[(10.0, 14.0, 10.0, 12.0, 1.0), (10.0, 10.0, 10.0, 10.0, 1.0)]);
        eq(
            &run(Kind::BalanceOfPower, json!({}), &d).lines[0].y,
            &[0.5, NAN],
        );
    }

    #[test]
    fn rvi_vigor() {
        let d = bars(&[(10.0, 12.0, 10.0, 11.0, 1.0); 8]);
        let r = run(Kind::Rvi, json!({"length":1}), &d);
        // constant bars: swma(1)/swma(2) = 0.5 from idx 3; signal swma from idx 6
        eq(&r.lines[0].y[..5], &[NAN, NAN, NAN, 0.5, 0.5]);
        assert_eq!(first_valid(&r.lines[1].y), 6);
    }

    #[test]
    fn smi_ergodic() {
        let p = json!({"long":2,"short":1,"signal":1});
        let r = run(Kind::SmiErgodic, p, &closes(&[10.0, 12.0, 11.0, 14.0]));
        eq(&r.lines[0].y, &[NAN, NAN, 1.0 / 3.0, 13.0 / 6.0 / 2.5]);
        eq(&r.bars[0].y, &[NAN, NAN, 0.0, 0.0]);
    }

    #[test]
    fn connors_rsi() {
        let c = [1.0, 2.0, 1.0, 2.0, 3.0, 2.0];
        let p = json!({"rsi_length":2,"updown_length":2,"roc_length":2});
        let y = run(Kind::ConnorsRsi, p, &closes(&c)).lines[0].y.clone();
        // components: rsi2 = [_,_,50,75,87.5,43.75]
        // streak = [-1,1,-1,1,2,-1], rsi2 of streak derived below
        // roc1 = [_,100,-50,100,50,-33.3] ; percentrank(2) idx3 = 100, idx4 = 50, idx5 = 0
        let streak = updown(&c);
        eq(&streak, &[-1.0, 1.0, -1.0, 1.0, 2.0, -1.0]);
        // streak changes = _,2,-2,2,1,-3 -> rma2 up seed (2+0)/2=1, dn (0+2)/2=1 -> rsi 50 at idx2
        // idx3: up=.5*2+.5*1=1.5, dn=.5 -> 75 ; idx4: up=1.25, dn=.25 -> 83.333
        eq(&rsi(&streak, 2)[2..5], &[50.0, 75.0, 100.0 - 100.0 / 6.0]);
        eq(
            &y,
            &[
                NAN,
                NAN,
                NAN,
                (75.0 + 75.0 + 100.0) / 3.0,
                (87.5 + (100.0 - 100.0 / 6.0) + 50.0) / 3.0,
                (43.75 + rsi(&streak, 2)[5] + 0.0) / 3.0,
            ],
        );
    }

    #[test]
    fn warmup_lengths_with_defaults() {
        let d = sample_data(300);
        let want = [
            (Kind::StochRsi, 29),
            (Kind::AwesomeOscillator, 33),
            (Kind::Momentum, 10),
            (Kind::Trix, 52),
            (Kind::UltimateOscillator, 28),
            (Kind::Ppo, 20),
            (Kind::Cmo, 9),
            (Kind::Dpo, 31),
            (Kind::ElderForceIndex, 13),
            (Kind::Tsi, 37),
            (Kind::Kst, 44),
            (Kind::Coppock, 23),
            (Kind::MassIndex, 25),
            (Kind::BbPercentB, 19),
            (Kind::BbWidth, 19),
            (Kind::Choppiness, 13),
            (Kind::FisherTransform, 8),
            (Kind::EaseOfMovement, 14),
            (Kind::VolumeOscillator, 9),
            (Kind::BalanceOfPower, 0),
            (Kind::Rvi, 12),
            (Kind::SmiErgodic, 24),
            (Kind::ConnorsRsi, 101),
        ];
        assert_eq!(want.len(), ALL.len());
        for (k, idx) in want {
            let r = run(k, json!({}), &d);
            let y = if k == Kind::AwesomeOscillator {
                &r.bars[0].y
            } else {
                &r.lines[0].y
            };
            assert_eq!(first_valid(y), idx, "{}", k.name());
            assert!(y[..idx].iter().all(|v| v.is_nan()));
            assert!(
                y[idx..].iter().filter(|v| v.is_finite()).count() > 100,
                "{}",
                k.name()
            );
        }
    }

    #[test]
    fn short_series_and_extreme_params_do_not_panic() {
        let full = sample_data(60);
        for k in ALL {
            for n in [0usize, 1, 2, 5] {
                let d = OhlcvData {
                    bars: full.bars[..n].to_vec(),
                    symbol: None,
                    interval: None,
                };
                let r = run(k, json!({}), &d);
                assert!(r.lines.iter().all(|l| l.y.len() == n));
            }
            for pick in ["min", "max"] {
                let mut p = serde_json::Map::new();
                for s in Osc::new(k).params().as_array().unwrap() {
                    p.insert(s["name"].as_str().unwrap().into(), s[pick].clone());
                }
                let r = run(k, Value::Object(p), &full);
                assert!(r.lines.iter().all(|l| l.y.len() == 60));
            }
        }
    }

    #[test]
    fn bbw_extremes_follow_bbw() {
        let r = run(
            Kind::BbWidth,
            json!({"highest_expansion":3,"lowest_contraction":3,"length":2}),
            &closes(&[1.0, 3.0, 2.0, 4.0, 4.0]),
        );
        let (w, he, lc) = (&r.lines[0].y, &r.lines[1].y, &r.lines[2].y);
        assert_eq!(first_valid(he), 3);
        for i in 3..5 {
            let win = &w[i - 2..=i];
            assert_eq!(he[i], win.iter().cloned().fold(f64::MIN, f64::max));
            assert_eq!(lc[i], win.iter().cloned().fold(f64::MAX, f64::min));
        }
    }

    #[test]
    fn names_and_aliases_resolve() {
        for k in ALL {
            assert_eq!(Kind::from_name(k.name()), Some(k));
            for a in k.aliases() {
                assert_eq!(Kind::from_name(a), Some(k), "{a}");
            }
        }
    }
}
