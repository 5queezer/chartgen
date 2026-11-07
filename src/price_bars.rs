//! Price-based bars (ADR-0047): Renko, Kagi, Point & Figure, Line Break and
//! Range. They are built on the server from the chart's own timeframe, so
//! indicators run on the same bars the chart draws, and they only approximate
//! what the same types would be on tick data.
//!
//! A bar's time is when it opened: the first source bar after the previous one
//! was confirmed. Only closed source bars confirm anything. What has not been
//! confirmed yet is the last bar, the open one, which keeps its time when it is
//! confirmed, so a live tail replaces it in place and a reload shows the same
//! bars. Several bars from one source bar take the next free second, so times
//! always increase. The newest bar of a live build closes on the last price,
//! whatever it is, as that is what alerts and the price line read.

use crate::data::Bar;
use crate::interval::Interval;
use serde_json::{json, Value};

/// Most bars one build may make. A box too small for its prices is refused, so
/// neither memory nor the indicators that run on the bars can run away.
pub const MAX_PRICE_BARS: usize = 20_000;

/// A build reads this many chunks of source bars, whose edges lie on a fixed
/// grid of time, so every read in one chunk starts at the same bar.
const CHUNKS: usize = 5;
/// Source bars per chunk: one Binance page.
const CHUNK_BARS: usize = 1_000;
/// An intraday bar of a session market exists for about a fifth of the hours, so
/// its rows cover about that much more time.
const SESSION_STRETCH: i64 = 5;
/// A box under this share of the last price is refused for every type. Grid levels
/// would stop being exact integers well before prices lose their precision.
const MIN_BOX_SHARE: f64 = 1e-6;

pub const STYLES: [&str; 5] = ["renko", "kagi", "point_figure", "line_break", "range"];

const DEFAULT_ATR_LENGTH: usize = 14;
const DEFAULT_PERCENT: f64 = 1.0;
const DEFAULT_REVERSAL: u32 = 3;
const DEFAULT_LINES: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Renko,
    Kagi,
    PointFigure,
    LineBreak,
    Range,
}

impl Kind {
    pub fn from_style(style: &str) -> Option<Self> {
        match style {
            "renko" => Some(Self::Renko),
            "kagi" => Some(Self::Kagi),
            "point_figure" => Some(Self::PointFigure),
            "line_break" => Some(Self::LineBreak),
            "range" => Some(Self::Range),
            _ => None,
        }
    }

