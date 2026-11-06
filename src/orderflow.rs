//! Footprints and TPO profiles from lower-timeframe bars (ADR-0048). Pure: what
//! fetches the bars is `intrabars` in the server, so all of this runs on fixtures.
//!
//! Klines say how much traded in a minute, not at which price. So a row's volume is
//! always an estimate, from each intrabar's volume spread over its high-low range;
//! the bar's buy and sell totals are only as good as the side of each intrabar.

use crate::data::Bar;
use chrono::{Datelike, TimeZone};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Most rows one bar has: a row size that would give more is widened, for memory
/// and for the payload.
pub const MAX_ROWS_PER_BAR: usize = 200;

#[derive(Clone, Debug, PartialEq)]
pub struct Intrabar {
    /// Open time, unix seconds.
    pub t: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    /// Volume bought by takers, where the source reports it (Binance klines).
    pub taker_buy: Option<f64>,
}

impl Intrabar {
    pub fn from_bar(bar: &Bar, taker_buy: Option<f64>) -> Option<Self> {
        Some(Self {
            t: bar.date.parse().ok()?,
            open: bar.open,
            high: bar.high,
            low: bar.low,
            close: bar.close,
            volume: bar.volume,
            taker_buy,
        })
    }
}

/// How an intrabar's volume was split into buying and selling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The exchange's taker buy volume: the real split, for the whole intrabar.
    TakerVolume,
    /// All of it to one side: close above open buys, below sells, a doji compares
    /// with the previous close, as TradingView documents.
    CloseVersusOpen,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::TakerVolume => "taker_volume",
            Side::CloseVersusOpen => "close_vs_open",
        }
    }
}

/// The share of `bar`'s volume that was buying, 0 to 1, and how it is known.
pub fn buy_share(bar: &Intrabar, prev_close: Option<f64>) -> (f64, Side) {
    if let Some(taker) = bar.taker_buy {
        if bar.volume > 0.0 {
            return ((taker / bar.volume).clamp(0.0, 1.0), Side::TakerVolume);
        }
    }
    let share = if bar.close > bar.open {
        1.0
    } else if bar.close < bar.open {
        0.0
    } else {
        match prev_close {
            Some(prev) if bar.close > prev => 1.0,
            Some(prev) if bar.close < prev => 0.0,
            _ => 0.5,
        }
    };
    (share, Side::CloseVersusOpen)
}

/// `value` rounded to `digits` significant digits: full doubles only bloat the payload.
pub fn round_sig(value: f64, digits: i32) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return 0.0;
    }
    let scale = 10f64.powi(digits - 1 - value.abs().log10().floor() as i32);
    (value * scale).round() / scale
}

/// Smallest power of ten all `prices` are multiples of, down to 1e-8. Data with more
/// decimals than that has no tick to read, so it goes by the price: 1/10000th of it.
pub fn infer_tick(prices: &[f64]) -> f64 {
    for d in 0..=8 {
        let scale = 10f64.powi(d);
        if prices
            .iter()
            .all(|p| ((p * scale).round() / scale - p).abs() < 1e-10)
        {
            return 1.0 / scale;
        }
    }
    let top = prices.iter().cloned().fold(0.0_f64, f64::max);
    10f64.powi((top.max(1e-8).log10().floor() as i32) - 4)
}

/// Wilder's ATR of `bars` over `length`; `None` with fewer than `length + 1` bars.
pub fn atr(bars: &[Bar], length: usize) -> Option<f64> {
    if length == 0 || bars.len() < length + 1 {
        return None;
    }
    let range = |i: usize| {
        let (b, prev) = (&bars[i], bars[i - 1].close);
        (b.high - b.low)
            .max((b.high - prev).abs())
            .max((b.low - prev).abs())
    };
    let seed: f64 = (1..=length).map(range).sum::<f64>() / length as f64;
    let n = length as f64;
    Some((length + 1..bars.len()).fold(seed, |a, i| (a * (n - 1.0) + range(i)) / n))
}

/// Row size for `raw`, on the ticks: at least one tick, rounded to 1, 2, 2.5 or 5
/// times a power of ten, so that row prices read well and stay put as `raw` drifts.
pub fn nice_row_size(raw: f64, tick: f64) -> f64 {
    if !(raw.is_finite() && raw > 0.0) {
        return tick;
    }
    let magnitude = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0]
        .into_iter()
        .rfind(|s| s * magnitude <= raw * (1.0 + 1e-9))
        .unwrap_or(1.0);
    let row = ((step * magnitude / tick).round().max(1.0)) * tick;
    round_sig(row, 10)
}

/// TradingView's auto row size is 0.2 times a normalized ATR. It does not publish
/// the normalization, so this is 0.2 times the ATR, rounded by `nice_row_size`.
pub fn auto_row_size(atr: f64, tick: f64) -> f64 {
    nice_row_size(0.2 * atr, tick)
}

/// `row`, widened if needed so that a range of `range` has at most `MAX_ROWS_PER_BAR`.
pub fn bounded_row_size(row: f64, range: f64, tick: f64) -> f64 {
    if range / row <= MAX_ROWS_PER_BAR as f64 {
        return row;
    }
    let wide = nice_row_size(range / MAX_ROWS_PER_BAR as f64, tick);
    if range / wide <= MAX_ROWS_PER_BAR as f64 {
        wide
    } else {
        nice_row_size(2.0 * wide, tick)
    }
}

/// The row to use for `asked`: not under a tick, not over the highest price `top`, and
/// fitting a bar of `widest` in `MAX_ROWS_PER_BAR`.
pub fn usable_row(asked: f64, widest: f64, top: f64, tick: f64) -> f64 {
    bounded_row_size(asked.clamp(tick, top.max(tick)), widest, tick)
}

/// The row of a footprint of `bars`, whose rows are built from `intrabars` in `groups`.
/// Those are read after the bars, so a bar that read flat says nothing of how many rows
/// they need: what has to fit is what gets bucketed.
pub fn footprint_row(
    asked: f64,
    bars: &[Bar],
    intrabars: &[Intrabar],
    groups: &[std::ops::Range<usize>],
    tick: f64,
) -> f64 {
    let widest = groups
        .iter()
        .map(|g| extent(&intrabars[g.clone()]))
        .chain(bars.iter().map(|b| b.high - b.low))
        .fold(0.0, f64::max);
    let top = bars
        .iter()
        .map(|b| b.high)
        .chain(intrabars.iter().map(|b| b.high))
        .fold(0.0, f64::max);
    usable_row(asked, widest, top, tick)
}

/// High minus low of `intrabars`, ignoring prices that are not numbers. 0 without any.
fn extent(intrabars: &[Intrabar]) -> f64 {
    let finite = || intrabars.iter().filter(|b| has_range(b));
    let high = finite().map(|b| b.high).fold(f64::NEG_INFINITY, f64::max);
    let low = finite().map(|b| b.low).fold(f64::INFINITY, f64::min);
    (high - low).max(0.0)
}

fn has_range(bar: &Intrabar) -> bool {
    bar.low.is_finite() && bar.high.is_finite()
}

/// Row of `price`: rows are multiples of the size, so they line up across bars.
fn row_index(price: f64, row: f64) -> i64 {
    (price / row + 1e-9).floor() as i64
}

fn row_price(index: i64, row: f64) -> f64 {
    round_sig(index as f64 * row, 12)
}

/// `(row index, share of the range)` of every row a high-low range touches: by
/// overlap, so a range clipping a row counts for that much of it. A high exactly on a
/// row's bottom touches it, with a share of 0.
fn rows_touched(low: f64, high: f64, row: f64) -> Vec<(i64, f64)> {
    let (first, last) = (row_index(low, row), row_index(high, row));
    let range = high - low;
    if first == last || range <= 0.0 {
        return vec![(first, 1.0)];
    }
    (first..=last)
        .map(|i| {
            let lo = (i as f64 * row).max(low);
            let hi = ((i + 1) as f64 * row).min(high);
            (i, ((hi - lo) / range).max(0.0))
        })
        .collect()
}

/// The rows a range holds volume on: a price touched is no volume traded there.
fn row_shares(low: f64, high: f64, row: f64) -> Vec<(i64, f64)> {
    let mut rows = rows_touched(low, high, row);
    rows.retain(|(_, share)| *share > 0.0);
    rows
}

/// Whether a value area grows down next, as TradingView documents for TPO and for its
/// volume profiles: the busier row, on a tie the one closest to the POC, and then the
/// higher one. `None`: no row left on that side. `reach`: rows from the POC to each.
pub fn value_area_grows_down(
    below: Option<f64>,
    above: Option<f64>,
    reach_below: usize,
    reach_above: usize,
) -> bool {
    match (below, above) {
        (Some(b), Some(a)) => b > a || (b == a && reach_below < reach_above),
        (below, _) => below.is_some(),
    }
}

