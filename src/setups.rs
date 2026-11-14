//! A setup: a thesis with levels that the server scores on paper (ADR-0059).
//!
//! This is the part with no I/O: the record, its checks, how one closed 1m bar moves
//! it, and its result in R. The live feed and the replay both call `apply_bar`, so
//! they cannot disagree.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::data::Bar;
use crate::drawings;

mod market;
pub use market::{Market, STOCK_FEE_BPS_PER_SIDE, STOCK_SLIPPAGE_BPS_PER_SIDE};

/// The cost assumption copied onto each setup, in basis points per side.
pub const FEE_BPS_PER_SIDE: f64 = 5.0;
pub const SLIPPAGE_BPS_PER_SIDE: f64 = 2.0;
/// The least risk a setup may have, in round-trip costs: a stop a hair from the entry
/// would turn any move into many R.
pub const MIN_RISK_IN_ROUND_TRIP_COSTS: f64 = 3.0;

/// The bracket a skip is scored with, on the closed bars of its interval.
pub const SKIP_ATR_LENGTH: usize = 14;
pub const SKIP_STOP_ATR: f64 = 1.0;
pub const SKIP_TARGET_ATR: f64 = 2.0;
pub const SKIP_EXPIRY_BARS: i64 = 24;

pub const MAX_THESIS: usize = 2000;
pub const MAX_REASON: usize = 500;
pub const MAX_TARGETS: usize = 4;
pub const MAX_EXPIRY_MS: i64 = 90 * 86_400_000;
pub const MAX_TAGS: usize = 10;
pub const MAX_TAG: usize = 64;
/// The characters a tag may have besides lowercase letters and digits.
pub const TAG_PUNCTUATION: &str = ":-_./";
const TAG_HINT: &str = "lowercase letters, digits and : - _ . /, starting with a letter or digit, such as strategy:retest-short";
/// What an agent may state when it makes a setup (ADR-0064): how many features, and how
/// long a feature's name, a feature's text and a playbook version may be.
pub const MAX_FEATURES: usize = 16;
pub const MAX_STATED_TEXT: usize = 32;
/// Fields taken when a setup is made and by no tool afterwards.
pub const STATED_FIELDS: [&str; 3] = ["features", "probability", "playbook_version"];

const MINUTE_MS: i64 = 60_000;

/// The shadow exit variants (ADR-0066). v1 closes this share at the first target.
pub const SHADOW_PARTIAL_AT_T1: f64 = 0.5;
/// v2 moves its stop to break-even once price has gone this far, in R from the fill.
pub const SHADOW_BE_AT_R: f64 = 1.0;
/// v3 judges the close of this many bars of the setup's interval after the fill bar:
/// the first for intervals shorter than 4h, the second from 4h on.
pub const SHADOW_TIME_STOP_BARS: (u32, u32) = (12, 6);
const SHADOW_TIME_STOP_LONG_FROM_SECS: i64 = 4 * 3600;
/// v3 exits at that close when it is below this R from the fill.
pub const SHADOW_TIME_STOP_KEEP_R: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Long,
    Short,
}

impl Direction {
    /// +1 for a long, -1 for a short.
    fn sign(self) -> f64 {
        match self {
            Direction::Long => 1.0,
            Direction::Short => -1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Take,
    Skip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Watching,
    Triggered,
    Won,
    Invalidated,
    Expired,
    Cancelled,
}

impl Status {
    pub fn is_open(self) -> bool {
        matches!(self, Status::Watching | Status::Triggered)
    }
}

/// Which rule set a setup's levels, so that a track record can tell them apart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bracket {
    /// The agent's own levels.
    #[default]
    Agent,
    /// A skip's bracket from the ATR of its interval.
    Atr,
    /// A skip's bracket at the least risk a setup may have, where the ATR was less.
    Floor,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Zone {
    pub low: f64,
    pub high: f64,
}

/// What the server scores a setup against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Levels {
    pub entry: Zone,
    pub invalidation: f64,
    /// Nearest first. The first decides a win.
    pub targets: Vec<f64>,
    /// ms since epoch.
    pub expires_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Costs {
    pub fee_bps_per_side: f64,
    pub slippage_bps_per_side: f64,
}

impl Default for Costs {
    fn default() -> Self {
        Costs {
            fee_bps_per_side: FEE_BPS_PER_SIDE,
            slippage_bps_per_side: SLIPPAGE_BPS_PER_SIDE,
        }
    }
}

impl Costs {
    /// Both sides, as a fraction of the price.
    fn round_trip(&self) -> f64 {
        2.0 * (self.fee_bps_per_side + self.slippage_bps_per_side) / 10_000.0
    }

    /// The least distance from an entry at `price` to its invalidation.
    fn least_risk(&self, price: f64) -> f64 {
        MIN_RISK_IN_ROUND_TRIP_COSTS * self.round_trip() * price
    }
}

/// A price and the 1m bar it happened in (its open time, ms).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    pub price: f64,
    pub bar_time: i64,
}

/// What the agent asked for that no bar has reached yet. A change counts from the
/// first bar that opens at or after `at`: the bar forming when it was sent still
/// meets the setup as it was, so a fill or a stop inside it cannot be undone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Pending {
    Levels { at: i64, levels: Levels },
    Cancel { at: i64 },
}

impl Pending {
    fn at(&self) -> i64 {
        match self {
            Pending::Levels { at, .. } | Pending::Cancel { at } => *at,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub owner: String,
    pub symbol: String,
    pub interval: String,
    pub direction: Direction,
    pub decision: Decision,
    pub thesis: String,
    /// Resolved drawings, as `generate_chart` reports them (ADR-0054).
    #[serde(default)]
    pub drawings: Vec<Value>,
    pub levels: Levels,
    #[serde(default)]
    pub bracket: Bracket,
    pub status: Status,
    pub created_at: i64,
    pub costs: Costs,
    #[serde(default)]
    pub fill: Option<Mark>,
    #[serde(default)]
    pub exit: Option<Mark>,
    #[serde(default)]
    pub result_r: Option<f64>,
    /// Open time of the newest 1m bar applied.
    #[serde(default)]
    pub scored_through: Option<i64>,
    /// Whether the server missed bars it could not read afterwards: the result does not
    /// cover them, and the log's `gap` entries say which.
    #[serde(default)]
    pub gap: bool,
    #[serde(default)]
    pub pending: Vec<Pending>,
    /// Revisions written so far, and those of them the agent made.
    #[serde(default)]
    pub revisions: u32,
    #[serde(default)]
    pub agent_revisions: u32,
    /// The agent's labels, such as `strategy:retest-short`, to group setups by.
    #[serde(default)]
    pub tags: Vec<String>,
    /// From the fill on: the furthest price went against the position (0 or less) and
    /// with it (0 or more), in R of the risk at the fill, before costs, read from 1m
    /// bar highs and lows as the scorer reads them. None before a fill, and on a setup
    /// filled before the server kept them.
    #[serde(default)]
    pub mae_r: Option<f64>,
    #[serde(default)]
    pub mfe_r: Option<f64>,
    /// Open time of the 1m bar that set `mae_r`, and of the one that set `mfe_r`: the
    /// first, where two bars went as far.
    #[serde(default)]
    pub mae_at: Option<i64>,
    #[serde(default)]
    pub mfe_at: Option<i64>,
    /// While watching: the least distance from a bar's high or low to the entry zone, in
    /// R of the risk from the zone's edge nearest the price to the invalidation. 0 once
    /// filled. None when the setup's first bars were not scored here, as on a setup
    /// stored before the server kept it.
    #[serde(default)]
    pub closest_approach_r: Option<f64>,
    /// The fixed exit variants scored on the same 1m bars from the fill on (ADR-0066).
    /// None before the fill, and on a setup filled before the server kept them.
    #[serde(default)]
    pub shadow_results: Option<Shadows>,
    /// What the agent stated when it made the setup (ADR-0064). The server gives them
    /// no meaning and never changes them; the first entry of the log holds them too.
    /// Not written when not stated, so a setup without them is stored as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probability: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playbook_version: Option<String>,
    /// A stock setup's market (ADR-0070); not written for a Binance setup, which trades
    /// around the clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<Market>,
    /// A stock setup's newest 1m close scored: where it ends when its expiry falls in a
    /// stretch with no bar, such as a weekend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_close: Option<Mark>,
    /// The splits and dividends applied to a stock setup, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adjustments: Vec<Adjustment>,
}

/// How a shadow variant ended, or that it has not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowStatus {
    Open,
    /// Out at its last target: T2 for v1, T1 for v2 and v3.
    Won,
    /// Out at or through the setup's invalidation, a moved stop or not.
    Stopped,
    /// Out at a moved stop, short of the invalidation: near 0 R, a little under after a
    /// gap or where v1's stop is T1.
    Breakeven,
    /// Out at the close the time stop judged.
    TimedOut,
    Expired,
    Cancelled,
}

/// A part of a shadow position closed: its price, its share of the position and the
/// 1m bar it happened in (its open time, ms).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShadowExit {
    pub price: f64,
    pub fraction: f64,
    pub bar_time: i64,
}

/// One shadow variant's position, from the canonical fill on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    pub status: ShadowStatus,
    /// The parts' R, each net of its own costs and weighted by its share. Null while
    /// open, and when it expired while nobody scored it.
    pub result_r: Option<f64>,
    /// The stop in force from the next bar on.
    pub stop: f64,
    pub exits: Vec<ShadowExit>,
    /// Open time of the 1m bar of the last exit.
    pub exited_at: Option<i64>,
    /// v3: the close time (ms) of the bar its time stop judges. Null once judged, and
    /// for the other variants.
    pub time_stop_at: Option<i64>,
}

/// The shadow variants of a filled setup, by id (ADR-0066).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shadows {
    /// Half out at T1, the rest to T2 with its stop at break-even after costs.
    pub v1_half_t1_be: Shadow,
    /// All out at T1, with the stop at break-even after costs from +1R on.
    pub v2_be_at_1r: Shadow,
    /// The canonical setup with a time stop.
    pub v3_time_stop: Shadow,
}

