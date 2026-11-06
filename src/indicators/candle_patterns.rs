//! Candlestick pattern recognition (ADR-0046).
//!
//! `detect` is pure: bars in, hits out. Shape rules follow the conventions of
//! TradingView's published Pine candlestick library (body against an EMA(14) of
//! bodies, a 5% doji body, 5% "no shadow"); TradingView does not publish the list
//! its own indicators cover, so this is not a claim of equivalence.
//!
//! A hit sits on the last bar of its pattern. Bars are read only up to that bar,
//! so a closed bar's hit never changes. The forming bar's hit is `tentative`.

use super::pine::{ema_n, sma_n};
use crate::data::{Bar, OhlcvData};
use crate::indicator::*;
use serde_json::{json, Value};

/// Prior trend a pattern needs, read on the bar before the pattern starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Needs {
    Downtrend,
    Uptrend,
    Nothing,
}

/// The `trend` setting: what counts as the prior trend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrendRule {
    /// Close against its SMA 50.
    Sma50,
    /// Close against its SMA 50, and the SMA 50 against the SMA 200.
    Sma50And200,
    /// Every bar counts as both, so the shape alone decides.
    None,
}

impl TrendRule {
    fn parse(s: &str) -> Self {
        match s {
            "sma50_200" => TrendRule::Sma50And200,
            "none" => TrendRule::None,
            _ => TrendRule::Sma50,
        }
    }

    /// History the rule needs before its first reading.
    pub fn warmup(self) -> usize {
        match self {
            TrendRule::Sma50 => 50,
            TrendRule::Sma50And200 => 200,
            TrendRule::None => 0,
        }
    }
}

/// The shape of one candle and the measures shape rules compare it with.
#[derive(Clone, Copy)]
struct C {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    range: f64,
    hi: f64,
    lo: f64,
    body: f64,
    avg: f64,
    up: f64,
    dn: f64,
}

impl C {
    fn new(b: &Bar, avg: f64) -> Self {
        let (hi, lo) = (b.close.max(b.open), b.close.min(b.open));
        C {
            open: b.open,
            high: b.high,
            low: b.low,
            close: b.close,
            range: b.high - b.low,
            hi,
            lo,
            body: hi - lo,
            avg,
            up: b.high - hi,
            dn: lo - b.low,
        }
    }
    fn white(&self) -> bool {
        self.close > self.open
    }
    fn black(&self) -> bool {
        self.close < self.open
    }
    fn long(&self) -> bool {
        self.body > self.avg
    }
    fn small(&self) -> bool {
        self.body < self.avg
    }
    fn mid(&self) -> f64 {
        self.lo + self.body / 2.0
    }
    fn hl2(&self) -> f64 {
        (self.high + self.low) / 2.0
    }
    fn has_up(&self) -> bool {
        self.up > 0.05 * self.body
    }
    fn has_dn(&self) -> bool {
        self.dn > 0.05 * self.body
    }
    fn doji_body(&self) -> bool {
        self.range > 0.0 && self.body <= self.range * 0.05
    }
    fn shadows_equal(&self) -> bool {
        self.up == self.dn
            || ((self.up - self.dn).abs() < self.dn && (self.up - self.dn).abs() < self.up)
    }
    fn marubozu(&self) -> bool {
        self.long() && self.up <= 0.05 * self.range && self.dn <= 0.05 * self.range
    }
    /// A tolerance for "the same price": 5% of the average body.
    fn near(&self) -> f64 {
        self.avg * 0.05
    }
}

/// `w[0]` is the pattern's last candle, `w[k]` the one `k` bars before it.
type Rule = fn(&[C]) -> bool;

/// One pattern: how it reads and what it needs.
pub struct Def {
    pub key: &'static str,
    pub name: &'static str,
    /// Text on the chart. The color tells a bullish one from a bearish one.
    pub text: &'static str,
    pub bias: Bias,
    pub needs: Needs,
    /// Candles the pattern spans.
    pub bars: usize,
    pub meaning: &'static str,
    rule: Rule,
}

fn doji(c: &C) -> bool {
    c.doji_body() && c.shadows_equal()
}
fn long_legged(c: &C) -> bool {
    doji(c) && c.up > 2.0 * c.avg && c.dn > 2.0 * c.avg
}
fn dragonfly(c: &C) -> bool {
    c.doji_body() && c.up <= c.range * 0.05
}
fn gravestone(c: &C) -> bool {
    c.doji_body() && c.dn <= c.range * 0.05
}
fn hammer_shape(c: &C) -> bool {
    c.small() && c.body > 0.0 && c.lo > c.hl2() && c.dn >= 2.0 * c.body && !c.has_up()
}
fn inverted_shape(c: &C) -> bool {
    c.small() && c.body > 0.0 && c.hi < c.hl2() && c.up >= 2.0 * c.body && !c.has_dn()
}
fn soldier(c: &C) -> bool {
    c.long() && c.white() && c.up < 0.05 * c.range
}
fn crow(c: &C) -> bool {
    c.long() && c.black() && c.dn < 0.05 * c.range
}
fn engulfs_up(c: &C, p: &C) -> bool {
    c.white()
        && c.long()
        && p.black()
        && p.small()
        && c.close >= p.open
        && c.open <= p.close
        && (c.close > p.open || c.open < p.close)
}
fn engulfs_down(c: &C, p: &C) -> bool {
    c.black()
        && c.long()
        && p.white()
        && p.small()
        && c.close <= p.open
        && c.open >= p.close
        && (c.close < p.open || c.open > p.close)
}
fn inside_up(c: &C, p: &C) -> bool {
    p.long() && p.black() && c.white() && c.small() && c.high <= p.hi && c.low >= p.lo
}
fn inside_down(c: &C, p: &C) -> bool {
    p.long() && p.white() && c.black() && c.small() && c.high <= p.hi && c.low >= p.lo
}

macro_rules! def {
    ($key:literal, $name:literal, $text:literal, $bias:ident, $needs:ident, $bars:literal, $meaning:literal, $rule:expr) => {
        Def {
            key: $key,
            name: $name,
            text: $text,
            bias: Bias::$bias,
            needs: Needs::$needs,
            bars: $bars,
            meaning: $meaning,
            rule: $rule,
        }
    };
}

