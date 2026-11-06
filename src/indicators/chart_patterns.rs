//! Auto chart patterns (ADR-0046): classic patterns on ZigZag pivots.
//!
//! `detect` is pure and reads closed bars only. It walks through time, deciding
//! each pattern from what was known then: a pivot is known `depth` bars after its
//! extreme, a pattern appears when its last pivot is known, and only a close from
//! then on can confirm it. So adding bars never changes a past confirmation, and
//! the hit rate on a random walk stays near chance (examples/pattern_stats.rs).
//! Elliott waves are left out: an honest count needs a degree and alternatives,
//! not one rule check on pivots.

use super::candle_patterns::{newest_bar_forming, now_ms};
use super::pine::atr;
use super::trend::{zigzag_pivots, zigzag_walk};
use crate::data::{Bar, OhlcvData};
use crate::indicator::*;
use serde_json::{json, Value};

/// Bars searched: TradingView's "most recent 600 bars".
pub const LOOKBACK: usize = 600;
const ATR_LEN: usize = 10;
/// A line's rise over its pattern, as a share of the height, still called flat.
const FLAT: f64 = 0.25;
/// Share of a pattern's span another one may overlap before one of them goes.
const MAX_OVERLAP: f64 = 0.3;
/// Most patterns reported, newest first.
const MAX_PATTERNS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pivot {
    pub bar: usize,
    pub price: f64,
    pub high: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    HeadAndShoulders,
    InverseHeadAndShoulders,
    TripleTop,
    TripleBottom,
    DoubleTop,
    DoubleBottom,
    BullFlag,
    BearFlag,
    BullPennant,
    BearPennant,
    AscendingTriangle,
    DescendingTriangle,
    SymmetricalTriangle,
    RisingWedge,
    FallingWedge,
    ChannelUp,
    ChannelDown,
    Rectangle,
}

pub const KINDS: &[Kind] = &[
    Kind::HeadAndShoulders,
    Kind::InverseHeadAndShoulders,
    Kind::TripleTop,
    Kind::TripleBottom,
    Kind::DoubleTop,
    Kind::DoubleBottom,
    Kind::BullFlag,
    Kind::BearFlag,
    Kind::BullPennant,
    Kind::BearPennant,
    Kind::AscendingTriangle,
    Kind::DescendingTriangle,
    Kind::SymmetricalTriangle,
    Kind::RisingWedge,
    Kind::FallingWedge,
    Kind::ChannelUp,
    Kind::ChannelDown,
    Kind::Rectangle,
];

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Kind::HeadAndShoulders => "head_and_shoulders",
            Kind::InverseHeadAndShoulders => "inverse_head_and_shoulders",
            Kind::TripleTop => "triple_top",
            Kind::TripleBottom => "triple_bottom",
            Kind::DoubleTop => "double_top",
            Kind::DoubleBottom => "double_bottom",
            Kind::BullFlag => "bull_flag",
            Kind::BearFlag => "bear_flag",
            Kind::BullPennant => "bull_pennant",
            Kind::BearPennant => "bear_pennant",
            Kind::AscendingTriangle => "ascending_triangle",
            Kind::DescendingTriangle => "descending_triangle",
            Kind::SymmetricalTriangle => "symmetrical_triangle",
            Kind::RisingWedge => "rising_wedge",
            Kind::FallingWedge => "falling_wedge",
            Kind::ChannelUp => "channel_up",
            Kind::ChannelDown => "channel_down",
            Kind::Rectangle => "rectangle",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::HeadAndShoulders => "Head and Shoulders",
            Kind::InverseHeadAndShoulders => "Inverse Head and Shoulders",
            Kind::TripleTop => "Triple Top",
            Kind::TripleBottom => "Triple Bottom",
            Kind::DoubleTop => "Double Top",
            Kind::DoubleBottom => "Double Bottom",
            Kind::BullFlag => "Bull Flag",
            Kind::BearFlag => "Bear Flag",
            Kind::BullPennant => "Bull Pennant",
            Kind::BearPennant => "Bear Pennant",
            Kind::AscendingTriangle => "Ascending Triangle",
            Kind::DescendingTriangle => "Descending Triangle",
            Kind::SymmetricalTriangle => "Symmetrical Triangle",
            Kind::RisingWedge => "Rising Wedge",
            Kind::FallingWedge => "Falling Wedge",
            Kind::ChannelUp => "Channel Up",
            Kind::ChannelDown => "Channel Down",
            Kind::Rectangle => "Rectangle",
        }
    }

    /// Which way a breakout confirms it. `None`: either way.
    fn expects(self) -> Option<bool> {
        match self {
            Kind::InverseHeadAndShoulders
            | Kind::TripleBottom
            | Kind::DoubleBottom
            | Kind::BullFlag
            | Kind::BullPennant
            | Kind::AscendingTriangle
            | Kind::FallingWedge => Some(true),
            Kind::HeadAndShoulders
            | Kind::TripleTop
            | Kind::DoubleTop
            | Kind::BearFlag
            | Kind::BearPennant
            | Kind::DescendingTriangle
            | Kind::RisingWedge => Some(false),
            Kind::SymmetricalTriangle | Kind::ChannelUp | Kind::ChannelDown | Kind::Rectangle => {
                None
            }
        }
    }

    fn meaning(self) -> &'static str {
        match self {
            Kind::HeadAndShoulders => {
                "A peak between two lower peaks: an uptrend may be turning down."
            }
            Kind::InverseHeadAndShoulders => {
                "A low between two higher lows: a downtrend may be turning up."
            }
            Kind::TripleTop => "Three peaks at one level: buyers failed there three times.",
            Kind::TripleBottom => "Three lows at one level: sellers failed there three times.",
            Kind::DoubleTop => "Two peaks at one level: buyers failed there twice.",
            Kind::DoubleBottom => "Two lows at one level: sellers failed there twice.",
            Kind::BullFlag => "A sharp rise, then a small drift down: the rise may go on.",
            Kind::BearFlag => "A sharp fall, then a small drift up: the fall may go on.",
            Kind::BullPennant => "A sharp rise, then a small narrowing range: the rise may go on.",
            Kind::BearPennant => "A sharp fall, then a small narrowing range: the fall may go on.",
            Kind::AscendingTriangle => "A flat top and rising lows: buyers are pressing a level.",
            Kind::DescendingTriangle => {
                "A flat bottom and falling highs: sellers are pressing a level."
            }
            Kind::SymmetricalTriangle => {
                "Falling highs and rising lows: a narrowing range before a move."
            }
            Kind::RisingWedge => "Both lines rise and narrow: a rise that is losing force.",
            Kind::FallingWedge => "Both lines fall and narrow: a fall that is losing force.",
            Kind::ChannelUp => "Two rising parallel lines hold the price.",
            Kind::ChannelDown => "Two falling parallel lines hold the price.",
            Kind::Rectangle => "Two flat lines hold the price: a range before a move.",
        }
    }
}

/// `y = y0 + slope * (x - x0)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrendLine {
    x0: f64,
    y0: f64,
    slope: f64,
}

impl TrendLine {
    fn through(a: &Pivot, b: &Pivot) -> Self {
        let run = (b.bar as f64 - a.bar as f64).max(1.0);
        TrendLine {
            x0: a.bar as f64,
            y0: a.price,
            slope: (b.price - a.price) / run,
        }
    }
    fn flat(y: f64) -> Self {
        TrendLine {
            x0: 0.0,
            y0: y,
            slope: 0.0,
        }
    }
    fn at(&self, x: usize) -> f64 {
        self.y0 + self.slope * (x as f64 - self.x0)
    }
}