/// `(low row, high row, row of most)` of the rows holding `pct` of the total: from
/// the most traded row, adding the busier neighbour until they do, ties as
/// `value_area_grows_down` says. The tie for the most goes to the row nearest the middle.
pub fn value_area(values: &[f64], pct: f64) -> Option<(usize, usize, usize)> {
    let total: f64 = values.iter().sum();
    if values.is_empty() || total <= 0.0 {
        return None;
    }
    let middle = (values.len() - 1) as f64 / 2.0;
    let poc = (0..values.len()).fold(0, |best, i| {
        let closer = (i as f64 - middle).abs() < (best as f64 - middle).abs();
        if values[i] > values[best] || (values[i] == values[best] && closer) {
            i
        } else {
            best
        }
    });
    let target = total * pct / 100.0;
    let (mut low, mut high, mut sum) = (poc, poc, values[poc]);
    while sum < target && (low > 0 || high + 1 < values.len()) {
        let below = low.checked_sub(1).map(|i| values[i]);
        let above = values.get(high + 1).copied();
        if value_area_grows_down(below, above, poc - low, high - poc) {
            low -= 1;
            sum += values[low];
        } else {
            high += 1;
            sum += values[high];
        }
    }
    Some((low, high, poc))
}

/// A price level of a footprint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    /// Bottom of the row.
    pub price: f64,
    pub buy: f64,
    pub sell: f64,
}

/// Rows of `(buy, sell)` where the buy at a level is at least `pct` percent of the
/// sell one level below, and the sell at least `pct` percent of the buy one level
/// above. A level with nothing to compare with is no imbalance: at 1-minute data a
/// row at the edge of a bar is often empty on one side for no reason in the market.
pub fn imbalances(rows: &[Row], pct: f64) -> (Vec<bool>, Vec<bool>) {
    let ratio = pct / 100.0;
    let beats = |value: f64, against: f64| against > 0.0 && value >= ratio * against;
    let buy = (0..rows.len())
        .map(|i| i > 0 && beats(rows[i].buy, rows[i - 1].sell))
        .collect();
    let sell = (0..rows.len())
        .map(|i| {
            rows.get(i + 1)
                .is_some_and(|above| beats(rows[i].sell, above.buy))
        })
        .collect();
    (buy, sell)
}