    fn uses_box(self) -> bool {
        self != Self::LineBreak
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoxMode {
    Atr,
    Fixed,
    Percent,
}

/// What a chart asked for. `box_size` is the size of a fixed box, or the size a
/// load resolved for an ATR or percentage box, sent back so that the chart's later
/// polls and pages build on the same one.
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub kind: Kind,
    pub mode: BoxMode,
    pub atr_length: usize,
    pub percent: f64,
    pub box_size: Option<f64>,
    pub reversal: u32,
    pub lines: u32,
}

impl Params {
    /// `chart_params` of a request; every key is optional and an unknown one is
    /// an error, so a typo cannot silently chart the defaults.
    pub fn parse(kind: Kind, args: Option<&Value>) -> Result<Self, String> {
        let empty = serde_json::Map::new();
        let map = match args {
            None | Some(Value::Null) => &empty,
            Some(Value::Object(m)) => m,
            Some(_) => return Err("chart_params must be an object.".into()),
        };
        for key in map.keys() {
            if ![
                "box",
                "atr_length",
                "percent",
                "box_size",
                "reversal",
                "lines",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("chart_params has no {key}."));
            }
        }
        let mode = match map.get("box").map(|v| v.as_str()) {
            None | Some(Some("atr")) => BoxMode::Atr,
            Some(Some("fixed")) => BoxMode::Fixed,
            Some(Some("percent")) => BoxMode::Percent,
            Some(_) => return Err("chart_params.box must be atr, fixed or percent.".into()),
        };
        let whole = |key: &str, default: u64, max: u64| -> Result<u64, String> {
            match map.get(key) {
                None => Ok(default),
                Some(v) => v.as_u64().filter(|n| (1..=max).contains(n)).ok_or_else(|| {
                    format!("chart_params.{key} must be a whole number from 1 to {max}.")
                }),
            }
        };
        let percent = match map.get("percent") {
            None => DEFAULT_PERCENT,
            Some(v) => v
                .as_f64()
                .filter(|p| *p > 0.0 && *p <= 100.0)
                .ok_or("chart_params.percent must be above 0 and at most 100.")?,
        };
        let box_size = match map.get("box_size") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_f64()
                    .filter(|b| b.is_finite() && *b > 0.0)
                    .ok_or("chart_params.box_size must be a number above 0.")?,
            ),
        };
        if mode == BoxMode::Fixed && box_size.is_none() && kind.uses_box() {
            return Err("chart_params.box fixed needs box_size.".into());
        }
        Ok(Self {
            kind,
            mode,
            atr_length: whole("atr_length", DEFAULT_ATR_LENGTH as u64, 500)? as usize,
            percent,
            box_size,
            reversal: whole("reversal", u64::from(DEFAULT_REVERSAL), 20)? as u32,
            lines: whole("lines", u64::from(DEFAULT_LINES), 20)? as u32,
        })
    }

    /// How a chart's title names these bars: `P&F 3.4 x 3`.
    pub fn label(&self, box_size: f64) -> String {
        match self.kind {
            Kind::Renko => format!("Renko {box_size}"),
            Kind::Kagi => format!("Kagi {box_size}"),
            Kind::PointFigure => format!("P&F {box_size} x {}", self.reversal),
            Kind::LineBreak => format!("Line Break {}", self.lines),
            Kind::Range => format!("Range {box_size}"),
        }
    }

    /// The inputs this type reads. With `box_size` set, this is what a response
    /// echoes and what a later request sends back.
    pub fn to_json(&self) -> Value {
        let mut out = serde_json::Map::new();
        if self.kind == Kind::LineBreak {
            out.insert("lines".into(), json!(self.lines));
            return Value::Object(out);
        }
        match self.mode {
            BoxMode::Atr => {
                out.insert("box".into(), json!("atr"));
                out.insert("atr_length".into(), json!(self.atr_length));
            }
            BoxMode::Fixed => {
                out.insert("box".into(), json!("fixed"));
            }
            BoxMode::Percent => {
                out.insert("box".into(), json!("percent"));
                out.insert("percent".into(), json!(self.percent));
            }
        }
        if let Some(size) = self.box_size {
            out.insert("box_size".into(), json!(size));
        }
        if self.kind == Kind::PointFigure {
            out.insert("reversal".into(), json!(self.reversal));
        }
        Value::Object(out)
    }

    /// Whether the box has to be resolved from bars, so two builds can differ.
    pub fn resolves_box(&self) -> bool {
        self.kind.uses_box() && self.box_size.is_none()
    }

    /// The box: the size given, or ATR or percent of the source, rounded. Line
    /// Break has none, and reports 0.
    fn resolve_box(&self, rows: &[Row], closed: usize) -> Result<f64, String> {
        if !self.kind.uses_box() {
            return Ok(0.0);
        }
        if let Some(size) = self.box_size {
            return Ok(size);
        }
        let size = match self.mode {
            BoxMode::Fixed => return Err("chart_params.box fixed needs box_size.".into()),
            BoxMode::Atr => atr(&rows[..closed], self.atr_length),
            BoxMode::Percent => rows.last().map_or(0.0, |r| r.close) * self.percent / 100.0,
        };
        let size = round_box(size);
        if size.is_finite() && size > 0.0 {
            Ok(size)
        } else {
            Err("The source bars give no box size (no price movement); use a fixed box.".into())
        }
    }
}