/// A detection, in bar indices of the bars given to `detect`.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub kind: Kind,
    pub bias: Bias,
    pub status: PatternStatus,
    /// Pivots of the outline, oldest first.
    pub points: Vec<Pivot>,
    pub upper: Option<TrendLine>,
    pub lower: Option<TrendLine>,
    /// First and last bar of the pattern before any breakout.
    pub start: usize,
    pub end: usize,
    /// First bar with its last pivot known; only closes from here on count.
    pub known_at: usize,
    pub confirmed_at: Option<usize>,
    pub failed_at: Option<usize>,
    /// Why it failed, for the tooltip.
    pub failed_why: Option<&'static str>,
    pub target: Option<f64>,
    pub invalidation: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub depth: usize,
    /// Pivots at least this many ATR(10) apart.
    pub deviation: f64,
    /// "Same level", as a share of the pattern's height.
    pub tolerance: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            depth: 5,
            deviation: 3.0,
            tolerance: 0.15,
        }
    }
}

/// ZigZag pivots of `bars`, `deviation` ATR(10) apart. Each threshold is read at
/// its pivot's bar, so a later bar's volatility does not move older pivots.
pub fn pivots(bars: &[Bar], depth: usize, deviation: f64) -> Vec<Pivot> {
    let a = atr_of(bars);
    zigzag_pivots(bars, depth, |i| threshold(bars, &a, i, deviation))
        .into_iter()
        .map(to_pivot)
        .collect()
}

fn atr_of(bars: &[Bar]) -> Vec<f64> {
    let data = OhlcvData {
        bars: bars.to_vec(),
        symbol: None,
        interval: None,
        session: None,
    };
    atr(&data, ATR_LEN)
}

/// `deviation` ATR(10) at bar `i`, in percent of the close.
fn threshold(bars: &[Bar], atr: &[f64], i: usize, deviation: f64) -> f64 {
    let (r, c) = (atr[i], bars[i].close);
    if r.is_finite() && c != 0.0 {
        r / c.abs() * 100.0 * deviation
    } else {
        0.0
    }
}

fn to_pivot((bar, price, high): (usize, f64, bool)) -> Pivot {
    Pivot { bar, price, high }
}

/// What decides a pattern's status after its last pivot.
struct Watch {
    upper: TrendLine,
    lower: TrendLine,
    /// A close past this (above when the breakout is down) fails the pattern.
    stop: Option<f64>,
    /// Bars a pattern may wait for its breakout.
    patience: usize,
    height: f64,
}

/// Status from the closes since bar `known`: `kind`'s breakout confirms it, a
/// breakout the other way or `Watch::stop` fails it, and so does waiting too long.
/// After a confirmation, a close past `stop` before the target fails it still.
fn resolve(
    kind: Kind,
    w: &Watch,
    bars: &[Bar],
    known: usize,
) -> (
    PatternStatus,
    Bias,
    Option<usize>,
    Option<usize>,
    Option<&'static str>,
    Option<f64>,
) {
    let expects = kind.expects();
    let mut bias = match expects {
        Some(true) => Bias::Bullish,
        Some(false) => Bias::Bearish,
        None => Bias::Neutral,
    };
    let mut confirmed = None;
    let mut target = None;
    for (i, bar) in bars.iter().enumerate().skip(known) {
        let c = bar.close;
        if confirmed.is_none() {
            if i > known + w.patience {
                return (
                    PatternStatus::Failed,
                    bias,
                    None,
                    Some(i),
                    Some("no breakout in time"),
                    None,
                );
            }
            let up = c > w.upper.at(i);
            let down = c < w.lower.at(i);
            let past_stop = w.stop.is_some_and(|s| match expects {
                Some(true) => c < s,
                Some(false) => c > s,
                None => false,
            });
            if past_stop || (up && expects == Some(false)) || (down && expects == Some(true)) {
                return (
                    PatternStatus::Failed,
                    bias,
                    None,
                    Some(i),
                    Some("closed through the far side"),
                    None,
                );
            }
            if up || down {
                confirmed = Some(i);
                if expects.is_none() {
                    bias = if up { Bias::Bullish } else { Bias::Bearish };
                }
                target = Some(if up { c + w.height } else { c - w.height });
            }
            continue;
        }
        let t = target.unwrap_or(f64::NAN);
        let up = bias == Bias::Bullish;
        if (up && bar.high >= t) || (!up && bar.low <= t) {
            break;
        }
        let back = match (expects, w.stop) {
            (Some(true), Some(s)) => c < s,
            (Some(false), Some(s)) => c > s,
            _ => false,
        };
        if back {
            return (
                PatternStatus::Failed,
                bias,
                confirmed,
                Some(i),
                Some("reversed before its target"),
                target,
            );
        }
    }
    match confirmed {
        Some(_) => (
            PatternStatus::Confirmed,
            bias,
            confirmed,
            None,
            None,
            target,
        ),
        None => (PatternStatus::Forming, bias, None, None, None, None),
    }
}

fn found(kind: Kind, p: &[Pivot], w: Watch, bars: &[Bar], depth: usize) -> Found {
    let (start, end) = (p[0].bar, p[p.len() - 1].bar);
    let known_at = end + depth;
    let (status, bias, confirmed_at, failed_at, failed_why, target) =
        resolve(kind, &w, bars, known_at);
    Found {
        kind,
        bias,
        status,
        points: p.to_vec(),
        upper: Some(w.upper),
        lower: Some(w.lower),
        start,
        end,
        known_at,
        confirmed_at,
        failed_at,
        failed_why,
        target,
        invalidation: w.stop,
    }
}

/// The pivots turned upside down, to read a bottom with the rules of a top.
fn flip(p: &[Pivot]) -> Vec<Pivot> {
    p.iter()
        .map(|x| Pivot {
            price: -x.price,
            high: !x.high,
            ..*x
        })
        .collect()
}

/// `[before, ls, l1, head, l2, rs]`: the neckline and height of a head and
/// shoulders. Written for the top; the inverse passes `flip`ped pivots.
fn head_and_shoulders_shape(p: &[Pivot], tol: f64) -> Option<(TrendLine, f64)> {
    let [before, ls, l1, head, l2, rs] = p else {
        return None;
    };
    let neck = TrendLine::through(l1, l2);
    let height = head.price - neck.at(head.bar);
    if height <= 0.0 {
        return None;
    }
    let shoulders_top = ls.price.max(rs.price);
    let level = (ls.price - rs.price).abs() <= 2.0 * tol * height;
    let head_clear = head.price - shoulders_top >= tol * height;
    let above_neck =
        ls.price - neck.at(ls.bar) >= 0.25 * height && rs.price - neck.at(rs.bar) >= 0.25 * height;
    let neck_gentle = (l1.price - l2.price).abs() <= 0.5 * height;
    let (d1, d2) = ((head.bar - ls.bar) as f64, (rs.bar - head.bar) as f64);
    let balanced = d1.max(d2) <= 2.5 * d1.min(d2);
    let from_below = before.price < neck.at(before.bar);
    (level && head_clear && above_neck && neck_gentle && balanced && from_below)
        .then_some((neck, height))
}

