//! A setup: a thesis with levels that the server scores on paper (ADR-0059).
//!
//! This is the part with no I/O: the record, its checks, how one closed 1m bar moves
//! it, and its result in R. The live feed and the replay both call `apply_bar`, so
//! they cannot disagree.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::data::Bar;
use crate::drawings;

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

const MINUTE_MS: i64 = 60_000;

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
const SERVER_FIELDS: [&str; 9] = [
    "status",
    "result_r",
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
        if SERVER_FIELDS.contains(&key.as_str()) {
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
    let floor = Costs::default().least_risk(last.close);
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
        expires_at: now_ms + (SKIP_EXPIRY_BARS * interval_secs * 1000).min(MAX_EXPIRY_MS),
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
}

pub const CREATE_FIELDS: [&str; 11] = [
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
    "as_of",
];

impl Draft {
    /// Read a `create_setup` call. Every problem goes to `problems`; a draft comes back
    /// only when there was none.
    pub fn parse(args: &Value, now_ms: i64, problems: &mut Vec<Problem>) -> Option<Draft> {
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
            _ => parse_levels(args, None, direction, &Costs::default(), now_ms, problems),
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
        };
        let created = record.log(now_ms, By::Agent, Kind::Created, None, Map::new());
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

    /// The agent revises its setup. The thesis and drawings change now. New levels
    /// count from the first bar that opens at or after `now_ms`, and only while the
    /// setup is not filled.
    pub fn revise(
        &mut self,
        thesis: Option<String>,
        drawings: Option<Vec<Value>>,
        levels: Option<Levels>,
        reason: String,
        now_ms: i64,
    ) -> Result<Revision, Vec<Problem>> {
        let mut problems = Vec::new();
        if !self.status.is_open() {
            problems.push(Problem::new(
                "id",
                "this setup is closed and cannot be revised",
            ));
        } else if self.cancel_pending() {
            problems.push(Problem::new("id", "this setup is being cancelled"));
        }
        if let Some(levels) = &levels {
            if self.decision == Decision::Skip {
                problems.push(
                    Problem::new("decision", "a skip keeps the server's default bracket")
                        .hint("only its thesis and drawings can be revised"),
                );
            } else if self.status == Status::Triggered {
                problems.push(
                    Problem::new("status", "this setup is filled, so its levels are fixed")
                        .hint("only its thesis and drawings can be revised now"),
                );
            } else {
                check_levels(self.direction, levels, &self.costs, now_ms, &mut problems);
            }
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

    /// R of a filled setup closed at `exit`: the move in units of the risk taken at the
    /// fill, less the assumed fee and slippage on both sides.
    fn score(&self, fill: f64, exit: f64) -> f64 {
        let risk = (fill - self.levels.invalidation).abs();
        if risk <= 0.0 {
            return 0.0;
        }
        let gross = self.direction.sign() * (exit - fill) / risk;
        let cost = (fill + exit) * self.costs.round_trip() / 2.0 / risk;
        ((gross - cost) * 10_000.0).round() / 10_000.0
    }

    fn close(&mut self, status: Status, exit: Option<f64>, bar: &MinuteBar) -> Revision {
        let from = self.status;
        self.status = status;
        let mut changes = Map::new();
        changed(&mut changes, "status", json!(from), json!(status));
        self.result_r = Some(match (self.fill, exit) {
            (Some(fill), Some(exit)) => {
                self.exit = Some(Mark {
                    price: exit,
                    bar_time: bar.time,
                });
                changes.insert("exit".into(), json!({ "from": null, "to": self.exit }));
                self.score(fill.price, exit)
            }
            _ => 0.0,
        });
        changes.insert(
            "result_r".into(),
            json!({ "from": null, "to": self.result_r }),
        );
        let dropped = std::mem::take(&mut self.pending);
        let closed = self.log(bar.time, By::Server, Kind::Status, None, changes);
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

    /// Apply one closed 1m bar. Returns the log entries it caused. A bar that is not
    /// newer than the last one applied, or that opened before the setup was created,
    /// changes nothing: the feed replays bars after a reconnect.
    pub fn apply_bar(&mut self, bar: &MinuteBar) -> Vec<Revision> {
        let mut out = Vec::new();
        if !self.status.is_open()
            || bar.time < self.created_at
            || self.scored_through.is_some_and(|t| bar.time <= t)
        {
            return out;
        }
        self.scored_through = Some(bar.time);

        // What the agent sent before this bar opened.
        let due: Vec<Pending> = {
            let (due, later) = std::mem::take(&mut self.pending)
                .into_iter()
                .partition(|p| p.at() <= bar.time);
            self.pending = later;
            due
        };
        for change in due {
            match change {
                Pending::Levels { levels, .. } if self.status == Status::Watching => {
                    self.levels = levels;
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
                    out.push(self.close(Status::Cancelled, exit, bar));
                    return out;
                }
            }
        }

        if bar.time >= self.levels.expires_at {
            let exit = self.fill.map(|_| bar.open);
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
            // say in which order it traded.
            if stop_touched {
                out.push(self.close(Status::Invalidated, Some(stop), bar));
            }
            return out;
        }

        if stop_touched {
            // A gap through the level closes at the open, which is worse.
            let exit = if stopped_at(bar.open) { bar.open } else { stop };
            out.push(self.close(Status::Invalidated, Some(exit), bar));
        } else if let Some(target) = self.levels.targets.first().copied() {
            let reached = match self.direction {
                Direction::Long => bar.high >= target,
                Direction::Short => bar.low <= target,
            };
            if reached {
                out.push(self.close(Status::Won, Some(target), bar));
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
        self.gap = true;
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
    /// was not scored is in the log and `gap` is set.
    pub fn expire_unscored(&mut self, now_ms: i64) -> Vec<Revision> {
        if !self.status.is_open() || now_ms < self.levels.expires_at {
            return Vec::new();
        }
        let gap = self.note_gap(self.next_bar_time(), now_ms, now_ms);
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

    /// As a tool answers it: everything but the owner, the levels as fields of the setup.
    pub fn view(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        if let Some(map) = v.as_object_mut() {
            map.remove("owner");
            if let Some(Value::Object(levels)) = map.remove("levels") {
                map.extend(levels);
            }
        }
        v
    }
}

#[cfg(test)]
mod tests;