#[derive(Clone, Copy)]
enum Variant {
    HalfAtT1,
    BeAt1R,
    TimeStop,
}

impl Shadow {
    fn open(stop: f64, time_stop_at: Option<i64>) -> Shadow {
        Shadow {
            status: ShadowStatus::Open,
            result_r: None,
            stop,
            exits: Vec::new(),
            exited_at: None,
            time_stop_at,
        }
    }

    pub fn is_open(&self) -> bool {
        self.status == ShadowStatus::Open
    }
}

impl Shadows {
    pub fn any_open(&self) -> bool {
        [&self.v1_half_t1_be, &self.v2_be_at_1r, &self.v3_time_stop]
            .iter()
            .any(|s| s.is_open())
    }

    fn each_mut(&mut self) -> [(Variant, &mut Shadow); 3] {
        [
            (Variant::HalfAtT1, &mut self.v1_half_t1_be),
            (Variant::BeAt1R, &mut self.v2_be_at_1r),
            (Variant::TimeStop, &mut self.v3_time_stop),
        ]
    }
}

/// How long a setup took, in bars of its own interval, from times the record keeps:
/// elapsed ms over the interval's ms, to two places.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Durations {
    /// Creation to the fill bar.
    pub bars_to_fill: Option<f64>,
    /// Fill bar to the exit bar; while filled, to the newest bar scored.
    pub bars_to_exit: Option<f64>,
    /// Fill bar to the bars that set `mae_r` and `mfe_r`.
    pub bars_to_mae: Option<f64>,
    pub bars_to_mfe: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum By {
    Agent,
    Server,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Created,
    Revised,
    Cancelled,
    Status,
    /// A change the agent asked for that could no longer take effect.
    Voided,
    /// Bars the server did not see.
    Gap,
    /// A split or a dividend of a stock setup's stock (ADR-0070).
    Adjusted,
}

/// A corporate action applied to a stock setup (ADR-0070). `at` is when it took effect:
/// the session open of the split's day or the dividend's ex-date, ms.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Adjustment {
    /// Every price divided by `ratio` (a 10:1 split is 10).
    Split { at: i64, ratio: f64 },
    /// Per share: paid to a long and by a short whose position was open at `at`.
    Dividend { at: i64, amount: f64 },
}

impl Adjustment {
    pub fn at(&self) -> i64 {
        match self {
            Adjustment::Split { at, .. } | Adjustment::Dividend { at, .. } => *at,
        }
    }

    /// The same action, whatever its amount has become since: a dividend is rescaled by
    /// a later split.
    fn same(&self, other: &Adjustment) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other) && self.at() == other.at()
    }
}

/// One entry of a setup's log. Entries are only ever added.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    pub seq: u32,
    pub at: i64,
    pub by: By,
    pub kind: Kind,
    #[serde(default)]
    pub reason: Option<String>,
    /// `{field: {from, to}}`.
    #[serde(default)]
    pub changes: Map<String, Value>,
}

/// What is wrong with one field of a call.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub field: String,
    pub message: String,
    pub hint: Option<String>,
}

impl Problem {
    pub fn new(field: &str, message: impl Into<String>) -> Problem {
        Problem {
            field: field.to_string(),
            message: message.into(),
            hint: None,
        }
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Problem {
        self.hint = Some(hint.into());
        self
    }

    /// A drawing's problem, under `drawings[i]`.
    pub fn of_drawing(p: &drawings::Problem) -> Problem {
        let field = match p.index {
            Some(i) => format!("drawings[{i}]"),
            None => "drawings".to_string(),
        };
        Problem {
            field,
            message: p.message.clone(),
            hint: p.hint.clone(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "field": self.field, "message": self.message, "hint": self.hint })
    }
}

/// The error text of a call that did not pass as a whole.
pub fn problems_text(problems: &[Problem]) -> String {
    let all: Vec<Value> = problems.iter().map(Problem::to_json).collect();
    serde_json::to_string_pretty(&json!({
        "error": format!(
            "{} problem(s) with this setup; nothing was stored. Fix them and send the call again.",
            problems.len()
        ),
        "setup_errors": all,
    }))
    .unwrap_or_default()
}

/// A closed 1m bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinuteBar {
    /// Open time, ms.
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

impl MinuteBar {
    /// From a feed or fixture bar, whose `date` is its open time in seconds.
    pub fn of(bar: &Bar) -> Option<MinuteBar> {
        let secs: i64 = bar.date.parse().ok()?;
        let b = MinuteBar {
            time: secs.checked_mul(1000)?,
            open: bar.open,
            high: bar.high,
            low: bar.low,
            close: bar.close,
        };
        let sane = [b.open, b.high, b.low, b.close]
            .iter()
            .all(|p| p.is_finite() && *p > 0.0)
            && b.low <= b.open.min(b.close)
            && b.high >= b.open.max(b.close);
        sane.then_some(b)
    }
}

/// The tags in `value`, each given once. None, with a problem, when any is not a tag.
pub fn tags(value: &Value, problems: &mut Vec<Problem>) -> Option<Vec<String>> {
    let Some(list) = value.as_array().filter(|l| l.len() <= MAX_TAGS) else {
        problems.push(
            Problem::new(
                "tags",
                format!("tags must be a list of at most {MAX_TAGS} strings"),
            )
            .hint(TAG_HINT),
        );
        return None;
    };
    let before = problems.len();
    let mut out: Vec<String> = Vec::new();
    for (i, v) in list.iter().enumerate() {
        let field = format!("tags[{i}]");
        let message = match v.as_str() {
            None => format!("{field} must be a string"),
            Some(t) if t.is_empty() || t.len() > MAX_TAG => {
                format!("{field} must have 1 to {MAX_TAG} characters")
            }
            Some(t)
                if !t.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
                    || !t.chars().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || TAG_PUNCTUATION.contains(c)
                    }) =>
            {
                format!("{field} has a character a tag may not have: {t:?}")
            }
            Some(t) => {
                if !out.iter().any(|o| o == t) {
                    out.push(t.to_string());
                }
                continue;
            }
        };
        problems.push(Problem::new(&field, message).hint(TAG_HINT));
    }
    (problems.len() == before).then_some(out)
}

/// A required, bounded string field.
pub fn text(args: &Value, field: &str, max: usize, problems: &mut Vec<Problem>) -> Option<String> {
    match args.get(field) {
        None | Some(Value::Null) => {
            problems.push(Problem::new(field, format!("{field} is required")));
            None
        }
        Some(Value::String(s)) if s.trim().is_empty() => {
            problems.push(Problem::new(field, format!("{field} is empty")));
            None
        }
        Some(Value::String(s)) if s.chars().count() > max => {
            problems.push(Problem::new(
                field,
                format!("{field} is longer than {max} characters"),
            ));
            None
        }
        Some(Value::String(s)) => Some(s.trim().to_string()),
        Some(_) => {
            problems.push(Problem::new(field, format!("{field} must be a string")));
            None
        }
    }
}

/// The agent's reason for a change.
pub fn reason(args: &Value, problems: &mut Vec<Problem>) -> Option<String> {
    text(args, "reason", MAX_REASON, problems)
}

fn price(v: Option<&Value>, field: &str, problems: &mut Vec<Problem>) -> Option<f64> {
    match v.and_then(Value::as_f64) {
        Some(p) if p.is_finite() && p > 0.0 => Some(p),
        _ if matches!(v, None | Some(Value::Null)) => {
            problems.push(Problem::new(field, format!("{field} is required")));
            None
        }
        _ => {
            problems.push(Problem::new(
                field,
                format!("{field} must be a price above 0"),
            ));
            None
        }
    }
}

fn one_of<T: for<'de> Deserialize<'de>>(
    args: &Value,
    field: &str,
    allowed: &str,
    problems: &mut Vec<Problem>,
) -> Option<T> {
    match args.get(field) {
        None | Some(Value::Null) => {
            problems.push(Problem::new(field, format!("{field} is required")).hint(allowed));
            None
        }
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(t) => Some(t),
            Err(_) => {
                problems.push(Problem::new(field, format!("{field} must be {allowed}")));
                None
            }
        },
    }
}

/// Fields the server sets. Named so that an agent sending one learns why it is refused.
const SERVER_FIELDS: [&str; 19] = [
    "status",
    "result_r",
    "shadow_results",
    "mae_r",
    "mfe_r",
    "mae_at",
    "mfe_at",
    "closest_approach_r",
    "bars_to_fill",
    "bars_to_exit",
    "bars_to_mae",
    "bars_to_mfe",
    "fill",
    "exit",
    "costs",
    "id",
    "owner",
    "created_at",
    "scored_through",
];
pub const LEVEL_FIELDS: [&str; 4] = ["entry", "invalidation", "targets", "expires_at"];

/// Refuse fields a tool does not take.
pub fn check_fields(args: &Value, allowed: &[&str], problems: &mut Vec<Problem>) {
    let Some(map) = args.as_object() else {
        problems.push(Problem::new("arguments", "arguments must be an object"));
        return;
    };
    for key in map.keys() {
        if allowed.contains(&key.as_str()) {
            continue;
        }
        if STATED_FIELDS.contains(&key.as_str()) {
            problems.push(
                Problem::new(key, format!("{key} is fixed when a setup is made"))
                    .hint("what a setup was made with is its record; cancel it and make a new one"),
            );
        } else if SERVER_FIELDS.contains(&key.as_str()) {
            problems.push(
                Problem::new(key, format!("{key} is set by the server"))
                    .hint("the server moves a setup from the price feed; leave this out"),
            );
        } else {
            problems.push(Problem::new(
                key,
                format!("{key} is not a field of this tool"),
            ));
        }
    }
}