/// Reversal patterns ending at pivot `j`, tops read straight and bottoms flipped.
fn reversals(p: &[Pivot], j: usize, bars: &[Bar], s: &Settings, out: &mut Vec<Found>) {
    let tol = s.tolerance;
    let last_high = p[j].high;
    let pick = |top: Kind, bottom: Kind| if last_high { top } else { bottom };
    let sign = if last_high { 1.0 } else { -1.0 };
    let upright = |w: &[Pivot]| -> Vec<Pivot> {
        if last_high {
            w.to_vec()
        } else {
            flip(w)
        }
    };
    let price = |y: f64| y * sign;
    // Lines in real prices: the upper one is the neck of a bottom.
    let watch = |neck: TrendLine, height: f64, stop: f64, patience: usize| {
        let real = TrendLine {
            y0: neck.y0 * sign,
            slope: neck.slope * sign,
            ..neck
        };
        let away = TrendLine::flat(if last_high {
            f64::INFINITY
        } else {
            f64::NEG_INFINITY
        });
        Watch {
            upper: if last_high { away } else { real },
            lower: if last_high { real } else { away },
            stop: Some(price(stop)),
            patience,
            height,
        }
    };

    if j >= 5 {
        let w = &p[j - 5..=j];
        let u = upright(w);
        let (ls, l1, mid, l2, rs) = (u[1], u[2], u[3], u[4], u[5]);
        let neck_low = l1.price.min(l2.price);
        let height = ls.price.max(mid.price).max(rs.price) - neck_low;
        let tops = [ls.price, mid.price, rs.price];
        let spread = tops.iter().cloned().fold(f64::MIN, f64::max)
            - tops.iter().cloned().fold(f64::MAX, f64::min);
        let span = rs.bar - ls.bar;
        // Falling highs over flat lows are a triangle, not a third bottom.
        let flat_neck = (l1.price - l2.price).abs() <= 2.0 * tol * height;
        if height > 0.0 && spread <= tol * height && flat_neck && u[0].price < neck_low {
            let top = tops.iter().cloned().fold(f64::MIN, f64::max);
            let kind = pick(Kind::TripleTop, Kind::TripleBottom);
            out.push(found(
                kind,
                &w[1..],
                watch(TrendLine::flat(neck_low), height, top + tol * height, span),
                bars,
                s.depth,
            ));
        } else if let Some((neck, h)) = head_and_shoulders_shape(&u, tol) {
            let kind = pick(Kind::HeadAndShoulders, Kind::InverseHeadAndShoulders);
            out.push(found(
                kind,
                &w[1..],
                watch(neck, h, mid.price, span),
                bars,
                s.depth,
            ));
        }
    }
    if j >= 3 {
        let w = &p[j - 3..=j];
        let u = upright(w);
        let (before, t1, neck, t2) = (u[0], u[1], u[2], u[3]);
        let top = t1.price.max(t2.price);
        let height = top - neck.price;
        if height > 0.0 && (t1.price - t2.price).abs() <= tol * height && before.price < neck.price
        {
            let kind = pick(Kind::DoubleTop, Kind::DoubleBottom);
            let span = t2.bar - t1.bar;
            out.push(found(
                kind,
                &w[1..],
                watch(
                    TrendLine::flat(neck.price),
                    height,
                    top + tol * height,
                    span,
                ),
                bars,
                s.depth,
            ));
        }
    }
}

/// Lines through the first and last high and low of `w`, if every pivot sits on
/// them within `tol` of the height and the closes between them stay inside.
fn fit(w: &[Pivot], bars: &[Bar], tol: f64) -> Option<(TrendLine, TrendLine, f64, f64, f64)> {
    let highs: Vec<&Pivot> = w.iter().filter(|p| p.high).collect();
    let lows: Vec<&Pivot> = w.iter().filter(|p| !p.high).collect();
    if highs.len() < 2 || lows.len() < 2 {
        return None;
    }
    let upper = TrendLine::through(highs[0], highs[highs.len() - 1]);
    let lower = TrendLine::through(lows[0], lows[lows.len() - 1]);
    let (x0, x1) = (w[0].bar, w[w.len() - 1].bar);
    let (h0, h1) = (upper.at(x0) - lower.at(x0), upper.at(x1) - lower.at(x1));
    if h0 <= 0.0 || h1 <= 0.0 {
        return None;
    }
    let height = (h0 + h1) / 2.0;
    let slack = tol * height;
    if highs
        .iter()
        .any(|p| (p.price - upper.at(p.bar)).abs() > slack)
        || lows
            .iter()
            .any(|p| (p.price - lower.at(p.bar)).abs() > slack)
    {
        return None;
    }
    let outside = (x0..=x1)
        .filter(|&i| bars[i].close > upper.at(i) + slack || bars[i].close < lower.at(i) - slack)
        .count();
    if outside as f64 > 0.1 * (x1 - x0 + 1) as f64 {
        return None;
    }
    let rise_up = (upper.at(x1) - upper.at(x0)) / height;
    let rise_lo = (lower.at(x1) - lower.at(x0)) / height;
    Some((upper, lower, rise_up, rise_lo, h1 / h0))
}

/// Which range pattern two lines make: rise of each as a share of the height, and
/// the end height against the start height.
fn classify_range(up: f64, lo: f64, ratio: f64) -> Option<Kind> {
    let flat = |v: f64| v.abs() <= FLAT;
    let converging = ratio < 0.7;
    let parallel = (0.7..=1.4).contains(&ratio);
    if flat(up) && flat(lo) {
        return parallel.then_some(Kind::Rectangle);
    }
    if flat(up) && lo > FLAT {
        return converging.then_some(Kind::AscendingTriangle);
    }
    if flat(lo) && up < -FLAT {
        return converging.then_some(Kind::DescendingTriangle);
    }
    if up < -FLAT && lo > FLAT {
        return Some(Kind::SymmetricalTriangle);
    }
    if up > FLAT && lo > FLAT {
        return if converging {
            Some(Kind::RisingWedge)
        } else if parallel {
            Some(Kind::ChannelUp)
        } else {
            None
        };
    }
    if up < -FLAT && lo < -FLAT {
        return if converging {
            Some(Kind::FallingWedge)
        } else if parallel {
            Some(Kind::ChannelDown)
        } else {
            None
        };
    }
    None
}

/// Triangles, wedges, channels and rectangles over five to seven pivots ending at `j`.
fn ranges(p: &[Pivot], j: usize, bars: &[Bar], s: &Settings, out: &mut Vec<Found>) {
    let tol = s.tolerance;
    for k in (5..=7).rev() {
        if j + 1 < k {
            continue;
        }
        let w = &p[j + 1 - k..=j];
        let Some((upper, lower, up, lo, ratio)) = fit(w, bars, tol) else {
            continue;
        };
        let Some(kind) = classify_range(up, lo, ratio) else {
            continue;
        };
        let (x0, x1) = (w[0].bar, w[w.len() - 1].bar);
        let height = upper.at(x0) - lower.at(x0);
        out.push(found(
            kind,
            w,
            Watch {
                upper,
                lower,
                stop: None,
                patience: x1 - x0,
                height,
            },
            bars,
            s.depth,
        ));
        return;
    }
}