/// Two significant digits. An ATR or a last price moves all the time, and the
/// bars of a chart should not change with each tick of it.
pub fn round_box(size: f64) -> f64 {
    format!("{size:.1e}").parse().unwrap_or(size)
}

/// Wilder's ATR of the newest bar: a mean of the first `length` true ranges, then
/// smoothed. Fewer bars than that give the mean of what there is.
fn atr(rows: &[Row], length: usize) -> f64 {
    let mut value = 0.0;
    for (i, r) in rows.iter().enumerate() {
        let tr = match i {
            0 => r.high - r.low,
            _ => {
                let prev = rows[i - 1].close;
                (r.high - r.low)
                    .max((r.high - prev).abs())
                    .max((r.low - prev).abs())
            }
        };
        let n = (i + 1).min(length) as f64;
        value = (value * (n - 1.0) + tr) / n;
    }
    value
}

/// The time a read of `limit` rows of `iv` covers at the least.
pub fn rows_reach_secs(iv: &Interval, limit: usize, session: bool) -> i64 {
    let stretch = if session && !iv.is_date_bar() {
        SESSION_STRETCH
    } else {
        1
    };
    limit as i64 * iv.secs() * stretch
}

/// Source bars per read: five chunks, fewer for an interval resampled from finer
/// klines so a read stays within `max_base` of them.
pub fn source_bars(iv: &Interval, max_base: usize) -> usize {
    chunk_bars(iv, max_base) * CHUNKS
}

fn chunk_bars(iv: &Interval, max_base: usize) -> usize {
    let per_bar = iv.binance_base().1.max(1) as usize;
    (max_base / per_bar / CHUNKS).clamp(1, CHUNK_BARS)
}

/// What a build needs besides the bars.
#[derive(Clone, Copy, Debug)]
pub struct BuildOpts {
    /// Bound on native bars per read (`fetch::MAX_BASE_BARS`).
    pub max_base: usize,
    /// A history page: only bars before this, and no open bar of a brick type.
    pub end_time_ms: Option<i64>,
    pub now_ms: i64,
    /// How far back the source's read reaches, when that is a matter of time and
    /// not only of rows (Yahoo's window). The build's start must stay inside it.
    pub reach_secs: Option<i64>,
}

#[derive(Debug)]
pub struct Built {
    pub bars: Vec<Bar>,
    pub box_size: f64,
    /// Time of the first source bar the build started from. A different one is a
    /// different build, whose bars need not line up with this one's.
    pub anchor: Option<i64>,
}

