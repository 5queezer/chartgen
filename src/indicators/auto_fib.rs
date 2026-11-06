//! Auto Fib Retracement (ADR-0046): fib levels over the last significant swing.
//!
//! `last_swing` and `levels` are pure. Only closed bars count, so the forming bar
//! moves nothing. The swing changes when a newer pivot is confirmed, `depth` bars
//! after its extreme, or when a close breaks the swing's start. Each level is a
//! plot, so it has a color, a width and alerts like any other plot.

use super::chart_patterns::{closed_window, pivots, price_text, Pivot, LOOKBACK};
use crate::data::{Bar, OhlcvData};
use crate::indicator::*;
use serde_json::{json, Value};

/// Slots for levels: a plot count that never changes with the settings.
const SLOTS: usize = 8;
/// TradingView's usual set; its built-in tool's defaults are not published.
const DEFAULTS: [Option<f64>; SLOTS] = [
    Some(0.0),
    Some(0.236),
    Some(0.382),
    Some(0.5),
    Some(0.618),
    Some(0.786),
    Some(1.0),
    None,
];
const COLORS: [u32; SLOTS] = [
    0x787B86, 0xF23645, 0xFF9800, 0x4CAF50, 0x089981, 0x00BCD4, 0x787B86, 0x9C27B0,
];

/// A swing from `from` to `to`, in bars of the slice it was read on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swing {
    pub from: Pivot,
    pub to: Pivot,
    /// `to` is the extreme since a broken start, with no pivot confirmed yet.
    pub provisional: bool,
}

impl Swing {
    pub fn up(&self) -> bool {
        self.to.price > self.from.price
    }
}

/// The last significant swing of closed `bars`: the two newest pivots. When a
/// later close broke the start of that swing, it no longer stands: the swing
/// then runs from the newest pivot to the extreme reached since.
pub fn last_swing(bars: &[Bar], depth: usize, deviation: f64) -> Option<Swing> {
    let p = pivots(bars, depth, deviation);
    let (from, to) = match p.as_slice() {
        [.., a, b] => (*a, *b),
        _ => return None,
    };
    let after = &bars[to.bar + 1..];
    let broke = after.iter().any(|b| {
        if to.high {
            b.close < from.price
        } else {
            b.close > from.price
        }
    });
    if !broke {
        return Some(Swing {
            from,
            to,
            provisional: false,
        });
    }
    let (mut bar, mut price) = (to.bar + 1, f64::NAN);
    for (k, b) in after.iter().enumerate() {
        let v = if to.high { b.low } else { b.high };
        let better = if to.high { v < price } else { v > price };
        if price.is_nan() || better {
            (bar, price) = (to.bar + 1 + k, v);
        }
    }
    Some(Swing {
        from: to,
        to: Pivot {
            bar,
            price,
            high: !to.high,
        },
        provisional: true,
    })
}

/// Price of level `ratio`: 0 at the end of the swing, 1 at its start, so the
/// ratios read how much of the move is retraced. `reverse` swaps the ends.
pub fn level_price(s: &Swing, ratio: f64, reverse: bool) -> f64 {
    let (zero, one) = if reverse {
        (s.from.price, s.to.price)
    } else {
        (s.to.price, s.from.price)
    };
    zero + (one - zero) * ratio
}

