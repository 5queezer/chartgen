//! `chartgen backtest`: a rule tested over replay fixtures, with no model in the loop
//! (ADR-0083).
//!
//! A rule's entry is the condition of a server alert, its `and` clauses read the same
//! way (ADR-0067, ADR-0084): at every close of the rule's interval, over the last 999
//! closed bars, as the alert reads them. Where every condition holds, the rule enters at
//! the open of the next 1m bar with a bracket set from that price, and the trade is
//! scored by `Record::apply_bar` over the fixture's 1m bars, as a setup is, with its
//! costs and exit variants. Nothing is written to a store.

use std::path::PathBuf;

use chartgen::data::Bar;
use chartgen::interval::{Interval, Unit};
use chartgen::setups::{
    self, Costs, Decision, Direction, Draft, GroupBy, Levels, MinuteBar, Record, Zone,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::replay::Replay;
use crate::server_alerts::clauses::{self, Clause, MAX_CLAUSES};
use crate::server_alerts::indicator::WINDOW_BARS;

const MINUTE_MS: i64 = 60_000;

#[derive(clap::Args)]
pub struct Args {
    /// The rule: a JSON file (see the strategy tester guide)
    #[arg(long)]
    rule: PathBuf,
    /// Directory of replay fixtures, as `chartgen replay-fixtures` writes them
    #[arg(long)]
    fixtures: PathBuf,
    /// Also write every trade to this file: CSV if it ends in .csv, else JSON
    #[arg(long)]
    trades: Option<PathBuf>,
}

/// A rule as its file gives it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    name: String,
    symbols: Vec<String>,
    interval: String,
    direction: Direction,
    /// Conditions joined by AND, each as a server alert's `and` clause takes it.
    when: Vec<Value>,
    stop: Stop,
    /// Targets in R of the risk at the fill, nearest first.
    targets_r: Vec<f64>,
    /// How long a trade may run from its signal, as an interval: `24h`, `3d`.
    expiry: String,
    /// The live costs (`Costs::default`) unless given.
    #[serde(default)]
    costs: Option<Costs>,
}

/// The stop's distance from the fill.
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Stop {
    /// Times the ATR(14) of the rule's interval at the signal, as a skip's bracket.
    Atr(f64),
    /// Percent of the fill price.
    Pct(f64),
}

struct Rule {
    file: RuleFile,
    when: Vec<Clause>,
    interval: Interval,
    expiry_ms: i64,
    costs: Costs,
}