#[derive(Clone, Copy, Debug)]
struct Row {
    t: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

fn rows(source: &[Bar]) -> Result<Vec<Row>, String> {
    let mut out = Vec::with_capacity(source.len());
    for b in source {
        let t = b
            .date
            .parse::<i64>()
            .map_err(|_| "Price-based charts need timed bars.".to_string())?;
        if b.close.is_finite() {
            let or_close = |v: f64| if v.is_finite() { v } else { b.close };
            out.push(Row {
                t,
                open: or_close(b.open),
                high: or_close(b.high),
                low: or_close(b.low),
                close: b.close,
                volume: if b.volume.is_finite() { b.volume } else { 0.0 },
            });
        }
    }
    Ok(out)
}

/// How many leading rows are closed. On a page all are. Otherwise the last one is
/// forming until its interval has passed.
fn closed_len(rows: &[Row], iv: &Interval, end_time_ms: Option<i64>, now_ms: i64) -> usize {
    match rows.last() {
        Some(last) if end_time_ms.is_none() && (last.t + iv.secs()) * 1000 > now_ms => {
            rows.len() - 1
        }
        _ => rows.len(),
    }
}

/// Length in seconds of a chunk: a fixed number of bars, or, where the source's
/// read is bounded in time, a sixth of it, so five chunks always lie inside.
fn chunk_secs(iv: &Interval, opts: &BuildOpts) -> i64 {
    match opts.reach_secs {
        Some(reach) => (reach / (CHUNKS as i64 + 1)).max(iv.secs()),
        None => chunk_bars(iv, opts.max_base) as i64 * iv.secs(),
    }
}

/// The first time a build starts at: the grid edge four chunks before the chunk
/// holding the newest bar. Reads inside one chunk therefore give the same bars.
pub fn anchor(last_t: i64, iv: &Interval, opts: &BuildOpts) -> i64 {
    let chunk = chunk_secs(iv, opts);
    last_t.div_euclid(chunk) * chunk - (CHUNKS as i64 - 1) * chunk
}

/// Build the bars of `params` from ascending source bars.
pub fn build(
    source: &[Bar],
    iv: &Interval,
    params: &Params,
    opts: &BuildOpts,
) -> Result<Built, String> {
    let all = rows(source)?;
    let Some(last) = all.last() else {
        return Ok(Built {
            bars: Vec::new(),
            box_size: params.box_size.unwrap_or(0.0),
            anchor: None,
        });
    };
    let from = anchor(last.t, iv, opts);
    let rows: Vec<Row> = all.into_iter().filter(|r| r.t >= from).collect();
    let closed = closed_len(&rows, iv, opts.end_time_ms, opts.now_ms);
    let box_size = params.resolve_box(&rows, closed)?;
    let last_close = rows.last().map(|r| r.close);
    if params.kind.uses_box() && box_size < last_close.unwrap_or(0.0).abs() * MIN_BOX_SHARE {
        return Err(format!(
            "Box size {box_size} is too small; choose a larger one."
        ));
    }
    let start = rows.first().map(|r| r.t);
    let (done, forming) = rows.split_at(closed);
    let forming = forming.first();
    let page = opts.end_time_ms.is_some();
    let mut out = match params.kind {
        Kind::Renko => renko(done, forming, box_size, page),
        Kind::LineBreak => line_break(done, forming, params.lines as usize, page),
        Kind::Range => range(done, forming, box_size, page),
        Kind::Kagi => kagi(done, forming, box_size),
        Kind::PointFigure => point_figure(done, forming, box_size, i64::from(params.reversal)),
    }
    .map_err(|why| match params.kind {
        Kind::LineBreak => why,
        _ => format!("{why} Box size {box_size} is too small; choose a larger one."),
    })?;
    if let Some(end) = opts.end_time_ms {
        out.retain(|b| {
            b.date
                .parse::<i64>()
                .is_ok_and(|t| t.saturating_mul(1000) < end)
        });
    } else if let (Some(newest), Some(close)) = (out.last_mut(), last_close) {
        // What alerts and the price line read. The next tail replaces this bar.
        newest.close = close;
        newest.high = newest.high.max(close);
        newest.low = newest.low.min(close);
    }
    Ok(Built {
        bars: out,
        box_size,
        anchor: start,
    })
}

const TOO_MANY: &str = "The chart would have too many bars.";

fn bar(t: i64, open: f64, high: f64, low: f64, close: f64, volume: f64) -> Bar {
    Bar {
        open,
        high,
        low,
        close,
        volume,
        date: t.to_string(),
    }
}

/// Grid index of the highest level at or under `price`. The small margin keeps a
/// price on a level there after float division (0.3 / 0.1 is just under 3).
fn floor_k(price: f64, size: f64) -> i64 {
    (price / size + 1e-9).floor() as i64
}

/// Grid index of the lowest level at or over `price`.
fn ceil_k(price: f64, size: f64) -> i64 {
    (price / size - 1e-9).ceil() as i64
}

/// Collects the bars of the types that close a bar at a time (Renko, Line Break,
/// Range): when the open bar started, and the volume it has taken in.
#[derive(Default)]
struct Bricks {
    bars: Vec<Bar>,
    start: Option<i64>,
    volume: f64,
}

impl Bricks {
    fn take_in(&mut self, r: &Row) {
        self.start.get_or_insert(r.t);
        self.volume += r.volume;
    }