/// The levels in `args`, each read on its own and then judged together as far as they
/// could be read, so that every problem is found in one pass (`check_parts`). With
/// `base`, a field left out keeps its value (a revision).
pub fn parse_levels(
    args: &Value,
    base: Option<&Levels>,
    direction: Option<Direction>,
    costs: &Costs,
    now_ms: i64,
    problems: &mut Vec<Problem>,
) -> Option<Levels> {
    let before = problems.len();
    let given = |field: &str| !matches!(args.get(field), None | Some(Value::Null));
    let entry = if given("entry") || base.is_none() {
        let e = args.get("entry");
        if e.is_some_and(|e| !e.is_object()) {
            problems.push(
                Problem::new("entry", "entry must be an object")
                    .hint("{\"low\": 100.0, \"high\": 101.0}; the same price twice is a level"),
            );
            None
        } else if e.is_none() {
            problems.push(
                Problem::new("entry", "entry is required")
                    .hint("{\"low\": 100.0, \"high\": 101.0}; the same price twice is a level"),
            );
            None
        } else {
            let low = price(e.and_then(|e| e.get("low")), "entry.low", problems);
            let high = price(e.and_then(|e| e.get("high")), "entry.high", problems);
            low.zip(high).map(|(low, high)| Zone { low, high })
        }
    } else {
        base.map(|b| b.entry)
    };
    let invalidation = if given("invalidation") || base.is_none() {
        if !given("invalidation") {
            problems.push(
                Problem::new("invalidation", "invalidation is required").hint(
                    "the price at which this setup is wrong; a setup that cannot be wrong is not stored",
                ),
            );
            None
        } else {
            price(args.get("invalidation"), "invalidation", problems)
        }
    } else {
        base.map(|b| b.invalidation)
    };
    let targets = if given("targets") || base.is_none() {
        match args.get("targets").and_then(Value::as_array) {
            Some(list) if (1..=MAX_TARGETS).contains(&list.len()) => {
                let prices: Vec<Option<f64>> = list
                    .iter()
                    .enumerate()
                    .map(|(i, v)| price(Some(v), &format!("targets[{i}]"), problems))
                    .collect();
                prices.into_iter().collect::<Option<Vec<f64>>>()
            }
            _ => {
                problems.push(
                    Problem::new(
                        "targets",
                        format!("targets must be a list of 1 to {MAX_TARGETS} prices"),
                    )
                    .hint("nearest first; the first one decides a win"),
                );
                None
            }
        }
    } else {
        base.map(|b| b.targets.clone())
    };
    let expires_at = if given("expires_at") || base.is_none() {
        if !given("expires_at") {
            problems.push(
                Problem::new("expires_at", "expires_at is required")
                    .hint("when this setup runs out: unix seconds, unix ms or an ISO 8601 time"),
            );
            None
        } else {
            match drawings::parse_time(&args["expires_at"], now_ms) {
                Ok(ms) => Some(ms),
                Err(e) => {
                    problems.push(Problem::new("expires_at", e));
                    None
                }
            }
        }
    } else {
        base.map(|b| b.expires_at)
    };
    // What could be read is judged, whatever else is missing or unreadable.
    let parts = Parts {
        entry,
        invalidation,
        targets,
        expires_at,
    };
    check_parts(direction, &parts, costs, now_ms, problems);
    if problems.len() > before {
        return None;
    }
    Some(Levels {
        entry: parts.entry?,
        invalidation: parts.invalidation?,
        targets: parts.targets?,
        expires_at: parts.expires_at?,
    })
}

/// The levels of a call that could be read. One that is missing or unreadable is
/// `None`, and has its own problem already.
struct Parts {
    entry: Option<Zone>,
    invalidation: Option<f64>,
    targets: Option<Vec<f64>>,
    expires_at: Option<i64>,
}

/// A number in a message, as a person would write it: six significant digits, no
/// trailing zeros, so that what a float cannot hold exactly does not show.
fn show(x: f64) -> String {
    if x == 0.0 || !x.is_finite() {
        return x.to_string();
    }
    let decimals = (5 - x.abs().log10().floor() as i32).clamp(0, 12) as usize;
    let text = format!("{x:.decimals$}");
    match text.contains('.') {
        true => text.trim_end_matches('0').trim_end_matches('.').to_string(),
        false => text,
    }
}

/// Whether `levels` can be proven wrong: the invalidation on the far side of the entry,
/// the targets on the near side, an expiry ahead, and enough risk to measure in.
pub fn check_levels(
    direction: Direction,
    levels: &Levels,
    costs: &Costs,
    now_ms: i64,
    problems: &mut Vec<Problem>,
) {
    let parts = Parts {
        entry: Some(levels.entry),
        invalidation: Some(levels.invalidation),
        targets: Some(levels.targets.clone()),
        expires_at: Some(levels.expires_at),
    };
    check_parts(Some(direction), &parts, costs, now_ms, problems);
}

/// Every check the levels that could be read allow. A relation between two of them is
/// judged when both are there and each is valid by itself, and never against one that
/// is missing or unreadable: that one has its own problem, and a second about it would
/// only be noise.
fn check_parts(
    direction: Option<Direction>,
    parts: &Parts,
    costs: &Costs,
    now_ms: i64,
    problems: &mut Vec<Problem>,
) {
    let entry = match parts.entry {
        Some(entry) if entry.low > entry.high => {
            problems.push(Problem::new("entry", "entry.low is above entry.high"));
            None
        }
        entry => entry,
    };
    let word = |long: &'static str, short: &'static str| match direction {
        Some(Direction::Short) => short,
        _ => long,
    };
    // The entry edge nearest the invalidation, and the one nearest the targets.
    let edges = direction
        .zip(entry)
        .map(|(direction, entry)| match direction {
            Direction::Long => (entry.low, entry.high),
            Direction::Short => (entry.high, entry.low),
        });
    let sign = direction.map(Direction::sign);
    if let (Some((near_stop, _)), Some(sign), Some(invalidation), Some(entry)) =
        (edges, sign, parts.invalidation, entry)
    {
        let risk = sign * (near_stop - invalidation);
        if risk <= 0.0 {
            problems.push(
                Problem::new(
                    "invalidation",
                    format!(
                        "the invalidation of a {} must be {} its entry zone",
                        word("long", "short"),
                        word("below", "above")
                    ),
                )
                .hint(format!(
                    "entry is {} to {}, invalidation is {invalidation}",
                    entry.low, entry.high
                )),
            );
        } else {
            let floor = costs.least_risk(near_stop);
            if risk < floor {
                problems.push(
                    Problem::new(
                        "invalidation",
                        format!(
                            "the invalidation is {} from the entry, closer than the least risk scored here, {}",
                            show(risk),
                            show(floor)
                        ),
                    )
                    .hint(format!(
                        "the least risk is {MIN_RISK_IN_ROUND_TRIP_COSTS} times the assumed cost of getting in and out, {} bps of the price",
                        show(costs.round_trip() * 10_000.0)
                    )),
                );
            }
        }
    }
    if let (Some(targets), Some(sign)) = (&parts.targets, sign) {
        let near_target = edges.map(|(_, near_target)| near_target);
        let mut last = None;
        for (i, t) in targets.iter().enumerate() {
            let field = format!("targets[{i}]");
            if near_target.is_some_and(|near| sign * (t - near) <= 0.0) {
                problems.push(
                    Problem::new(&field, format!("target {t} is not beyond the entry zone")).hint(
                        format!(
                            "a target of this setup is {} {}",
                            word("above", "below"),
                            near_target.unwrap_or_default()
                        ),
                    ),
                );
            } else if last.is_some_and(|last: f64| sign * (t - last) <= 0.0) {
                problems.push(
                    Problem::new(
                        &field,
                        format!("target {t} is not beyond the one before it"),
                    )
                    .hint("list targets nearest first"),
                );
            }
            last = Some(*t);
        }
    }
    if let Some(expires_at) = parts.expires_at {
        if expires_at <= now_ms {
            problems.push(
                Problem::new("expires_at", "expires_at is not in the future")
                    .hint(format!("it is {} now", drawings::iso(now_ms))),
            );
        } else if expires_at - now_ms > MAX_EXPIRY_MS {
            problems.push(Problem::new(
                "expires_at",
                format!(
                    "expires_at is more than {} days ahead",
                    MAX_EXPIRY_MS / 86_400_000
                ),
            ));
        }
    }
}

/// The bracket a skip is scored with: in at the next bar's open, out one ATR against
/// or two ATR with it, on `bars`, the closed bars of the setup's interval. It runs out
/// after 24 of those bars, or after the longest expiry any setup may have if that is
/// sooner: a skip cannot be cancelled, so it must not stay open for years. Where one
/// ATR is less than the least risk a setup may have, the stop is that far and the target
/// twice that; the rule that applied comes back with the levels.
pub fn skip_bracket(
    direction: Direction,
    bars: &[Bar],
    interval_secs: i64,
    now_ms: i64,
) -> Result<(Levels, Bracket), Problem> {
    let expires_at = now_ms + (SKIP_EXPIRY_BARS * interval_secs * 1000).min(MAX_EXPIRY_MS);
    bracket(direction, bars, &Costs::default(), expires_at)
}

/// A skip on a stock (ADR-0070): the same bracket, with the stock costs' least risk, and
/// an expiry after `SKIP_EXPIRY_BARS` bars of its interval of session time.
pub fn stock_skip_bracket(
    market: Market,
    direction: Direction,
    bars: &[Bar],
    interval: &str,
    now_ms: i64,
) -> Result<(Levels, Bracket), Problem> {
    let iv = crate::interval::Interval::parse(interval).map_err(|e| Problem::new("interval", e))?;
    let expires_at = market
        .skip_expiry(&iv, now_ms, SKIP_EXPIRY_BARS)
        .unwrap_or(i64::MAX)
        .min(now_ms + MAX_EXPIRY_MS);
    bracket(direction, bars, &market.costs(), expires_at)
}