fn parse_rule(text: &str) -> Result<Rule, String> {
    let file: RuleFile = serde_json::from_str(text).map_err(|e| format!("rule: {e}"))?;
    if file.symbols.is_empty() {
        return Err("symbols: name at least one".into());
    }
    if let Some(s) = file
        .symbols
        .iter()
        .find(|s| crate::fetch::market_of(s).is_some())
    {
        return Err(format!(
            "symbols: {s} is a stock; the tester scores Binance fixtures only"
        ));
    }
    let interval = Interval::parse(&file.interval).map_err(|e| format!("interval: {e}"))?;
    if interval.unit == Unit::Second {
        return Err("interval: fixtures hold 1m bars, so a rule's interval is 1m or longer".into());
    }
    if file.when.is_empty() || file.when.len() > MAX_CLAUSES + 1 {
        return Err(format!(
            "when: from 1 to {} conditions, as an alert's own and its and",
            MAX_CLAUSES + 1
        ));
    }
    let when = file
        .when
        .iter()
        .enumerate()
        .map(|(i, c)| clauses::clause(c).map_err(|e| format!("when[{i}]: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    match file.stop {
        Stop::Atr(k) if k.is_finite() && k > 0.0 => {}
        Stop::Pct(p) if p.is_finite() && p > 0.0 && p < 100.0 => {}
        _ => return Err("stop: atr above 0, or pct above 0 and below 100".into()),
    }
    let rising = file.targets_r.windows(2).all(|w| w[1] > w[0]);
    if file.targets_r.is_empty()
        || file.targets_r.len() > setups::MAX_TARGETS
        || !rising
        || file.targets_r.iter().any(|r| !r.is_finite() || *r <= 0.0)
    {
        return Err(format!(
            "targets_r: from 1 to {} R multiples above 0, nearest first",
            setups::MAX_TARGETS
        ));
    }
    let expiry_ms = Interval::parse(&file.expiry)
        .map(|iv| iv.secs() * 1000)
        .map_err(|e| format!("expiry: {e}"))?;
    if expiry_ms > setups::MAX_EXPIRY_MS {
        return Err("expiry: at most 90 days, as a setup's".into());
    }
    let costs = file.costs.unwrap_or_default();
    if [costs.fee_bps_per_side, costs.slippage_bps_per_side]
        .iter()
        .any(|c| !c.is_finite() || *c < 0.0)
    {
        return Err("costs: bps of 0 or more".into());
    }
    Ok(Rule {
        file,
        when,
        interval,
        expiry_ms,
        costs,
    })
}

/// What a run counted besides its trades.
#[derive(Default)]
struct Run {
    records: Vec<Record>,
    /// Closes of the rule's interval read.
    closes: u32,
    /// Closes where a condition, or the ATR of an `atr` stop, had no value yet.
    before_warmup: u32,
    /// Closes where every condition held.
    signals: u32,
    /// Signals while the rule's trade on that symbol was still open.
    skipped_open: u32,
    /// Signals whose levels `create_setup` would refuse: a stop closer than the risk floor.
    refused: u32,
    /// Signals at the last close, with no 1m bar after them to fill in.
    no_fill: u32,
}

fn start_ms(bar: &Bar) -> i64 {
    bar.date.parse::<i64>().unwrap_or(0) * 1000
}

/// `rule` over `symbol`'s fixture, into `run`.
fn run_symbol(rule: &Rule, replay: &Replay, symbol: &str, run: &mut Run) -> Result<(), String> {
    let minutes = replay.minute_bars(symbol, i64::MIN, i64::MAX);
    let Some(last) = minutes.last() else {
        return Err(format!("the fixtures have no bars for {symbol}"));
    };
    // Every bar of the interval the fixture closes, built as the replay builds them.
    let end = start_ms(last) + MINUTE_MS;
    let (bars, _) = replay.bars(symbol, &rule.file.interval, usize::MAX, end, None)?;
    let sign = match rule.file.direction {
        Direction::Long => 1.0,
        Direction::Short => -1.0,
    };
    // One trade per rule and symbol: the close of the 1m bar its trade ended in.
    let mut busy_until = i64::MIN;
    for i in 0..bars.len() {
        run.closes += 1;
        // What an alert reads at this close: the closed bars up to it, at most 999.
        let window = &bars[(i + 1).saturating_sub(WINDOW_BARS - 1)..=i];
        let close_ms = rule.interval.grid_end(start_ms(&bars[i]) / 1000) * 1000;
        let read: Vec<(bool, Option<f64>)> = rule.when.iter().map(|c| c.holds(window)).collect();
        let atr = match rule.file.stop {
            Stop::Atr(k) => {
                chartgen::orderflow::atr(window, setups::SKIP_ATR_LENGTH).map(|a| k * a)
            }
            Stop::Pct(_) => Some(0.0),
        };
        if atr.is_none() || read.iter().any(|(_, v)| v.is_none()) {
            run.before_warmup += 1;
            continue;
        }
        if !read.iter().all(|(holds, _)| *holds) {
            continue;
        }
        run.signals += 1;
        if close_ms < busy_until {
            run.skipped_open += 1;
            continue;
        }
        // In at the open of the first 1m bar at or after the close: never inside the
        // signal bar.
        let after = replay.minute_bars(symbol, close_ms, i64::MAX);
        let Some(first) = after.first() else {
            run.no_fill += 1;
            continue;
        };
        let fill = first.open;
        let risk = match rule.file.stop {
            Stop::Atr(_) => atr.unwrap_or_default(),
            Stop::Pct(p) => fill * p / 100.0,
        };
        let levels = Levels {
            entry: Zone {
                low: fill,
                high: fill,
            },
            invalidation: fill - sign * risk,
            targets: rule
                .file
                .targets_r
                .iter()
                .map(|r| fill + sign * risk * r)
                .collect(),
            expires_at: close_ms + rule.expiry_ms,
        };
        let mut problems = Vec::new();
        setups::check_levels(
            rule.file.direction,
            &levels,
            &rule.costs,
            close_ms,
            &mut problems,
        );
        if !problems.is_empty() || levels.invalidation <= 0.0 {
            run.refused += 1;
            continue;
        }
        let draft = Draft {
            symbol: symbol.to_string(),
            interval: rule.file.interval.clone(),
            direction: rule.file.direction,
            // A take whose zone is the fill price: it fills at once, at that open.
            decision: Decision::Take,
            thesis: format!("backtest of rule {}", rule.file.name),
            levels: None,
            tags: vec!["backtest".into(), format!("rule:{}", rule.file.name)],
            features: None,
            probability: None,
            playbook_version: None,
            skip_reason: None,
        };
        let id = format!("bt-{symbol}-{}", run.records.len() + 1);
        let (mut record, _) = Record::new(
            id,
            "backtest",
            symbol.to_string(),
            draft,
            levels,
            Vec::new(),
            close_ms,
        );
        record.costs = rule.costs;
        busy_until = i64::MAX;
        for bar in after.iter().filter_map(MinuteBar::of) {
            record.apply_bar(&bar);
            if busy_until == i64::MAX && !record.status.is_open() {
                busy_until = bar.time + MINUTE_MS;
            }
            if !record.is_scoring() {
                break;
            }
        }
        run.records.push(record);
    }
    Ok(())
}

/// The run's answer: what it ran, what it counted, and the track record of its closed
/// trades, by the functions `get_track_record` answers with.
fn report(rule: &Rule, rule_text: &str, replay: &Replay) -> Result<(Value, Vec<Record>), String> {
    let mut run = Run::default();
    let mut fixtures = serde_json::Map::new();
    for symbol in &rule.file.symbols {
        run_symbol(rule, replay, symbol, &mut run)?;
        let minutes = replay.minute_bars(symbol, i64::MIN, i64::MAX);
        if let (Some(first), Some(last)) = (minutes.first(), minutes.last()) {
            fixtures.insert(
                symbol.clone(),
                json!({
                    "from": chartgen::drawings::iso(start_ms(first)),
                    "to": chartgen::drawings::iso(start_ms(last) + MINUTE_MS),
                    "minutes": minutes.len(),
                }),
            );
        }
    }
    let closed: Vec<&Record> = run.records.iter().filter(|r| !r.status.is_open()).collect();
    let (total, by_symbol) = setups::track_record(&closed, Some(&GroupBy::Symbol))?;
    let n = closed.len();
    let won = total["won"].as_u64().unwrap_or(0);
    let given: Value = serde_json::from_str(rule_text).unwrap_or_default();
    let answer = json!({
        "backtest": true,
        "rule": given,
        "rule_sha256": hex::encode(Sha256::digest(rule_text.as_bytes())),
        "fixtures": fixtures,
        "costs": rule.costs,
        "closes": run.closes,
        "before_warmup": run.before_warmup,
        "signals": run.signals,
        "traded": run.records.len(),
        "skipped_open": run.skipped_open,
        "refused": run.refused,
        "no_fill": run.no_fill,
        "open_at_end": run.records.len() - n,
        "summary": {
            "n": n,
            "won": won,
            "invalidated": total["invalidated"],
            "expired": total["expired"],
            "win_rate": (n > 0).then(|| won as f64 / n as f64),
            "mean_r": total["mean_r"],
            "net_r": total["total_r"],
        },
        "total": total,
        "by_symbol": by_symbol,
        "shadows": setups::shadow_record(&closed),
    });
    Ok((answer, run.records))
}

/// One line per trade: where it came in and out, how it ended, and its R after costs.
fn trades_csv(records: &[Record]) -> String {
    let mut out = String::from(
        "id,symbol,direction,signal_at,fill_at,fill,exit_at,exit,status,net,result_r,mae_r,mfe_r,gap\n",
    );
    let num = |v: Option<f64>| v.map(|v| v.to_string()).unwrap_or_default();
    let at = |t: Option<i64>| t.map(chartgen::drawings::iso).unwrap_or_default();
    for r in records {
        let row = [
            r.id.clone(),
            r.symbol.clone(),
            json!(r.direction).as_str().unwrap_or_default().to_string(),
            chartgen::drawings::iso(r.created_at),
            at(r.fill.map(|m| m.bar_time)),
            num(r.fill.map(|m| m.price)),
            at(r.exit.map(|m| m.bar_time)),
            num(r.exit.map(|m| m.price)),
            json!(r.status).as_str().unwrap_or_default().to_string(),
            r.net().unwrap_or_default().to_string(),
            num(r.result_r),
            num(r.mae_r),
            num(r.mfe_r),
            r.gap.to_string(),
        ];
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

pub fn run(args: &Args) -> Result<(), String> {
    let text =
        std::fs::read_to_string(&args.rule).map_err(|e| format!("{}: {e}", args.rule.display()))?;
    let rule = parse_rule(&text)?;
    let replay = Replay::load(&args.fixtures)?;
    let (answer, records) = report(&rule, &text, &replay)?;
    if let Some(path) = &args.trades {
        let body = match path.extension().is_some_and(|e| e == "csv") {
            true => trades_csv(&records),
            false => {
                serde_json::to_string_pretty(&records.iter().map(Record::view).collect::<Vec<_>>())
                    .map_err(|e| e.to_string())?
            }
        };
        std::fs::write(path, body).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&answer).map_err(|e| e.to_string())?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// On the hour grid, so hour `h` is the bar of 1h opening at `T0 + h * 3600`.
    const T0: i64 = 1_700_006_400;

    /// One flat hour at `price` per entry, 60 minutes each.
    fn hours(prices: &[f64]) -> Vec<Bar> {
        prices
            .iter()
            .enumerate()
            .flat_map(|(h, &p)| {
                (0..60).map(move |m| Bar {
                    open: p,
                    high: p,
                    low: p,
                    close: p,
                    volume: 1.0,
                    date: (T0 + h as i64 * 3600 + m * 60).to_string(),
                })
            })
            .collect()
    }

    fn replay_of(bars: Vec<Bar>) -> Replay {
        Replay::of(BTreeMap::from([("BTCUSDT".to_string(), bars)]))
    }

    /// Long when the close crosses up through 102: stop 2 % under the fill, target 2 R,
    /// out after 3 hours.
    const CROSS_RULE: &str = r#"{
        "name": "made-up-cross", "symbols": ["BTCUSDT"], "interval": "1h", "direction": "long",
        "when": [{ "condition": "crossing_up", "level": 102 }],
        "stop": { "pct": 2 }, "targets_r": [2], "expiry": "3h"
    }"#;

    fn at(hour: i64, minute: i64) -> i64 {
        (T0 + hour * 3600 + minute * 60) * 1000
    }

    /// Every count and result of this run is worked out by hand in the comments.
    #[test]
    fn a_run_whose_outcome_is_known_by_hand() {
        //            h0     h1     h2     h3     h4     h5     h6     h7     h8     h9     h10
        let mut bars = hours(&[
            100.0, 105.0, 105.0, 100.0, 103.0, 103.0, 101.0, 104.0, 104.0, 100.0, 105.0,
        ]);
        // h2, minute 10, trades up to 109.5: through the first trade's target.
        bars[2 * 60 + 10].high = 109.5;
        // Half of h11: the fixture ends inside it, so the bar is never whole.
        bars.extend(hours(&[105.0]).into_iter().take(30).map(|b| Bar {
            date: (b.date.parse::<i64>().unwrap() + 11 * 3600).to_string(),
            ..b
        }));
        let rule = parse_rule(CROSS_RULE).unwrap();
        let (answer, records) = report(&rule, CROSS_RULE, &replay_of(bars)).unwrap();

        // Crossings up through 102 at the closes of h1, h4, h7 and h10.
        assert_eq!(answer["closes"], 11);
        assert_eq!(answer["signals"], 4);
        assert_eq!(answer["before_warmup"], 0);
        // h1: in at 105 at the open of h2, stop 102.9, target 105 + 2 * 2.1 = 109.2,
        // reached in h2's minute 10.
        let won = &records[0];
        assert_eq!(won.created_at, at(2, 0));
        assert_eq!(won.fill.unwrap().price, 105.0);
        assert_eq!(won.status, setups::Status::Won);
        assert_eq!(won.exit.unwrap().bar_time, at(2, 10));
        assert!((won.exit.unwrap().price - 109.2).abs() < 1e-9);
        // (4.2 - (105 + 109.2) * 7 bps) / 2.1 R, after costs.
        assert_eq!(won.result_r, Some(1.9286));
        // h4: in at 103 at the open of h5, stop 100.94; h6 at 101 does not reach it.
        // Out at the open of the 1m bar at the expiry, h8's first, at 104:
        // (1 - (103 + 104) * 7 bps) / 2.06 R.
        let expired = &records[1];
        assert_eq!(expired.status, setups::Status::Expired);
        assert_eq!(expired.exit.unwrap().bar_time, at(8, 0));
        assert_eq!(expired.result_r, Some(0.4151));
        // h7's signal comes while that trade is in its last minute: skipped.
        assert_eq!(answer["skipped_open"], 1);
        // h10: in at the open of h11, still open when the fixture ends.
        assert_eq!(records[2].status, setups::Status::Triggered);
        assert_eq!(answer["traded"], 3);
        assert_eq!(answer["open_at_end"], 1);
        assert_eq!(answer["refused"], 0);
        let summary = &answer["summary"];
        assert_eq!(summary["n"], 2);
        assert_eq!(summary["won"], 1);
        assert_eq!(summary["expired"], 1);
        assert_eq!(summary["win_rate"], 0.5);
        assert!((summary["net_r"].as_f64().unwrap() - (1.9286 + 0.4151)).abs() < 1e-9);
        assert_eq!(answer["by_symbol"][0]["key"], "BTCUSDT");
        assert_eq!(answer["total"]["net_gain"], 2);
        assert_eq!(answer["shadows"]["v2_be_at_1r"]["closed"], 2);
        // The same rule on the same fixtures answers the same, byte for byte.
        let mut again = hours(&[
            100.0, 105.0, 105.0, 100.0, 103.0, 103.0, 101.0, 104.0, 104.0, 100.0, 105.0,
        ]);
        again[2 * 60 + 10].high = 109.5;
        again.extend(hours(&[105.0]).into_iter().take(30).map(|b| Bar {
            date: (b.date.parse::<i64>().unwrap() + 11 * 3600).to_string(),
            ..b
        }));
        let (twice, _) = report(&rule, CROSS_RULE, &replay_of(again)).unwrap();
        assert_eq!(answer.to_string(), twice.to_string());
        let csv = trades_csv(&records);
        assert_eq!(csv.lines().count(), 4);
        assert!(csv.lines().nth(1).unwrap().contains(",won,gain,1.9286,"));
    }

    #[test]
    fn a_stop_under_the_risk_floor_is_refused_and_counted() {
        // 0.3 % is under 3 round trips of 14 bps (0.42 %).
        let text = CROSS_RULE.replace(r#""pct": 2"#, r#""pct": 0.3"#);
        let rule = parse_rule(&text).unwrap();
        let (answer, records) =
            report(&rule, &text, &replay_of(hours(&[100.0, 105.0, 105.0]))).unwrap();
        assert_eq!(
            (answer["signals"].as_u64(), answer["refused"].as_u64()),
            (Some(1), Some(1))
        );
        assert!(records.is_empty());
    }

    /// RSI over the bars an alert reads: a cut of the fixture after a signal gives the
    /// same signals up to it, and no fill is inside its signal bar.
    #[test]
    fn signals_do_not_look_ahead() {
        let wobble: Vec<f64> = (0..400)
            .map(|h| 100.0 + (h as f64 * 0.31).sin() * 4.0 + (h as f64 * 0.07).sin() * 6.0)
            .collect();
        let text = r#"{
            "name": "made-up-rsi", "symbols": ["BTCUSDT"], "interval": "1h", "direction": "short",
            "when": [{ "indicator": "rsi", "params": { "length": 14 }, "condition": "crossing_up", "level": 60 }],
            "stop": { "atr": 1 }, "targets_r": [1.5], "expiry": "6h"
        }"#;
        let rule = parse_rule(text).unwrap();
        let (full, records) = report(&rule, text, &replay_of(hours(&wobble))).unwrap();
        assert!(records.len() >= 3, "{full}");
        assert!(full["before_warmup"].as_u64().unwrap() > 0);
        for r in &records {
            assert!(r.fill.unwrap().bar_time >= r.created_at);
        }
        let cut_ms = records[2].created_at;
        let cut_hours = ((cut_ms / 1000 - T0) / 3600) as usize;
        let (_, early) = report(&rule, text, &replay_of(hours(&wobble[..cut_hours + 1]))).unwrap();
        let signals = |rs: &[Record]| -> Vec<i64> {
            rs.iter()
                .map(|r| r.created_at)
                .filter(|t| *t <= cut_ms)
                .collect()
        };
        assert_eq!(signals(&records), signals(&early));
    }

    #[test]
    fn what_a_rule_refuses() {
        let err = |from: &str, to: &str| parse_rule(&CROSS_RULE.replace(from, to)).err().unwrap();
        assert!(err(r#""BTCUSDT""#, r#""NVDA""#).contains("is a stock"));
        assert!(err(r#""level": 102"#, r#""level": 102, "x": 1"#).starts_with("when[0]:"));
        assert!(err(r#""pct": 2"#, r#""pct": 0"#).starts_with("stop:"));
        assert!(err("[2]", "[2, 1.5]").starts_with("targets_r:"));
        assert!(err(r#""3h""#, r#""100d""#).starts_with("expiry:"));
        assert!(err(r#""interval": "1h""#, r#""interval": "5s""#).starts_with("interval:"));
        assert!(err(r#""expiry""#, r#""expires""#).starts_with("rule:"));
    }

    #[test]
    fn the_example_rule_parses() {
        let text = include_str!("../examples/rules/rsi-overbought-short.json");
        let rule = parse_rule(text).unwrap();
        assert_eq!(rule.when.len(), 1);
        assert_eq!(rule.expiry_ms, 24 * 3_600_000);
    }
}