/// Flags and pennants: a pole, then a small range of three to five more pivots.
fn flags(p: &[Pivot], j: usize, bars: &[Bar], s: &Settings, out: &mut Vec<Found>) {
    let tol = s.tolerance;
    for k in (4..=6).rev() {
        if j + 1 < k + 1 {
            continue;
        }
        let base = j + 1 - k;
        let pole = (p[base - 1], p[base]);
        let w = &p[base..=j];
        // The pole ends on the range's first pivot.
        let up = pole.1.price > pole.0.price;
        let size = (pole.1.price - pole.0.price).abs();
        let legs_small = w
            .windows(2)
            .all(|l| (l[1].price - l[0].price).abs() <= 0.5 * size);
        let pole_bars = pole.1.bar - pole.0.bar;
        let rest_bars = w[w.len() - 1].bar - w[0].bar;
        if !legs_small || rest_bars > 3 * pole_bars {
            continue;
        }
        let Some((upper, lower, rise_up, rise_lo, ratio)) = fit(w, bars, 2.0 * tol) else {
            continue;
        };
        let kind = if ratio < 0.7 && rise_up <= FLAT && rise_lo >= -FLAT {
            if up {
                Kind::BullPennant
            } else {
                Kind::BearPennant
            }
        } else if (0.6..=1.4).contains(&ratio) && up && rise_up < 0.0 && rise_lo < 0.0 {
            Kind::BullFlag
        } else if (0.6..=1.4).contains(&ratio) && !up && rise_up > 0.0 && rise_lo > 0.0 {
            Kind::BearFlag
        } else {
            continue;
        };
        let stop = Some(pole.0.price);
        let mut points = vec![pole.0];
        points.extend_from_slice(w);
        out.push(found(
            kind,
            &points,
            Watch {
                upper,
                lower,
                stop,
                patience: rest_bars,
                height: size,
            },
            bars,
            s.depth,
        ));
        return;
    }
}

/// Lower is more specific: at one moment, more pivots win first, then this. The
/// last two lows of a descending triangle are a double bottom too, on less evidence.
fn rank(k: Kind) -> usize {
    KINDS.iter().position(|x| *x == k).unwrap_or(usize::MAX)
}

/// Chart patterns on closed `bars`, newest first. Pure. `choice` is "all" or a key.
///
/// Decided in the order things became known, so a later bar cannot rewrite a past
/// decision. A pattern still open when its last pivot moves is dropped, which is
/// what "forming" warns of; one already confirmed or failed stays. A newer pattern
/// overlapping an older one appears only if the older one failed by then, or is
/// still open with fewer pivots, in which case the newer one takes its place.
pub fn detect(bars: &[Bar], s: &Settings, choice: &str) -> Vec<Found> {
    let a = atr_of(bars);
    let mut kept: Vec<Found> = vec![];
    zigzag_walk(
        bars,
        s.depth,
        |i| threshold(bars, &a, i, s.deviation),
        |raw, moved| {
            let p: Vec<Pivot> = raw.iter().copied().map(to_pivot).collect();
            let j = p.len() - 1;
            let now = p[j].bar + s.depth;
            if moved {
                kept.retain(|f| {
                    // Its last pivot is the one that moved: it ended on an older bar.
                    let on_moved = f.points.last().is_some_and(|l| {
                        j > 0 && l.high == p[j].high && l.bar > p[j - 1].bar && l.bar <= p[j].bar
                    });
                    !on_moved
                        || f.confirmed_at.is_some_and(|at| at < now)
                        || f.failed_at.is_some_and(|at| at < now)
                });
            }
            let mut fresh = vec![];
            reversals(&p, j, bars, s, &mut fresh);
            flags(&p, j, bars, s, &mut fresh);
            ranges(&p, j, bars, s, &mut fresh);
            fresh.retain(|f| choice == "all" || f.kind.key() == choice);
            fresh.sort_by_key(|f| (std::cmp::Reverse(f.points.len()), rank(f.kind)));
            for f in fresh {
                let overlaps = |o: &Found| {
                    let shared = f.end.min(o.end) as f64 - f.start.max(o.start) as f64;
                    let shorter = (f.end - f.start).min(o.end - o.start).max(1) as f64;
                    shared / shorter > MAX_OVERLAP && o.failed_at.is_none_or(|at| at >= now)
                };
                let open = |o: &Found| o.confirmed_at.is_none_or(|at| at >= now);
                let blocked = kept
                    .iter()
                    .any(|o| overlaps(o) && (!open(o) || o.points.len() >= f.points.len()));
                if !blocked {
                    kept.retain(|o| !overlaps(o));
                    kept.push(f);
                }
            }
        },
    );
    kept.reverse();
    kept.truncate(MAX_PATTERNS);
    kept
}

/// A price in plain text: enough digits to read it, whatever its size.
pub(crate) fn price_text(p: f64) -> String {
    let a = p.abs();
    let digits = if a >= 1000.0 {
        0
    } else if a >= 10.0 {
        2
    } else if a >= 1.0 {
        3
    } else {
        6
    };
    format!("{p:.digits$}")
}

const FORMING_RULE: &str = "Forming: it can reshape or vanish when a new swing confirms. It is confirmed only after a later bar closes past its line.";

fn tooltip(f: &Found) -> String {
    let mut out = format!("{}: {}", f.kind.name(), f.kind.meaning());
    match f.status {
        PatternStatus::Forming => {
            out.push(' ');
            out.push_str(FORMING_RULE);
        }
        PatternStatus::Confirmed => {
            out.push_str(" Confirmed: a bar closed past its line.");
            if let Some(t) = f.target {
                out.push_str(&format!(" Measured move: {}.", price_text(t)));
            }
        }
        PatternStatus::Failed => {
            out.push_str(&format!(
                " Failed: {}.",
                f.failed_why.unwrap_or("closed through the far side")
            ));
        }
    }
    if f.status != PatternStatus::Failed {
        if let Some(s) = f.invalidation {
            out.push_str(&format!(" A close past {} fails it.", price_text(s)));
        }
    }
    out.push_str(" Read from closed bars; the forming bar changes nothing.");
    out
}

/// `f`'s outline and label. Indices are shifted by `base` into the caller's bars.
pub(crate) fn shape(f: &Found, base: usize, last: usize) -> PatternShape {
    let shift = |x: usize| (x + base) as i64;
    let mut strokes: Vec<Stroke> = f
        .points
        .windows(2)
        .map(|w| Stroke {
            x1: shift(w[0].bar),
            y1: w[0].price,
            x2: shift(w[1].bar),
            y2: w[1].price,
            dashed: false,
        })
        .collect();
    let line_end = f.confirmed_at.or(f.failed_at).unwrap_or(last).max(f.end);
    let line_start = f.points.first().map_or(f.start, |p| p.bar);
    let finite_line = |l: &TrendLine| l.at(line_start).is_finite() && l.at(line_end).is_finite();
    for l in [&f.upper, &f.lower].into_iter().flatten() {
        if finite_line(l) {
            strokes.push(Stroke {
                x1: shift(line_start),
                y1: l.at(line_start),
                x2: shift(line_end),
                y2: l.at(line_end),
                dashed: true,
            });
        }
    }
    // Over the highest point for a bearish reading, under the lowest otherwise.
    let bearish = f.bias == Bias::Bearish;
    let extreme =
        |pick: fn(f64, f64) -> f64, seed: f64| f.points.iter().map(|p| p.price).fold(seed, pick);
    let (label_y, label_above) = if bearish {
        (extreme(f64::max, f64::MIN), true)
    } else {
        (extreme(f64::min, f64::MAX), false)
    };
    let anchor = f.points[f.points.len() / 2].bar;
    PatternShape {
        key: f.kind.key().to_string(),
        name: f.kind.name().to_string(),
        bias: f.bias,
        status: f.status,
        strokes,
        label_x: shift(anchor),
        label_y,
        label_above,
        tooltip: tooltip(f),
        target: f.target,
        invalidation: f.invalidation,
        confirmed_x: f.confirmed_at.map(shift),
    }
}

/// The bars chart patterns read: closed ones, at most `lookback`, and where they start.
pub(crate) fn closed_window(data: &OhlcvData, lookback: usize) -> (&[Bar], usize) {
    let closed = data.len() - usize::from(newest_bar_forming(data, now_ms()) && !data.is_empty());
    let from = closed.saturating_sub(lookback);
    (&data.bars[from..closed], from)
}