    fn next_t(&self, wanted: i64) -> i64 {
        match self.bars.last().and_then(|b| b.date.parse::<i64>().ok()) {
            Some(prev) => wanted.max(prev + 1),
            None => wanted,
        }
    }

    fn room_for(&self, n: i64) -> Result<(), String> {
        if self.bars.len() as i64 + n > MAX_PRICE_BARS as i64 {
            Err(TOO_MANY.into())
        } else {
            Ok(())
        }
    }

    /// Bars confirmed by the source bar `r`: the first opened when the open bar
    /// did, the others at `r`. They share the volume evenly.
    fn confirm(&mut self, r: &Row, ohlc: &[(f64, f64, f64, f64)]) {
        let each = self.volume / ohlc.len().max(1) as f64;
        for (i, &(o, h, l, c)) in ohlc.iter().enumerate() {
            let wanted = if i == 0 {
                self.start.unwrap_or(r.t)
            } else {
                r.t
            };
            let t = self.next_t(wanted);
            self.bars.push(bar(t, o, h, l, c, each));
        }
        self.start = None;
        self.volume = 0.0;
    }

    /// The open bar, from what is unconfirmed: from `from` to the last price, and
    /// out to any `reach`.
    fn finish(
        mut self,
        forming: Option<&Row>,
        last_close: Option<f64>,
        from: f64,
        reach: (f64, f64),
        page: bool,
    ) -> Vec<Bar> {
        if page {
            return self.bars;
        }
        let start = self.start.or(forming.map(|r| r.t));
        let close = forming.map(|r| r.close).or(last_close);
        if let (Some(start), Some(close)) = (start, close) {
            let t = self.next_t(start);
            let volume = self.volume + forming.map_or(0.0, |r| r.volume);
            self.bars.push(bar(
                t,
                from,
                reach.0.max(from).max(close),
                reach.1.min(from).min(close),
                close,
                volume,
            ));
        }
        self.bars
    }
}

/// Renko on closes, on a grid of multiples of the box: a brick continues when the
/// close is a whole box past the last one's end, and reverses a whole box past its
/// start, so two boxes from its end.
fn renko(rows: &[Row], forming: Option<&Row>, size: f64, page: bool) -> Result<Vec<Bar>, String> {
    let mut out = Bricks::default();
    let seed = rows.first().or(forming).map(|r| r.close);
    let Some(seed) = seed else {
        return Ok(Vec::new());
    };
    let mut bottom = floor_k(seed, size);
    let mut top = bottom + 1;
    let level = |k: i64| k as f64 * size;
    let mut last_close = seed;
    for r in rows {
        out.take_in(r);
        let up = floor_k(r.close, size) - top;
        let down = bottom - ceil_k(r.close, size);
        let n = up.max(down);
        if n < 1 {
            continue;
        }
        out.room_for(n)?;
        let mut bricks = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let (o, c) = if up >= 1 {
                let brick = (level(top), level(top + 1));
                bottom = top;
                top += 1;
                brick
            } else {
                let brick = (level(bottom), level(bottom - 1));
                top = bottom;
                bottom -= 1;
                brick
            };
            bricks.push((o, o.max(c), o.min(c), c));
        }
        last_close = bricks[bricks.len() - 1].3;
        out.confirm(r, &bricks);
    }
    let closed = rows.last().map(|r| r.close);
    Ok(out.finish(forming, closed, last_close, (f64::MIN, f64::MAX), page))
}