fn bracket(
    direction: Direction,
    bars: &[Bar],
    costs: &Costs,
    expires_at: i64,
) -> Result<(Levels, Bracket), Problem> {
    let atr = crate::orderflow::atr(bars, SKIP_ATR_LENGTH).filter(|a| a.is_finite() && *a > 0.0);
    let (Some(atr), Some(last)) = (atr, bars.last()) else {
        return Err(Problem::new(
            "decision",
            format!(
                "a skip needs {} closed bars of its interval to be scored",
                SKIP_ATR_LENGTH + 1
            ),
        )
        .hint("pick an interval with more history"));
    };
    let sign = direction.sign();
    // Closer than the least risk, a skip would be scored on costs alone.
    let floor = costs.least_risk(last.close);
    let (risk, rule) = match SKIP_STOP_ATR * atr {
        by_atr if by_atr >= floor => (by_atr, Bracket::Atr),
        _ => (floor, Bracket::Floor),
    };
    let invalidation = last.close - sign * risk;
    if invalidation <= 0.0 {
        return Err(Problem::new(
            "decision",
            "the default bracket of this skip would have a price at or below 0",
        ));
    }
    let levels = Levels {
        entry: Zone {
            low: last.close,
            high: last.close,
        },
        invalidation,
        targets: vec![last.close + sign * risk * SKIP_TARGET_ATR / SKIP_STOP_ATR],
        expires_at,
    };
    Ok((levels, rule))
}

/// What `create_setup` asked for, before the symbol and the drawings are checked.
pub struct Draft {
    pub symbol: String,
    pub interval: String,
    pub direction: Direction,
    pub decision: Decision,
    pub thesis: String,
    /// `None` for a skip, which takes the default bracket.
    pub levels: Option<Levels>,
    pub tags: Vec<String>,
    pub features: Option<Map<String, Value>>,
    pub probability: Option<f64>,
    pub playbook_version: Option<String>,
}

/// The features of a call: a flat object of numbers, booleans and short strings under
/// short plain names. What they mean is the agent's own business.
fn features(args: &Value, problems: &mut Vec<Problem>) -> Option<Map<String, Value>> {
    let map = match args.get("features") {
        None | Some(Value::Null) => return None,
        Some(Value::Object(map)) => map,
        Some(_) => {
            problems.push(Problem::new("features", "features must be an object"));
            return None;
        }
    };
    if map.len() > MAX_FEATURES {
        problems.push(Problem::new(
            "features",
            format!("features holds more than {MAX_FEATURES} entries"),
        ));
        return None;
    }
    for (name, value) in map {
        let shown: String = name.chars().take(MAX_STATED_TEXT).collect();
        let field = format!("features.{shown}");
        let plain = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_';
        if name.is_empty() || name.len() > MAX_STATED_TEXT || !name.bytes().all(plain) {
            problems.push(Problem::new(
                &field,
                format!("a feature's name is at most {MAX_STATED_TEXT} characters of a-z, 0-9 and _, and not empty"),
            ));
            continue;
        }
        let takes = match value {
            Value::Number(_) | Value::Bool(_) => true,
            Value::String(text) => text.chars().count() <= MAX_STATED_TEXT,
            _ => false,
        };
        if !takes {
            problems.push(Problem::new(
                &field,
                format!("a feature's value is a number, true or false, or a string of at most {MAX_STATED_TEXT} characters"),
            ));
        }
    }
    Some(map.clone())
}

fn probability(args: &Value, problems: &mut Vec<Problem>) -> Option<f64> {
    let value = args.get("probability").filter(|v| !v.is_null())?;
    let stated = value.as_f64().filter(|p| (0.0..=1.0).contains(p));
    if stated.is_none() {
        problems.push(Problem::new(
            "probability",
            "probability must be a number from 0 to 1",
        ));
    }
    stated
}

pub const CREATE_FIELDS: [&str; 15] = [
    "symbol",
    "interval",
    "direction",
    "decision",
    "thesis",
    "drawings",
    "entry",
    "invalidation",
    "targets",
    "expires_at",
    "tags",
    "as_of",
    "features",
    "probability",
    "playbook_version",
];

impl Draft {
    /// Read a `create_setup` call. Every problem goes to `problems`; a draft comes back
    /// only when there was none.
    pub fn parse(args: &Value, now_ms: i64, problems: &mut Vec<Problem>) -> Option<Draft> {
        Self::parse_for(args, None, now_ms, problems)
    }

    /// `parse` for a setup on `market`, whose costs set the least risk (ADR-0070).
    pub fn parse_for(
        args: &Value,
        market: Option<Market>,
        now_ms: i64,
        problems: &mut Vec<Problem>,
    ) -> Option<Draft> {
        let costs = market.map_or_else(Costs::default, Market::costs);
        let before = problems.len();
        check_fields(args, &CREATE_FIELDS, problems);
        let symbol = text(args, "symbol", 40, problems);
        let interval = text(args, "interval", 16, problems);
        if let Some(iv) = &interval {
            if let Err(e) = crate::interval::Interval::parse(iv) {
                problems.push(Problem::new("interval", e));
            }
        }
        let direction: Option<Direction> = one_of(args, "direction", "long or short", problems);
        let decision = match args.get("decision") {
            None | Some(Value::Null) => Some(Decision::Take),
            Some(_) => one_of(args, "decision", "take or skip", problems),
        };
        let thesis = text(args, "thesis", MAX_THESIS, problems);
        let tags = match args.get("tags") {
            None | Some(Value::Null) => Some(Vec::new()),
            Some(v) => tags(v, problems),
        };
        let levels = match decision {
            Some(Decision::Skip) => {
                for field in LEVEL_FIELDS {
                    if !matches!(args.get(field), None | Some(Value::Null)) {
                        problems.push(
                            Problem::new(field, format!("a skip takes no {field}")).hint(
                                "a skip is scored with the server's default bracket, the same for every skip",
                            ),
                        );
                    }
                }
                None
            }
            _ => parse_levels(args, None, direction, &costs, now_ms, problems),
        };
        let features = features(args, problems);
        let probability = probability(args, problems);
        let playbook_version = match args.get("playbook_version") {
            None | Some(Value::Null) => None,
            Some(_) => text(args, "playbook_version", MAX_STATED_TEXT, problems),
        };
        if problems.len() > before {
            return None;
        }
        Some(Draft {
            symbol: symbol?,
            interval: interval?,
            direction: direction?,
            decision: decision?,
            thesis: thesis?,
            levels,
            tags: tags?,
            features,
            probability,
            playbook_version,
        })
    }
}

fn changed(changes: &mut Map<String, Value>, field: &str, from: Value, to: Value) {
    if from != to {
        changes.insert(field.to_string(), json!({ "from": from, "to": to }));
    }
}

fn level_changes(changes: &mut Map<String, Value>, from: &Levels, to: &Levels) {
    changed(changes, "entry", json!(from.entry), json!(to.entry));
    changed(
        changes,
        "invalidation",
        json!(from.invalidation),
        json!(to.invalidation),
    );
    changed(changes, "targets", json!(from.targets), json!(to.targets));
    changed(
        changes,
        "expires_at",
        json!(from.expires_at),
        json!(to.expires_at),
    );
}

impl Record {
    /// A new setup, watching, and the first entry of its log.
    pub fn new(
        id: String,
        owner: &str,
        symbol: String,
        draft: Draft,
        levels: Levels,
        drawings: Vec<Value>,
        now_ms: i64,
    ) -> (Record, Revision) {
        let mut record = Record {
            id,
            owner: owner.to_string(),
            symbol,
            interval: draft.interval,
            direction: draft.direction,
            decision: draft.decision,
            thesis: draft.thesis,
            drawings,
            levels,
            bracket: Bracket::Agent,
            status: Status::Watching,
            created_at: now_ms,
            costs: Costs::default(),
            fill: None,
            exit: None,
            result_r: None,
            scored_through: None,
            gap: false,
            pending: Vec::new(),
            revisions: 0,
            agent_revisions: 0,
            tags: draft.tags,
            mae_r: None,
            mfe_r: None,
            mae_at: None,
            mfe_at: None,
            closest_approach_r: None,
            shadow_results: None,
            features: draft.features,
            probability: draft.probability,
            playbook_version: draft.playbook_version,
            market: None,
            last_close: None,
            adjustments: Vec::new(),
        };
        // What was stated goes into the log's first entry: the log is append-only in
        // the store, so this is the copy that cannot be rewritten after the outcome.
        let mut stated = Map::new();
        for (field, value) in [
            ("features", json!(record.features)),
            ("probability", json!(record.probability)),
            ("playbook_version", json!(record.playbook_version)),
        ] {
            if !value.is_null() {
                stated.insert(field.to_string(), json!({ "to": value }));
            }
        }
        let created = record.log(now_ms, By::Agent, Kind::Created, None, stated);
        (record, created)
    }

    fn log(
        &mut self,
        at: i64,
        by: By,
        kind: Kind,
        reason: Option<String>,
        changes: Map<String, Value>,
    ) -> Revision {
        self.revisions += 1;
        if by == By::Agent && kind != Kind::Created {
            self.agent_revisions += 1;
        }
        Revision {
            seq: self.revisions,
            at,
            by,
            kind,
            reason,
            changes,
        }
    }

    /// Take what the setup was made with from the first entry of its log. The log is
    /// append-only in the store and the record row is not, so the log is what counts:
    /// a row written by a build from before these fields has lost them, and gets them
    /// back here.
    pub fn restate(&mut self, created: &Revision) {
        if created.kind != Kind::Created {
            return;
        }
        let stated = |field: &str| created.changes.get(field).and_then(|c| c.get("to"));
        self.features = stated("features").and_then(Value::as_object).cloned();
        self.probability = stated("probability").and_then(Value::as_f64);
        self.playbook_version = stated("playbook_version")
            .and_then(Value::as_str)
            .map(str::to_string);
    }

    /// The levels as they will be once every pending change has taken effect.
    pub fn latest_levels(&self) -> &Levels {
        self.pending
            .iter()
            .rev()
            .find_map(|p| match p {
                Pending::Levels { levels, .. } => Some(levels),
                Pending::Cancel { .. } => None,
            })
            .unwrap_or(&self.levels)
    }