fn choices() -> Vec<&'static str> {
    let mut out = vec!["all"];
    out.extend(KINDS.iter().map(|k| k.key()));
    out
}

pub struct ChartPatterns {
    pub pattern: String,
    pub settings: Settings,
    pub lookback: usize,
    pub show_failed: bool,
}

impl Default for ChartPatterns {
    fn default() -> Self {
        Self {
            pattern: "all".into(),
            settings: Settings::default(),
            lookback: LOOKBACK,
            show_failed: true,
        }
    }
}

impl ChartPatterns {
    fn result_on(&self, window: &[Bar], base: usize) -> PanelResult {
        let found = detect(window, &self.settings, &self.pattern);
        let last_closed = window.len().saturating_sub(1);
        let mut patterns = vec![];
        let mut dots = vec![];
        let (mut bull, mut bear) = (None::<i64>, None::<i64>);
        for f in &found {
            if let Some(at) = f.confirmed_at {
                let x = (at + base) as i64;
                let slot = if f.bias == Bias::Bullish {
                    &mut bull
                } else {
                    &mut bear
                };
                *slot = Some(slot.map_or(x, |o| o.max(x)));
                // Only the newest closed bar: the MCP `indicator_signal` alert
                // fires on any labeled dot it is handed.
                if at == last_closed && f.status == PatternStatus::Confirmed {
                    dots.push(Dot {
                        x: at + base,
                        y: window[at].close,
                        color: f.bias.color(),
                        size: 3,
                        label: Some(format!("{}_confirmed", f.kind.key())),
                    });
                }
            }
            if f.status == PatternStatus::Failed && !self.show_failed {
                continue;
            }
            patterns.push(shape(f, base, last_closed));
        }
        let event = |key: &str, name: &str, x: Option<i64>| IndicatorEvent {
            key: key.into(),
            name: name.into(),
            x,
        };
        let label = if self.pattern == "all" {
            "Chart Patterns".to_string()
        } else {
            format!("Chart Patterns: {}", self.pattern)
        };
        PanelResult {
            patterns,
            dots,
            events: vec![
                event("bullish", "Bullish pattern confirmed", bull),
                event("bearish", "Bearish pattern confirmed", bear),
            ],
            label,
            is_overlay: true,
            ..Default::default()
        }
    }
}

impl Indicator for ChartPatterns {
    fn name(&self) -> &str {
        "Chart Patterns"
    }