/// `0.618`, not `0.6180000000000001`.
fn ratio_text(r: f64) -> String {
    let s = format!("{r:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

pub struct AutoFib {
    pub depth: usize,
    pub deviation: f64,
    pub levels: [Option<f64>; SLOTS],
    pub reverse: bool,
    pub lookback: usize,
}

impl Default for AutoFib {
    fn default() -> Self {
        Self {
            depth: 10,
            deviation: 3.0,
            levels: DEFAULTS,
            reverse: false,
            lookback: LOOKBACK,
        }
    }
}

impl Indicator for AutoFib {
    fn name(&self) -> &str {
        "Auto Fib Retracement"
    }

    fn description(&self) -> &str {
        "Auto Fib Retracement: fibonacci levels over the last significant ZigZag swing, redrawn as swings change. Each level has its own value and color and can be alerted on"
    }

    fn params(&self) -> Value {
        let mut out = vec![
            json!({"name": "depth", "type": "integer", "default": 10, "min": 1, "max": 100,
                   "description": "ZigZag depth: bars on each side of a swing high or low. A swing is known this many bars after it"}),
            json!({"name": "deviation", "type": "number", "default": 3.0, "min": 0, "max": 50,
                   "description": "ZigZag deviation: least move between swings, in ATR(10)"}),
            json!({"name": "reverse", "type": "boolean", "default": false,
                   "description": "Swap the ends: level 0 at the start of the swing"}),
        ];
        for (i, d) in DEFAULTS.iter().enumerate() {
            let mut p = json!({"name": format!("level{}", i + 1), "type": "number", "min": -10, "max": 10,
                               "description": "Ratio of the swing: 0 at its end, 1 at its start. Colors and hiding are on the Style tab"});
            if let Some(v) = d {
                p["default"] = json!(v);
            }
            out.push(p);
        }
        Value::Array(out)
    }

    fn configure(&mut self, p: &Value) {
        if let Some(v) = p.get("depth").and_then(|v| v.as_u64()) {
            self.depth = (v as usize).max(1);
        }
        if let Some(v) = p.get("deviation").and_then(|v| v.as_f64()) {
            self.deviation = v;
        }
        if let Some(v) = p.get("reverse").and_then(|v| v.as_bool()) {
            self.reverse = v;
        }
        for i in 0..SLOTS {
            let key = format!("level{}", i + 1);
            match p.get(&key) {
                Some(Value::Null) => self.levels[i] = None,
                Some(v) => {
                    if let Some(r) = v.as_f64() {
                        self.levels[i] = Some(r);
                    }
                }
                None => {}
            }
        }
    }

    fn warmup_bars(&self) -> usize {
        // A live tail must see the same window as the full load (ADR-0046).
        self.lookback
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        let (window, base) = closed_window(data, self.lookback);
        let swing = last_swing(window, self.depth, self.deviation);
        let n = data.len();
        let mut lines = Vec::with_capacity(SLOTS);
        let mut levels = vec![];
        for (i, ratio) in self.levels.iter().enumerate() {
            let mut y = vec![f64::NAN; n];
            let label = ratio.map_or_else(|| format!("Level {}", i + 1), ratio_text);
            if let (Some(s), Some(r)) = (&swing, ratio) {
                let price = level_price(s, *r, self.reverse);
                let from = s.from.bar + base;
                y[from..].fill(price);
                levels.push(LevelLine {
                    line: i,
                    x: from as i64,
                    y: price,
                });
            }
            lines.push(Line {
                y,
                color: rgba(COLORS[i], 0.9),
                width: 1,
                label: Some(label),
                breaks: false,
            });
        }
        let patterns = swing.iter().map(|s| swing_shape(s, base)).collect();
        PanelResult {
            lines,
            levels,
            patterns,
            label: format!("Auto Fib {} {}", self.depth, self.deviation),
            is_overlay: true,
            ..Default::default()
        }
    }
}

/// The swing as a dashed outline whose tooltip says what can move it.
fn swing_shape(s: &Swing, base: usize) -> PatternShape {
    let up = s.up();
    let (a, b) = ((s.from.bar + base) as i64, (s.to.bar + base) as i64);
    let what = if up { "Up swing" } else { "Down swing" };
    let (status, how) = if s.provisional {
        (
            PatternStatus::Forming,
            "A close broke the start of the last swing, so this runs to the extreme since, until a new swing is confirmed.",
        )
    } else {
        (
            PatternStatus::Confirmed,
            "Both ends are confirmed swings. A newer swing, or a close past its start, redraws the levels.",
        )
    };
    PatternShape {
        key: "fib_swing".into(),
        name: "Fib swing".into(),
        bias: if up { Bias::Bullish } else { Bias::Bearish },
        status,
        strokes: vec![Stroke {
            x1: a,
            y1: s.from.price,
            x2: b,
            y2: s.to.price,
            dashed: true,
        }],
        label_x: b,
        label_y: s.to.price,
        label_above: up,
        tooltip: format!(
            "{what} from {} to {}. {how} Read from closed bars; the forming bar moves nothing.",
            price_text(s.from.price),
            price_text(s.to.price),
        ),
        target: None,
        invalidation: None,
        confirmed_x: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        bars
    }

    fn data(bars: Vec<Bar>) -> OhlcvData {
        OhlcvData {
            bars,
            symbol: None,
            interval: None,
            session: None,
        }
    }

    fn up_swing() -> Vec<Bar> {
        // A low at bar 20 (100), a high at bar 60 (140), then a pullback.
        path(&[(0, 120.0), (20, 100.0), (60, 140.0), (80, 125.0)])
    }

    #[test]
    fn the_swing_is_the_last_two_pivots() {
        let s = last_swing(&up_swing(), 5, 3.0).unwrap();
        assert_eq!(s.from.bar, 20);
        assert_eq!(s.from.price, 99.9);
        assert_eq!(s.to.bar, 60);
        assert_eq!(s.to.price, 140.1);
        assert!(s.up());
        assert!(!s.provisional);
    }

    #[test]
    fn levels_read_the_retracement_from_the_swings_end() {
        let s = last_swing(&up_swing(), 5, 3.0).unwrap();
        let close = |a: f64, b: f64| assert!((a - b).abs() < 1e-9, "{a} vs {b}");
        close(level_price(&s, 0.0, false), 140.1);
        close(level_price(&s, 1.0, false), 99.9);
        close(level_price(&s, 0.5, false), 120.0);
        close(level_price(&s, 0.618, false), 140.1 - 40.2 * 0.618);
        // Reversed: 0 at the start.
        close(level_price(&s, 0.0, true), 99.9);
        close(level_price(&s, 1.0, true), 140.1);
    }

    #[test]
    fn a_down_swing_reads_upward() {
        let bars = path(&[(0, 120.0), (20, 140.0), (60, 100.0), (80, 115.0)]);
        let s = last_swing(&bars, 5, 3.0).unwrap();
        assert!(!s.up());
        assert!((level_price(&s, 0.0, false) - 99.9).abs() < 1e-9);
        assert!((level_price(&s, 1.0, false) - 140.1).abs() < 1e-9);
    }

    #[test]
    fn a_new_confirmed_swing_redraws_the_levels() {
        let bars = up_swing();
        let before = last_swing(&bars, 5, 3.0).unwrap();
        // The pullback turns up: its low is a pivot once `depth` bars followed it.
        let mut more = path(&[
            (0, 120.0),
            (20, 100.0),
            (60, 140.0),
            (80, 120.0),
            (100, 135.0),
        ]);
        let just_before = last_swing(&more[..84], 5, 3.0).unwrap();
        assert_eq!(just_before, before);
        more.truncate(95);
        let after = last_swing(&more, 5, 3.0).unwrap();
        assert_eq!(after.from.bar, 60);
        assert_eq!(after.to.bar, 80);
        assert!(!after.up());
    }

    #[test]
    fn a_close_past_the_start_breaks_the_swing() {
        // Down through the swing's start, 99.9, with no new pivot confirmed yet.
        let bars = path(&[(0, 120.0), (20, 100.0), (60, 140.0), (100, 96.0)]);
        let s = last_swing(&bars, 5, 3.0).unwrap();
        assert!(s.provisional);
        assert_eq!(s.from.bar, 60);
        assert!(!s.up());
        assert_eq!(s.to.bar, bars.len() - 1);
        assert!(s.to.price < 97.5);
    }

    #[test]
    fn deviation_and_depth_decide_what_is_significant() {
        // Small swings inside the big one.
        let mut k = vec![
            (0, 120.0),
            (20, 100.0),
            (60, 140.0),
            (70, 137.0),
            (75, 139.0),
            (90, 125.0),
        ];
        let bars = path(&k);
        // Depth 3 at a low threshold takes the small wiggle; a large one does not.
        let fine = last_swing(&bars, 3, 0.1).unwrap();
        assert!(fine.from.bar >= 60, "{fine:?}");
        let coarse = last_swing(&bars, 3, 10.0).unwrap();
        assert_eq!(coarse.from.bar, 20);
        // A depth wider than the data has no pivot at all.
        k.truncate(2);
        assert_eq!(last_swing(&path(&k), 15, 3.0), None);
    }

    #[test]
    fn plots_keep_their_slots_and_start_at_the_swing() {
        let r = AutoFib {
            depth: 5,
            ..Default::default()
        }
        .compute(&data(up_swing()));
        assert_eq!(r.lines.len(), SLOTS);
        assert_eq!(r.lines[4].label.as_deref(), Some("0.618"));
        assert_eq!(r.lines[7].label.as_deref(), Some("Level 8"));
        assert!(r.lines[7].y.iter().all(|v| v.is_nan()));
        // NaN before the swing starts, the price from there on.
        assert!(r.lines[0].y[19].is_nan());
        assert_eq!(r.lines[0].y[20], 140.1);
        assert_eq!(r.lines[0].y[79], 140.1);
        // A level per shown slot, naming its plot.
        assert_eq!(r.levels.len(), 7);
        assert_eq!(r.levels[4].line, 4);
        assert_eq!(r.levels[4].x, 20);
        assert_eq!(r.patterns[0].key, "fib_swing");
        assert_eq!(r.patterns[0].status, PatternStatus::Confirmed);
    }

    #[test]
    fn levels_are_editable_and_can_be_hidden() {
        let mut ind = AutoFib {
            depth: 5,
            ..Default::default()
        };
        ind.configure(&json!({"level2": null, "level8": 1.618, "reverse": true}));
        let r = ind.compute(&data(up_swing()));
        assert!(r.lines[1].y.iter().all(|v| v.is_nan()));
        assert_eq!(r.lines[1].label.as_deref(), Some("Level 2"));
        assert_eq!(r.lines[7].label.as_deref(), Some("1.618"));
        let want = 99.9 + 40.2 * 1.618;
        assert!((r.lines[7].y[50] - want).abs() < 1e-9);
    }

    #[test]
    fn the_forming_bar_moves_nothing() {
        // A newest bar still inside its interval, far beyond the swing's start.
        let mut bars = up_swing();
        let now = super::super::candle_patterns::now_ms() / 1000;
        let n = bars.len() as i64;
        for (i, b) in bars.iter_mut().enumerate() {
            b.date = (now - (n - 1 - i as i64) * 3600).to_string();
        }
        let timed = |bars: Vec<Bar>| OhlcvData {
            bars,
            symbol: None,
            interval: Some("1h".into()),
            session: None,
        };
        let ind = AutoFib {
            depth: 5,
            ..Default::default()
        };
        let mut wild = bars.clone();
        let last = wild.last_mut().unwrap();
        last.close = 1.0;
        last.low = 1.0;
        let a = ind.compute(&timed(bars));
        let b = ind.compute(&timed(wild));
        assert_eq!(a.levels, b.levels);
        assert_eq!(a.patterns[0].status, b.patterns[0].status);
    }

    #[test]
    fn too_few_bars_draw_nothing_but_keep_the_plots() {
        let r = AutoFib::default().compute(&data(path(&[(0, 100.0), (3, 101.0)])));
        assert_eq!(r.lines.len(), SLOTS);
        assert!(r.levels.is_empty());
        assert!(r.patterns.is_empty());
    }
}