    fn cancel_pending(&self) -> bool {
        self.pending
            .iter()
            .any(|p| matches!(p, Pending::Cancel { .. }))
    }

    /// Whether the setup takes no change at all any more: closed, or being cancelled.
    pub fn takes_no_change(&self) -> bool {
        !self.status.is_open() || self.cancel_pending()
    }

    /// Why this setup's state forbids a revision, with new levels or without. When it
    /// does, that is the whole answer: what is wrong with levels that could not be
    /// applied anyway is not worth a round trip. Tags alone may change on any setup,
    /// closed or being cancelled too: they label it, they do not move it.
    pub fn revise_refusal(&self, levels: bool, tags_only: bool) -> Option<Problem> {
        let only_tags = "only its tags can be revised now, in a call that changes nothing else";
        if !self.status.is_open() && !tags_only {
            return Some(
                Problem::new("id", "this setup is closed and cannot be revised").hint(only_tags),
            );
        }
        if self.cancel_pending() && !tags_only {
            return Some(Problem::new("id", "this setup is being cancelled").hint(only_tags));
        }
        if levels && self.decision == Decision::Skip {
            return Some(
                Problem::new("decision", "a skip keeps the server's default bracket")
                    .hint("only its thesis, drawings and tags can be revised"),
            );
        }
        if levels && self.status == Status::Triggered {
            return Some(
                Problem::new("status", "this setup is filled, so its levels are fixed")
                    .hint("only its thesis, drawings and tags can be revised now"),
            );
        }
        None
    }

    /// The agent revises its setup. The thesis, drawings and tags change now. New
    /// levels count from the first bar that opens at or after `now_ms`, and only while
    /// the setup is not filled.
    pub fn revise(
        &mut self,
        thesis: Option<String>,
        drawings: Option<Vec<Value>>,
        tags: Option<Vec<String>>,
        levels: Option<Levels>,
        reason: String,
        now_ms: i64,
    ) -> Result<Revision, Vec<Problem>> {
        let mut problems = Vec::new();
        let tags_only =
            tags.is_some() && thesis.is_none() && drawings.is_none() && levels.is_none();
        match (self.revise_refusal(levels.is_some(), tags_only), &levels) {
            (Some(refusal), _) => problems.push(refusal),
            (None, Some(levels)) => {
                check_levels(self.direction, levels, &self.costs, now_ms, &mut problems)
            }
            (None, None) => {}
        }
        let mut changes = Map::new();
        if let Some(thesis) = &thesis {
            changed(&mut changes, "thesis", json!(self.thesis), json!(thesis));
        }
        if let Some(drawings) = &drawings {
            changed(
                &mut changes,
                "drawings",
                json!(self.drawings),
                json!(drawings),
            );
        }
        if let Some(tags) = &tags {
            changed(&mut changes, "tags", json!(self.tags), json!(tags));
        }
        if let Some(levels) = &levels {
            level_changes(&mut changes, self.latest_levels(), levels);
        }
        if problems.is_empty() && changes.is_empty() {
            problems.push(Problem::new("id", "this revision changes nothing"));
        }
        if !problems.is_empty() {
            return Err(problems);
        }
        if let Some(thesis) = thesis {
            self.thesis = thesis;
        }
        if let Some(drawings) = drawings {
            self.drawings = drawings;
        }
        if let Some(tags) = tags {
            self.tags = tags;
        }
        if let Some(levels) = levels.filter(|l| l != self.latest_levels()) {
            self.pending.push(Pending::Levels { at: now_ms, levels });
        }
        Ok(self.log(now_ms, By::Agent, Kind::Revised, Some(reason), changes))
    }

    /// The agent cancels its setup. It counts from the first bar that opens at or after
    /// `now_ms`: a filled setup is closed at that bar's open and scored.
    pub fn cancel(&mut self, reason: String, now_ms: i64) -> Result<Revision, Vec<Problem>> {
        let problem = if !self.status.is_open() {
            Some(Problem::new("id", "this setup is already closed"))
        } else if self.cancel_pending() {
            Some(Problem::new("id", "this setup is already being cancelled"))
        } else if self.decision == Decision::Skip {
            Some(
                Problem::new("decision", "a skip cannot be cancelled")
                    .hint("it is scored to its end, so that skipped decisions are not lost"),
            )
        } else {
            None
        };
        if let Some(problem) = problem {
            return Err(vec![problem]);
        }
        self.pending.push(Pending::Cancel { at: now_ms });
        Ok(self.log(now_ms, By::Agent, Kind::Cancelled, Some(reason), Map::new()))
    }

    /// R of a filled setup closed at `exit` in the bar that opened at `exit_at`: the move
    /// in units of the risk taken at the fill, less the assumed fee and slippage on both
    /// sides. A stock's dividends that went ex by then count as part of the move: paid to
    /// a long, by a short.
    fn score_at(&self, fill: f64, exit: f64, exit_at: i64) -> f64 {
        let risk = (fill - self.levels.invalidation).abs();
        if risk <= 0.0 {
            return 0.0;
        }
        let paid: f64 = self
            .adjustments
            .iter()
            .filter_map(|a| match a {
                Adjustment::Dividend { at, amount } if *at <= exit_at => Some(amount),
                _ => None,
            })
            .sum();
        let gross = self.direction.sign() * (exit - fill + paid) / risk;
        let cost = (fill + exit) * self.costs.round_trip() / 2.0 / risk;
        ((gross - cost) * 10_000.0).round() / 10_000.0
    }

    /// `score_at` with no dividend counted.
    #[cfg(test)]
    fn score(&self, fill: f64, exit: f64) -> f64 {
        self.score_at(fill, exit, i64::MIN)
    }

    /// Widen the excursions by a price the position went `against` and one it went
    /// `with` in the bar that opened at `at`, in R from the fill, before costs. Only
    /// once they are kept: a setup filled before they were has none, rather than ones
    /// that miss its start.
    fn excursion(&mut self, against: f64, with: f64, at: i64) {
        let (Some(fill), Some(mae), Some(mfe)) = (self.fill, self.mae_r, self.mfe_r) else {
            return;
        };
        let risk = (fill.price - self.levels.invalidation).abs();
        if risk <= 0.0 {
            return;
        }
        let r = |p: f64| {
            (self.direction.sign() * (p - fill.price) / risk * 10_000.0).round() / 10_000.0
        };
        if r(against) < mae {
            (self.mae_r, self.mae_at) = (Some(r(against)), Some(at));
        }
        if r(with) > mfe {
            (self.mfe_r, self.mfe_at) = (Some(r(with)), Some(at));
        }
    }

    /// Narrow the closest approach of a watching setup by this bar. Only when it was
    /// kept from the first bar scored, so that it never misses a closer one before.
    fn approach(&mut self, bar: &MinuteBar, reached: bool, first: bool) {
        let entry = self.levels.entry;
        let r = match reached {
            true => 0.0,
            false => {
                let edge = if bar.low > entry.high {
                    entry.high
                } else {
                    entry.low
                };
                let risk = (edge - self.levels.invalidation).abs();
                if risk <= 0.0 {
                    return;
                }
                let gap = (bar.low - entry.high).max(entry.low - bar.high);
                (gap / risk * 10_000.0).round() / 10_000.0
            }
        };
        self.closest_approach_r = match (first, self.closest_approach_r) {
            (true, _) => Some(r),
            (false, kept) => kept.map(|c| c.min(r)),
        };
    }

    /// Whether bars still move this setup: it is open, or one of its shadows is
    /// (ADR-0066). Only an open setup counts toward the limits on open setups.
    pub fn is_scoring(&self) -> bool {
        self.status.is_open() || self.shadow_results.as_ref().is_some_and(Shadows::any_open)
    }

    /// The price at which a position filled at `fill` comes out even after the assumed
    /// costs of both sides: an exit there scores 0 R.
    fn break_even(&self, fill: f64) -> f64 {
        let half = self.direction.sign() * self.costs.round_trip() / 2.0;
        fill * (1.0 + half) / (1.0 - half)
    }

    /// The close time (ms) of the Nth bar of the setup's interval after the bar holding
    /// the fill, on the grid every chart here uses.
    fn time_stop_at(&self, fill_time: i64) -> Option<i64> {
        let iv = crate::interval::Interval::parse(&self.interval).ok()?;
        let secs = crate::data::interval_secs(&self.interval)?;
        let n = match secs < SHADOW_TIME_STOP_LONG_FROM_SECS {
            true => SHADOW_TIME_STOP_BARS.0,
            false => SHADOW_TIME_STOP_BARS.1,
        };
        // A stock's bars are on its session grid (ADR-0070).
        if let Some(market) = self.market {
            return market.bar_close_after(&iv, fill_time, i64::from(n));
        }
        let mut t = iv.grid_start(fill_time.div_euclid(1000));
        for _ in 0..=n {
            t = iv.grid_end(t);
        }
        Some(t * 1000)
    }

    /// Close what is left of a shadow at `price` in the bar at `at`, and score it. With
    /// no price, nothing was read: it closes with no result.
    fn settle(&self, s: &mut Shadow, status: ShadowStatus, price: Option<f64>, at: i64) {
        let Some(fill) = self.fill else {
            return;
        };
        s.status = status;
        let Some(price) = price else {
            return;
        };
        let left = 1.0 - s.exits.iter().map(|e| e.fraction).sum::<f64>();
        s.exits.push(ShadowExit {
            price,
            fraction: left,
            bar_time: at,
        });
        let r: f64 = s
            .exits
            .iter()
            .map(|e| e.fraction * self.score_at(fill.price, e.price, e.bar_time))
            .sum();
        // + 0.0 makes a -0 from rounding a break-even exit read as 0.
        s.result_r = Some((r * 10_000.0).round() / 10_000.0 + 0.0);
        s.exited_at = Some(at);
    }