/// Runs of at least `min` consecutive true flags, as `(first, last)` indices.
pub fn stacked_runs(flags: &[bool], min: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, &on) in flags.iter().chain(std::iter::once(&false)).enumerate() {
        match (on, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                if i - s >= min.max(1) {
                    out.push((s, i - 1));
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// Intrabars that must have traded on a level, and on the one it is compared with, for
/// it to count towards a stacked imbalance.
const MIN_STACKED_INTRABARS: usize = 2;

/// A stack of imbalances in one bar: a zone that holds until price comes back.
#[derive(Clone, Debug, PartialEq)]
pub struct Stacked {
    pub buy: bool,
    pub low: f64,
    pub high: f64,
    pub count: usize,
}

/// One bar's footprint.
#[derive(Clone, Debug, PartialEq)]
pub struct Footprint {
    pub t: i64,
    /// Height of this bar's rows: the asked one unless it would not fit (`usable_row`).
    pub row: f64,
    /// Ascending, contiguous from the bar's low to its high. Empty without intrabars.
    pub rows: Vec<Row>,
    pub buy: f64,
    pub sell: f64,
    /// Lowest and highest running delta within the bar, intrabar by intrabar.
    pub min_delta: f64,
    pub max_delta: f64,
    pub poc: Option<f64>,
    pub vah: Option<f64>,
    pub val: Option<f64>,
    /// Row prices of the imbalances.
    pub buy_imbalances: Vec<f64>,
    pub sell_imbalances: Vec<f64>,
    pub stacked: Vec<Stacked>,
}

/// What a footprint is computed with.
#[derive(Clone, Copy, Debug)]
pub struct FootprintOptions {
    pub row: f64,
    /// Value area, percent of the volume.
    pub value_area: f64,
    /// Imbalance, percent.
    pub imbalance: f64,
    /// Levels in a row that make a stacked imbalance.
    pub stacked: usize,
}

/// The footprint of a bar opening at `t` from its intrabars, oldest first.
pub fn build_footprint(
    t: i64,
    intrabars: &[Intrabar],
    prev_close: Option<f64>,
    o: &FootprintOptions,
) -> Footprint {
    // The hard cap: the caller bounds the row for all its bars, but no row size may
    // ever size this bar's rows beyond the limit, whatever the data does.
    let row = bounded_row_size(o.row, extent(intrabars), o.row);
    // Buy, sell, and how many intrabars traded there.
    let mut levels: BTreeMap<i64, (f64, f64, usize)> = BTreeMap::new();
    let (mut delta, mut min_delta, mut max_delta) = (0.0_f64, 0.0_f64, 0.0_f64);
    let (mut buy, mut sell) = (0.0, 0.0);
    let mut prev = prev_close;
    for bar in intrabars.iter().filter(|b| has_range(b)) {
        let (share, _) = buy_share(bar, prev);
        prev = Some(bar.close);
        let (bought, sold) = (bar.volume * share, bar.volume * (1.0 - share));
        buy += bought;
        sell += sold;
        delta += bought - sold;
        min_delta = min_delta.min(delta);
        max_delta = max_delta.max(delta);
        for (index, part) in row_shares(bar.low, bar.high, row) {
            let level = levels.entry(index).or_default();
            level.0 += bought * part;
            level.1 += sold * part;
            level.2 += usize::from(bar.volume > 0.0);
        }
    }
    let mut fp = Footprint {
        t,
        row,
        rows: Vec::new(),
        buy,
        sell,
        min_delta,
        max_delta,
        poc: None,
        vah: None,
        val: None,
        buy_imbalances: Vec::new(),
        sell_imbalances: Vec::new(),
        stacked: Vec::new(),
    };
    let (Some(&first), Some(&last)) = (levels.keys().next(), levels.keys().next_back()) else {
        return fp;
    };
    fp.rows = (first..=last)
        .map(|i| {
            let (b, s, _) = levels.get(&i).copied().unwrap_or_default();
            Row {
                price: row_price(i, row),
                buy: b,
                sell: s,
            }
        })
        .collect();
    let totals: Vec<f64> = fp.rows.iter().map(|r| r.buy + r.sell).collect();
    if let Some((low, high, poc)) = value_area(&totals, o.value_area) {
        fp.poc = Some(fp.rows[poc].price);
        fp.val = Some(fp.rows[low].price);
        fp.vah = Some(round_sig(fp.rows[high].price + row, 12));
    }
    let (buys, sells) = imbalances(&fp.rows, o.imbalance);
    let prices = |flags: &[bool]| -> Vec<f64> {
        flags
            .iter()
            .zip(&fp.rows)
            .filter(|(on, _)| **on)
            .map(|(_, r)| r.price)
            .collect()
    };
    fp.buy_imbalances = prices(&buys);
    fp.sell_imbalances = prices(&sells);
    // One kline has one split and is spread evenly, so rows it alone reaches all agree
    // whenever its split passes the ratio: that is arithmetic, not levels of aggression.
    // A marker on a cell can say so much; a zone projected across the chart cannot.
    let traded = |row_at: usize| {
        let index = first + row_at as i64;
        levels.get(&index).map_or(0, |l| l.2) >= MIN_STACKED_INTRABARS
    };
    let stacked_buys: Vec<bool> = (0..buys.len())
        .map(|i| buys[i] && traded(i) && traded(i - 1))
        .collect();
    let stacked_sells: Vec<bool> = (0..sells.len())
        .map(|i| sells[i] && traded(i) && traded(i + 1))
        .collect();
    for (flags, is_buy) in [(&stacked_buys, true), (&stacked_sells, false)] {
        for (a, b) in stacked_runs(flags, o.stacked) {
            fp.stacked.push(Stacked {
                buy: is_buy,
                low: fp.rows[a].price,
                high: round_sig(fp.rows[b].price + row, 12),
                count: b - a + 1,
            });
        }
    }
    fp
}

impl Footprint {
    pub fn to_json(&self) -> Value {
        let rows: Vec<Value> = self
            .rows
            .iter()
            .map(|r| json!([r.price, round_sig(r.buy, 6), round_sig(r.sell, 6)]))
            .collect();
        let stacked: Vec<Value> = self
            .stacked
            .iter()
            .map(|s| {
                json!({
                    "side": if s.buy { "buy" } else { "sell" },
                    "low": s.low,
                    "high": s.high,
                    "count": s.count,
                })
            })
            .collect();
        json!({
            "t": self.t,
            "row": self.row,
            "buy": round_sig(self.buy, 8),
            "sell": round_sig(self.sell, 8),
            "min_delta": round_sig(self.min_delta, 8),
            "max_delta": round_sig(self.max_delta, 8),
            "poc": self.poc,
            "vah": self.vah,
            "val": self.val,
            "rows": rows,
            "buy_imbalances": self.buy_imbalances,
            "sell_imbalances": self.sell_imbalances,
            "stacked": stacked,
        })
    }
}

/// For each of the bars opening at `opens` (ascending), the range of `intrabars`
/// (ascending) inside it. A bar's end is the next bar's open, so bars with gaps
/// between them (a stock's nights) and calendar months need no interval here. The
/// last bar ends at `last_end`.
pub fn group_by_bar(
    opens: &[i64],
    last_end: i64,
    intrabars: &[Intrabar],
) -> Vec<std::ops::Range<usize>> {
    let start = |t: i64| intrabars.partition_point(|b| b.t < t);
    opens
        .iter()
        .enumerate()
        .map(|(k, &t)| {
            let end = opens.get(k + 1).copied().unwrap_or(last_end);
            start(t)..start(end)
        })
        .collect()
}

// ---- TPO ----

/// Letter of a block: A to Z, then a to z, then around again. TradingView's chart
/// colours them by those two ranges too.
pub fn tpo_letter(block: usize) -> char {
    const LETTERS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    LETTERS[block % LETTERS.len()] as char
}

/// A price level of a TPO profile.
#[derive(Clone, Debug, PartialEq)]
pub struct TpoRow {
    pub price: f64,
    /// The blocks that traded here, ascending.
    pub blocks: Vec<u32>,
    /// The blocks' volume, each spread over its range.
    pub volume: f64,
}

/// One period's profile.
#[derive(Clone, Debug, PartialEq)]
pub struct TpoProfile {
    /// The start of its period, plus its session's: what identifies it between replies.
    /// The first block can be later, and with a window that moves it would move too.
    pub t: i64,
    pub end: i64,
    /// Height of its rows.
    pub row: f64,
    /// Open time of each block, for splitting the profile by blocks.
    pub block_times: Vec<i64>,
    pub rows: Vec<TpoRow>,
    pub open: f64,
    pub close: f64,
    pub high: f64,
    pub low: f64,
    pub poc: f64,
    pub vah: f64,
    pub val: f64,
    /// More than one block at the extreme: the auction did not finish there.
    pub poor_high: bool,
    pub poor_low: bool,
    /// Runs of rows with one block, as `(low, high)`, without those at the extremes:
    /// a tail is not a gap in the profile.
    pub single_prints: Vec<(f64, f64)>,
    /// High and low of the first `ib_blocks` blocks.
    pub ib_high: f64,
    pub ib_low: f64,
    pub total_blocks: usize,
    pub blocks_above_poc: usize,
    pub blocks_below_poc: usize,
    pub rotation_factor: i64,
    pub volume: f64,
}

/// Rotation factor, as in market profile practice: each block adds 1 for a higher
/// high and 1 for a higher low, and takes 1 for a lower high and 1 for a lower low.
pub fn rotation_factor(blocks: &[Intrabar]) -> i64 {
    blocks
        .windows(2)
        .map(|w| {
            let step = |now: f64, before: f64| match now.partial_cmp(&before) {
                Some(std::cmp::Ordering::Greater) => 1,
                Some(std::cmp::Ordering::Less) => -1,
                _ => 0,
            };
            step(w[1].high, w[0].high) + step(w[1].low, w[0].low)
        })
        .sum()
}

/// Build a profile from a period's blocks, oldest first. `None` without any.
pub fn build_tpo(
    t: i64,
    end: i64,
    blocks: &[Intrabar],
    row: f64,
    value_area_pct: f64,
    ib_blocks: usize,
) -> Option<TpoProfile> {
    let (first, last) = (blocks.first()?, blocks.last()?);
    let mut levels: BTreeMap<i64, TpoRow> = BTreeMap::new();
    for (k, block) in blocks.iter().enumerate() {
        // A block prints wherever it traded, its high included, however little of its
        // range that row has.
        for (index, part) in rows_touched(block.low, block.high, row) {
            let level = levels.entry(index).or_insert_with(|| TpoRow {
                price: row_price(index, row),
                blocks: Vec::new(),
                volume: 0.0,
            });
            level.blocks.push(k as u32);
            level.volume += block.volume * part;
        }
    }
    let (&low_index, &high_index) = (levels.keys().next()?, levels.keys().next_back()?);
    // A level no block reached still takes its place, so rows are contiguous.
    let rows: Vec<TpoRow> = (low_index..=high_index)
        .map(|i| {
            levels.remove(&i).unwrap_or_else(|| TpoRow {
                price: row_price(i, row),
                blocks: Vec::new(),
                volume: 0.0,
            })
        })
        .collect();
    let counts: Vec<f64> = rows.iter().map(|r| r.blocks.len() as f64).collect();
    let (va_low, va_high, poc) = value_area(&counts, value_area_pct)?;
    let top = rows.len() - 1;
    let mut single_prints = Vec::new();
    let flags: Vec<bool> = rows.iter().map(|r| r.blocks.len() == 1).collect();
    for (a, b) in stacked_runs(&flags, 1) {
        if a > 0 && b < top {
            single_prints.push((rows[a].price, round_sig(rows[b].price + row, 12)));
        }
    }
    let high = blocks.iter().map(|b| b.high).fold(f64::MIN, f64::max);
    let low = blocks.iter().map(|b| b.low).fold(f64::MAX, f64::min);
    let ib = &blocks[..ib_blocks.clamp(1, blocks.len())];
    let sum = |range: &[TpoRow]| range.iter().map(|r| r.blocks.len()).sum::<usize>();
    Some(TpoProfile {
        t,
        end,
        row,
        block_times: blocks.iter().map(|b| b.t).collect(),
        open: first.open,
        close: last.close,
        high,
        low,
        poc: rows[poc].price,
        vah: round_sig(rows[va_high].price + row, 12),
        val: rows[va_low].price,
        poor_high: rows[top].blocks.len() > 1,
        poor_low: rows[0].blocks.len() > 1,
        single_prints,
        ib_high: ib.iter().map(|b| b.high).fold(f64::MIN, f64::max),
        ib_low: ib.iter().map(|b| b.low).fold(f64::MAX, f64::min),
        total_blocks: sum(&rows),
        blocks_above_poc: sum(&rows[poc + 1..]),
        blocks_below_poc: sum(&rows[..poc]),
        rotation_factor: rotation_factor(blocks),
        volume: blocks.iter().map(|b| b.volume).sum(),
        rows,
    })
}

impl TpoProfile {
    pub fn to_json(&self) -> Value {
        let rows: Vec<Value> = self
            .rows
            .iter()
            .map(|r| {
                let letters: String = r.blocks.iter().map(|&b| tpo_letter(b as usize)).collect();
                json!({
                    "p": r.price,
                    "letters": letters,
                    "blocks": r.blocks,
                    "v": round_sig(r.volume, 6),
                })
            })
            .collect();
        let singles: Vec<Value> = self
            .single_prints
            .iter()
            .map(|s| json!([s.0, s.1]))
            .collect();
        json!({
            "t": self.t,
            "end": self.end,
            "row": self.row,
            "block_times": self.block_times,
            "open": self.open,
            "close": self.close,
            "high": self.high,
            "low": self.low,
            "mid": round_sig((self.high + self.low) / 2.0, 12),
            "poc": self.poc,
            "vah": self.vah,
            "val": self.val,
            "poor_high": self.poor_high,
            "poor_low": self.poor_low,
            "single_prints": singles,
            "ib_high": self.ib_high,
            "ib_low": self.ib_low,
            "total_blocks": self.total_blocks,
            "blocks_above_poc": self.blocks_above_poc,
            "blocks_below_poc": self.blocks_below_poc,
            "rotation_factor": self.rotation_factor,
            "volume": round_sig(self.volume, 8),
            "rows": rows,
        })
    }
}

/// A TPO period: `n` days, weeks or months.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    Days(u32),
    Weeks(u32),
    Months(u32),
}

impl Period {
    /// `1D`, `2D`, `1W`, `1M`: a count and a unit, the count from 1 to 30.
    pub fn parse(text: &str) -> Result<Self, String> {
        let bad = || format!("period must be a count and D, W or M, such as 1D or 1W: {text}");
        let unit = text.chars().last().ok_or_else(bad)?;
        let n: u32 = text[..text.len() - unit.len_utf8()]
            .parse()
            .map_err(|_| bad())?;
        if !(1..=30).contains(&n) {
            return Err(bad());
        }
        match unit {
            'D' | 'd' => Ok(Period::Days(n)),
            'W' | 'w' => Ok(Period::Weeks(n)),
            'M' => Ok(Period::Months(n)),
            _ => Err(bad()),
        }
    }

    /// Longest it can last, in seconds.
    pub fn max_secs(self) -> i64 {
        match self {
            Period::Days(n) => i64::from(n) * 86_400,
            Period::Weeks(n) => i64::from(n) * 7 * 86_400,
            Period::Months(n) => i64::from(n) * 31 * 86_400,
        }
    }

    /// Start (unix s) of the period holding `t`, on the calendar of `tz`.
    pub fn start_of<Tz: TimeZone>(self, t: i64, tz: &Tz) -> i64 {
        let Some(local) = tz.timestamp_opt(t, 0).earliest() else {
            return t;
        };
        let date = local.date_naive();
        let start = match self {
            Period::Days(n) => {
                let days = i64::from(date.num_days_from_ce());
                date - chrono::Duration::days(days.rem_euclid(i64::from(n)))
            }
            Period::Weeks(n) => {
                let monday =
                    date - chrono::Duration::days(i64::from(date.weekday().num_days_from_monday()));
                let weeks = i64::from(monday.num_days_from_ce()) / 7;
                monday - chrono::Duration::weeks(weeks.rem_euclid(i64::from(n)))
            }
            Period::Months(n) => {
                let months = date.year() * 12 + date.month0() as i32;
                let first = months - months.rem_euclid(n as i32);
                chrono::NaiveDate::from_ymd_opt(
                    first.div_euclid(12),
                    first.rem_euclid(12) as u32 + 1,
                    1,
                )
                .unwrap_or(date)
            }
        };
        start
            .and_hms_opt(0, 0, 0)
            .and_then(|d| tz.from_local_datetime(&d).earliest())
            .map_or(t, |d| d.timestamp())
    }
}

/// A custom session in the exchange's time: minutes of the day, from and to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DaySession {
    pub from: u32,
    pub to: u32,
}

impl DaySession {
    /// `HH:MM-HH:MM`, on one day.
    pub fn parse(text: &str) -> Result<Self, String> {
        let bad = || format!("a session is HH:MM-HH:MM, later than it starts, on one day: {text}");
        let minutes = |s: &str| -> Option<u32> {
            let (h, m) = s.trim().split_once(':')?;
            let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
            (h <= 24 && m < 60 && h * 60 + m <= 1440).then_some(h * 60 + m)
        };
        let (a, b) = text.split_once('-').ok_or_else(bad)?;
        let (from, to) = (minutes(a).ok_or_else(bad)?, minutes(b).ok_or_else(bad)?);
        if from >= to {
            return Err(bad());
        }
        Ok(Self { from, to })
    }

    pub fn contains<Tz: TimeZone>(&self, t: i64, tz: &Tz) -> bool {
        use chrono::Timelike;
        let Some(local) = tz.timestamp_opt(t, 0).earliest() else {
            return false;
        };
        let minute = local.hour() * 60 + local.minute();
        (self.from..self.to).contains(&minute)
    }
}

/// What TPO profiles are built with.
#[derive(Clone, Copy, Debug)]
pub struct TpoOptions {
    /// Block length, seconds: a profile ends where its last block does.
    pub block_secs: i64,
    pub row: f64,
    pub value_area: f64,
    pub ib_blocks: usize,
}

/// Group `blocks` (oldest first) by period, and with `sessions` also by session, in
/// time order, each under its key: the period's start, plus when its session starts.
fn tpo_groups<Tz: TimeZone>(
    blocks: &[Intrabar],
    period: Period,
    sessions: &[DaySession],
    tz: &Tz,
) -> BTreeMap<i64, Vec<Intrabar>> {
    let mut groups: BTreeMap<i64, Vec<Intrabar>> = BTreeMap::new();
    for block in blocks {
        let start = period.start_of(block.t, tz);
        if sessions.is_empty() {
            groups.entry(start).or_default().push(block.clone());
        } else if let Some(s) = sessions.iter().find(|s| s.contains(block.t, tz)) {
            groups
                .entry(start + i64::from(s.from) * 60)
                .or_default()
                .push(block.clone());
        }
    }
    groups
}

/// The shortest of the usual block lengths from `asked` up that can hold a whole
/// `period` in `max_blocks`, and its seconds: `asked` itself if it can. A shorter one
/// would leave a profile with only the end of its period, with the wrong open, letters
/// and initial balance. `None`: even the longest cannot.
pub fn block_for_period(
    asked: (&str, i64),
    period: Period,
    max_blocks: i64,
) -> Option<(String, i64)> {
    const LONGER: [(&str, i64); 5] = [
        ("15m", 900),
        ("30m", 1_800),
        ("1h", 3_600),
        ("2h", 7_200),
        ("4h", 14_400),
    ];
    let holds = |secs: i64| period.max_secs() / secs <= max_blocks;
    if holds(asked.1) {
        return Some((asked.0.to_string(), asked.1));
    }
    LONGER
        .iter()
        .find(|(_, secs)| *secs > asked.1 && holds(*secs))
        .map(|(name, secs)| (name.to_string(), *secs))
}

/// Auto row size of a TPO chart. TradingView does not publish its rule: this is a
/// 24th of the median range of the periods, so a typical profile has about 24 rows.
pub fn tpo_auto_row<Tz: TimeZone>(
    blocks: &[Intrabar],
    period: Period,
    sessions: &[DaySession],
    tz: &Tz,
    tick: f64,
) -> f64 {
    let mut ranges: Vec<f64> = tpo_groups(blocks, period, sessions, tz)
        .values()
        .map(|g| {
            let high = g.iter().map(|b| b.high).fold(f64::MIN, f64::max);
            let low = g.iter().map(|b| b.low).fold(f64::MAX, f64::min);
            high - low
        })
        .collect();
    ranges.sort_by(f64::total_cmp);
    let median = ranges.get(ranges.len() / 2).copied().unwrap_or(0.0);
    nice_row_size(median / 24.0, tick)
}

/// The widest range of a single profile, which is what a row size has to fit.
pub fn tpo_widest_range<Tz: TimeZone>(
    blocks: &[Intrabar],
    period: Period,
    sessions: &[DaySession],
    tz: &Tz,
) -> f64 {
    tpo_groups(blocks, period, sessions, tz)
        .values()
        .map(|g| {
            let high = g.iter().map(|b| b.high).fold(f64::MIN, f64::max);
            let low = g.iter().map(|b| b.low).fold(f64::MAX, f64::min);
            high - low
        })
        .fold(0.0, f64::max)
}

/// The profiles of `blocks` (oldest first): one per period, or with `sessions` one
/// per session of each period.
pub fn tpo_profiles<Tz: TimeZone>(
    blocks: &[Intrabar],
    period: Period,
    sessions: &[DaySession],
    tz: &Tz,
    o: &TpoOptions,
) -> Vec<TpoProfile> {
    tpo_groups(blocks, period, sessions, tz)
        .iter()
        .filter_map(|(t, g)| {
            let last = g.last()?;
            build_tpo(
                *t,
                last.t + o.block_secs,
                g,
                o.row,
                o.value_area,
                o.ib_blocks,
            )
        })
        .collect()
}

// ---- Sample data ----

fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn unit(x: u64) -> f64 {
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// Intrabars inside a sample `bar` opening at `t`: a walk from its open to its
/// extremes and its close, with its volume. The sample source draws each interval
/// on its own, so its 1m bars do not add up to its 1h bars; these do. `count` of
/// `slots` are there, which is fewer than all of them in the forming bar. With
/// `taker` they carry a taker buy volume, as Binance's do.
pub fn sample_intrabars(
    seed: u64,
    t: i64,
    bar: &Bar,
    secs: i64,
    slots: usize,
    count: usize,
    taker: bool,
) -> Vec<Intrabar> {
    let count = count.clamp(1, slots.max(1));
    let up = bar.close >= bar.open;
    let (first, second) = if up {
        (bar.low, bar.high)
    } else {
        (bar.high, bar.low)
    };
    // Open, first extreme, second extreme, close, at these fractions of the walk.
    let at = |x: f64| -> f64 {
        let leg = |a: f64, b: f64, f: f64| a + (b - a) * f;
        if x < 0.25 {
            leg(bar.open, first, x / 0.25)
        } else if x < 0.75 {
            leg(first, second, (x - 0.25) / 0.5)
        } else {
            leg(second, bar.close, (x - 0.75) / 0.25)
        }
    };
    let weights: Vec<f64> = (0..count)
        .map(|k| 0.2 + unit(mix(seed ^ (t as u64).wrapping_add(k as u64))))
        .collect();
    let total: f64 = weights.iter().sum();
    (0..count)
        .map(|k| {
            let a = if k == 0 {
                bar.open
            } else {
                at(k as f64 / count as f64)
            };
            let b = if k + 1 == count {
                bar.close
            } else {
                at((k + 1) as f64 / count as f64)
            };
            let volume = bar.volume * weights[k] / total;
            let noise = unit(mix(seed ^ (t as u64).wrapping_mul(31) ^ k as u64));
            let lean = if b >= a { 0.6 } else { 0.4 };
            Intrabar {
                t: t + k as i64 * secs,
                open: a,
                high: a.max(b),
                low: a.min(b),
                close: b,
                volume,
                taker_buy: taker.then_some(volume * (lean + (noise - 0.5) * 0.3)),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ib(t: i64, o: f64, h: f64, l: f64, c: f64, v: f64) -> Intrabar {
        Intrabar {
            t,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: v,
            taker_buy: None,
        }
    }

    fn opts(row: f64) -> FootprintOptions {
        FootprintOptions {
            row,
            value_area: 70.0,
            imbalance: 300.0,
            stacked: 3,
        }
    }

    fn rows(values: &[(f64, f64)]) -> Vec<Row> {
        values
            .iter()
            .enumerate()
            .map(|(i, &(buy, sell))| Row {
                price: i as f64,
                buy,
                sell,
            })
            .collect()
    }

    // ---- side ----

    #[test]
    fn side_follows_close_versus_open() {
        let (up, side) = buy_share(&ib(0, 10.0, 11.0, 9.0, 10.5, 1.0), None);
        assert_eq!((up, side), (1.0, Side::CloseVersusOpen));
        assert_eq!(buy_share(&ib(0, 10.0, 11.0, 9.0, 9.5, 1.0), None).0, 0.0);
    }

    #[test]
    fn a_doji_compares_with_the_previous_close() {
        let doji = ib(0, 10.0, 11.0, 9.0, 10.0, 1.0);
        assert_eq!(buy_share(&doji, Some(9.0)).0, 1.0);
        assert_eq!(buy_share(&doji, Some(11.0)).0, 0.0);
        // Nothing moved at all: neither side.
        assert_eq!(buy_share(&doji, Some(10.0)).0, 0.5);
        assert_eq!(buy_share(&doji, None).0, 0.5);
    }

    #[test]
    fn taker_volume_gives_the_real_split_whatever_the_candle_did() {
        let mut bar = ib(0, 10.0, 11.0, 9.0, 9.5, 10.0);
        bar.taker_buy = Some(7.0);
        assert_eq!(buy_share(&bar, None), (0.7, Side::TakerVolume));
        // More than the volume is bad data, not more than everything.
        bar.taker_buy = Some(12.0);
        assert_eq!(buy_share(&bar, None).0, 1.0);
        // No volume: nothing to split, so the candle says.
        bar.volume = 0.0;
        assert_eq!(buy_share(&bar, None), (0.0, Side::CloseVersusOpen));
    }

    // ---- rows ----

    #[test]
    fn a_range_spreads_over_its_rows_by_overlap() {
        let got = row_shares(10.5, 12.5, 1.0);
        assert_eq!(got, vec![(10, 0.25), (11, 0.5), (12, 0.25)]);
        assert_eq!(row_shares(10.2, 10.8, 1.0), vec![(10, 1.0)]);
        assert_eq!(row_shares(10.0, 10.0, 1.0), vec![(10, 1.0)]);
        // A high on a row's bottom takes nothing from it.
        assert_eq!(row_shares(10.0, 11.0, 1.0), vec![(10, 1.0)]);
    }

    #[test]
    fn rows_line_up_across_bars_and_survive_float_noise() {
        assert_eq!(row_index(0.3, 0.1), 3);
        assert_eq!(row_index(100.0, 0.25), 400);
        assert_eq!(row_price(3, 0.1), 0.3);
    }

    #[test]
    fn footprint_buckets_volume_and_keeps_every_row_contiguous() {
        // Two rows apart: the row between them is empty but present.
        let bars = [
            ib(0, 10.0, 10.5, 10.0, 10.5, 4.0),
            ib(60, 12.5, 12.5, 12.0, 12.0, 6.0),
        ];
        let fp = build_footprint(0, &bars, None, &opts(1.0));
        assert_eq!(
            fp.rows,
            vec![
                Row {
                    price: 10.0,
                    buy: 4.0,
                    sell: 0.0
                },
                Row {
                    price: 11.0,
                    buy: 0.0,
                    sell: 0.0
                },
                Row {
                    price: 12.0,
                    buy: 0.0,
                    sell: 6.0
                },
            ]
        );
        assert_eq!((fp.buy, fp.sell), (4.0, 6.0));
    }

    #[test]
    fn min_and_max_delta_are_of_the_running_delta() {
        let bars = [
            ib(0, 10.0, 11.0, 10.0, 11.0, 5.0),
            ib(60, 11.0, 11.0, 9.0, 9.0, 8.0),
            ib(120, 9.0, 10.0, 9.0, 10.0, 1.0),
        ];
        let fp = build_footprint(0, &bars, None, &opts(1.0));
        assert_eq!(fp.max_delta, 5.0);
        assert_eq!(fp.min_delta, -3.0);
        assert_eq!(fp.buy - fp.sell, -2.0);
    }

    #[test]
    fn a_doji_intrabar_reads_the_close_before_it_in_the_same_bar() {
        let bars = [
            ib(0, 10.0, 11.0, 10.0, 11.0, 1.0),
            ib(60, 12.0, 12.5, 11.5, 12.0, 2.0),
        ];
        let fp = build_footprint(0, &bars, None, &opts(1.0));
        assert_eq!(fp.buy, 3.0);
    }

    #[test]
    fn no_intrabars_is_an_empty_footprint() {
        let fp = build_footprint(5, &[], None, &opts(1.0));
        assert!(fp.rows.is_empty());
        assert_eq!(fp.poc, None);
        assert_eq!(fp.buy, 0.0);
    }

    // ---- row size ----

    #[test]
    fn auto_row_size_is_a_fifth_of_the_atr_on_nice_numbers() {
        assert_eq!(auto_row_size(50.0, 0.01), 10.0);
        assert_eq!(auto_row_size(13.0, 0.01), 2.5);
        assert_eq!(auto_row_size(3.0, 0.01), 0.5);
        // Never under a tick, and always on them.
        assert_eq!(auto_row_size(0.001, 0.01), 0.01);
        assert_eq!(nice_row_size(7.0, 1.0), 5.0);
        assert_eq!(nice_row_size(f64::NAN, 0.5), 0.5);
    }

    #[test]
    fn atr_is_wilders() {
        let bar = |h: f64, l: f64, c: f64| Bar {
            open: c,
            high: h,
            low: l,
            close: c,
            volume: 1.0,
            date: "0".into(),
        };
        let bars = [
            bar(11.0, 9.0, 10.0),
            bar(12.0, 10.0, 11.0),
            bar(14.0, 10.0, 12.0),
        ];
        // Seed over 1 bar is its range, 2; then (2 * 0 + 4) / 1.
        assert_eq!(atr(&bars, 1), Some(4.0));
        // Ranges 2 and 4: seed over 2 is 3.
        assert_eq!(atr(&bars, 2), Some(3.0));
        assert_eq!(atr(&bars, 3), None);
    }

    #[test]
    fn tick_is_read_from_the_decimals() {
        assert_eq!(infer_tick(&[100.0, 101.0]), 1.0);
        assert_eq!(infer_tick(&[100.25, 101.5]), 0.01);
        assert_eq!(infer_tick(&[0.00012, 0.00013]), 0.00001);
        // No tick to read: by the price.
        assert_eq!(infer_tick(&[123.123456789012]), 0.01);
    }

    #[test]
    fn a_row_size_that_would_give_too_many_rows_is_widened() {
        assert_eq!(bounded_row_size(1.0, 100.0, 0.01), 1.0);
        let wide = bounded_row_size(0.01, 100.0, 0.01);
        assert!(100.0 / wide <= MAX_ROWS_PER_BAR as f64, "{wide}");
    }

    // ---- value area, POC ----

    #[test]
    fn a_row_is_floored_at_a_tick_and_capped_at_the_price() {
        assert_eq!(usable_row(1e-9, 0.0, 65_000.0, 0.01), 0.01);
        assert_eq!(usable_row(5e-324, 0.0, 65_000.0, 0.01), 0.01);
        assert_eq!(usable_row(1e308, 500.0, 65_000.0, 0.01), 65_000.0);
        assert_eq!(usable_row(2.5, 100.0, 65_000.0, 0.01), 2.5);
    }

    /// The chart's bar read flat, so nothing bounded the row by it; its intrabars, read
    /// a moment later, moved. Far more rows than this used to be asked for at once.
    #[test]
    fn a_tiny_row_cannot_size_a_bar_beyond_the_limit() {
        let list = [ib(0, 65_000.0, 65_005.0, 65_000.0, 65_005.0, 10.0)];
        let fp = build_footprint(0, &list, None, &opts(1e-5));
        assert!(fp.rows.len() <= MAX_ROWS_PER_BAR + 1, "{}", fp.rows.len());
        assert!(fp.row >= 5.0 / MAX_ROWS_PER_BAR as f64, "{}", fp.row);
        let total: f64 = fp.rows.iter().map(|r| r.buy + r.sell).sum();
        assert!((total - 10.0).abs() < 1e-9, "{total}");
    }

    /// The review's case: the chart's bar read flat, and each intrabar is narrow, but
    /// the two together are 5 wide. Nothing may leave a row of 1e-9 standing for them.
    #[test]
    fn a_flat_bar_with_wider_intrabars_gets_rows_that_fit_them() {
        let flat = Bar {
            open: 65_000.0,
            high: 65_000.0,
            low: 65_000.0,
            close: 65_000.0,
            volume: 1.0,
            date: "0".into(),
        };
        let list = [
            ib(0, 65_000.0, 65_000.0, 65_000.0, 65_000.0, 1.0),
            ib(1, 65_005.0, 65_005.0, 65_005.0, 65_005.0, 1.0),
        ];
        let groups = group_by_bar(&[0], 60, &list);
        let row = footprint_row(1e-9, &[flat], &list, &groups, 0.01);
        assert!(5.0 / row <= MAX_ROWS_PER_BAR as f64, "{row}");
        let fp = build_footprint(0, &list, None, &opts(row));
        assert!(fp.rows.len() <= MAX_ROWS_PER_BAR + 1, "{}", fp.rows.len());
        assert_eq!(fp.row, row);
    }

    #[test]
    fn a_price_that_is_no_number_sizes_nothing() {
        let list = [
            ib(0, 1.0, f64::INFINITY, f64::NAN, 1.0, 1.0),
            ib(1, 100.0, 101.0, 100.0, 100.5, 2.0),
        ];
        let fp = build_footprint(0, &list, None, &opts(0.5));
        assert_eq!(fp.rows.len(), 2);
        assert_eq!(fp.buy + fp.sell, 2.0);
    }

    #[test]
    fn value_area_grows_from_the_poc_by_the_busier_neighbour() {
        // 100 in total, POC 40 at index 2: 20 above (a tie goes up), then 20 below, the
        // busier of it and 15, and that is 80, past 70.
        let (low, high, poc) = value_area(&[5.0, 20.0, 40.0, 20.0, 15.0], 70.0).unwrap();
        assert_eq!((low, high, poc), (1, 3, 2));
    }

    /// TradingView's TPO page: the busier row; on a tie the one closest to the POC; then
    /// the higher one. By hand: `[1,5,10,5,1]` of 22 to 13.2 is 10, then the 5 above.
    #[test]
    fn a_value_area_tie_goes_to_the_closer_row_and_then_up() {
        assert_eq!(value_area(&[1., 5., 10., 5., 1.], 60.0), Some((2, 3, 2)));
        // 22 to 11: 6, the 3 above on the tie; the 3 below is closer than the next above.
        assert_eq!(
            value_area(&[2., 3., 3., 6., 3., 3., 2.], 50.0),
            Some((2, 4, 3))
        );
    }

    #[test]
    fn a_value_area_takes_the_busier_row_before_any_tie_rule() {
        assert!(value_area_grows_down(Some(5.0), Some(1.0), 3, 0));
        assert!(!value_area_grows_down(Some(1.0), Some(5.0), 0, 3));
        // A tie: closer first, up when they are as close.
        assert!(value_area_grows_down(Some(2.0), Some(2.0), 0, 1));
        assert!(!value_area_grows_down(Some(2.0), Some(2.0), 1, 0));
        assert!(!value_area_grows_down(Some(2.0), Some(2.0), 1, 1));
        // Only one side left.
        assert!(value_area_grows_down(Some(0.0), None, 5, 0));
        assert!(!value_area_grows_down(None, Some(0.0), 0, 5));
    }

    #[test]
    fn value_area_takes_the_only_side_left_at_an_edge() {
        let (low, high, poc) = value_area(&[50.0, 30.0, 20.0], 90.0).unwrap();
        assert_eq!((low, high, poc), (0, 2, 0));
    }

    #[test]
    fn value_area_stops_as_soon_as_it_holds_the_percentage() {
        assert_eq!(value_area(&[1.0, 98.0, 1.0], 70.0), Some((1, 1, 1)));
        assert_eq!(value_area(&[0.0, 0.0], 70.0), None);
        assert_eq!(value_area(&[], 70.0), None);
    }

    #[test]
    fn the_poc_tie_goes_to_the_row_nearest_the_middle() {
        let (_, _, poc) = value_area(&[3.0, 1.0, 3.0, 1.0, 1.0], 10.0).unwrap();
        assert_eq!(poc, 2);
    }

    #[test]
    fn footprint_reports_poc_and_value_area_as_row_prices() {
        // One intrabar per row, on rows 10, 11 and 12.
        let bars = [
            ib(0, 10.0, 10.9, 10.0, 10.9, 10.0),
            ib(60, 11.0, 11.9, 11.0, 11.9, 80.0),
            ib(120, 12.0, 12.9, 12.0, 12.9, 10.0),
        ];
        let fp = build_footprint(0, &bars, None, &opts(1.0));
        assert_eq!(fp.poc, Some(11.0));
        assert_eq!((fp.val, fp.vah), (Some(11.0), Some(12.0)));
    }

    // ---- imbalance ----

    #[test]
    fn buy_imbalance_compares_with_the_sell_one_level_below() {
        // Row 1 buys 30 against 10 sold on row 0: exactly 300%.
        let (buy, sell) = imbalances(&rows(&[(0.0, 10.0), (30.0, 0.0)]), 300.0);
        assert_eq!(buy, vec![false, true]);
        assert_eq!(sell, vec![false, false]);
        // 29 is short of it.
        let (buy, _) = imbalances(&rows(&[(0.0, 10.0), (29.0, 0.0)]), 300.0);
        assert_eq!(buy, vec![false, false]);
    }

    #[test]
    fn sell_imbalance_compares_with_the_buy_one_level_above() {
        let (buy, sell) = imbalances(&rows(&[(0.0, 30.0), (10.0, 0.0)]), 300.0);
        assert_eq!(sell, vec![true, false]);
        assert_eq!(buy, vec![false, false]);
    }

    #[test]
    fn imbalance_compares_diagonally_not_on_the_same_level() {
        // 100 bought against 1 sold on the same row is nothing by itself.
        let (buy, sell) = imbalances(&rows(&[(100.0, 1.0)]), 300.0);
        assert_eq!((buy, sell), (vec![false], vec![false]));
    }

    #[test]
    fn nothing_to_compare_with_is_no_imbalance() {
        let (buy, sell) = imbalances(&rows(&[(0.0, 0.0), (50.0, 50.0), (0.0, 0.0)]), 300.0);
        assert_eq!(buy, vec![false, false, false]);
        assert_eq!(sell, vec![false, false, false]);
    }

    #[test]
    fn the_percentage_is_a_setting() {
        let r = rows(&[(0.0, 10.0), (20.0, 0.0)]);
        assert_eq!(imbalances(&r, 300.0).0, vec![false, false]);
        assert_eq!(imbalances(&r, 200.0).0, vec![false, true]);
    }

    #[test]
    fn stacked_needs_that_many_in_a_row() {
        let f = [true, true, false, true, true, true, false, true];
        assert_eq!(stacked_runs(&f, 3), vec![(3, 5)]);
        assert_eq!(stacked_runs(&f, 2), vec![(0, 1), (3, 5)]);
        // A run to the very end counts.
        assert_eq!(stacked_runs(&[false, true, true, true], 3), vec![(1, 3)]);
        assert_eq!(stacked_runs(&[], 3), Vec::<(usize, usize)>::new());
    }

    #[test]
    fn two_imbalances_apart_are_not_a_stack() {
        // Rows 11 and 13 buy 3x what the row below sold; row 12 between them sells.
        let bars = [
            ib(0, 10.5, 10.9, 10.0, 10.1, 10.0),
            ib(60, 11.1, 11.9, 11.0, 11.9, 30.0),
            ib(120, 12.5, 12.9, 12.0, 12.1, 10.0),
            ib(180, 13.1, 13.9, 13.0, 13.9, 30.0),
        ];
        let mut o = opts(1.0);
        o.stacked = 2;
        let fp = build_footprint(0, &bars, None, &o);
        assert_eq!(fp.buy_imbalances, vec![11.0, 13.0]);
        assert_eq!(fp.sell_imbalances, Vec::<f64>::new());
        assert!(fp.stacked.is_empty());
    }

    /// Taker buy of `share` on one kline spread over `rows` rows from 100.
    fn spread(t: i64, rows: f64, share: f64) -> Intrabar {
        let mut bar = ib(t, 100.0, 100.0 + rows, 100.0, 100.0 + rows, 100.0);
        bar.taker_buy = Some(100.0 * share);
        bar
    }

    #[test]
    fn one_kline_marks_imbalances_but_makes_no_stacked_zone() {
        // Its split is on every row, so it passes the ratio on all of them or none.
        let fp = build_footprint(0, &[spread(0, 10.0, 0.80)], None, &opts(1.0));
        assert_eq!(fp.buy_imbalances.len(), 9);
        assert!(fp.stacked.is_empty(), "{:?}", fp.stacked);
    }

    #[test]
    fn a_stack_counts_levels_where_two_klines_traded() {
        let two = [spread(0, 10.0, 0.80), spread(60, 10.0, 0.80)];
        let fp = build_footprint(0, &two, None, &opts(1.0));
        assert_eq!(fp.stacked.len(), 1);
        assert_eq!((fp.stacked[0].low, fp.stacked[0].high), (101.0, 110.0));
        // Only rows 100 to 104 have the second kline: the wick above it is no zone. Row
        // 105 is compared with 104, which has it, but it has only one of its own.
        let wick = [spread(0, 10.0, 0.80), spread(60, 5.0, 0.80)];
        let fp = build_footprint(0, &wick, None, &opts(1.0));
        assert_eq!(fp.stacked.len(), 1);
        assert_eq!((fp.stacked[0].low, fp.stacked[0].high), (101.0, 105.0));
    }

    #[test]
    fn a_kline_with_no_volume_is_no_second_kline() {
        let quiet = [
            spread(0, 10.0, 0.80),
            ib(60, 100.0, 110.0, 100.0, 110.0, 0.0),
        ];
        assert!(build_footprint(0, &quiet, None, &opts(1.0))
            .stacked
            .is_empty());
    }

    #[test]
    fn stacked_zone_spans_from_its_first_row_to_the_top_of_its_last() {
        let mut bars = Vec::new();
        // Sells on row 10; buys of 3x on rows 11, with sells to carry on the stack. Two
        // klines on each: one kline's rows are no zone.
        for (k, (row, buy, sell)) in [
            (10, 0.0, 10.0),
            (10, 0.0, 10.0),
            (11, 30.0, 10.0),
            (11, 30.0, 10.0),
            (12, 30.0, 10.0),
            (12, 30.0, 10.0),
            (13, 30.0, 0.0),
            (13, 30.0, 0.0),
        ]
        .into_iter()
        .enumerate()
        {
            let p = f64::from(row);
            let mut bar = ib(k as i64, p, p + 0.5, p, p + 0.5, buy + sell);
            bar.taker_buy = Some(buy);
            bars.push(bar);
        }
        let fp = build_footprint(0, &bars, None, &opts(1.0));
        assert_eq!(fp.buy_imbalances, vec![11.0, 12.0, 13.0]);
        assert_eq!(
            fp.stacked,
            vec![Stacked {
                buy: true,
                low: 11.0,
                high: 14.0,
                count: 3
            }]
        );
    }

    // ---- grouping ----

    #[test]
    fn intrabars_go_to_the_bar_they_open_in_even_across_a_gap() {
        let bars: Vec<Intrabar> = [0, 60, 3600, 3660, 90_000]
            .iter()
            .map(|&t| ib(t, 1.0, 1.0, 1.0, 1.0, 1.0))
            .collect();
        let got = group_by_bar(&[0, 3600, 90_000], 93_600, &bars);
        assert_eq!(got, vec![0..2, 2..4, 4..5]);
        // A bar nothing fell in has an empty range.
        assert_eq!(group_by_bar(&[7200], 10_800, &bars), vec![4..4]);
    }

    // ---- TPO ----

    #[test]
    fn letters_run_upper_then_lower_then_around() {
        assert_eq!(tpo_letter(0), 'A');
        assert_eq!(tpo_letter(25), 'Z');
        assert_eq!(tpo_letter(26), 'a');
        assert_eq!(tpo_letter(51), 'z');
        assert_eq!(tpo_letter(52), 'A');
    }

    /// Blocks covering rows `low..=high` (row size 1), 30 minutes apart.
    fn blocks(ranges: &[(i32, i32)]) -> Vec<Intrabar> {
        ranges
            .iter()
            .enumerate()
            .map(|(k, &(low, high))| {
                let (l, h) = (f64::from(low) + 0.1, f64::from(high) + 0.9);
                ib(k as i64 * 1800, l, h, l, h, 10.0)
            })
            .collect()
    }

    fn profile(ranges: &[(i32, i32)]) -> TpoProfile {
        build_tpo(0, 86_400, &blocks(ranges), 1.0, 70.0, 2).unwrap()
    }

    #[test]
    fn each_block_prints_on_every_row_it_touched() {
        let p = profile(&[(10, 12), (11, 13)]);
        let got: Vec<(f64, Vec<u32>)> =
            p.rows.iter().map(|r| (r.price, r.blocks.clone())).collect();
        assert_eq!(
            got,
            vec![
                (10.0, vec![0]),
                (11.0, vec![0, 1]),
                (12.0, vec![0, 1]),
                (13.0, vec![1]),
            ]
        );
        assert_eq!(p.total_blocks, 6);
    }

    /// By hand, a block printing at each price it traded, its high included.
    #[test]
    fn a_block_prints_at_its_exact_high() {
        let list = [
            ib(0, 100.0, 102.0, 100.0, 101.0, 1.0),
            ib(1800, 101.0, 103.0, 101.0, 102.0, 1.0),
            ib(3600, 102.0, 104.0, 102.0, 104.0, 1.0),
        ];
        let p = build_tpo(0, 5400, &list, 1.0, 70.0, 2).unwrap();
        let got: Vec<(f64, String)> = p
            .rows
            .iter()
            .map(|r| {
                (
                    r.price,
                    r.blocks.iter().map(|b| tpo_letter(*b as usize)).collect(),
                )
            })
            .collect();
        let want = [
            (100.0, "A"),
            (101.0, "AB"),
            (102.0, "ABC"),
            (103.0, "BC"),
            (104.0, "C"),
        ];
        assert_eq!(got, want.map(|(p, l)| (p, l.to_string())));
        assert_eq!(p.total_blocks, 9);
        assert_eq!(p.poc, 102.0);
        assert!(!p.poor_high);
        // No volume on a row only touched: it all stays where the range is.
        let volume: f64 = p.rows.iter().map(|r| r.volume).sum();
        assert!((volume - 3.0).abs() < 1e-9, "{volume}");
        assert_eq!(p.rows.last().unwrap().volume, 0.0);
    }

    #[test]
    fn two_blocks_topping_on_a_row_edge_are_a_poor_high_and_one_is_not() {
        let double = [
            ib(0, 100.0, 104.0, 100.0, 104.0, 1.0),
            ib(1800, 104.0, 104.0, 101.0, 102.0, 1.0),
        ];
        let p = build_tpo(0, 3600, &double, 1.0, 70.0, 1).unwrap();
        assert_eq!(p.rows.last().unwrap().price, 104.0);
        assert!(p.poor_high);
        let single = [
            ib(0, 100.0, 104.0, 100.0, 102.0, 1.0),
            ib(1800, 102.0, 103.0, 101.0, 102.0, 1.0),
        ];
        let p = build_tpo(0, 3600, &single, 1.0, 70.0, 1).unwrap();
        assert_eq!(p.rows.last().unwrap().price, 104.0);
        assert!(!p.poor_high);
    }

    #[test]
    fn a_footprint_only_counts_volume_on_rows_with_a_share() {
        // The high is on row 104's bottom: no volume there, unlike a TPO's print.
        let fp = build_footprint(
            0,
            &[ib(0, 100.0, 104.0, 100.0, 104.0, 4.0)],
            None,
            &opts(1.0),
        );
        assert_eq!(fp.rows.last().unwrap().price, 103.0);
    }

    #[test]
    fn the_json_spells_the_letters_of_each_row() {
        let json = profile(&[(10, 11), (11, 11)]).to_json();
        assert_eq!(json["rows"][0]["letters"], "A");
        assert_eq!(json["rows"][1]["letters"], "AB");
    }

    #[test]
    fn poc_and_value_area_count_blocks_not_volume() {
        let mut b = blocks(&[(10, 10), (11, 11), (11, 11), (12, 12)]);
        // A huge volume on row 10 must not move a time profile's POC.
        b[0].volume = 1000.0;
        let p = build_tpo(0, 1, &b, 1.0, 50.0, 1).unwrap();
        assert_eq!(p.poc, 11.0);
        assert_eq!((p.val, p.vah), (11.0, 12.0));
        assert_eq!((p.blocks_above_poc, p.blocks_below_poc), (1, 1));
    }

    #[test]
    fn a_poor_extreme_has_more_than_one_block() {
        let p = profile(&[(10, 12), (10, 11), (11, 12)]);
        assert!(p.poor_low && p.poor_high);
        let p = profile(&[(10, 12), (11, 12), (11, 13)]);
        // One block on row 10 and one on row 13: both ends finished.
        assert!(!p.poor_low && !p.poor_high);
    }

    #[test]
    fn single_prints_are_inside_the_profile_not_its_tails() {
        // Row 10 and 14 are tails; rows 12 and 13 are one block, between two.
        let p = profile(&[(10, 11), (11, 14), (14, 15), (15, 15)]);
        assert_eq!(p.single_prints, vec![(12.0, 14.0)]);
        // Nothing but tails.
        let p = profile(&[(10, 12), (11, 11)]);
        assert!(p.single_prints.is_empty());
    }

    #[test]
    fn a_row_no_block_reached_is_kept_and_is_no_single_print() {
        let p = profile(&[(10, 10), (10, 10), (12, 12), (12, 12)]);
        assert_eq!(p.rows.len(), 3);
        assert!(p.rows[1].blocks.is_empty());
        assert!(p.single_prints.is_empty());
    }

    #[test]
    fn initial_balance_is_the_first_blocks() {
        let p = build_tpo(0, 1, &blocks(&[(10, 12), (11, 14), (5, 20)]), 1.0, 70.0, 2).unwrap();
        assert_eq!((p.ib_low, p.ib_high), (10.1, 14.9));
        assert_eq!((p.low, p.high), (5.1, 20.9));
        // Asking for more than there are is all of them.
        let p = build_tpo(0, 1, &blocks(&[(10, 12)]), 1.0, 70.0, 5).unwrap();
        assert_eq!((p.ib_low, p.ib_high), (10.1, 12.9));
    }

    #[test]
    fn open_close_and_volume_come_from_the_blocks() {
        let p = profile(&[(10, 12), (11, 13)]);
        assert_eq!(p.open, 10.1);
        assert_eq!(p.close, 13.9);
        assert_eq!(p.volume, 20.0);
    }

    #[test]
    fn rotation_factor_counts_higher_and_lower_highs_and_lows() {
        // Higher high and higher low: +2. Lower high, equal low: -1.
        let b = [
            ib(0, 1.0, 10.0, 5.0, 1.0, 1.0),
            ib(1, 1.0, 11.0, 6.0, 1.0, 1.0),
            ib(2, 1.0, 10.5, 6.0, 1.0, 1.0),
        ];
        assert_eq!(rotation_factor(&b), 1);
        assert_eq!(rotation_factor(&b[..1]), 0);
    }

    #[test]
    fn no_blocks_is_no_profile() {
        assert!(build_tpo(0, 1, &[], 1.0, 70.0, 2).is_none());
    }

    #[test]
    fn period_parses_and_refuses() {
        assert_eq!(Period::parse("1D"), Ok(Period::Days(1)));
        assert_eq!(Period::parse("2w"), Ok(Period::Weeks(2)));
        assert_eq!(Period::parse("1M"), Ok(Period::Months(1)));
        for bad in ["", "D", "0D", "31D", "1h", "1m"] {
            assert!(Period::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn periods_start_on_the_calendar_of_the_time_zone() {
        use chrono::{TimeZone, Utc};
        let at = |y, m, d, h| Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap().timestamp();
        // 6 Nov 2026 is a Friday.
        let t = at(2026, 11, 6, 15);
        assert_eq!(Period::Days(1).start_of(t, &Utc), at(2026, 11, 6, 0));
        assert_eq!(Period::Weeks(1).start_of(t, &Utc), at(2026, 11, 2, 0));
        assert_eq!(Period::Months(1).start_of(t, &Utc), at(2026, 11, 1, 0));
        // New York's day starts at 05:00 UTC in November.
        let ny = chrono_tz::America::New_York;
        assert_eq!(
            Period::Days(1).start_of(at(2026, 11, 6, 3), &ny),
            at(2026, 11, 5, 5)
        );
    }

    #[test]
    fn sessions_parse_and_filter_by_the_local_time() {
        use chrono::{TimeZone, Utc};
        let s = DaySession::parse("09:30-16:00").unwrap();
        assert_eq!((s.from, s.to), (570, 960));
        for bad in [
            "",
            "9-10",
            "16:00-09:30",
            "10:00-10:00",
            "25:00-26:00",
            "10:61-11:00",
        ] {
            assert!(DaySession::parse(bad).is_err(), "{bad}");
        }
        let at = |h, m| {
            Utc.with_ymd_and_hms(2026, 11, 6, h, m, 0)
                .unwrap()
                .timestamp()
        };
        assert!(s.contains(at(9, 30), &Utc));
        assert!(!s.contains(at(16, 0), &Utc));
        assert!(s.contains(at(15, 0), &chrono_tz::America::New_York));
        assert!(!s.contains(at(9, 30), &chrono_tz::America::New_York));
    }

    #[test]
    fn profiles_split_by_period_and_by_session() {
        use chrono::Utc;
        let block = |t: i64| ib(t, 10.0, 10.5, 10.0, 10.5, 1.0);
        let b: Vec<Intrabar> = [0, 3600, 86_400, 90_000]
            .iter()
            .map(|&t| block(t))
            .collect();
        let o = TpoOptions {
            block_secs: 3600,
            row: 1.0,
            value_area: 70.0,
            ib_blocks: 1,
        };
        let by_day = tpo_profiles(&b, Period::Days(1), &[], &Utc, &o);
        assert_eq!(by_day.len(), 2);
        assert_eq!((by_day[0].t, by_day[0].end), (0, 7200));
        assert_eq!(by_day[1].total_blocks, 2);

        let sessions = [
            DaySession::parse("00:00-01:00").unwrap(),
            DaySession::parse("01:00-02:00").unwrap(),
        ];
        let split = tpo_profiles(&b, Period::Days(1), &sessions, &Utc, &o);
        assert_eq!(split.len(), 4);
        // Outside every session: left out.
        let late = [block(7200)];
        assert!(tpo_profiles(&late, Period::Days(1), &sessions, &Utc, &o).is_empty());
    }

    #[test]
    fn auto_row_of_a_tpo_is_a_fraction_of_the_typical_range() {
        use chrono::Utc;
        let day = |d: i64, range: f64| ib(d * 86_400, 100.0, 100.0 + range, 100.0, 100.0, 1.0);
        let b = [day(0, 24.0), day(1, 48.0), day(2, 480.0)];
        // Median 48, a 24th is 2. The wide day does not move it.
        assert_eq!(tpo_auto_row(&b, Period::Days(1), &[], &Utc, 0.01), 2.0);
    }

    // ---- sample ----

    #[test]
    fn sample_intrabars_add_up_to_their_bar() {
        let bar = Bar {
            open: 100.0,
            high: 105.0,
            low: 98.0,
            close: 103.0,
            volume: 500.0,
            date: "3600".into(),
        };
        let got = sample_intrabars(7, 3600, &bar, 60, 60, 60, true);
        assert_eq!(got.len(), 60);
        assert_eq!(got[0].open, 100.0);
        assert_eq!(got[59].close, 103.0);
        assert!((got.iter().map(|b| b.volume).sum::<f64>() - 500.0).abs() < 1e-6);
        assert!(got
            .iter()
            .all(|b| b.high <= 105.0 + 1e-9 && b.low >= 98.0 - 1e-9));
        assert_eq!(got[1].t, 3660);
        assert!(got.iter().all(|b| b.taker_buy.is_some()));
        // Both extremes are reached.
        assert!(got.iter().any(|b| b.high > 104.5) && got.iter().any(|b| b.low < 98.5));
        // The same inputs give the same bars.
        assert_eq!(got, sample_intrabars(7, 3600, &bar, 60, 60, 60, true));
        assert!(sample_intrabars(7, 3600, &bar, 60, 60, 60, false)[0]
            .taker_buy
            .is_none());
    }
}