/// Every pattern. The order is the order of hits on one bar.
pub static PATTERNS: &[Def] =
    &[
        def!(
            "doji",
            "Doji",
            "D",
            Neutral,
            Nothing,
            1,
            "Open and close almost equal: buyers and sellers are in balance.",
            |w| doji(&w[0]) && !long_legged(&w[0]) && !dragonfly(&w[0]) && !gravestone(&w[0])
        ),
        def!(
            "long_legged_doji",
            "Long-Legged Doji",
            "LLD",
            Neutral,
            Nothing,
            1,
            "A doji with long shadows both ways: a wide fight that ended level.",
            |w| long_legged(&w[0])
        ),
        def!(
            "dragonfly_doji",
            "Dragonfly Doji",
            "DD",
            Bullish,
            Nothing,
            1,
            "Sellers pushed price down and buyers took all of it back.",
            |w| dragonfly(&w[0]) && w[0].dn > 0.0
        ),
        def!(
            "gravestone_doji",
            "Gravestone Doji",
            "GD",
            Bearish,
            Nothing,
            1,
            "Buyers pushed price up and sellers took all of it back.",
            |w| gravestone(&w[0]) && w[0].up > 0.0
        ),
        def!(
            "doji_star_bullish",
            "Doji Star",
            "DS",
            Bullish,
            Downtrend,
            2,
            "After a long black candle, a doji gaps below it: the fall may be stalling.",
            |w| w[1].long() && w[1].black() && w[0].doji_body() && w[0].hi < w[1].lo
        ),
        def!(
            "doji_star_bearish",
            "Doji Star",
            "DS",
            Bearish,
            Uptrend,
            2,
            "After a long white candle, a doji gaps above it: the rise may be stalling.",
            |w| w[1].long() && w[1].white() && w[0].doji_body() && w[0].lo > w[1].hi
        ),
        def!(
            "tri_star_bullish",
            "Tri-Star",
            "TS",
            Bullish,
            Downtrend,
            3,
            "Three dojis, the middle one gapped below: a rare sign that a fall is ending.",
            |w| w[2].doji_body()
                && w[1].doji_body()
                && w[0].doji_body()
                && w[1].hi < w[2].lo
                && w[0].lo > w[1].hi
        ),
        def!(
            "tri_star_bearish",
            "Tri-Star",
            "TS",
            Bearish,
            Uptrend,
            3,
            "Three dojis, the middle one gapped above: a rare sign that a rise is ending.",
            |w| w[2].doji_body()
                && w[1].doji_body()
                && w[0].doji_body()
                && w[1].lo > w[2].hi
                && w[0].hi < w[1].lo
        ),
        def!(
            "spinning_top",
            "Spinning Top",
            "ST",
            Neutral,
            Nothing,
            1,
            "A small body between shadows on both sides: indecision.",
            |w| !w[0].doji_body()
                && w[0].small()
                && w[0].up >= w[0].range * 0.34
                && w[0].dn >= w[0].range * 0.34
        ),
        def!(
            "hammer",
            "Hammer",
            "H",
            Bullish,
            Downtrend,
            1,
            "After a fall, a long lower shadow under a small body: buyers rejected the lows.",
            |w| hammer_shape(&w[0])
        ),
        def!("hanging_man", "Hanging Man", "HM", Bearish, Uptrend, 1,
        "After a rise, a hammer's shape: selling showed up, and needs a weak next bar to count.",
        |w| hammer_shape(&w[0])),
        def!(
            "inverted_hammer",
            "Inverted Hammer",
            "IH",
            Bullish,
            Downtrend,
            1,
            "After a fall, a long upper shadow over a small body: buyers tested higher prices.",
            |w| inverted_shape(&w[0])
        ),
        def!(
            "shooting_star",
            "Shooting Star",
            "SS",
            Bearish,
            Uptrend,
            1,
            "After a rise, a long upper shadow over a small body: sellers rejected the highs.",
            |w| inverted_shape(&w[0])
        ),
        def!(
            "marubozu_white",
            "Marubozu",
            "M",
            Bullish,
            Nothing,
            1,
            "A long white candle with almost no shadows: buyers held control all bar.",
            |w| w[0].white() && w[0].marubozu()
        ),
        def!(
            "marubozu_black",
            "Marubozu",
            "M",
            Bearish,
            Nothing,
            1,
            "A long black candle with almost no shadows: sellers held control all bar.",
            |w| w[0].black() && w[0].marubozu()
        ),
        def!(
            "long_lower_shadow",
            "Long Lower Shadow",
            "LLS",
            Bullish,
            Nothing,
            1,
            "The lower shadow is over three quarters of the range: lows were rejected.",
            |w| !w[0].doji_body() && w[0].range > 0.0 && w[0].dn > w[0].range * 0.75
        ),
        def!(
            "long_upper_shadow",
            "Long Upper Shadow",
            "LUS",
            Bearish,
            Nothing,
            1,
            "The upper shadow is over three quarters of the range: highs were rejected.",
            |w| !w[0].doji_body() && w[0].range > 0.0 && w[0].up > w[0].range * 0.75
        ),
        def!(
            "belt_hold_bullish",
            "Belt Hold",
            "BH",
            Bullish,
            Downtrend,
            1,
            "After a fall, a long white candle opens at its low and never looks back.",
            |w| w[0].long() && w[0].white() && w[0].dn <= 0.05 * w[0].range
        ),
        def!(
            "belt_hold_bearish",
            "Belt Hold",
            "BH",
            Bearish,
            Uptrend,
            1,
            "After a rise, a long black candle opens at its high and never looks back.",
            |w| w[0].long() && w[0].black() && w[0].up <= 0.05 * w[0].range
        ),
        def!(
            "engulfing_bullish",
            "Engulfing",
            "E",
            Bullish,
            Downtrend,
            2,
            "A long white body swallows the small black body before it: buyers took over.",
            |w| engulfs_up(&w[0], &w[1])
        ),
        def!(
            "engulfing_bearish",
            "Engulfing",
            "E",
            Bearish,
            Uptrend,
            2,
            "A long black body swallows the small white body before it: sellers took over.",
            |w| engulfs_down(&w[0], &w[1])
        ),
        def!(
            "harami_bullish",
            "Harami",
            "HR",
            Bullish,
            Downtrend,
            2,
            "A small white candle inside the long black body before it: the fall is losing force.",
            |w| inside_up(&w[0], &w[1])
        ),
        def!(
            "harami_bearish",
            "Harami",
            "HR",
            Bearish,
            Uptrend,
            2,
            "A small black candle inside the long white body before it: the rise is losing force.",
            |w| inside_down(&w[0], &w[1])
        ),
        def!(
            "harami_cross_bullish",
            "Harami Cross",
            "HC",
            Bullish,
            Downtrend,
            2,
            "A doji inside the long black body before it: the fall may be stalling.",
            |w| w[1].long()
                && w[1].black()
                && w[0].doji_body()
                && w[0].high <= w[1].hi
                && w[0].low >= w[1].lo
        ),
        def!(
            "harami_cross_bearish",
            "Harami Cross",
            "HC",
            Bearish,
            Uptrend,
            2,
            "A doji inside the long white body before it: the rise may be stalling.",
            |w| w[1].long()
                && w[1].white()
                && w[0].doji_body()
                && w[0].high <= w[1].hi
                && w[0].low >= w[1].lo
        ),
        def!(
            "piercing",
            "Piercing Line",
            "P",
            Bullish,
            Downtrend,
            2,
            "Opens below the prior low and closes above the middle of the long black body.",
            |w| w[1].long()
                && w[1].black()
                && w[0].long()
                && w[0].white()
                && w[0].open <= w[1].low
                && w[0].close > w[1].mid()
                && w[0].close < w[1].open
        ),
        def!(
            "dark_cloud_cover",
            "Dark Cloud Cover",
            "DC",
            Bearish,
            Uptrend,
            2,
            "Opens above the prior high and closes below the middle of the long white body.",
            |w| w[1].long()
                && w[1].white()
                && w[0].long()
                && w[0].black()
                && w[0].open >= w[1].high
                && w[0].close < w[1].mid()
                && w[0].close > w[1].open
        ),
        def!("on_neck", "On Neck", "ON", Bearish, Downtrend, 2,
        "A small white candle gaps down and only reaches the prior low: the fall likely goes on.",
        |w| w[1].long() && w[1].black() && w[0].white() && w[0].small()
            && w[0].open < w[1].close && (w[0].close - w[1].low).abs() <= w[0].near()),
        def!(
            "tweezer_bottom",
            "Tweezer Bottom",
            "TB",
            Bullish,
            Downtrend,
            2,
            "Two candles with the same low: the level held twice.",
            |w| (!w[0].doji_body() || (w[0].has_up() && w[0].has_dn()))
                && (w[0].low - w[1].low).abs() <= w[0].near()
                && w[1].black()
                && w[1].long()
                && w[0].white()
        ),
        def!(
            "tweezer_top",
            "Tweezer Top",
            "TT",
            Bearish,
            Uptrend,
            2,
            "Two candles with the same high: the level held twice.",
            |w| (!w[0].doji_body() || (w[0].has_up() && w[0].has_dn()))
                && (w[0].high - w[1].high).abs() <= w[0].near()
                && w[1].white()
                && w[1].long()
                && w[0].black()
        ),
        def!(
            "kicking_bullish",
            "Kicking",
            "K",
            Bullish,
            Nothing,
            2,
            "A black marubozu, then a white one gapping clear above it: a sharp change of side.",
            |w| w[1].black()
                && w[1].marubozu()
                && w[0].white()
                && w[0].marubozu()
                && w[0].low > w[1].high
        ),
        def!(
            "kicking_bearish",
            "Kicking",
            "K",
            Bearish,
            Nothing,
            2,
            "A white marubozu, then a black one gapping clear below it: a sharp change of side.",
            |w| w[1].white()
                && w[1].marubozu()
                && w[0].black()
                && w[0].marubozu()
                && w[0].high < w[1].low
        ),
        def!(
            "rising_window",
            "Rising Window",
            "RW",
            Bullish,
            Uptrend,
            2,
            "A gap up between two bars in a rise: the gap often acts as support.",
            |w| w[0].range > 0.0 && w[1].range > 0.0 && w[0].low > w[1].high
        ),
        def!(
            "falling_window",
            "Falling Window",
            "FW",
            Bearish,
            Downtrend,
            2,
            "A gap down between two bars in a fall: the gap often acts as resistance.",
            |w| w[0].range > 0.0 && w[1].range > 0.0 && w[0].high < w[1].low
        ),
        def!(
            "morning_star",
            "Morning Star",
            "MS",
            Bullish,
            Downtrend,
            3,
            "Long black, a small candle gapping below it, long white closing into the first body.",
            |w| w[2].long()
                && w[2].black()
                && w[1].small()
                && w[1].hi < w[2].lo
                && !w[1].doji_body()
                && w[0].long()
                && w[0].white()
                && w[0].hi >= w[2].lo
                && w[0].hi <= w[2].hi
                && w[0].hi > w[2].mid()
        ),
        def!(
            "evening_star",
            "Evening Star",
            "ES",
            Bearish,
            Uptrend,
            3,
            "Long white, a small candle gapping above it, long black closing into the first body.",
            |w| w[2].long()
                && w[2].white()
                && w[1].small()
                && w[1].lo > w[2].hi
                && !w[1].doji_body()
                && w[0].long()
                && w[0].black()
                && w[0].lo <= w[2].hi
                && w[0].lo >= w[2].lo
                && w[0].lo < w[2].mid()
        ),
        def!(
            "morning_doji_star",
            "Morning Doji Star",
            "MDS",
            Bullish,
            Downtrend,
            3,
            "A morning star whose middle candle is a doji.",
            |w| w[2].long()
                && w[2].black()
                && w[1].doji_body()
                && w[1].hi < w[2].lo
                && w[0].long()
                && w[0].white()
                && w[0].hi > w[2].mid()
        ),
        def!(
            "evening_doji_star",
            "Evening Doji Star",
            "EDS",
            Bearish,
            Uptrend,
            3,
            "An evening star whose middle candle is a doji.",
            |w| w[2].long()
                && w[2].white()
                && w[1].doji_body()
                && w[1].lo > w[2].hi
                && w[0].long()
                && w[0].black()
                && w[0].lo < w[2].mid()
        ),
        def!(
            "abandoned_baby_bullish",
            "Abandoned Baby",
            "AB",
            Bullish,
            Downtrend,
            3,
            "A doji with gaps on both sides of it, shadows included: a rare, sharp turn up.",
            |w| w[2].long()
                && w[2].black()
                && w[1].doji_body()
                && w[0].white()
                && w[2].low > w[1].high
                && w[0].low > w[1].high
        ),
        def!(
            "abandoned_baby_bearish",
            "Abandoned Baby",
            "AB",
            Bearish,
            Uptrend,
            3,
            "A doji with gaps on both sides of it, shadows included: a rare, sharp turn down.",
            |w| w[2].long()
                && w[2].white()
                && w[1].doji_body()
                && w[0].black()
                && w[2].high < w[1].low
                && w[0].high < w[1].low
        ),
        def!(
            "three_white_soldiers",
            "Three White Soldiers",
            "3WS",
            Bullish,
            Nothing,
            3,
            "Three long white candles, each opening inside the last body and closing higher.",
            |w| soldier(&w[0])
                && soldier(&w[1])
                && soldier(&w[2])
                && w[0].close > w[1].close
                && w[1].close > w[2].close
                && w[0].open > w[1].open
                && w[0].open < w[1].close
                && w[1].open > w[2].open
                && w[1].open < w[2].close
        ),
        def!(
            "three_black_crows",
            "Three Black Crows",
            "3BC",
            Bearish,
            Nothing,
            3,
            "Three long black candles, each opening inside the last body and closing lower.",
            |w| crow(&w[0])
                && crow(&w[1])
                && crow(&w[2])
                && w[0].close < w[1].close
                && w[1].close < w[2].close
                && w[0].open < w[1].open
                && w[0].open > w[1].close
                && w[1].open < w[2].open
                && w[1].open > w[2].close
        ),
        def!(
            "three_inside_up",
            "Three Inside Up",
            "3IU",
            Bullish,
            Downtrend,
            3,
            "A bullish harami, then a white close above the first candle's open: confirmed turn.",
            |w| inside_up(&w[1], &w[2]) && w[0].white() && w[0].close > w[2].open
        ),
        def!(
            "three_inside_down",
            "Three Inside Down",
            "3ID",
            Bearish,
            Uptrend,
            3,
            "A bearish harami, then a black close below the first candle's open: confirmed turn.",
            |w| inside_down(&w[1], &w[2]) && w[0].black() && w[0].close < w[2].open
        ),
        def!(
            "three_outside_up",
            "Three Outside Up",
            "3OU",
            Bullish,
            Downtrend,
            3,
            "A bullish engulfing, then a white close above it: confirmed turn.",
            |w| engulfs_up(&w[1], &w[2]) && w[0].white() && w[0].close > w[1].close
        ),
        def!(
            "three_outside_down",
            "Three Outside Down",
            "3OD",
            Bearish,
            Uptrend,
            3,
            "A bearish engulfing, then a black close below it: confirmed turn.",
            |w| engulfs_down(&w[1], &w[2]) && w[0].black() && w[0].close < w[1].close
        ),
        def!(
            "upside_tasuki_gap",
            "Upside Tasuki Gap",
            "UTG",
            Bullish,
            Uptrend,
            3,
            "A gap up, then a black candle that does not close it: the rise likely goes on.",
            |w| w[2].long()
                && w[2].white()
                && w[1].small()
                && w[1].white()
                && w[1].lo > w[2].hi
                && w[0].black()
                && w[0].open >= w[1].lo
                && w[0].open <= w[1].hi
                && w[0].close < w[1].lo
                && w[0].close > w[2].hi
        ),
        def!(
            "downside_tasuki_gap",
            "Downside Tasuki Gap",
            "DTG",
            Bearish,
            Downtrend,
            3,
            "A gap down, then a white candle that does not close it: the fall likely goes on.",
            |w| w[2].long()
                && w[2].black()
                && w[1].small()
                && w[1].black()
                && w[1].hi < w[2].lo
                && w[0].white()
                && w[0].open <= w[1].hi
                && w[0].open >= w[1].lo
                && w[0].close > w[1].hi
                && w[0].close < w[2].lo
        ),
        def!(
            "rising_three_methods",
            "Rising Three Methods",
            "R3M",
            Bullish,
            Uptrend,
            5,
            "Three small candles rest inside a long white one, then a long white close above it.",
            |w| w[4].long()
                && w[4].white()
                && w[0].long()
                && w[0].white()
                && w[0].close > w[4].close
                && w[1..=3]
                    .iter()
                    .all(|c| c.small() && c.high < w[4].high && c.low > w[4].low)
        ),
        def!(
            "falling_three_methods",
            "Falling Three Methods",
            "F3M",
            Bearish,
            Downtrend,
            5,
            "Three small candles rest inside a long black one, then a long black close below it.",
            |w| w[4].long()
                && w[4].black()
                && w[0].long()
                && w[0].black()
                && w[0].close < w[4].close
                && w[1..=3]
                    .iter()
                    .all(|c| c.small() && c.high < w[4].high && c.low > w[4].low)
        ),
    ];