    /// Close every open shadow the same way: at a cancel, an expiry, or the fill bar's
    /// stop.
    fn end_shadows(&mut self, status: ShadowStatus, price: Option<f64>, at: i64) {
        let Some(mut shadows) = self.shadow_results.take() else {
            return;
        };
        for (_, s) in shadows.each_mut() {
            if s.is_open() {
                self.settle(s, status, price, at);
            }
        }
        self.shadow_results = Some(shadows);
    }

    /// Move the open shadows by a bar after the fill bar. Each reads it as the canonical
    /// setup does: past the expiry out at the open; else its stop first, a gap through
    /// it at the open; then its targets at their price. A stop it moves counts from the
    /// next bar.
    fn shadow_bar(&mut self, bar: &MinuteBar) {
        if bar.time >= self.levels.expires_at {
            self.end_shadows(ShadowStatus::Expired, Some(bar.open), bar.time);
            return;
        }
        let (Some(mut shadows), Some(fill)) = (self.shadow_results.take(), self.fill) else {
            return;
        };
        let sign = self.direction.sign();
        let (against, with) = match self.direction {
            Direction::Long => (bar.low, bar.high),
            Direction::Short => (bar.high, bar.low),
        };
        let risk = (fill.price - self.levels.invalidation).abs();
        let r = |p: f64| sign * (p - fill.price) / risk;
        let reached = |target: f64| sign * (with - target) >= 0.0;
        let t1 = self.levels.targets[0];
        let t2 = self.levels.targets.get(1).copied();
        let be = self.break_even(fill.price);
        for (variant, s) in shadows.each_mut() {
            if !s.is_open() {
                continue;
            }
            if sign * (against - s.stop) <= 0.0 {
                let exit = match sign * (bar.open - s.stop) <= 0.0 {
                    true => bar.open,
                    false => s.stop,
                };
                // Named by where it got out: at or through the invalidation is stopped,
                // even past a moved stop; short of it, the moved stop was hit.
                let status = match sign * (exit - self.levels.invalidation) <= 0.0 {
                    true => ShadowStatus::Stopped,
                    false => ShadowStatus::Breakeven,
                };
                self.settle(s, status, Some(exit), bar.time);
                continue;
            }
            match variant {
                Variant::HalfAtT1 => {
                    if s.exits.is_empty() && reached(t1) {
                        s.exits.push(ShadowExit {
                            price: self.target_exit(bar, t1),
                            fraction: SHADOW_PARTIAL_AT_T1,
                            bar_time: bar.time,
                        });
                        // Never beyond T1: where T1 is nearer the fill than the costs,
                        // break-even would be a stop on the far side of the market.
                        s.stop = match sign * (be - t1) > 0.0 {
                            true => t1,
                            false => be,
                        };
                    }
                    // T2 may come in the bar that took T1: price passed T1 to reach it.
                    if !s.exits.is_empty() && t2.is_some_and(reached) {
                        let exit = t2.map(|t2| self.target_exit(bar, t2));
                        self.settle(s, ShadowStatus::Won, exit, bar.time);
                    }
                }
                Variant::BeAt1R => {
                    if reached(t1) {
                        let exit = self.target_exit(bar, t1);
                        self.settle(s, ShadowStatus::Won, Some(exit), bar.time);
                    } else if s.stop == self.levels.invalidation && r(with) >= SHADOW_BE_AT_R {
                        s.stop = be;
                    }
                }
                Variant::TimeStop => {
                    if reached(t1) {
                        let exit = self.target_exit(bar, t1);
                        self.settle(s, ShadowStatus::Won, Some(exit), bar.time);
                    } else if let Some(at) = s.time_stop_at {
                        // The 1m bar that closes the judged bar, or the first after it
                        // when that one was not seen.
                        if bar.time + MINUTE_MS >= at {
                            s.time_stop_at = None;
                            if r(bar.close) < SHADOW_TIME_STOP_KEEP_R {
                                self.settle(s, ShadowStatus::TimedOut, Some(bar.close), bar.time);
                            }
                        }
                    }
                }
            }
        }
        self.shadow_results = Some(shadows);
    }

    fn close(&mut self, status: Status, exit: Option<f64>, bar: &MinuteBar) -> Revision {
        let exit = exit.map(|price| Mark {
            price,
            bar_time: bar.time,
        });
        self.close_at(status, exit, bar.time)
    }

    /// Close the setup at `exit`, logged at `at`.
    fn close_at(&mut self, status: Status, exit: Option<Mark>, at: i64) -> Revision {
        let from = self.status;
        self.status = status;
        let mut changes = Map::new();
        changed(&mut changes, "status", json!(from), json!(status));
        self.result_r = Some(match (self.fill, exit) {
            (Some(fill), Some(exit)) => {
                self.exit = Some(exit);
                changes.insert("exit".into(), json!({ "from": null, "to": self.exit }));
                self.score_at(fill.price, exit.price, exit.bar_time)
            }
            _ => 0.0,
        });
        changes.insert(
            "result_r".into(),
            json!({ "from": null, "to": self.result_r }),
        );
        let dropped = std::mem::take(&mut self.pending);
        let closed = self.log(at, By::Server, Kind::Status, None, changes);
        if dropped.is_empty() {
            return closed;
        }
        // Kept in the same entry: one bar, one entry.
        Revision {
            reason: Some(format!(
                "{} change(s) sent after this bar opened did not take effect",
                dropped.len()
            )),
            ..closed
        }
    }

    /// Whether a bar moves this setup: it is scoring, and the bar opened after it was
    /// created and after the last bar applied.
    fn takes(&self, bar: &MinuteBar) -> bool {
        self.is_scoring()
            && bar.time >= self.created_at
            && self.scored_through.is_none_or(|t| bar.time > t)
    }

    /// Apply one closed 1m bar. Returns the log entries it caused. A bar that is not
    /// newer than the last one applied, or that opened before the setup was created,
    /// changes nothing: the feed replays bars after a reconnect.
    ///
    /// A stock setup (ADR-0070) ends at the last 1m close at or before its expiry, so a
    /// bar that closes after the expiry ends it at the close before, which may be
    /// Friday's for an expiry on a weekend.
    pub fn apply_bar(&mut self, bar: &MinuteBar) -> Vec<Revision> {
        if self.market.is_none() {
            return self.step(bar);
        }
        if !self.takes(bar) {
            return Vec::new();
        }
        if bar.time + MINUTE_MS > self.expiry_for(bar.time) {
            return self.expire_at_last_close(bar.time);
        }
        let out = self.step(bar);
        self.last_close = Some(Mark {
            price: bar.close,
            bar_time: bar.time,
        });
        out
    }

    /// A stock setup whose expiry the server has read past, with every bar that closed by
    /// `through` applied: no bar closed after the expiry, so it ends at the last close it
    /// had. A Binance setup expires on its bars only.
    pub fn expire_by(&mut self, through: i64) -> Vec<Revision> {
        if self.market.is_none() || !self.is_scoring() || self.expiry_for(through) > through {
            return Vec::new();
        }
        self.expire_at_last_close(through)
    }

    /// The expiry in force for a bar that opens at `t`: a watching setup's new levels
    /// count from the first bar at or after they were sent.
    fn expiry_for(&self, t: i64) -> i64 {
        self.due_levels(t)
            .map_or(self.levels.expires_at, |l| l.expires_at)
    }

    fn due_levels(&self, t: i64) -> Option<&Levels> {
        if self.status != Status::Watching {
            return None;
        }
        self.pending.iter().rev().find_map(|p| match p {
            Pending::Levels { at, levels } if *at <= t => Some(levels),
            _ => None,
        })
    }

    /// End a stock setup at its expiry: the levels sent before `t` take effect first, and
    /// a filled one exits at its last close, its open shadows with it. Unfilled, it
    /// scores 0. Logged at the expiry.
    fn expire_at_last_close(&mut self, t: i64) -> Vec<Revision> {
        if let Some(levels) = self.due_levels(t).cloned() {
            self.levels = levels;
            self.pending.retain(|p| p.at() > t);
        }
        let at = self.levels.expires_at;
        let exit = self.fill.and(self.last_close);
        let price = exit.map(|m| m.price);
        self.end_shadows(
            ShadowStatus::Expired,
            price,
            exit.map_or(at, |m| m.bar_time),
        );
        if !self.status.is_open() {
            return Vec::new();
        }
        vec![self.close_at(Status::Expired, exit, at)]
    }

    /// Where a bar that reaches `target` takes a position out: at the target, or for a
    /// stock at the open when the bar opened past it, as a resting limit fills at a gap
    /// (ADR-0070). Binance setups keep the target.
    fn target_exit(&self, bar: &MinuteBar, target: f64) -> f64 {
        let past = self.direction.sign() * (bar.open - target) > 0.0;
        match self.market.is_some() && past {
            true => bar.open,
            false => target,
        }
    }

    /// A split or a dividend of a stock setup's stock (ADR-0070), once. A split dated
    /// after the setup was made divides every price of it by its ratio: Yahoo's bars are
    /// in the new terms from then on, older ones included. A dividend is kept when the
    /// position was open on its ex-date, to count in the result from the exit on.
    /// Returns the log entry when it changed the setup.
    pub fn adjust(&mut self, action: Adjustment) -> Option<Revision> {
        if self.market.is_none()
            || !self.is_scoring()
            || self.adjustments.iter().any(|a| a.same(&action))
        {
            return None;
        }
        let before = json!(self.adjustments);
        let mut changes = Map::new();
        let reason = match action {
            Adjustment::Split { at, ratio } => {
                if at <= self.created_at || !ratio.is_finite() || ratio <= 0.0 || ratio == 1.0 {
                    return None;
                }
                let (levels, fill) = (self.levels.clone(), self.fill);
                self.scale(ratio, at);
                level_changes(&mut changes, &levels, &self.levels);
                changed(&mut changes, "fill", json!(fill), json!(self.fill));
                format!(
                    "a split of {} for 1 took effect at {}: every price of this setup is divided by {}",
                    show(ratio),
                    drawings::iso(at),
                    show(ratio)
                )
            }
            Adjustment::Dividend { at, amount } => {
                let held = self.fill.is_some_and(|f| f.bar_time < at);
                if !held || !amount.is_finite() || amount <= 0.0 {
                    return None;
                }
                format!(
                    "a dividend of {} per share went ex at {}: a long is paid it and a short pays it, from its exit on",
                    show(amount),
                    drawings::iso(at)
                )
            }
        };
        self.adjustments.push(action);
        changed(&mut changes, "adjustments", before, json!(self.adjustments));
        Some(self.log(
            action.at(),
            By::Server,
            Kind::Adjusted,
            Some(reason),
            changes,
        ))
    }