    fn description(&self) -> &str {
        "Auto chart patterns on ZigZag pivots of the last 600 bars: head and shoulders, double and triple tops and bottoms, triangles, wedges, channels, flags, pennants, rectangles. Each with a status: forming, confirmed or failed"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "pattern", "type": "string", "default": "all", "enum": choices(),
             "description": "Which patterns: all or one"},
            {"name": "depth", "type": "integer", "default": 5, "min": 1, "max": 100,
             "description": "ZigZag depth: bars on each side of a swing high or low. A swing is known this many bars after it"},
            {"name": "deviation", "type": "number", "default": 3.0, "min": 0, "max": 50,
             "description": "ZigZag deviation: least move between swings, in ATR(10)"},
            {"name": "tolerance", "type": "number", "default": 15.0, "min": 1, "max": 50,
             "description": "Same level, as a percentage of the pattern's height"},
            {"name": "lookback", "type": "integer", "default": 600, "min": 50, "max": 1000,
             "description": "Most recent bars searched"},
            {"name": "show_failed", "type": "boolean", "default": true,
             "description": "Keep failed patterns on the chart"}
        ])
    }

    fn configure(&mut self, p: &Value) {
        if let Some(v) = p.get("pattern").and_then(|v| v.as_str()) {
            if choices().contains(&v) {
                self.pattern = v.to_string();
            }
        }
        if let Some(v) = p.get("depth").and_then(|v| v.as_u64()) {
            self.settings.depth = (v as usize).max(1);
        }
        if let Some(v) = p.get("deviation").and_then(|v| v.as_f64()) {
            self.settings.deviation = v;
        }
        if let Some(v) = p.get("tolerance").and_then(|v| v.as_f64()) {
            self.settings.tolerance = v / 100.0;
        }
        if let Some(v) = p.get("lookback").and_then(|v| v.as_u64()) {
            self.lookback = (v as usize).max(1);
        }
        if let Some(v) = p.get("show_failed").and_then(|v| v.as_bool()) {
            self.show_failed = v;
        }
    }

    fn warmup_bars(&self) -> usize {
        // A live tail must see the same window as the full load (ADR-0046).
        self.lookback
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let (window, base) = closed_window(data, self.lookback);
        self.result_on(window, base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bars walking in straight lines through `(bar, price)` knots, with candles
    /// spreading 0.2 around the path, so swings fall exactly on the knots.
    fn path(knots: &[(usize, f64)]) -> Vec<Bar> {
        let mut bars = vec![];
        for w in knots.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            for x in x0..x1 {
                let c = y0 + (y1 - y0) * (x - x0) as f64 / (x1 - x0) as f64;
                bars.push(Bar {
                    open: c,
                    high: c + 0.1,
                    low: c - 0.1,
                    close: c,
                    volume: 1.0,
                    date: "x".into(),
                });
            }
        }
        let &(_, y) = knots.last().unwrap();
        bars.push(Bar {
            open: y,
            high: y + 0.1,
            low: y - 0.1,
            close: y,
            volume: 1.0,
            date: "x".into(),
        });
        bars
    }

    /// Knots at `gap`-bar steps through `prices`, after a lead-in that makes the
    /// first price a swing too. The last price only ends the last leg: no swing.
    fn knots(gap: usize, prices: &[f64]) -> Vec<(usize, f64)> {
        let lead = prices[0] + (prices[1] - prices[0]) / 2.0;
        std::iter::once((0, lead))
            .chain(prices.iter().enumerate().map(|(i, p)| ((i + 1) * gap, *p)))
            .collect()
    }

    fn settings() -> Settings {
        Settings {
            depth: 3,
            deviation: 3.0,
            tolerance: 0.15,
        }
    }

    fn only(prices: &[f64], kind: Kind) -> Found {
        let bars = path(&knots(10, prices));
        let all = detect(&bars, &settings(), "all");
        all.iter()
            .find(|f| f.kind == kind)
            .cloned()
            .unwrap_or_else(|| panic!("{kind:?} not in {:?}", kinds(&all)))
    }

    fn kinds(all: &[Found]) -> Vec<Kind> {
        all.iter().map(|f| f.kind).collect()
    }

    fn absent(prices: &[f64], kind: Kind) {
        let bars = path(&knots(10, prices));
        let all = detect(&bars, &settings(), "all");
        assert!(
            !kinds(&all).contains(&kind),
            "{kind:?} in {:?}",
            kinds(&all)
        );
    }

    // The last price ends the leg after the last pivot, and decides the status.
    const HS: [f64; 7] = [90.0, 110.0, 100.0, 120.0, 100.0, 110.0, 90.0];

    #[test]
    fn head_and_shoulders_confirmed_by_a_close_under_the_neck() {
        let f = only(&HS, Kind::HeadAndShoulders);
        assert_eq!(f.status, PatternStatus::Confirmed);
        assert_eq!(f.bias, Bias::Bearish);
        assert_eq!(f.points.len(), 5);
        assert_eq!(f.points[2].price, 120.1);
        assert_eq!(f.invalidation, Some(120.1));
        // Confirmed on the first close under 99.9, the neck: on the last leg.
        let at = f.confirmed_at.unwrap();
        assert!(at > 60 && at < 70, "{at}");
        assert!(f.target.unwrap() < 90.0, "{:?}", f.target);
    }

    #[test]
    fn head_and_shoulders_near_misses() {
        // The head is no higher than the shoulders.
        absent(
            &[90.0, 110.0, 100.0, 110.0, 100.0, 110.0, 90.0],
            Kind::HeadAndShoulders,
        );
        // One shoulder far from the other.
        absent(
            &[90.0, 104.0, 100.0, 120.0, 100.0, 116.0, 90.0],
            Kind::HeadAndShoulders,
        );
        // No rise into it: the first low is above the neckline.
        absent(
            &[105.0, 110.0, 100.0, 120.0, 100.0, 110.0, 90.0],
            Kind::HeadAndShoulders,
        );
    }

    #[test]
    fn head_and_shoulders_stays_forming_until_the_neck_breaks() {
        // The last leg stops above the neck.
        let f = only(
            &[90.0, 110.0, 100.0, 120.0, 100.0, 110.0, 103.0],
            Kind::HeadAndShoulders,
        );
        assert_eq!(f.status, PatternStatus::Forming);
        assert_eq!(f.confirmed_at, None);
        assert_eq!(f.target, None);
    }

    #[test]
    fn head_and_shoulders_fails_on_a_close_over_the_head() {
        // Up through the head without a close under the neck first.
        let f = only(
            &[90.0, 110.0, 100.0, 120.0, 100.0, 110.0, 105.0, 125.0],
            Kind::HeadAndShoulders,
        );
        assert_eq!(f.status, PatternStatus::Failed);
        assert_eq!(f.confirmed_at, None);
        assert!(f.failed_at.is_some());
    }

    #[test]
    fn inverse_head_and_shoulders_mirrors_the_top() {
        let f = only(
            &[130.0, 110.0, 120.0, 100.0, 120.0, 110.0, 130.0],
            Kind::InverseHeadAndShoulders,
        );
        assert_eq!(f.status, PatternStatus::Confirmed);
        assert_eq!(f.bias, Bias::Bullish);
        assert_eq!(f.invalidation, Some(99.9));
        assert!(f.target.unwrap() > 130.0);
        absent(
            &[130.0, 110.0, 120.0, 110.0, 120.0, 110.0, 130.0],
            Kind::InverseHeadAndShoulders,
        );
    }

    #[test]
    fn double_top_and_bottom() {
        let f = only(&[90.0, 110.0, 100.0, 110.0, 95.0], Kind::DoubleTop);
        assert_eq!(f.status, PatternStatus::Confirmed);
        assert_eq!(f.bias, Bias::Bearish);
        // Second peak far under the first.
        absent(&[90.0, 110.0, 100.0, 105.0, 95.0], Kind::DoubleTop);
        let f = only(&[120.0, 100.0, 110.0, 100.0, 115.0], Kind::DoubleBottom);
        assert_eq!(f.status, PatternStatus::Confirmed);
        assert_eq!(f.bias, Bias::Bullish);
        absent(&[120.0, 100.0, 110.0, 105.0, 115.0], Kind::DoubleBottom);
    }

    #[test]
    fn double_top_fails_on_a_close_over_the_peaks() {
        let f = only(&[90.0, 110.0, 100.0, 110.0, 106.0, 120.0], Kind::DoubleTop);
        assert_eq!(f.status, PatternStatus::Failed);
    }

    #[test]
    fn triple_top_and_bottom() {
        let f = only(
            &[90.0, 110.0, 100.0, 110.5, 100.0, 110.0, 95.0],
            Kind::TripleTop,
        );
        assert_eq!(f.status, PatternStatus::Confirmed);
        assert_eq!(f.bias, Bias::Bearish);
        // The middle peak stands out: that is a head, not a third top.
        absent(
            &[90.0, 110.0, 100.0, 118.0, 100.0, 110.0, 95.0],
            Kind::TripleTop,
        );
        let f = only(
            &[120.0, 100.0, 110.0, 100.5, 110.0, 100.0, 115.0],
            Kind::TripleBottom,
        );
        assert_eq!(f.status, PatternStatus::Confirmed);
        assert_eq!(f.bias, Bias::Bullish);
        absent(
            &[120.0, 100.0, 110.0, 92.0, 110.0, 100.0, 115.0],
            Kind::TripleBottom,
        );
    }

    #[test]
    fn triangles() {
        let f = only(
            &[100.0, 110.0, 102.0, 110.0, 105.0, 110.0, 107.0, 115.0],
            Kind::AscendingTriangle,
        );
        assert_eq!(f.status, PatternStatus::Confirmed);
        assert_eq!(f.bias, Bias::Bullish);
        let f = only(
            &[110.0, 100.0, 108.0, 100.0, 105.0, 100.0, 103.0, 95.0],
            Kind::DescendingTriangle,
        );
        assert_eq!(f.status, PatternStatus::Confirmed);
        assert_eq!(f.bias, Bias::Bearish);
        // A descending triangle broken upward failed.
        let f = only(
            &[110.0, 100.0, 108.0, 100.0, 105.0, 100.0, 103.0, 112.0],
            Kind::DescendingTriangle,
        );
        assert_eq!(f.status, PatternStatus::Failed);
        // Symmetrical: either side confirms, and the side sets the bias.
        let sym = [90.0, 120.0, 95.0, 115.0, 100.0, 110.0];
        let mut up = sym.to_vec();
        up.push(125.0);
        let f = only(&up, Kind::SymmetricalTriangle);
        assert_eq!(f.status, PatternStatus::Confirmed);
        assert_eq!(f.bias, Bias::Bullish);
        let mut down = sym.to_vec();
        down.push(90.0);
        let f = only(&down, Kind::SymmetricalTriangle);
        assert_eq!(f.bias, Bias::Bearish);
        // Widening instead of narrowing is none of them.
        absent(
            &[100.0, 105.0, 95.0, 110.0, 90.0, 115.0, 85.0, 120.0],
            Kind::SymmetricalTriangle,
        );
    }

    #[test]
    fn wedges_and_channels() {
        let f = only(
            &[100.0, 120.0, 112.0, 126.0, 121.0, 131.0, 128.0, 115.0],
            Kind::RisingWedge,
        );
        assert_eq!(f.status, PatternStatus::Confirmed);
        assert_eq!(f.bias, Bias::Bearish);
        let f = only(
            &[131.0, 111.0, 119.0, 105.0, 110.0, 100.0, 103.0, 118.0],
            Kind::FallingWedge,
        );
        assert_eq!(f.status, PatternStatus::Confirmed);
        assert_eq!(f.bias, Bias::Bullish);
        let f = only(
            &[100.0, 110.0, 105.0, 115.0, 110.0, 120.0, 115.0],
            Kind::ChannelUp,
        );
        assert_eq!(f.status, PatternStatus::Forming);
        let f = only(
            &[120.0, 110.0, 115.0, 105.0, 110.0, 100.0, 105.0],
            Kind::ChannelDown,
        );
        assert_eq!(f.status, PatternStatus::Forming);
        // One line steeper than the other by far: no channel.
        absent(
            &[100.0, 110.0, 101.0, 120.0, 102.0, 130.0, 115.0],
            Kind::ChannelUp,
        );
    }

    #[test]
    fn rectangle() {
        let f = only(
            &[100.0, 110.0, 100.0, 110.0, 100.0, 110.0, 118.0],
            Kind::Rectangle,
        );
        assert_eq!(f.bias, Bias::Bullish);
        assert_eq!(f.status, PatternStatus::Confirmed);
        // A sloped top is not a rectangle.
        absent(
            &[100.0, 110.0, 100.0, 114.0, 100.0, 118.0, 108.0],
            Kind::Rectangle,
        );
    }

    #[test]
    fn flags_and_pennants_need_a_pole() {
        // A 30-bar pole, then quick swings: a flag rests for less than the rise took.
        let with_pole = |prices: &[f64]| -> Vec<Found> {
            let mut k = vec![
                (0, prices[0] + (prices[1] - prices[0]) / 2.0),
                (10, prices[0]),
            ];
            k.extend(
                prices[1..]
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (40 + i * 8, *p)),
            );
            detect(&path(&k), &settings(), "all")
        };
        let of = |prices: &[f64], kind: Kind| -> Found {
            let all = with_pole(prices);
            all.iter()
                .find(|f| f.kind == kind)
                .cloned()
                .unwrap_or_else(|| panic!("{kind:?} not in {:?}", kinds(&all)))
        };
        let f = of(
            &[80.0, 120.0, 112.0, 118.0, 110.0, 116.0, 108.0, 130.0],
            Kind::BullFlag,
        );
        assert_eq!(f.status, PatternStatus::Confirmed);
        assert_eq!(f.bias, Bias::Bullish);
        assert_eq!(f.points[0].price, 79.9);
        // Without the pole, the same drift is only a channel.
        let weak = with_pole(&[106.0, 120.0, 112.0, 118.0, 110.0, 116.0, 108.0, 130.0]);
        assert!(
            !kinds(&weak).contains(&Kind::BullFlag),
            "{:?}",
            kinds(&weak)
        );
        let f = of(
            &[120.0, 80.0, 88.0, 82.0, 90.0, 84.0, 92.0, 70.0],
            Kind::BearFlag,
        );
        assert_eq!(f.bias, Bias::Bearish);
        assert_eq!(f.status, PatternStatus::Confirmed);
        let f = of(
            &[80.0, 120.0, 110.0, 118.0, 112.0, 116.0, 113.0, 130.0],
            Kind::BullPennant,
        );
        assert_eq!(f.status, PatternStatus::Confirmed);
        let f = of(
            &[120.0, 80.0, 90.0, 82.0, 88.0, 84.0, 87.0, 70.0],
            Kind::BearPennant,
        );
        assert_eq!(f.bias, Bias::Bearish);
    }

    #[test]
    fn a_range_with_a_close_far_outside_its_lines_is_not_a_range() {
        // A spike between pivots that is no pivot of its own.
        let mut bars = path(&knots(
            10,
            &[100.0, 110.0, 100.0, 110.0, 100.0, 110.0, 100.0],
        ));
        for i in 21..28 {
            bars[i].close = 130.0;
        }
        let all = detect(&bars, &settings(), "rectangle");
        assert!(all.is_empty(), "{:?}", kinds(&all));
    }

    #[test]
    fn a_pattern_waits_only_so_long_for_its_breakout() {
        // Sideways for far longer than the pattern took.
        let mut k = knots(10, &[90.0, 110.0, 100.0, 110.0, 104.0]);
        k.push((300, 104.5));
        let bars = path(&k);
        let f = detect(&bars, &settings(), "double_top");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].status, PatternStatus::Failed);
        assert_eq!(f[0].failed_why, Some("no breakout in time"));
    }

    #[test]
    fn a_confirmed_pattern_that_reverses_before_its_target_fails() {
        let f = only(&[90.0, 110.0, 100.0, 110.0, 97.0, 120.0], Kind::DoubleTop);
        assert!(f.confirmed_at.is_some());
        assert_eq!(f.status, PatternStatus::Failed);
        assert_eq!(f.failed_why, Some("reversed before its target"));
    }

    #[test]
    fn a_confirmed_pattern_that_reached_its_target_stays_confirmed() {
        let f = only(&[90.0, 110.0, 100.0, 110.0, 80.0, 120.0], Kind::DoubleTop);
        assert_eq!(f.status, PatternStatus::Confirmed);
    }

    #[test]
    fn a_breakout_before_the_last_pivot_is_known_does_not_count_yet() {
        // Straight through the neck within `depth` bars of the right shoulder.
        let mut k = knots(10, &[90.0, 110.0, 100.0, 120.0, 100.0, 110.0]);
        let (rs, _) = *k.last().unwrap();
        k.push((rs + 2, 98.0));
        k.push((rs + 30, 60.0));
        let bars = path(&k);
        let s = Settings {
            depth: 4,
            ..settings()
        };
        let f = detect(&bars, &s, "head_and_shoulders");
        assert_eq!(f.len(), 1, "{:?}", kinds(&f));
        assert_eq!(f[0].known_at, rs + 4);
        // The close under the neck came first; the pattern confirms when known.
        assert_eq!(f[0].confirmed_at, Some(rs + 4));
    }

    /// `n` bars of a repeatable random walk, with no structure in it.
    fn walk(seed: u64, n: usize) -> Vec<Bar> {
        let mut state = seed;
        let mut unit = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as f64 + 0.5) / (1u64 << 31) as f64
        };
        let mut close = 100.0_f64;
        (0..n)
            .map(|_| {
                let open = close;
                let z = (-2.0 * unit().ln()).sqrt() * (std::f64::consts::TAU * unit()).cos();
                close = open * (1.0 + 0.01 * z);
                Bar {
                    open,
                    high: open.max(close) * (1.0 + 0.005 * unit()),
                    low: open.min(close) * (1.0 - 0.005 * unit()),
                    close,
                    volume: 1.0,
                    date: "x".into(),
                }
            })
            .collect()
    }

    #[test]
    fn more_bars_never_change_a_past_confirmation() {
        let s = Settings::default();
        let mut checked = 0;
        for seed in 1..=6 {
            let bars = walk(seed, 900);
            let full = detect(&bars, &s, "all");
            for cut in (300..900).step_by(37) {
                for f in detect(&bars[..cut], &s, "all") {
                    let Some(at) = f.confirmed_at else { continue };
                    checked += 1;
                    let same = full.iter().any(|o| {
                        o.kind == f.kind && o.confirmed_at == Some(at) && o.start == f.start
                    });
                    // At most MAX_PATTERNS are reported: older ones can age out.
                    let aged_out = full.len() == MAX_PATTERNS
                        && full.last().is_some_and(|o| o.known_at > f.known_at);
                    assert!(
                        same || aged_out,
                        "seed {seed} cut {cut}: {f:?} gone from {:?}",
                        kinds(&full)
                    );
                }
            }
        }
        assert!(checked > 50, "too few to mean anything: {checked}");
    }

    #[test]
    fn on_a_random_walk_patterns_are_rare_and_no_better_than_chance() {
        // A hindsight leak shows up here first: confirmations "right" far too often.
        let s = Settings::default();
        let (mut bars_seen, mut found, mut judged, mut right) = (0usize, 0usize, 0usize, 0usize);
        for seed in 1..=40 {
            let bars = walk(seed * 7919, LOOKBACK);
            bars_seen += bars.len();
            for f in detect(&bars, &s, "all") {
                found += 1;
                let Some(at) = f.confirmed_at else { continue };
                let Some(later) = bars.get(at + 10) else {
                    continue;
                };
                let up = later.close > bars[at].close;
                match f.bias {
                    Bias::Bullish => right += usize::from(up),
                    Bias::Bearish => right += usize::from(!up),
                    Bias::Neutral => continue,
                }
                judged += 1;
            }
        }
        let per_1000 = 1000.0 * found as f64 / bars_seen as f64;
        assert!(per_1000 < 20.0, "{per_1000} per 1000 bars");
        assert!(judged >= 50, "too few to mean anything: {judged}");
        let rate = right as f64 / judged as f64;
        assert!((0.38..=0.62).contains(&rate), "right {rate} of {judged}");
    }

    #[test]
    fn an_open_pattern_moves_with_its_last_pivot() {
        let k = knots(10, &[90.0, 110.0, 100.0, 110.0]);
        let (top, _) = *k.last().unwrap();
        let mut more = k.clone();
        more.push((top + 4, 108.5));
        let seen = detect(&path(&more), &settings(), "double_top");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].end, top);
        // A slightly higher peak, with no low confirmed since, replaces the pivot.
        more.push((top + 8, 111.0));
        more.push((top + 16, 106.0));
        let moved = detect(&path(&more), &settings(), "double_top");
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].end, top + 8);
    }

    #[test]
    fn overlapping_patterns_are_only_shown_after_the_older_one_failed() {
        let s = Settings::default();
        for seed in 1..=4 {
            let bars = walk(seed, 900);
            let all = detect(&bars, &s, "all");
            for (i, a) in all.iter().enumerate() {
                for b in &all[i + 1..] {
                    // `all` is newest first: `b` was decided before `a`.
                    assert!(b.known_at <= a.known_at);
                    let shared = a.end.min(b.end) as f64 - a.start.max(b.start) as f64;
                    let shorter = (a.end - a.start).min(b.end - b.start).max(1) as f64;
                    if shared / shorter > MAX_OVERLAP {
                        // Only if `b` had failed by the time `a` appeared.
                        assert!(
                            b.failed_at.is_some_and(|at| at < a.known_at),
                            "{a:?} over {b:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn choosing_one_pattern_filters_the_others() {
        let bars = path(&knots(10, &HS));
        assert!(detect(&bars, &settings(), "triple_top").is_empty());
        let one = detect(&bars, &settings(), "head_and_shoulders");
        assert_eq!(kinds(&one), [Kind::HeadAndShoulders]);
    }

    #[test]
    fn deviation_filters_small_swings() {
        let bars = path(&knots(10, &HS));
        let few = pivots(&bars, 3, 500.0);
        let many = pivots(&bars, 3, 3.0);
        assert!(few.len() < many.len(), "{} vs {}", few.len(), many.len());
        assert_eq!(many.len(), HS.len() - 1);
    }

    #[test]
    fn nothing_on_a_flat_or_short_series() {
        assert!(detect(&[], &settings(), "all").is_empty());
        let bars = path(&[(0, 100.0), (2, 100.0)]);
        assert!(detect(&bars, &settings(), "all").is_empty());
    }

    #[test]
    fn the_shape_has_an_outline_a_label_and_a_tooltip_with_its_status() {
        let f = only(&HS, Kind::HeadAndShoulders);
        let s = shape(&f, 100, 60);
        assert_eq!(s.key, "head_and_shoulders");
        assert_eq!(s.status, PatternStatus::Confirmed);
        // Four pivot legs and the dashed neckline.
        assert_eq!(s.strokes.iter().filter(|t| !t.dashed).count(), 4);
        assert_eq!(s.strokes.iter().filter(|t| t.dashed).count(), 1);
        assert_eq!(s.strokes[0].x1, 120);
        assert!(s.label_above);
        assert_eq!(s.label_y, 120.1);
        assert_eq!(s.confirmed_x, f.confirmed_at.map(|x| x as i64 + 100));
        assert!(s.tooltip.contains("Confirmed"), "{}", s.tooltip);
        let forming = only(
            &[90.0, 110.0, 100.0, 120.0, 100.0, 110.0, 103.0],
            Kind::HeadAndShoulders,
        );
        let t = shape(&forming, 0, 60).tooltip;
        assert!(t.contains("can reshape or vanish"), "{t}");
    }

    #[test]
    fn the_forming_bar_changes_nothing() {
        // A timed series whose newest bar is in the future: forming.
        let mut bars = path(&knots(10, &HS));
        let now = now_ms() / 1000;
        let n = bars.len() as i64;
        for (i, b) in bars.iter_mut().enumerate() {
            b.date = (now - (n - 1 - i as i64) * 3600).to_string();
        }
        let data = |bars: Vec<Bar>| OhlcvData {
            bars,
            symbol: None,
            interval: Some("1h".into()),
            session: None,
        };
        let mut wild = bars.clone();
        let last = wild.last_mut().unwrap();
        // A forming bar far over the head would fail the pattern if it counted.
        last.close = 500.0;
        last.high = 500.0;
        let ind = ChartPatterns {
            settings: settings(),
            ..Default::default()
        };
        let a = ind.compute(&data(bars));
        let b = ind.compute(&data(wild));
        let brief = |r: &PanelResult| -> Vec<(String, PatternStatus)> {
            r.patterns
                .iter()
                .map(|p| (p.key.clone(), p.status))
                .collect()
        };
        assert!(!a.patterns.is_empty());
        assert_eq!(brief(&a), brief(&b));
        assert_eq!(a.events, b.events);
    }

    #[test]
    fn events_name_the_newest_confirmation_and_dots_only_the_newest_bar() {
        let bars = path(&knots(10, &HS));
        let found = detect(&bars, &settings(), "head_and_shoulders");
        let at = found[0].confirmed_at.unwrap();
        let ind = ChartPatterns {
            settings: settings(),
            pattern: "head_and_shoulders".into(),
            ..Default::default()
        };
        // The window ends on the confirming bar: its dot is there.
        let data = OhlcvData {
            bars: bars[..=at].to_vec(),
            symbol: None,
            interval: None,
            session: None,
        };
        let r = ind.compute(&data);
        assert_eq!(r.events[1].x, Some(at as i64));
        assert_eq!(r.events[0].x, None);
        assert_eq!(r.dots.len(), 1);
        assert_eq!(
            r.dots[0].label.as_deref(),
            Some("head_and_shoulders_confirmed")
        );
        // A later bar: the event stays, the dot goes.
        let data = OhlcvData {
            bars: bars[..at + 2].to_vec(),
            symbol: None,
            interval: None,
            session: None,
        };
        let r = ind.compute(&data);
        assert_eq!(r.events[1].x, Some(at as i64));
        assert!(r.dots.is_empty());
    }

    #[test]
    fn indices_are_those_of_the_full_series_past_the_lookback() {
        let mut bars = path(&[(0, 90.0), (400, 90.0)]);
        bars.pop();
        let tail = path(&knots(10, &HS));
        let n0 = bars.len();
        bars.extend(tail);
        let ind = ChartPatterns {
            settings: settings(),
            lookback: 70,
            pattern: "head_and_shoulders".into(),
            ..Default::default()
        };
        let data = OhlcvData {
            bars,
            symbol: None,
            interval: None,
            session: None,
        };
        let r = ind.compute(&data);
        assert_eq!(r.patterns.len(), 1);
        assert_eq!(r.patterns[0].strokes[0].x1, n0 as i64 + 20);
    }

    #[test]
    fn settings_reach_the_detector() {
        let mut ind = ChartPatterns::default();
        ind.configure(
            &json!({"depth": 8, "deviation": 1.5, "tolerance": 20.0, "lookback": 100,
                              "pattern": "rectangle", "show_failed": false}),
        );
        assert_eq!(ind.settings.depth, 8);
        assert_eq!(ind.settings.deviation, 1.5);
        assert!((ind.settings.tolerance - 0.2).abs() < 1e-9);
        assert_eq!(ind.pattern, "rectangle");
        assert_eq!(ind.warmup_bars(), 100);
        assert!(!ind.show_failed);
        ind.configure(&json!({"pattern": "nonsense"}));
        assert_eq!(ind.pattern, "rectangle");
    }
}