/// Line Break on closes: a new line when the close passes the highest or lowest
/// of the last `count` lines. An up line starts at the last line's top, a down line
/// at its bottom. The first line starts at the first close.
fn line_break(
    rows: &[Row],
    forming: Option<&Row>,
    count: usize,
    page: bool,
) -> Result<Vec<Bar>, String> {
    let mut out = Bricks::default();
    let Some(seed) = rows.first().or(forming).map(|r| r.close) else {
        return Ok(Vec::new());
    };
    let mut lines: Vec<(f64, f64)> = Vec::new();
    let mut last_close = seed;
    for r in rows {
        out.take_in(r);
        let recent = &lines[lines.len().saturating_sub(count)..];
        let (high, low) = if recent.is_empty() {
            (seed, seed)
        } else {
            (
                recent.iter().map(|l| l.0).fold(f64::MIN, f64::max),
                recent.iter().map(|l| l.1).fold(f64::MAX, f64::min),
            )
        };
        let prev = lines.last().copied().unwrap_or((seed, seed));
        let open = if r.close > high {
            prev.0
        } else if r.close < low {
            prev.1
        } else {
            continue;
        };
        out.room_for(1)?;
        out.confirm(r, &[(open, open.max(r.close), open.min(r.close), r.close)]);
        lines.push((open.max(r.close), open.min(r.close)));
        last_close = r.close;
    }
    let closed = rows.last().map(|r| r.close);
    Ok(out.finish(forming, closed, last_close, (f64::MIN, f64::MAX), page))
}

/// The path a source bar is taken to have walked, as its tape is not known: open,
/// then the wick away from the close, then the other, then close.
fn path(r: &Row) -> [f64; 4] {
    if r.close >= r.open {
        [r.open, r.low, r.high, r.close]
    } else {
        [r.open, r.high, r.low, r.close]
    }
}

/// Range bars: a bar closes once its high and low are `size` apart, at the side
/// reached, and the next opens there. A gap makes as many bars as it spans.
fn range(rows: &[Row], forming: Option<&Row>, size: f64, page: bool) -> Result<Vec<Bar>, String> {
    let mut out = Bricks::default();
    let Some(seed) = rows.first().or(forming).map(|r| r.open) else {
        return Ok(Vec::new());
    };
    let (mut open, mut high, mut low) = (seed, seed, seed);
    let mut price = seed;
    for r in rows {
        out.take_in(r);
        let mut closed: Vec<(f64, f64, f64, f64)> = Vec::new();
        for p in path(r) {
            price = p;
            loop {
                let close = if p - low >= size {
                    low + size
                } else if high - p >= size {
                    high - size
                } else {
                    high = high.max(p);
                    low = low.min(p);
                    break;
                };
                out.room_for(closed.len() as i64 + 1)?;
                closed.push((open, close.max(high), close.min(low), close));
                (open, high, low) = (close, close, close);
            }
        }
        if !closed.is_empty() {
            out.confirm(r, &closed);
        }
    }
    let reach = match forming {
        Some(f) => (high.max(f.high), low.min(f.low)),
        None => (high, low),
    };
    Ok(out.finish(forming, Some(price), open, reach, page))
}