    /// Divide every price of the setup by `ratio`, for a split that took effect at
    /// `split_at`. Levels sent at or after it are in the new terms already: the agent
    /// read them from charts Yahoo had adjusted. Results in R do not change.
    fn scale(&mut self, ratio: f64, split_at: i64) {
        let levels = |l: &mut Levels| {
            l.entry.low /= ratio;
            l.entry.high /= ratio;
            l.invalidation /= ratio;
            l.targets.iter_mut().for_each(|t| *t /= ratio);
        };
        levels(&mut self.levels);
        for pending in &mut self.pending {
            if let Pending::Levels { levels: l, at } = pending {
                if *at < split_at {
                    levels(l);
                }
            }
        }
        for mark in [&mut self.fill, &mut self.exit, &mut self.last_close]
            .into_iter()
            .flatten()
        {
            mark.price /= ratio;
        }
        if let Some(shadows) = &mut self.shadow_results {
            for (_, s) in shadows.each_mut() {
                s.stop /= ratio;
                s.exits.iter_mut().for_each(|e| e.price /= ratio);
            }
        }
        for a in &mut self.adjustments {
            if let Adjustment::Dividend { amount, .. } = a {
                *amount /= ratio;
            }
        }
    }

    fn step(&mut self, bar: &MinuteBar) -> Vec<Revision> {
        let mut out = Vec::new();
        if !self.takes(bar) {
            return out;
        }
        let first = self.scored_through.is_none();
        self.scored_through = Some(bar.time);
        // Closed with a shadow still open: only the shadows move (ADR-0066).
        if !self.status.is_open() {
            self.shadow_bar(bar);
            return out;
        }
        // The prices of this bar that went against a position and with it.
        let (against, with) = match self.direction {
            Direction::Long => (bar.low, bar.high),
            Direction::Short => (bar.high, bar.low),
        };

        // What the agent sent before this bar opened.
        let due: Vec<Pending> = {
            let (due, later) = std::mem::take(&mut self.pending)
                .into_iter()
                .partition(|p| p.at() <= bar.time);
            self.pending = later;
            due
        };
        // New levels in effect start the closest approach again: it is to the zone the
        // setup has, not one it had.
        let mut restart = false;
        for change in due {
            match change {
                Pending::Levels { levels, .. } if self.status == Status::Watching => {
                    self.levels = levels;
                    restart = true;
                }
                Pending::Levels { at, .. } => {
                    out.push(self.log(
                        bar.time,
                        By::Server,
                        Kind::Voided,
                        Some(format!(
                            "the levels sent at {} did not take effect: the setup was filled first",
                            drawings::iso(at)
                        )),
                        Map::new(),
                    ));
                }
                Pending::Cancel { .. } => {
                    let exit = self.fill.map(|_| bar.open);
                    self.excursion(bar.open, bar.open, bar.time);
                    self.end_shadows(ShadowStatus::Cancelled, exit, bar.time);
                    out.push(self.close(Status::Cancelled, exit, bar));
                    return out;
                }
            }
        }

        if bar.time >= self.levels.expires_at {
            let exit = self.fill.map(|_| bar.open);
            self.excursion(bar.open, bar.open, bar.time);
            self.end_shadows(ShadowStatus::Expired, exit, bar.time);
            out.push(self.close(Status::Expired, exit, bar));
            return out;
        }

        let sign = self.direction.sign();
        let stop = self.levels.invalidation;
        // Beyond a level, on the losing side of it.
        let stopped_at = |price: f64| sign * (price - stop) <= 0.0;
        let stop_touched = match self.direction {
            Direction::Long => bar.low <= stop,
            Direction::Short => bar.high >= stop,
        };

        if self.status == Status::Watching {
            // The thesis broke before the entry could be reached.
            if stopped_at(bar.open) {
                out.push(self.close(Status::Invalidated, None, bar));
                return out;
            }
            let entry = self.levels.entry;
            let reached =
                self.decision == Decision::Skip || (bar.low <= entry.high && bar.high >= entry.low);
            self.approach(bar, reached, first || restart);
            if !reached {
                if stop_touched {
                    out.push(self.close(Status::Invalidated, None, bar));
                }
                return out;
            }
            let price = match self.decision {
                Decision::Skip => bar.open,
                Decision::Take => bar.open.clamp(entry.low, entry.high),
            };
            let fill = Mark {
                price,
                bar_time: bar.time,
            };
            self.fill = Some(fill);
            self.status = Status::Triggered;
            (self.mae_r, self.mfe_r) = (Some(0.0), Some(0.0));
            (self.mae_at, self.mfe_at) = (Some(bar.time), Some(bar.time));
            let stop = self.levels.invalidation;
            self.shadow_results = Some(Shadows {
                v1_half_t1_be: Shadow::open(stop, None),
                v2_be_at_1r: Shadow::open(stop, None),
                v3_time_stop: Shadow::open(stop, self.time_stop_at(bar.time)),
            });
            let mut changes = Map::new();
            changed(
                &mut changes,
                "status",
                json!(Status::Watching),
                json!(Status::Triggered),
            );
            changes.insert("fill".into(), json!({ "from": null, "to": fill }));
            out.push(self.log(bar.time, By::Server, Kind::Status, None, changes));
            // The bar that filled it can stop it out, and cannot win it: a bar does not
            // say in which order it traded. So it counts against the position only.
            if stop_touched {
                self.excursion(stop, price, bar.time);
                self.end_shadows(ShadowStatus::Stopped, Some(stop), bar.time);
                out.push(self.close(Status::Invalidated, Some(stop), bar));
            } else {
                self.excursion(against, price, bar.time);
            }
            return out;
        }

        self.shadow_bar(bar);
        if stop_touched {
            // A gap through the level closes at the open, which is worse. The stop counts
            // first, so of this bar only its open went with the position.
            let exit = if stopped_at(bar.open) { bar.open } else { stop };
            // Nothing past the first target counts for it either: the scorer reads the
            // stop first, but a position is not further in profit than its target.
            let with = match (self.direction, self.levels.targets.first()) {
                (Direction::Long, Some(&t)) => bar.open.min(t),
                (Direction::Short, Some(&t)) => bar.open.max(t),
                (_, None) => bar.open,
            };
            self.excursion(exit, with, bar.time);
            out.push(self.close(Status::Invalidated, Some(exit), bar));
        } else if let Some(target) = self.levels.targets.first().copied() {
            let reached = match self.direction {
                Direction::Long => bar.high >= target,
                Direction::Short => bar.low <= target,
            };
            if reached {
                // Nothing beyond the target counts: the position was out there. A bar
                // that opens at or past it was out at the open, so its later low (or
                // high, for a short) does not count either.
                let opened_past = match self.direction {
                    Direction::Long => bar.open >= target,
                    Direction::Short => bar.open <= target,
                };
                let against = if opened_past { bar.open } else { against };
                let exit = self.target_exit(bar, target);
                self.excursion(against, exit, bar.time);
                out.push(self.close(Status::Won, Some(exit), bar));
            } else {
                self.excursion(against, with, bar.time);
            }
        }
        out
    }

    /// The bars the server did not see, written to the log: this setup's result does
    /// not cover them.
    pub fn note_gap(&mut self, from_ms: i64, to_ms: i64, now_ms: i64) -> Revision {
        let mut changes = Map::new();
        changes.insert("not_scored".into(), json!({ "from": from_ms, "to": to_ms }));
        self.scored_through = Some(self.scored_through.unwrap_or(0).max(to_ms - MINUTE_MS));
        // After the close only shadows were being scored: the result covers its bars,
        // and the log says which the shadows missed.
        if self.status.is_open() {
            self.gap = true;
        }
        self.log(
            now_ms,
            By::Server,
            Kind::Gap,
            Some("the server did not see these bars".into()),
            changes,
        )
    }

    /// End a setup that ran out while nobody scored it: its owner was off the operator's
    /// list, so it got no bars. An expiry is a time, so it still expires. Never filled,
    /// it scores 0 like any unfilled setup. Filled, it has no exit: no bar was read, and
    /// a price is not invented, so it closes with no result. Either way the stretch that
    /// was not scored is in the log and `gap` is set. Closed with a shadow still open,
    /// only the shadows end so; the setup's own result and `gap` stay as they were.
    pub fn expire_unscored(&mut self, now_ms: i64) -> Vec<Revision> {
        if !self.is_scoring() || now_ms < self.levels.expires_at {
            return Vec::new();
        }
        let gap = self.note_gap(self.next_bar_time(), now_ms, now_ms);
        // Its open shadows end the same way, with no exit and no result.
        self.end_shadows(ShadowStatus::Expired, None, now_ms);
        if !self.status.is_open() {
            return vec![gap];
        }
        let from = self.status;
        self.status = Status::Expired;
        self.result_r = self.fill.is_none().then_some(0.0);
        self.pending.clear();
        let mut changes = Map::new();
        changed(&mut changes, "status", json!(from), json!(self.status));
        changes.insert(
            "result_r".into(),
            json!({ "from": null, "to": self.result_r }),
        );
        let reason = "expired while its owner was not on the operator's list, so it was not scored";
        let closed = self.log(
            now_ms,
            By::Server,
            Kind::Status,
            Some(reason.into()),
            changes,
        );
        vec![gap, closed]
    }