/// One detection: pattern `def` ends on bar `bar`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub bar: usize,
    pub def: usize,
}

/// The setting's choice: all patterns, one side, or one pattern.
fn chosen(choice: &str, d: &Def) -> bool {
    match choice {
        "all" => true,
        "bullish" => d.bias == Bias::Bullish,
        "bearish" => d.bias == Bias::Bearish,
        "neutral" => d.bias == Bias::Neutral,
        key => {
            d.key == key
                || d.key.strip_suffix("_bullish") == Some(key)
                || d.key.strip_suffix("_bearish") == Some(key)
                || d.key.strip_suffix("_white") == Some(key)
                || d.key.strip_suffix("_black") == Some(key)
        }
    }
}

/// `choice`'s patterns on `bars`, oldest bar first. Pure, and causal: a hit on
/// bar `i` only reads bars up to `i`, so appending bars never changes it.
pub fn detect(bars: &[Bar], choice: &str, trend: TrendRule) -> Vec<Hit> {
    let n = bars.len();
    let bodies: Vec<f64> = bars.iter().map(|b| (b.close - b.open).abs()).collect();
    let avg = ema_n(&bodies, 14);
    let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();
    let (sma50, sma200) = match trend {
        TrendRule::None => (vec![], vec![]),
        TrendRule::Sma50 => (sma_n(&closes, 50), vec![]),
        TrendRule::Sma50And200 => (sma_n(&closes, 50), sma_n(&closes, 200)),
    };
    // Needs a reading on that bar: no SMA yet means no known trend.
    let trend_at = |needs: Needs, i: usize| match (needs, trend) {
        (Needs::Nothing, _) | (_, TrendRule::None) => true,
        (Needs::Downtrend, TrendRule::Sma50) => closes[i] < sma50[i],
        (Needs::Uptrend, TrendRule::Sma50) => closes[i] > sma50[i],
        (Needs::Downtrend, TrendRule::Sma50And200) => closes[i] < sma50[i] && sma50[i] < sma200[i],
        (Needs::Uptrend, TrendRule::Sma50And200) => closes[i] > sma50[i] && sma50[i] > sma200[i],
    };
    let defs: Vec<(usize, &Def)> = PATTERNS
        .iter()
        .enumerate()
        .filter(|(_, d)| chosen(choice, d))
        .collect();
    let mut out = vec![];
    let mut w: Vec<C> = Vec::with_capacity(5);
    for i in 0..n {
        w.clear();
        for k in 0..5.min(i + 1) {
            // Newest-first; every measure on the pattern's last bar's average.
            w.push(C::new(&bars[i - k], avg[i - k]));
        }
        if w.iter().take(3).any(|c| !c.avg.is_finite()) {
            continue;
        }
        for &(id, d) in &defs {
            if i + 1 < d.bars || (w.len() < d.bars) {
                continue;
            }
            if d.bars == 5 && w.iter().any(|c| !c.avg.is_finite()) {
                continue;
            }
            // The bar before the pattern; none means no prior trend to read.
            let before = i.checked_sub(d.bars);
            let trend_ok = match before {
                Some(b) => trend_at(d.needs, b),
                None => d.needs == Needs::Nothing || trend == TrendRule::None,
            };
            if trend_ok && (d.rule)(&w) {
                out.push(Hit { bar: i, def: id });
            }
        }
    }
    out
}