/// The line or column still going: a bar of Kagi or Point & Figure, one per swing.
struct Leg {
    t: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

impl Leg {
    fn into_bar(self) -> Bar {
        bar(
            self.t,
            self.open,
            self.high,
            self.low,
            self.close,
            self.volume,
        )
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Dir {
    Unset,
    Up,
    Down,
}

fn push_leg(bars: &mut Vec<Bar>, leg: Leg) -> Result<(), String> {
    if bars.len() >= MAX_PRICE_BARS {
        return Err(TOO_MANY.into());
    }
    bars.push(leg.into_bar());
    Ok(())
}

/// Kagi on closes: a line runs on while the close goes its way, and turns at its
/// extreme once the close has come back by `amount`. One bar per line.
fn kagi(rows: &[Row], forming: Option<&Row>, amount: f64) -> Result<Vec<Bar>, String> {
    let Some(first) = rows.first().or(forming) else {
        return Ok(Vec::new());
    };
    let mut bars = Vec::new();
    let mut dir = Dir::Unset;
    let (mut t, mut open, mut extreme, mut volume) = (first.t, first.close, first.close, 0.0);
    for r in rows {
        let turn = match dir {
            Dir::Unset => {
                if (r.close - open).abs() >= amount {
                    dir = if r.close > open { Dir::Up } else { Dir::Down };
                    extreme = r.close;
                }
                false
            }
            Dir::Up if r.close > extreme => {
                extreme = r.close;
                false
            }
            Dir::Down if r.close < extreme => {
                extreme = r.close;
                false
            }
            Dir::Up | Dir::Down => (extreme - r.close).abs() >= amount,
        };
        if turn {
            let leg = Leg {
                t,
                open,
                high: open.max(extreme),
                low: open.min(extreme),
                close: extreme,
                volume,
            };
            push_leg(&mut bars, leg)?;
            dir = if dir == Dir::Up { Dir::Down } else { Dir::Up };
            (t, open, extreme, volume) = (r.t, extreme, r.close, 0.0);
        }
        volume += r.volume;
    }
    let leg = Leg {
        t,
        open,
        high: open.max(extreme),
        low: open.min(extreme),
        close: extreme,
        volume: volume + forming.map_or(0.0, |f| f.volume),
    };
    push_leg(&mut bars, leg)?;
    Ok(bars)
}

/// Point & Figure on closes, on a grid of multiples of the box. A column of Xs runs
/// up to the highest level the close has passed, and turns when the close is
/// `reversal` boxes under its top. The new column starts a box from that top, so a
/// reversal of 3 draws at least 3 of the new mark. One bar per column, from its
/// lowest to its highest level.
fn point_figure(
    rows: &[Row],
    forming: Option<&Row>,
    size: f64,
    reversal: i64,
) -> Result<Vec<Bar>, String> {
    let Some(first) = rows.first().or(forming) else {
        return Ok(Vec::new());
    };
    let level = |k: i64| k as f64 * size;
    let column = |t, dir, top: i64, bottom: i64, volume| {
        let (open, close) = match dir {
            Dir::Down => (level(top), level(bottom)),
            _ => (level(bottom), level(top)),
        };
        Leg {
            t,
            open,
            high: level(top),
            low: level(bottom),
            close,
            volume,
        }
    };
    let mut bars = Vec::new();
    let mut dir = Dir::Unset;
    let start = floor_k(first.close, size);
    let (mut t, mut top, mut bottom, mut volume) = (first.t, start, start, 0.0);
    for r in rows {
        let (at_or_under, at_or_over) = (floor_k(r.close, size), ceil_k(r.close, size));
        match dir {
            Dir::Unset => {
                if at_or_under > top {
                    dir = Dir::Up;
                    top = at_or_under;
                } else if at_or_over < bottom {
                    dir = Dir::Down;
                    bottom = at_or_over;
                }
            }
            Dir::Up if at_or_under > top => top = at_or_under,
            Dir::Down if at_or_over < bottom => bottom = at_or_over,
            Dir::Up if top - at_or_over >= reversal => {
                push_leg(&mut bars, column(t, dir, top, bottom, volume))?;
                dir = Dir::Down;
                (t, top, bottom, volume) = (r.t, top - 1, at_or_over, 0.0);
            }
            Dir::Down if at_or_under - bottom >= reversal => {
                push_leg(&mut bars, column(t, dir, top, bottom, volume))?;
                dir = Dir::Up;
                (t, top, bottom, volume) = (r.t, at_or_under, bottom + 1, 0.0);
            }
            Dir::Up | Dir::Down => {}
        }
        volume += r.volume;
    }
    // Only a closed source bar adds a box or turns the column.
    let leg = column(
        t,
        dir,
        top,
        bottom,
        volume + forming.map_or(0.0, |f| f.volume),
    );
    push_leg(&mut bars, leg)?;
    Ok(bars)
}

#[cfg(test)]
mod tests;