    /// The first bar this setup still needs, by open time.
    pub fn next_bar_time(&self) -> i64 {
        match self.scored_through {
            Some(t) => t + MINUTE_MS,
            None => (self.created_at + MINUTE_MS - 1).div_euclid(MINUTE_MS) * MINUTE_MS,
        }
    }

    /// How long it took, in bars of its interval. A month counts as 30 days, as
    /// everywhere else here (`Interval::secs`).
    pub fn durations(&self) -> Durations {
        let (Some(fill), Some(secs)) = (self.fill, crate::data::interval_secs(&self.interval))
        else {
            return Durations::default();
        };
        // A stock's bars are on its session grid (ADR-0070).
        let session = self
            .market
            .zip(crate::interval::Interval::parse(&self.interval).ok());
        let bars = |from: i64, to: i64| {
            let n = match session {
                Some((market, iv)) => market.bars_between(&iv, from, to),
                None => (to - from) as f64 / (secs * 1000) as f64,
            };
            (n * 100.0).round() / 100.0
        };
        let last = match (self.exit, self.status) {
            (Some(exit), _) => Some(exit.bar_time),
            (None, Status::Triggered) => self.scored_through,
            // Closed with no exit: it expired while nobody scored it.
            (None, _) => None,
        };
        Durations {
            bars_to_fill: Some(bars(self.created_at, fill.bar_time)),
            bars_to_exit: last.map(|t| bars(fill.bar_time, t)),
            bars_to_mae: self.mae_at.map(|t| bars(fill.bar_time, t)),
            bars_to_mfe: self.mfe_at.map(|t| bars(fill.bar_time, t)),
        }
    }

    /// As a tool answers it: everything but the owner, the levels as fields of the
    /// setup, and its durations.
    pub fn view(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        if let Some(map) = v.as_object_mut() {
            map.remove("owner");
            if let Some(Value::Object(levels)) = map.remove("levels") {
                map.extend(levels);
            }
            if let Ok(Value::Object(durations)) = serde_json::to_value(self.durations()) {
                map.extend(durations);
            }
        }
        v
    }
}

/// What `get_track_record` groups closed setups by (ADR-0064).
#[derive(Clone, Debug, PartialEq)]
pub enum GroupBy {
    PlaybookVersion,
    /// One feature, by its name.
    Feature(String),
    /// The stated probability, in tenths: 0.0, 0.1 ... 0.9, with 1 in the last.
    Probability,
    Symbol,
    Direction,
}

/// Groups one answer holds. More is not a grouping but a list, and is refused.
pub const MAX_RECORD_GROUPS: usize = 50;

impl Record {
    /// The value this setup is grouped under; null where it stated none. A number is
    /// written as a decimal, so that 1 and 1.0 are one group and read as `1.0`.
    fn group_key(&self, by: &GroupBy) -> Value {
        let key = match by {
            GroupBy::PlaybookVersion => json!(self.playbook_version),
            GroupBy::Feature(name) => self
                .features
                .as_ref()
                .and_then(|f| f.get(name))
                .cloned()
                .unwrap_or(Value::Null),
            GroupBy::Probability => json!(self
                .probability
                .map(|p| ((p * 10.0).floor().min(9.0)) / 10.0)),
            GroupBy::Symbol => json!(self.symbol),
            GroupBy::Direction => json!(self.direction),
        };
        match key.as_f64() {
            Some(number) => json!(number),
            None => key,
        }
    }
}

/// Counts over closed setups. Counts and sums only: what they mean is the reader's
/// to judge, and a handful of setups means little.
#[derive(Default)]
struct Tally {
    count: u32,
    won: u32,
    invalidated: u32,
    expired: u32,
    cancelled: u32,
    with_result: u32,
    total_r: f64,
    /// Setups whose entry was reached, and the results of those.
    entered: u32,
    entered_with_result: u32,
    entered_r: f64,
    gaps: u32,
    stated: u32,
    stated_sum: f64,
    stated_won: u32,
    /// Of those that stated a probability, the ones whose entry was reached, and how
    /// many of those won: the share without the setups that never traded.
    stated_entered: u32,
    stated_won_entered: u32,
}

impl Tally {
    fn add(&mut self, r: &Record) {
        self.count += 1;
        match r.status {
            Status::Won => self.won += 1,
            Status::Invalidated => self.invalidated += 1,
            Status::Expired => self.expired += 1,
            Status::Cancelled => self.cancelled += 1,
            Status::Watching | Status::Triggered => {}
        }
        // Entered: the setup was filled at some time. One that was cancelled, ran out or
        // was invalidated before that never traded; its result is 0 R all the same
        // (ADR-0059).
        let entered = r.fill.is_some();
        self.entered += u32::from(entered);
        if let Some(result) = r.result_r {
            self.with_result += 1;
            self.total_r += result;
            if entered {
                self.entered_with_result += 1;
                self.entered_r += result;
            }
        }
        self.gaps += u32::from(r.gap);
        if let Some(p) = r.probability {
            self.stated += 1;
            self.stated_sum += p;
            self.stated_won += u32::from(r.status == Status::Won);
            if entered {
                self.stated_entered += 1;
                self.stated_won_entered += u32::from(r.status == Status::Won);
            }
        }
    }

    fn json(&self) -> Value {
        let per = |sum: f64, n: u32| (n > 0).then(|| sum / f64::from(n));
        json!({
            "count": self.count,
            "won": self.won,
            "invalidated": self.invalidated,
            "expired": self.expired,
            "cancelled": self.cancelled,
            "with_result": self.with_result,
            "total_r": self.total_r,
            "mean_r": per(self.total_r, self.with_result),
            "entered": self.entered,
            "mean_r_entered": per(self.entered_r, self.entered_with_result),
            "gaps": self.gaps,
            "probability": {
                "stated": self.stated,
                "mean": per(self.stated_sum, self.stated),
                "won": self.stated_won,
                "observed": per(f64::from(self.stated_won), self.stated),
                "stated_entered": self.stated_entered,
                "won_entered": self.stated_won_entered,
                "observed_entered": per(f64::from(self.stated_won_entered), self.stated_entered),
            },
        })
    }
}

/// Where a group's key sorts: none first, then false and true, numbers, strings.
fn key_order(a: &Value, b: &Value) -> std::cmp::Ordering {
    let rank = |v: &Value| match v {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        _ => 3,
    };
    rank(a).cmp(&rank(b)).then_with(|| match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Number(x), Value::Number(y)) => x
            .as_f64()
            .partial_cmp(&y.as_f64())
            .unwrap_or(std::cmp::Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    })
}

/// The counts over `closed`, in all and per group: `(total, groups)`, each group with
/// its `key`, in the order of the keys.
pub fn track_record(
    closed: &[&Record],
    by: Option<&GroupBy>,
) -> Result<(Value, Vec<Value>), String> {
    let mut total = Tally::default();
    let mut groups: Vec<(Value, Tally)> = Vec::new();
    for record in closed {
        total.add(record);
        let Some(by) = by else { continue };
        let key = record.group_key(by);
        let at = match groups.iter().position(|(k, _)| *k == key) {
            Some(at) => at,
            None => {
                if groups.len() == MAX_RECORD_GROUPS {
                    return Err(format!(
                        "this grouping gives more than {MAX_RECORD_GROUPS} groups; group by something that takes fewer values"
                    ));
                }
                groups.push((key, Tally::default()));
                groups.len() - 1
            }
        };
        groups[at].1.add(record);
    }
    groups.sort_by(|a, b| key_order(&a.0, &b.0));
    let groups = groups
        .into_iter()
        .map(|(key, tally)| {
            let mut group = json!({ "key": key });
            if let (Some(group), Value::Object(counts)) = (group.as_object_mut(), tally.json()) {
                group.extend(counts);
            }
            group
        })
        .collect();
    Ok((total.json(), groups))
}

/// The fixed exit variants over `closed` (ADR-0066), per variant: how many of their
/// positions closed, the total and mean of their results, and how many are still open,
/// which a closed setup's can be for a while. Only filled setups have variants, so these
/// are over fewer setups than `total` is.
///
/// The open ones are not a random part: v1's are the runners after T1, which is to say
/// winners, so its closed set leans to losers until they end. Each variant therefore
/// also gives the canonical result over the very setups it has a result for, and only
/// that pair compares like with like.
pub fn shadow_record(closed: &[&Record]) -> Value {
    #[derive(Default)]
    struct Variant {
        closed: u32,
        with_result: u32,
        total_r: f64,
        open: u32,
        /// The canonical results of the setups in `with_result` that have one.
        canonical_with_result: u32,
        canonical_total_r: f64,
    }
    let mut v: [Variant; 3] = Default::default();
    for (record, shadows) in closed
        .iter()
        .filter_map(|r| Some((r, r.shadow_results.as_ref()?)))
    {
        let each = [
            &shadows.v1_half_t1_be,
            &shadows.v2_be_at_1r,
            &shadows.v3_time_stop,
        ];
        for (tally, shadow) in v.iter_mut().zip(each) {
            if shadow.is_open() {
                tally.open += 1;
                continue;
            }
            tally.closed += 1;
            if let Some(r) = shadow.result_r {
                tally.with_result += 1;
                tally.total_r += r;
                if let Some(canonical) = record.result_r {
                    tally.canonical_with_result += 1;
                    tally.canonical_total_r += canonical;
                }
            }
        }
    }
    let per = |sum: f64, n: u32| (n > 0).then(|| sum / f64::from(n));
    let view = |t: &Variant| {
        json!({
            "closed": t.closed,
            "with_result": t.with_result,
            "total_r": t.total_r,
            "mean_r": per(t.total_r, t.with_result),
            "canonical_total_r": t.canonical_total_r,
            "canonical_mean_r": per(t.canonical_total_r, t.canonical_with_result),
            "open": t.open,
        })
    };
    json!({
        "v1_half_t1_be": view(&v[0]),
        "v2_be_at_1r": view(&v[1]),
        "v3_time_stop": view(&v[2]),
    })
}

#[cfg(test)]
mod tests;