/// True while the newest bar of `data` can still change: a timed bar whose
/// interval has not ended at `now_ms`. Bars without time (the demo data) or an
/// interval count as closed. A daily bar counts as forming for its whole UTC day,
/// also after its exchange closed: a label there is marked too early, not too late.
pub fn newest_bar_forming(data: &OhlcvData, now_ms: i64) -> bool {
    let Some(open) = data.bars.last().and_then(|b| b.date.parse::<i64>().ok()) else {
        return false;
    };
    let Some(len) = data
        .interval
        .as_deref()
        .and_then(crate::data::bar_interval_secs)
    else {
        return false;
    };
    now_ms < (open + len) * 1000
}

/// Wall clock in unix ms, for `newest_bar_forming`.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub const FORMING_NOTE: &str =
    "Forming bar: this can change or vanish until the bar closes. Alerts wait for the close.";

fn needs_text(n: Needs) -> &'static str {
    match n {
        Needs::Downtrend => " Counts only after a downtrend.",
        Needs::Uptrend => " Counts only after an uptrend.",
        Needs::Nothing => "",
    }
}

/// Choices of the `pattern` setting: everything, a side, or one pattern.
fn choices() -> Vec<String> {
    let mut out: Vec<String> = ["all", "bullish", "bearish", "neutral"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for d in PATTERNS {
        out.push(d.key.to_string());
    }
    out
}

pub struct CandlestickPatterns {
    pub pattern: String,
    pub trend: TrendRule,
}

impl Default for CandlestickPatterns {
    fn default() -> Self {
        Self {
            pattern: "all".into(),
            trend: TrendRule::Sma50,
        }
    }
}

impl CandlestickPatterns {
    fn result_at(&self, data: &OhlcvData, forming: bool) -> PanelResult {
        let hits = detect(&data.bars, &self.pattern, self.trend);
        let last = data.len().saturating_sub(1);
        let closed_last = if forming {
            last.checked_sub(1)
        } else {
            Some(last)
        };
        let mut labels = Vec::with_capacity(hits.len());
        let mut dots = vec![];
        let mut newest: [Option<i64>; 3] = [None; 3];
        for h in &hits {
            let d = &PATTERNS[h.def];
            let bar = &data.bars[h.bar];
            let tentative = forming && h.bar == last;
            let above = d.bias == Bias::Bearish || d.bias == Bias::Neutral;
            let y = if above { bar.high } else { bar.low };
            let kind = match d.bias {
                Bias::Bullish => 0,
                Bias::Bearish => 1,
                Bias::Neutral => 2,
            };
            let mut tooltip = format!(
                "{} ({}): {}{}",
                d.name,
                d.bias.as_str(),
                d.meaning,
                needs_text(d.needs)
            );
            if tentative {
                tooltip.push(' ');
                tooltip.push_str(FORMING_NOTE);
            } else {
                newest[kind] = Some(h.bar as i64);
                // Only the newest closed bar: the MCP `indicator_signal` alert
                // fires on any labeled dot it is handed.
                if Some(h.bar) == closed_last {
                    dots.push(Dot {
                        x: h.bar,
                        y,
                        color: d.bias.color(),
                        size: 3,
                        label: Some(d.key.to_string()),
                    });
                }
            }
            labels.push(BarLabel {
                x: h.bar,
                y,
                above,
                text: d.text.to_string(),
                key: d.key.to_string(),
                name: d.name.to_string(),
                bias: d.bias,
                tooltip,
                tentative,
            });
        }
        let label = if self.pattern == "all" {
            "Candlestick Patterns".to_string()
        } else {
            format!("Candlestick Patterns: {}", self.pattern)
        };
        let event = |key: &str, name: &str, x: Option<i64>| IndicatorEvent {
            key: key.into(),
            name: name.into(),
            x,
        };
        PanelResult {
            labels,
            dots,
            events: vec![
                event("bullish", "Bullish pattern", newest[0]),
                event("bearish", "Bearish pattern", newest[1]),
                event("neutral", "Neutral pattern", newest[2]),
            ],
            label,
            is_overlay: true,
            ..Default::default()
        }
    }
}

impl Indicator for CandlestickPatterns {
    fn name(&self) -> &str {
        "Candlestick Patterns"
    }

    fn description(&self) -> &str {
        "Candlestick pattern recognition: doji variants, hammer, hanging man, inverted hammer, shooting star, engulfing, harami, piercing, dark cloud, morning and evening star, three white soldiers, three black crows, tweezers, marubozu, spinning top, kicking, abandoned baby, three inside and outside and more. All at once or one by one"
    }

    fn params(&self) -> Value {
        json!([
            {"name": "pattern", "type": "string", "default": "all", "enum": choices(),
             "description": "Which patterns: all, one side, or a single pattern"},
            {"name": "trend", "type": "string", "default": "sma50",
             "enum": ["sma50", "sma50_200", "none"],
             "description": "Prior trend for patterns that need one: close against its SMA 50, also SMA 50 against SMA 200, or no check"}
        ])
    }

    fn configure(&mut self, p: &Value) {
        if let Some(v) = p.get("pattern").and_then(|v| v.as_str()) {
            if choices().iter().any(|c| c == v) {
                self.pattern = v.to_string();
            }
        }
        if let Some(v) = p.get("trend").and_then(|v| v.as_str()) {
            self.trend = TrendRule::parse(v);
        }
    }

    fn warmup_bars(&self) -> usize {
        // The body average, the five-bar patterns, and the trend's SMA.
        self.trend.warmup() + 30
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        self.result_at(data, newest_bar_forming(data, now_ms()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(o: f64, h: f64, l: f64, c: f64) -> Bar {
        Bar {
            open: o,
            high: h,
            low: l,
            close: c,
            volume: 1.0,
            date: "x".into(),
        }
    }

    /// 30 even bars with bodies of 1 along `slope`: the average body is 1.
    fn lead(slope: f64) -> Vec<Bar> {
        (0..30)
            .map(|i| {
                let base = 100.0 + slope * i as f64;
                if i % 2 == 0 {
                    b(base, base + 1.2, base - 0.2, base + 1.0)
                } else {
                    b(base + 1.0, base + 1.2, base - 0.2, base)
                }
            })
            .collect()
    }

    /// Keys of the patterns on the last bar of `lead(slope)` + `tail`, no trend rule.
    fn hits(tail: &[Bar]) -> Vec<&'static str> {
        let mut bars = lead(0.0);
        bars.extend_from_slice(tail);
        detect(&bars, "all", TrendRule::None)
            .into_iter()
            .filter(|h| h.bar == bars.len() - 1)
            .map(|h| PATTERNS[h.def].key)
            .collect()
    }

    /// `key` is found on `yes` and not on `no` (a near miss).
    fn check(key: &str, yes: &[Bar], no: &[Bar]) {
        let found = hits(yes);
        assert!(found.contains(&key), "{key} not in {found:?}");
        let found = hits(no);
        assert!(!found.contains(&key), "{key} on its near miss: {found:?}");
    }

    #[test]
    fn doji_variants() {
        check(
            "doji",
            &[b(100.0, 101.0, 99.0, 100.05)],
            &[b(100.0, 101.0, 99.0, 100.4)],
        );
        check(
            "long_legged_doji",
            &[b(100.0, 104.0, 96.0, 100.1)],
            &[b(100.0, 101.0, 99.0, 100.05)],
        );
        check(
            "dragonfly_doji",
            &[b(100.0, 100.0, 96.0, 100.1)],
            &[b(100.0, 102.0, 96.0, 100.1)],
        );
        check(
            "gravestone_doji",
            &[b(100.0, 104.0, 99.95, 100.0)],
            &[b(100.0, 104.0, 98.0, 100.0)],
        );
    }

    #[test]
    fn single_candle_shapes() {
        // Body 0.5 at the top; lower shadow 2 is 4x the body.
        let hammer = b(100.0, 100.5, 98.0, 100.5);
        // Same with a real upper shadow.
        let not = b(100.0, 101.5, 98.0, 100.5);
        check(
            "hammer",
            std::slice::from_ref(&hammer),
            std::slice::from_ref(&not),
        );
        check("hanging_man", &[hammer], &[not]);
        let star = b(100.5, 102.5, 100.0, 100.0);
        let not = b(100.5, 102.5, 98.5, 100.0);
        check(
            "inverted_hammer",
            std::slice::from_ref(&star),
            std::slice::from_ref(&not),
        );
        check("shooting_star", &[star], &[not]);
        check(
            "marubozu_white",
            &[b(100.0, 103.0, 100.0, 103.0)],
            &[b(100.0, 104.0, 100.0, 103.0)],
        );
        check(
            "marubozu_black",
            &[b(103.0, 103.0, 100.0, 100.0)],
            &[b(103.0, 103.0, 99.0, 100.0)],
        );
        check(
            "spinning_top",
            &[b(100.0, 101.0, 99.0, 100.3)],
            &[b(100.0, 100.4, 99.0, 100.3)],
        );
    }

    #[test]
    fn two_candle_patterns() {
        check(
            "engulfing_bullish",
            &[b(100.5, 100.7, 99.8, 100.0), b(99.9, 101.6, 99.7, 101.5)],
            // Closes inside the prior open: no engulfing.
            &[b(100.5, 100.7, 99.8, 100.0), b(99.9, 100.4, 99.7, 100.4)],
        );
        check(
            "engulfing_bearish",
            &[b(100.0, 100.7, 99.8, 100.5), b(100.6, 100.8, 98.8, 98.9)],
            &[b(100.0, 100.7, 99.8, 100.5), b(100.3, 100.8, 98.8, 98.9)],
        );
        check(
            "harami_bullish",
            &[b(103.0, 103.2, 99.8, 100.0), b(101.0, 101.5, 100.5, 101.4)],
            // Pokes out of the prior body.
            &[b(103.0, 103.2, 99.8, 100.0), b(101.0, 103.5, 100.5, 101.4)],
        );
        check(
            "harami_bearish",
            &[b(100.0, 103.2, 99.8, 103.0), b(102.0, 102.5, 101.5, 101.6)],
            &[b(100.0, 103.2, 99.8, 103.0), b(102.0, 103.5, 101.5, 101.6)],
        );
        check(
            "harami_cross_bullish",
            &[b(103.0, 103.2, 99.8, 100.0), b(101.0, 101.5, 100.5, 101.02)],
            &[b(103.0, 103.2, 99.8, 100.0), b(101.0, 101.5, 100.5, 101.4)],
        );
        check(
            "piercing",
            &[b(103.0, 103.2, 100.0, 100.2), b(99.8, 102.5, 99.7, 102.4)],
            // Stops below the middle of the prior body.
            &[b(103.0, 103.2, 100.0, 100.2), b(99.8, 101.5, 99.7, 101.4)],
        );
        check(
            "dark_cloud_cover",
            &[b(100.2, 103.0, 100.0, 102.8), b(103.2, 103.3, 100.5, 100.6)],
            &[b(100.2, 103.0, 100.0, 102.8), b(103.2, 103.3, 101.8, 101.9)],
        );
        check(
            "tweezer_bottom",
            &[b(102.0, 102.2, 99.0, 100.0), b(100.1, 102.0, 99.0, 101.8)],
            &[b(102.0, 102.2, 99.0, 100.0), b(100.1, 102.0, 99.5, 101.8)],
        );
        check(
            "tweezer_top",
            &[b(100.0, 103.0, 99.8, 102.0), b(102.0, 103.0, 100.0, 100.2)],
            &[b(100.0, 103.0, 99.8, 102.0), b(102.0, 102.5, 100.0, 100.2)],
        );
        check(
            "kicking_bullish",
            &[b(103.0, 103.0, 100.0, 100.0), b(104.0, 107.0, 104.0, 107.0)],
            // Overlaps the prior candle: no gap.
            &[b(103.0, 103.0, 100.0, 100.0), b(102.0, 105.0, 102.0, 105.0)],
        );
        check(
            "kicking_bearish",
            &[b(104.0, 107.0, 104.0, 107.0), b(103.0, 103.0, 100.0, 100.0)],
            &[b(104.0, 107.0, 104.0, 107.0), b(105.0, 105.0, 102.0, 102.0)],
        );
    }

    #[test]
    fn three_candle_patterns() {
        check(
            "morning_star",
            &[
                b(104.0, 104.2, 100.8, 101.0),
                b(100.3, 100.5, 99.8, 100.0),
                b(100.5, 103.2, 100.4, 103.0),
            ],
            // The last close stays below the middle of the first body.
            &[
                b(104.0, 104.2, 100.8, 101.0),
                b(100.3, 100.5, 99.8, 100.0),
                b(100.5, 102.2, 100.4, 102.0),
            ],
        );
        check(
            "evening_star",
            &[
                b(100.0, 103.2, 99.8, 103.0),
                b(103.7, 104.2, 103.5, 104.0),
                b(103.5, 103.6, 100.8, 101.0),
            ],
            &[
                b(100.0, 103.2, 99.8, 103.0),
                b(103.7, 104.2, 103.5, 104.0),
                b(103.5, 103.6, 101.8, 102.0),
            ],
        );
        check(
            "abandoned_baby_bullish",
            &[
                b(104.0, 104.2, 100.8, 101.0),
                b(100.0, 100.3, 99.7, 100.01),
                b(101.0, 103.0, 100.9, 102.9),
            ],
            // The doji's high reaches the first candle's low: no gap.
            &[
                b(104.0, 104.2, 100.8, 101.0),
                b(100.0, 101.2, 99.7, 100.01),
                b(101.0, 103.0, 100.9, 102.9),
            ],
        );
        check(
            "abandoned_baby_bearish",
            &[
                b(100.0, 103.0, 99.8, 102.8),
                b(104.0, 104.3, 103.7, 104.01),
                b(103.0, 103.1, 100.8, 101.0),
            ],
            &[
                b(100.0, 103.0, 99.8, 102.8),
                b(104.0, 104.3, 102.5, 104.01),
                b(103.0, 103.1, 100.8, 101.0),
            ],
        );
        check(
            "three_white_soldiers",
            &[
                b(100.0, 102.05, 99.8, 102.0),
                b(101.0, 103.55, 100.8, 103.5),
                b(102.5, 105.05, 102.3, 105.0),
            ],
            // The last candle opens above the middle one's close.
            &[
                b(100.0, 102.05, 99.8, 102.0),
                b(101.0, 103.55, 100.8, 103.5),
                b(103.8, 105.85, 103.6, 105.8),
            ],
        );
        check(
            "three_black_crows",
            &[
                b(105.0, 105.2, 102.95, 103.0),
                b(104.0, 104.2, 101.45, 101.5),
                b(102.5, 102.7, 99.95, 100.0),
            ],
            &[
                b(105.0, 105.2, 102.95, 103.0),
                b(104.0, 104.2, 101.45, 101.5),
                b(101.2, 101.4, 99.15, 99.2),
            ],
        );
        check(
            "three_inside_up",
            &[
                b(103.0, 103.2, 99.8, 100.0),
                b(101.0, 101.5, 100.5, 101.4),
                b(101.5, 103.8, 101.4, 103.7),
            ],
            // Does not close above the first candle's open.
            &[
                b(103.0, 103.2, 99.8, 100.0),
                b(101.0, 101.5, 100.5, 101.4),
                b(101.5, 102.8, 101.4, 102.7),
            ],
        );
        check(
            "three_inside_down",
            &[
                b(100.0, 103.2, 99.8, 103.0),
                b(102.0, 102.5, 101.5, 101.6),
                b(101.5, 101.6, 99.3, 99.4),
            ],
            &[
                b(100.0, 103.2, 99.8, 103.0),
                b(102.0, 102.5, 101.5, 101.6),
                b(101.5, 101.6, 100.3, 100.4),
            ],
        );
        check(
            "three_outside_up",
            &[
                b(100.5, 100.7, 99.8, 100.0),
                b(99.9, 101.6, 99.7, 101.5),
                b(101.5, 102.6, 101.4, 102.5),
            ],
            &[
                b(100.5, 100.7, 99.8, 100.0),
                b(99.9, 101.6, 99.7, 101.5),
                b(101.5, 101.6, 100.9, 101.0),
            ],
        );
        check(
            "three_outside_down",
            &[
                b(100.0, 100.7, 99.8, 100.5),
                b(100.6, 100.8, 98.8, 98.9),
                b(98.9, 99.0, 97.9, 98.0),
            ],
            &[
                b(100.0, 100.7, 99.8, 100.5),
                b(100.6, 100.8, 98.8, 98.9),
                b(98.9, 99.5, 98.8, 99.4),
            ],
        );
    }

    #[test]
    fn gap_star_and_five_bar_patterns() {
        check(
            "doji_star_bullish",
            &[b(104.0, 104.2, 100.8, 101.0), b(100.0, 100.3, 99.7, 100.01)],
            &[
                b(104.0, 104.2, 100.8, 101.0),
                b(102.0, 102.3, 101.7, 102.01),
            ],
        );
        check(
            "morning_doji_star",
            &[
                b(104.0, 104.2, 100.8, 101.0),
                b(100.0, 100.3, 99.7, 100.01),
                b(100.5, 103.2, 100.4, 103.0),
            ],
            &[
                b(104.0, 104.2, 100.8, 101.0),
                b(100.0, 100.3, 99.7, 100.01),
                b(100.5, 101.2, 100.4, 101.0),
            ],
        );
        check(
            "rising_window",
            &[b(100.0, 100.5, 99.5, 100.2), b(101.0, 101.5, 100.8, 101.2)],
            &[b(100.0, 100.5, 99.5, 100.2), b(100.4, 101.5, 100.3, 101.2)],
        );
        check(
            "rising_three_methods",
            &[
                b(100.0, 103.2, 99.8, 103.0),
                b(102.5, 102.7, 102.0, 102.1),
                b(102.1, 102.3, 101.6, 101.7),
                b(101.7, 101.9, 101.2, 101.3),
                b(101.5, 104.2, 101.4, 104.0),
            ],
            // A resting candle breaks below the first candle.
            &[
                b(100.0, 103.2, 99.8, 103.0),
                b(102.5, 102.7, 102.0, 102.1),
                b(102.1, 102.3, 99.0, 101.7),
                b(101.7, 101.9, 101.2, 101.3),
                b(101.5, 104.2, 101.4, 104.0),
            ],
        );
    }

    #[test]
    fn the_trend_rule_decides_hammer_from_hanging_man() {
        let hammer_shape_bar = |c: f64| b(c, c + 0.5, c - 2.0, c + 0.5);
        // A long fall: the close sits under its SMA 50.
        let mut falling: Vec<Bar> = (0..60)
            .map(|i| {
                let c = 200.0 - i as f64;
                b(c + 1.0, c + 1.1, c - 0.1, c)
            })
            .collect();
        falling.push(hammer_shape_bar(139.0));
        let keys = |bars: &[Bar], t| -> Vec<&str> {
            detect(bars, "all", t)
                .into_iter()
                .filter(|h| h.bar == bars.len() - 1)
                .map(|h| PATTERNS[h.def].key)
                .collect()
        };
        let down = keys(&falling, TrendRule::Sma50);
        assert!(down.contains(&"hammer"), "{down:?}");
        assert!(!down.contains(&"hanging_man"), "{down:?}");
        // A long rise: the same candle is a hanging man.
        let mut rising: Vec<Bar> = (0..60)
            .map(|i| {
                let c = 100.0 + i as f64;
                b(c - 1.0, c + 0.1, c - 1.1, c)
            })
            .collect();
        rising.push(hammer_shape_bar(160.0));
        let up = keys(&rising, TrendRule::Sma50);
        assert!(up.contains(&"hanging_man"), "{up:?}");
        assert!(!up.contains(&"hammer"), "{up:?}");
        // "none" reads the shape only, so both.
        let both = keys(&rising, TrendRule::None);
        assert!(
            both.contains(&"hammer") && both.contains(&"hanging_man"),
            "{both:?}"
        );
        // Too little history for an SMA 50: no known trend, no hammer.
        let short = keys(&falling[40..], TrendRule::Sma50);
        assert!(!short.contains(&"hammer"), "{short:?}");
    }

    #[test]
    fn sma50_200_needs_the_longer_trend_too() {
        // Under the SMA 50, but the SMA 50 is above the SMA 200.
        let mut bars: Vec<Bar> = (0..240)
            .map(|i| {
                let c = 100.0 + i as f64;
                b(c - 1.0, c + 0.1, c - 1.1, c)
            })
            .collect();
        for i in 0..5 {
            let c = 300.0 - i as f64 * 2.0;
            bars.push(b(c + 2.0, c + 2.1, c - 0.1, c));
        }
        let c = 290.0;
        bars.push(b(c, c + 0.5, c - 2.0, c + 0.5));
        let keys = |t| -> Vec<&str> {
            detect(&bars, "hammer", t)
                .into_iter()
                .filter(|h| h.bar == bars.len() - 1)
                .map(|h| PATTERNS[h.def].key)
                .collect()
        };
        assert_eq!(keys(TrendRule::Sma50), ["hammer"]);
        assert!(keys(TrendRule::Sma50And200).is_empty());
    }

    #[test]
    fn choosing_a_pattern_or_a_side() {
        let mut bars = lead(0.0);
        bars.push(b(100.0, 100.5, 98.0, 100.5));
        let keys = |choice: &str| -> Vec<&str> {
            detect(&bars, choice, TrendRule::None)
                .into_iter()
                .filter(|h| h.bar == bars.len() - 1)
                .map(|h| PATTERNS[h.def].key)
                .collect()
        };
        assert_eq!(keys("hammer"), ["hammer"]);
        assert!(keys("bullish").contains(&"hammer"));
        assert!(!keys("bearish").contains(&"hammer"));
        assert!(keys("bearish").contains(&"hanging_man"));
        assert!(keys("shooting_star").is_empty());
        // A two-sided pattern is chosen by its plain name too.
        let mut engulf = lead(0.0);
        engulf.extend([b(100.5, 100.7, 99.8, 100.0), b(99.9, 101.6, 99.7, 101.5)]);
        let found = detect(&engulf, "engulfing", TrendRule::None);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(PATTERNS[found[0].def].key, "engulfing_bullish");
    }

    #[test]
    fn a_closed_bars_hits_do_not_change_when_bars_are_added() {
        let data = crate::data::sample_data(400);
        let full = detect(&data.bars, "all", TrendRule::Sma50);
        for cut in [120, 200, 333] {
            let part = detect(&data.bars[..cut], "all", TrendRule::Sma50);
            let same: Vec<Hit> = full.iter().copied().filter(|h| h.bar < cut).collect();
            assert_eq!(part, same, "cut {cut}");
        }
    }

    #[test]
    fn keys_are_unique_and_every_pattern_is_explained() {
        let mut seen = std::collections::HashSet::new();
        for d in PATTERNS {
            assert!(seen.insert(d.key), "{} twice", d.key);
            assert!(!d.meaning.is_empty() && !d.text.is_empty(), "{}", d.key);
            assert!(d.bars >= 1 && d.bars <= 5, "{}", d.key);
        }
    }

    #[test]
    fn the_forming_bars_label_is_tentative_and_stays_out_of_events_and_dots() {
        let mut bars = lead(0.0);
        bars.push(b(100.0, 100.5, 98.0, 100.5)); // hammer shape
        bars.push(b(100.0, 100.5, 98.0, 100.5)); // again, on the forming bar
        for (i, bar) in bars.iter_mut().enumerate() {
            bar.date = (1_700_000_000 + i as i64 * 3600).to_string();
        }
        let data = OhlcvData {
            bars,
            symbol: None,
            interval: Some("1h".into()),
            session: None,
        };
        let ind = CandlestickPatterns {
            pattern: "hammer".into(),
            trend: TrendRule::None,
        };
        let last = data.len() - 1;

        let forming = ind.result_at(&data, true);
        assert_eq!(forming.labels.len(), 2);
        assert!(!forming.labels[0].tentative);
        assert!(forming.labels[1].tentative);
        assert!(forming.labels[1].tooltip.contains("until the bar closes"));
        // The newest occurrence an alert may fire on is the closed one.
        assert_eq!(forming.events[0].x, Some(last as i64 - 1));
        assert_eq!(forming.dots.len(), 1);
        assert_eq!(forming.dots[0].x, last - 1);
        assert_eq!(forming.dots[0].label.as_deref(), Some("hammer"));

        let closed = ind.result_at(&data, false);
        assert!(closed.labels.iter().all(|l| !l.tentative));
        assert_eq!(closed.events[0].x, Some(last as i64));
        assert_eq!(closed.dots[0].x, last);
    }

    #[test]
    fn a_bar_forms_until_its_interval_ended() {
        let data = |date: &str, iv: Option<&str>| OhlcvData {
            bars: vec![Bar {
                date: date.into(),
                ..b(1.0, 1.0, 1.0, 1.0)
            }],
            symbol: None,
            interval: iv.map(String::from),
            session: None,
        };
        let open = 1_700_000_000_i64;
        let hour = data(&open.to_string(), Some("1h"));
        assert!(newest_bar_forming(&hour, open * 1000));
        assert!(newest_bar_forming(&hour, (open + 3599) * 1000));
        assert!(!newest_bar_forming(&hour, (open + 3600) * 1000));
        // Untimed demo data, or no known interval: nothing to wait for.
        assert!(!newest_bar_forming(&data("D7", Some("1h")), 0));
        assert!(!newest_bar_forming(&data(&open.to_string(), None), 0));
    }

    #[test]
    fn labels_sit_on_the_right_side_of_the_bar() {
        let mut bars = lead(0.0);
        bars.push(b(100.0, 100.5, 98.0, 100.5));
        let data = OhlcvData {
            bars,
            symbol: None,
            interval: None,
            session: None,
        };
        let r = CandlestickPatterns {
            pattern: "all".into(),
            trend: TrendRule::None,
        }
        .compute(&data);
        let bull = r.labels.iter().find(|l| l.key == "hammer").unwrap();
        assert!(!bull.above);
        assert_eq!(bull.y, 98.0);
        let bear = r.labels.iter().find(|l| l.key == "hanging_man").unwrap();
        assert!(bear.above);
        assert_eq!(bear.y, 100.5);
    }
}
