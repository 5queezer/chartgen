use super::*;

const T0: i64 = 1_700_000_040_000;
const MIN: i64 = 60_000;

fn bar(minute: i64, open: f64, high: f64, low: f64, close: f64) -> MinuteBar {
    MinuteBar {
        time: T0 + minute * MIN,
        open,
        high,
        low,
        close,
    }
}

fn long_args() -> Value {
    json!({
        "symbol": "BTCUSDT", "interval": "1h", "direction": "long",
        "thesis": "example thesis",
        "entry": { "low": 100.0, "high": 101.0 },
        "invalidation": 98.0, "targets": [104.0, 106.0],
        "expires_at": T0 + 86_400_000,
    })
}

fn short_args() -> Value {
    json!({
        "symbol": "BTCUSDT", "interval": "1h", "direction": "short",
        "thesis": "example thesis",
        "entry": { "low": 100.0, "high": 101.0 },
        "invalidation": 103.0, "targets": [97.0],
        "expires_at": T0 + 86_400_000,
    })
}

fn create(args: Value) -> Record {
    let mut problems = Vec::new();
    let draft = Draft::parse(&args, T0, &mut problems);
    assert_eq!(problems, vec![], "the example must pass");
    let draft = draft.unwrap();
    let levels = draft.levels.clone().unwrap();
    Record::new(
        "setup-1".into(),
        "owner",
        "BTCUSDT".into(),
        draft,
        levels,
        vec![],
        T0,
    )
    .0
}

fn problems_of(args: Value) -> Vec<Problem> {
    let mut problems = Vec::new();
    assert!(Draft::parse(&args, T0, &mut problems).is_none());
    problems
}

fn fields(problems: &[Problem]) -> Vec<&str> {
    problems.iter().map(|p| p.field.as_str()).collect()
}

fn without(mut args: Value, field: &str) -> Value {
    args.as_object_mut().unwrap().remove(field);
    args
}

fn with(mut args: Value, field: &str, value: Value) -> Value {
    args[field] = value;
    args
}

/// Walk a setup through bars and return its statuses after each.
fn walk(record: &mut Record, bars: &[MinuteBar]) -> Vec<Status> {
    bars.iter()
        .map(|b| {
            record.apply_bar(b);
            record.status
        })
        .collect()
}

#[test]
fn create_refuses_a_setup_without_invalidation_or_expiry() {
    let args = without(without(long_args(), "invalidation"), "expires_at");
    let problems = problems_of(args);
    assert_eq!(fields(&problems), ["invalidation", "expires_at"]);
    assert!(problems.iter().all(|p| p.hint.is_some()));
}

#[test]
fn stop_must_be_on_the_far_side_of_entry() {
    for (args, word) in [
        (with(long_args(), "invalidation", json!(100.5)), "below"),
        (with(short_args(), "invalidation", json!(100.5)), "above"),
    ] {
        let problems = problems_of(args);
        assert_eq!(fields(&problems), ["invalidation"]);
        assert!(problems[0].message.contains(word), "{:?}", problems[0]);
        assert!(problems[0].hint.as_deref().unwrap().contains("100.5"));
    }
}

#[test]
fn every_problem_is_reported_at_once() {
    let mut args = long_args();
    args["direction"] = json!("up");
    args["thesis"] = json!("x".repeat(MAX_THESIS + 1));
    args["entry"] = json!({ "low": -1.0, "high": 101.0 });
    args["targets"] = json!([]);
    args["expires_at"] = json!("soon");
    let problems = problems_of(args);
    assert_eq!(
        fields(&problems),
        ["direction", "thesis", "entry.low", "targets", "expires_at"]
    );
}

#[test]
fn targets_lie_beyond_the_entry_nearest_first() {
    let problems = problems_of(with(long_args(), "targets", json!([100.5, 104.0])));
    assert_eq!(fields(&problems), ["targets[0]"]);
    let problems = problems_of(with(long_args(), "targets", json!([106.0, 104.0])));
    assert_eq!(fields(&problems), ["targets[1]"]);
    let problems = problems_of(with(short_args(), "targets", json!([102.0])));
    assert_eq!(fields(&problems), ["targets[0]"]);
}

#[test]
fn expiry_is_ahead_and_bounded() {
    let problems = problems_of(with(long_args(), "expires_at", json!(T0)));
    assert_eq!(fields(&problems), ["expires_at"]);
    let far = with(long_args(), "expires_at", json!(T0 + MAX_EXPIRY_MS + 1));
    assert_eq!(fields(&problems_of(far)), ["expires_at"]);
}

#[test]
fn risk_below_the_floor_is_refused() {
    // 14 bps round trip, three times that of 100 is 0.42.
    let near = with(long_args(), "invalidation", json!(99.7));
    let problems = problems_of(near);
    assert_eq!(fields(&problems), ["invalidation"]);
    assert!(problems[0].hint.as_deref().unwrap().contains("14 bps"));
    create(with(long_args(), "invalidation", json!(99.5)));
}

#[test]
fn the_agent_cannot_set_status_or_result() {
    let mut args = long_args();
    args["status"] = json!("won");
    args["result_r"] = json!(5.0);
    args["mood"] = json!("good");
    let problems = problems_of(args);
    assert_eq!(fields(&problems), ["mood", "result_r", "status"]);
    assert!(problems[1].message.contains("set by the server"));
}

#[test]
fn a_skip_takes_no_levels() {
    let mut args = long_args();
    args["decision"] = json!("skip");
    let problems = problems_of(args);
    assert_eq!(
        fields(&problems),
        ["entry", "invalidation", "targets", "expires_at"]
    );
}

#[test]
fn status_walk_watching_triggered_won() {
    let mut r = create(long_args());
    let statuses = walk(
        &mut r,
        &[
            bar(0, 103.0, 103.5, 102.0, 102.5),
            bar(1, 102.5, 102.6, 100.8, 101.2),
            bar(2, 101.2, 103.0, 101.0, 102.9),
            bar(3, 102.9, 104.2, 102.8, 104.0),
        ],
    );
    use Status::*;
    assert_eq!(statuses, [Watching, Triggered, Triggered, Won]);
    // Came down into the zone: filled at its upper edge.
    assert_eq!(r.fill.unwrap().price, 101.0);
    assert_eq!(r.exit.unwrap().price, 104.0);
    assert_eq!(r.exit.unwrap().bar_time, T0 + 3 * MIN);
}

#[test]
fn status_walk_watching_triggered_invalidated() {
    let mut r = create(short_args());
    let statuses = walk(
        &mut r,
        &[
            bar(0, 99.0, 100.4, 98.9, 100.2),
            bar(1, 100.2, 102.0, 100.1, 101.9),
            bar(2, 101.9, 103.1, 101.5, 102.0),
        ],
    );
    use Status::*;
    assert_eq!(statuses, [Triggered, Triggered, Invalidated]);
    // Came up into the zone: filled at its lower edge.
    assert_eq!(r.fill.unwrap().price, 100.0);
    assert_eq!(r.exit.unwrap().price, 103.0);
    assert!(r.result_r.unwrap() < -1.0);
}

#[test]
fn status_walk_expired() {
    let mut never = create(long_args());
    let day = 1440;
    walk(&mut never, &[bar(day, 103.0, 103.5, 102.0, 102.5)]);
    assert_eq!(never.status, Status::Expired);
    assert_eq!(never.result_r, Some(0.0));
    assert_eq!(never.exit, None);

    let mut filled = create(long_args());
    walk(
        &mut filled,
        &[
            bar(0, 100.5, 100.9, 100.2, 100.6),
            bar(day, 102.0, 105.0, 101.0, 103.0),
        ],
    );
    assert_eq!(filled.status, Status::Expired);
    // Out at the open of the first bar past the expiry, whatever that bar did next.
    assert_eq!(filled.exit.unwrap().price, 102.0);
    assert!(filled.result_r.unwrap() > 0.0);
}

#[test]
fn a_bar_touching_stop_and_target_is_a_stop() {
    let mut r = create(long_args());
    walk(
        &mut r,
        &[
            bar(0, 100.5, 100.9, 100.2, 100.6),
            bar(1, 100.6, 104.5, 97.9, 104.0),
        ],
    );
    assert_eq!(r.status, Status::Invalidated);
    assert_eq!(r.exit.unwrap().price, 98.0);
}

#[test]
fn the_fill_bar_can_stop_but_cannot_win() {
    let mut won = create(long_args());
    walk(&mut won, &[bar(0, 100.5, 104.5, 100.2, 104.2)]);
    assert_eq!(won.status, Status::Triggered);

    let mut stopped = create(long_args());
    let log = stopped.apply_bar(&bar(0, 102.0, 102.1, 97.5, 97.8));
    assert_eq!(stopped.status, Status::Invalidated);
    assert_eq!(stopped.fill.unwrap().price, 101.0);
    assert_eq!(stopped.exit.unwrap().price, 98.0);
    assert_eq!(log.len(), 2, "the fill and the stop are both in the log");
}

#[test]
fn invalidation_before_entry_scores_zero() {
    // Opens beyond the level: the entry was never there to take.
    let mut gapped = create(long_args());
    walk(&mut gapped, &[bar(0, 97.0, 100.5, 96.5, 100.2)]);
    assert_eq!(gapped.status, Status::Invalidated);
    assert_eq!(gapped.fill, None);
    assert_eq!(gapped.result_r, Some(0.0));

    // A short whose price falls away without reaching the zone is still watching.
    let mut away = create(short_args());
    walk(&mut away, &[bar(0, 99.0, 99.5, 96.0, 96.5)]);
    assert_eq!(away.status, Status::Watching);
}

#[test]
fn a_stop_gapped_through_closes_at_the_open() {
    let mut r = create(long_args());
    walk(
        &mut r,
        &[
            bar(0, 100.5, 100.9, 100.2, 100.6),
            bar(1, 96.0, 96.5, 95.0, 96.2),
        ],
    );
    assert_eq!(r.exit.unwrap().price, 96.0);
}

#[test]
fn the_creation_bar_is_not_scored() {
    let mut r = create(long_args());
    r.created_at = T0 + 30_000;
    assert_eq!(r.next_bar_time(), T0 + MIN);
    assert!(r.apply_bar(&bar(0, 100.5, 100.9, 97.0, 97.5)).is_empty());
    assert_eq!(r.status, Status::Watching);
    assert_eq!(r.scored_through, None);
}

#[test]
fn a_replayed_or_late_bar_is_not_scored_twice() {
    let mut r = create(long_args());
    r.apply_bar(&bar(5, 103.0, 103.5, 102.0, 102.5));
    let before = r.clone();
    assert!(r.apply_bar(&bar(5, 100.5, 100.9, 97.0, 97.5)).is_empty());
    assert!(r.apply_bar(&bar(4, 100.5, 100.9, 97.0, 97.5)).is_empty());
    assert_eq!(r, before);
    assert_eq!(r.next_bar_time(), T0 + 6 * MIN);
}

#[test]
fn r_includes_fee_and_slippage() {
    // Long: in at 101, out at 104, risk 3. Gross 1. Cost (101 + 104) × 7 bps / 3.
    let mut long = create(long_args());
    walk(
        &mut long,
        &[
            bar(0, 102.0, 102.1, 100.8, 101.2),
            bar(1, 101.2, 104.2, 101.0, 104.0),
        ],
    );
    let cost: f64 = 205.0 * 0.0007 / 3.0;
    let expected = ((1.0 - cost) * 10_000.0).round() / 10_000.0;
    assert_eq!(long.result_r, Some(expected));
    assert_eq!(long.result_r, Some(0.9522));

    // Short: in at 100, stopped at 103, risk 3. Gross -1. Cost 203 × 7 bps / 3.
    let mut short = create(short_args());
    walk(
        &mut short,
        &[
            bar(0, 99.0, 100.4, 98.9, 100.2),
            bar(1, 100.2, 103.5, 100.1, 103.2),
        ],
    );
    assert_eq!(short.result_r, Some(-1.0474));
    assert_eq!(long.costs, Costs::default());
}

fn hourly(n: usize) -> Vec<Bar> {
    (0..n)
        .map(|i| Bar {
            open: 100.0,
            high: 101.0,
            low: 99.0,
            close: 100.0,
            volume: 1.0,
            date: (i as i64 * 3600).to_string(),
        })
        .collect()
}

fn skip(direction: &str) -> Record {
    let args = json!({ "symbol": "BTCUSDT", "interval": "1h", "direction": direction,
        "decision": "skip", "thesis": "example reason" });
    let mut problems = Vec::new();
    let draft = Draft::parse(&args, T0, &mut problems).unwrap();
    let (levels, rule) = skip_bracket(draft.direction, &hourly(40), 3600, T0).unwrap();
    assert_eq!(rule, Bracket::Atr);
    Record::new(
        "setup-1".into(),
        "owner",
        "BTCUSDT".into(),
        draft,
        levels,
        vec![],
        T0,
    )
    .0
}

#[test]
fn skip_is_scored_with_the_default_bracket() {
    // Every bar ranges 2, so ATR(14) is 2.
    let mut long = skip("long");
    assert_eq!(long.levels.invalidation, 98.0);
    assert_eq!(long.levels.targets, [104.0]);
    assert_eq!(long.levels.expires_at, T0 + 24 * 3_600_000);
    // In at the next bar's open, wherever that is.
    walk(
        &mut long,
        &[
            bar(0, 100.4, 100.6, 100.1, 100.5),
            bar(1, 100.5, 104.1, 100.4, 104.0),
        ],
    );
    assert_eq!(long.fill.unwrap().price, 100.4);
    assert_eq!(long.status, Status::Won);
    assert!(long.result_r.unwrap() > 1.0);

    let short = skip("short");
    assert_eq!(short.levels.invalidation, 102.0);
    assert_eq!(short.levels.targets, [96.0]);
}

#[test]
fn a_skip_needs_bars_to_be_scored() {
    let problem = skip_bracket(Direction::Long, &hourly(14), 3600, T0).unwrap_err();
    assert_eq!(problem.field, "decision");
}

#[test]
fn a_skip_keeps_its_bracket_and_runs_to_its_end() {
    let mut r = skip("long");
    assert!(r.cancel("changed my mind".into(), T0).is_err());
    let levels = Some(r.levels.clone());
    assert!(r
        .revise(None, None, None, levels, "tighter".into(), T0)
        .is_err());
    assert_eq!(r.revisions, 1);
}

fn revised_levels(invalidation: f64) -> Levels {
    Levels {
        entry: Zone {
            low: 100.0,
            high: 101.0,
        },
        invalidation,
        targets: vec![104.0, 106.0],
        expires_at: T0 + 86_400_000,
    }
}

#[test]
fn a_cancel_inside_a_bar_that_fills_and_stops_is_scored_as_the_stop() {
    let mut r = create(long_args());
    // Sent 20 s into the bar that goes on to fill and stop.
    r.cancel("it looks wrong".into(), T0 + 20_000).unwrap();
    assert_eq!(r.status, Status::Watching);
    let log = r.apply_bar(&bar(0, 102.0, 102.1, 97.5, 97.8));
    assert_eq!(r.status, Status::Invalidated);
    assert!(r.result_r.unwrap() < -1.0);
    assert!(r.pending.is_empty());
    assert!(log
        .last()
        .unwrap()
        .reason
        .as_deref()
        .unwrap()
        .contains("did not take effect"));
}

#[test]
fn a_cancel_while_watching_ends_at_the_next_bar_with_zero() {
    let mut r = create(long_args());
    r.cancel("no longer valid".into(), T0 + 20_000).unwrap();
    walk(
        &mut r,
        &[
            bar(0, 103.0, 103.5, 102.0, 102.5),
            bar(1, 100.5, 100.9, 97.0, 97.5),
        ],
    );
    assert_eq!(r.status, Status::Cancelled);
    assert_eq!(r.fill, None);
    assert_eq!(r.result_r, Some(0.0));
}

#[test]
fn cancel_after_trigger_is_scored_not_erased() {
    let mut r = create(long_args());
    r.apply_bar(&bar(0, 100.5, 100.9, 100.2, 100.6));
    r.cancel("closing early".into(), T0 + MIN + 20_000).unwrap();
    // The forming bar's stop beats the cancel.
    let mut stopped = r.clone();
    stopped.apply_bar(&bar(1, 100.6, 100.7, 97.9, 98.2));
    assert_eq!(stopped.status, Status::Invalidated);
    // Otherwise it closes at the next bar's open, at a loss here.
    walk(
        &mut r,
        &[
            bar(1, 100.6, 100.7, 99.0, 99.2),
            bar(2, 99.2, 104.5, 99.0, 104.0),
        ],
    );
    assert_eq!(r.status, Status::Cancelled);
    assert_eq!(r.exit.unwrap().price, 99.2);
    assert!(r.result_r.unwrap() < 0.0);
}

#[test]
fn a_revise_inside_a_filling_bar_does_not_undo_the_fill() {
    let mut r = create(long_args());
    // Moves the stop out of the way while the bar that fills and stops is forming.
    r.revise(
        None,
        None,
        None,
        Some(revised_levels(95.0)),
        "more room".into(),
        T0 + 20_000,
    )
    .unwrap();
    assert_eq!(r.levels.invalidation, 98.0, "not yet in effect");
    r.apply_bar(&bar(0, 102.0, 102.1, 97.5, 97.8));
    assert_eq!(r.status, Status::Invalidated);
    assert_eq!(r.exit.unwrap().price, 98.0);

    // Filled but not stopped: the new levels arrive too late and are voided.
    let mut filled = create(long_args());
    filled
        .revise(
            None,
            None,
            None,
            Some(revised_levels(95.0)),
            "more room".into(),
            T0 + 20_000,
        )
        .unwrap();
    filled.apply_bar(&bar(0, 100.5, 100.9, 100.2, 100.6));
    let log = filled.apply_bar(&bar(1, 100.6, 100.8, 100.3, 100.5));
    assert_eq!(filled.levels.invalidation, 98.0);
    assert_eq!(log[0].kind, Kind::Voided);
}

#[test]
fn a_revise_counts_from_the_next_bar() {
    let mut r = create(long_args());
    r.revise(
        None,
        None,
        None,
        Some(revised_levels(97.0)),
        "wider".into(),
        T0 + 20_000,
    )
    .unwrap();
    r.apply_bar(&bar(0, 103.0, 103.5, 102.0, 102.5));
    assert_eq!(r.levels.invalidation, 98.0);
    r.apply_bar(&bar(1, 102.5, 102.6, 102.0, 102.2));
    assert_eq!(r.levels.invalidation, 97.0);
    assert!(r.pending.is_empty());
}

#[test]
fn revise_after_trigger_changes_thesis_and_drawings_only() {
    let mut r = create(long_args());
    r.apply_bar(&bar(0, 100.5, 100.9, 100.2, 100.6));
    let refused = r
        .revise(
            None,
            None,
            None,
            Some(revised_levels(95.0)),
            "more room".into(),
            T0 + MIN,
        )
        .unwrap_err();
    assert_eq!(fields(&refused), ["status"]);
    let revision = r
        .revise(
            Some("updated thesis".into()),
            None,
            None,
            None,
            "new information".into(),
            T0 + MIN,
        )
        .unwrap();
    assert_eq!(r.thesis, "updated thesis");
    assert_eq!(
        revision.changes["thesis"],
        json!({ "from": "example thesis", "to": "updated thesis" })
    );
}

#[test]
fn a_revision_must_change_something_and_be_valid() {
    let mut r = create(long_args());
    let same = Some(r.levels.clone());
    assert!(r
        .revise(None, None, None, same, "no change".into(), T0)
        .is_err());
    let bad = r
        .revise(
            None,
            None,
            None,
            Some(revised_levels(100.5)),
            "closer".into(),
            T0,
        )
        .unwrap_err();
    assert_eq!(fields(&bad), ["invalidation"]);
    assert_eq!(r.revisions, 1);
}

#[test]
fn the_log_numbers_every_entry_in_order() {
    let mut r = create(long_args());
    let mut log = vec![];
    log.push(
        r.revise(
            Some("second".into()),
            None,
            None,
            None,
            "why".into(),
            T0 + 1,
        )
        .unwrap(),
    );
    log.extend(r.apply_bar(&bar(0, 100.5, 100.9, 100.2, 100.6)));
    log.push(r.cancel("done".into(), T0 + MIN + 1).unwrap());
    log.extend(r.apply_bar(&bar(2, 100.6, 100.9, 100.2, 100.6)));
    let seqs: Vec<u32> = log.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [2, 3, 4, 5]);
    assert_eq!(r.revisions, 5);
    assert_eq!(r.agent_revisions, 2);
    let kinds: Vec<Kind> = log.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [Kind::Revised, Kind::Status, Kind::Cancelled, Kind::Status]
    );
}

#[test]
fn a_closed_setup_takes_no_change_and_no_bar() {
    let mut r = create(long_args());
    r.apply_bar(&bar(0, 97.0, 97.5, 96.5, 97.2));
    assert_eq!(r.status, Status::Invalidated);
    assert!(r.cancel("late".into(), T0 + MIN).is_err());
    assert!(r
        .revise(
            Some("late".into()),
            None,
            None,
            None,
            "late".into(),
            T0 + MIN
        )
        .is_err());
    assert!(r.apply_bar(&bar(1, 100.5, 104.5, 100.2, 104.0)).is_empty());
}

#[test]
fn a_record_round_trips_and_its_view_hides_the_owner() {
    let mut r = create(long_args());
    r.cancel("stop".into(), T0 + 1).unwrap();
    let text = serde_json::to_string(&r).unwrap();
    assert_eq!(serde_json::from_str::<Record>(&text).unwrap(), r);
    let view = r.view();
    assert!(view.get("owner").is_none());
    assert_eq!(view["entry"], json!({ "low": 100.0, "high": 101.0 }));
    assert_eq!(view["status"], "watching");
}

#[test]
fn a_bar_with_impossible_prices_is_not_a_bar() {
    let good = Bar {
        open: 1.0,
        high: 2.0,
        low: 0.5,
        close: 1.5,
        volume: 0.0,
        date: "60".into(),
    };
    assert_eq!(MinuteBar::of(&good).unwrap().time, 60_000);
    for bad in [
        Bar {
            high: 0.9,
            ..good.clone()
        },
        Bar {
            low: f64::NAN,
            ..good.clone()
        },
        Bar {
            date: "x".into(),
            ..good.clone()
        },
    ] {
        assert!(MinuteBar::of(&bad).is_none());
    }
}

// One principle: a bar that does not say which level it reached first counts the
// worse outcome for the agent. Two exceptions end unfilled, at 0 R.

#[test]
fn a_bar_reaching_zone_and_invalidation_is_a_fill_and_a_stop() {
    let mut r = create(long_args());
    r.apply_bar(&bar(0, 101.5, 101.6, 97.9, 98.5));
    assert_eq!(r.status, Status::Invalidated);
    assert_eq!(r.fill.unwrap().price, 101.0);
    assert!(r.result_r.unwrap() < -1.0);
}

#[test]
fn a_bar_opening_beyond_the_invalidation_ends_unfilled() {
    let mut r = create(short_args());
    r.apply_bar(&bar(0, 103.5, 104.0, 100.5, 100.8));
    assert_eq!(r.status, Status::Invalidated);
    assert_eq!((r.fill, r.result_r), (None, Some(0.0)));
}

#[test]
fn a_bar_reaching_the_invalidation_but_never_the_zone_ends_unfilled() {
    let mut r = create(long_args());
    r.apply_bar(&bar(0, 99.5, 99.8, 97.5, 98.2));
    assert_eq!(r.status, Status::Invalidated);
    assert_eq!((r.fill, r.result_r), (None, Some(0.0)));
}

/// A stored price must read back as the same number, or a setup's levels would move
/// by a hair at every restart. serde_json only promises that with `float_roundtrip`.
#[test]
fn a_stored_price_reads_back_as_the_same_number() {
    for text in [
        "235.83115467366719",
        "230.96865663915858",
        "242.09508302864626",
    ] {
        let price: f64 = serde_json::from_str(text).unwrap();
        assert_eq!(price, text.parse::<f64>().unwrap(), "{text}");
        assert_eq!(serde_json::to_string(&price).unwrap(), text);
    }
}

/// A skip cannot be cancelled, so its expiry is bounded like any other setup's: the
/// earlier of 24 bars and the 90 days.
#[test]
fn a_skip_expires_within_the_longest_expiry_whatever_its_interval() {
    let day = 86_400;
    for (interval_secs, expected_ms) in [
        (3600, 24 * 3_600_000),
        (day, 24 * 86_400_000),
        (7 * day, MAX_EXPIRY_MS),
        (30 * day, MAX_EXPIRY_MS),
        (90 * day, MAX_EXPIRY_MS),
    ] {
        let levels = skip_bracket(Direction::Long, &hourly(40), interval_secs, T0)
            .unwrap()
            .0;
        assert_eq!(
            levels.expires_at - T0,
            expected_ms,
            "{interval_secs} s bars"
        );
    }
}

/// On a quiet interval one ATR is less than the costs of the trade, and a skip scored
/// on it would lose whatever the market did. The stop is then as far as the least risk
/// any setup may have, and the target twice that.
#[test]
fn a_skip_on_a_quiet_interval_is_scored_at_the_risk_floor() {
    let quiet: Vec<Bar> = (0..40)
        .map(|i| Bar {
            open: 100.0,
            high: 100.05,
            low: 99.95,
            close: 100.0,
            volume: 1.0,
            date: (i * 60).to_string(),
        })
        .collect();
    // ATR is 0.1, 10 bps. The floor is 3 round trips of 14 bps: 0.42.
    let (long, rule) = skip_bracket(Direction::Long, &quiet, 60, T0).unwrap();
    assert_eq!(rule, Bracket::Floor);
    assert!(
        (long.invalidation - 99.58).abs() < 1e-9,
        "{}",
        long.invalidation
    );
    assert!(
        (long.targets[0] - 100.84).abs() < 1e-9,
        "{}",
        long.targets[0]
    );
    let (short, rule) = skip_bracket(Direction::Short, &quiet, 60, T0).unwrap();
    assert_eq!(rule, Bracket::Floor);
    assert!((short.invalidation - 100.42).abs() < 1e-9);
    assert!((short.targets[0] - 99.16).abs() < 1e-9);

    // Its target is worth having: about 2 R less a third of an R in costs.
    let args = json!({ "symbol": "BTCUSDT", "interval": "1m", "direction": "long",
        "decision": "skip", "thesis": "example reason" });
    let draft = Draft::parse(&args, T0, &mut Vec::new()).unwrap();
    let mut r = Record::new("s".into(), "o", "BTCUSDT".into(), draft, long, vec![], T0).0;
    walk(
        &mut r,
        &[
            bar(0, 100.0, 100.05, 99.95, 100.0),
            bar(1, 100.0, 100.9, 99.95, 100.85),
        ],
    );
    assert_eq!(r.status, Status::Won);
    assert!(
        (r.result_r.unwrap() - 1.6653).abs() < 1e-9,
        "{:?}",
        r.result_r
    );
}

#[test]
fn a_record_says_which_rule_set_its_bracket() {
    let mut r = create(long_args());
    assert_eq!(r.view()["bracket"], "agent");
    r.bracket = Bracket::Floor;
    assert_eq!(r.view()["bracket"], "floor");
    // A row stored before the field existed is the agent's own levels or an ATR skip;
    // it reads as the agent's, the default.
    let mut old = serde_json::to_value(&r).unwrap();
    old.as_object_mut().unwrap().remove("bracket");
    assert_eq!(
        serde_json::from_value::<Record>(old).unwrap().bracket,
        Bracket::Agent
    );
}

// Every check that can be made from the fields that are present is made, in the same
// answer. A missing field does not hide what is wrong with the others.

#[test]
fn a_missing_field_does_not_hide_a_stop_on_the_wrong_side() {
    let args = with(
        without(long_args(), "expires_at"),
        "invalidation",
        json!(102.0),
    );
    let problems = problems_of(args);
    assert_eq!(fields(&problems), ["expires_at", "invalidation"]);
    assert!(problems[1].message.contains("below its entry zone"));
}

#[test]
fn missing_fields_do_not_hide_a_target_on_the_wrong_side() {
    let args = without(without(long_args(), "invalidation"), "expires_at");
    let problems = problems_of(with(args, "targets", json!([98.0])));
    assert_eq!(
        fields(&problems),
        ["invalidation", "expires_at", "targets[0]"]
    );
}

#[test]
fn problems_of_every_kind_come_in_one_answer() {
    // A short: thesis missing, entry upside down, stop on the wrong side of nothing
    // that can be judged, first target wrong side, second not beyond the first,
    // expiry in the past, and a field the server sets.
    let args = json!({
        "symbol": "BTCUSDT", "interval": "1h", "direction": "short",
        "entry": { "low": 101.0, "high": 100.0 },
        "invalidation": 99.0, "targets": [97.0, 98.0],
        "expires_at": T0 - 1, "result_r": 3,
    });
    let problems = problems_of(args);
    assert_eq!(
        fields(&problems),
        ["result_r", "thesis", "entry", "targets[1]", "expires_at"]
    );
    // With the entry the right way up, what depends on it is judged too.
    let args = json!({
        "symbol": "BTCUSDT", "interval": "1h", "direction": "short",
        "entry": { "low": 100.0, "high": 101.0 },
        "invalidation": 99.0, "targets": [102.0, 98.0, 99.0],
        "expires_at": T0 - 1, "result_r": 3,
    });
    let problems = problems_of(args);
    assert_eq!(
        fields(&problems),
        [
            "result_r",
            "thesis",
            "invalidation",
            "targets[0]",
            "targets[2]",
            "expires_at"
        ]
    );
    // A relation is never reported against a field that is missing or unreadable.
    let args = json!({
        "symbol": "BTCUSDT", "interval": "1h", "direction": "sideways",
        "thesis": "example", "entry": { "low": 100.0, "high": "x" },
        "invalidation": 150.0, "targets": [1.0], "expires_at": T0 + 1000,
    });
    assert_eq!(fields(&problems_of(args)), ["direction", "entry.high"]);
}

#[test]
fn a_revision_is_checked_the_same_way() {
    let r = create(long_args());
    // Only the new stop is sent; it is judged against the entry the setup has. The
    // target sent with it is unreadable and is named too.
    let args = json!({ "invalidation": 100.5, "targets": [104.0, "x"] });
    let mut problems = Vec::new();
    let levels = parse_levels(
        &args,
        Some(r.latest_levels()),
        Some(r.direction),
        &r.costs,
        T0,
        &mut problems,
    );
    assert!(levels.is_none());
    assert_eq!(fields(&problems), ["targets[1]", "invalidation"]);
}

#[test]
fn a_message_shows_numbers_as_a_person_would_write_them() {
    // 0.1 + 0.2 away from the entry, which a float does not hold exactly.
    let args = with(
        with(
            long_args(),
            "entry",
            json!({ "low": 36820.3, "high": 36900.0 }),
        ),
        "invalidation",
        json!(36783.48),
    );
    let args = with(args, "targets", json!([37500.0]));
    let problems = problems_of(args);
    assert_eq!(fields(&problems), ["invalidation"]);
    let text = format!(
        "{} {}",
        problems[0].message,
        problems[0].hint.as_deref().unwrap()
    );
    assert!(text.contains("is 36.82 from the entry"), "{text}");
    assert!(text.contains("least risk scored here, 154.645"), "{text}");
    assert!(text.contains("14 bps of the price"), "{text}");
    assert!(
        !text.contains("000000") && !text.contains("999999"),
        "{text}"
    );
}

/// The excursions with the bars that set them, in minutes from T0.
fn extremes(r: &Record) -> (Option<f64>, Option<i64>, Option<f64>, Option<i64>) {
    let minute = |t: Option<i64>| t.map(|t| (t - T0) / MIN);
    (r.mae_r, minute(r.mae_at), r.mfe_r, minute(r.mfe_at))
}

#[test]
fn excursions_of_a_long_are_read_from_the_fill_to_the_exit() {
    let mut r = create(long_args());
    walk(
        &mut r,
        &[
            bar(0, 103.0, 103.5, 102.0, 102.5),
            // Filled at 101, risk 3. Its high is not counted: the fill bar cannot win.
            bar(30, 102.5, 102.6, 100.8, 101.2),
        ],
    );
    assert_eq!(r.status, Status::Triggered);
    assert_eq!(extremes(&r), (Some(-0.0667), Some(30), Some(0.0), Some(30)));
    r.apply_bar(&bar(90, 101.2, 103.0, 99.5, 102.9));
    assert_eq!(extremes(&r), (Some(-0.5), Some(90), Some(0.6667), Some(90)));
    // Filled, the durations so far, in bars of its 1h interval.
    assert_eq!(
        r.durations(),
        Durations {
            bars_to_fill: Some(0.5),
            bars_to_exit: Some(1.0),
            bars_to_mae: Some(1.0),
            bars_to_mfe: Some(1.0),
        }
    );
    // Won at 104: what traded beyond it is not counted.
    r.apply_bar(&bar(150, 102.9, 104.6, 102.8, 104.0));
    assert_eq!(r.status, Status::Won);
    assert_eq!(extremes(&r), (Some(-0.5), Some(90), Some(1.0), Some(150)));
    assert_eq!(
        r.durations(),
        Durations {
            bars_to_fill: Some(0.5),
            bars_to_exit: Some(2.0),
            bars_to_mae: Some(1.0),
            bars_to_mfe: Some(2.0),
        }
    );
    // Before costs, unlike the result.
    assert!(r.result_r.unwrap() < 1.0);
    let view = r.view();
    assert_eq!(view["bars_to_exit"], 2.0);
    assert_eq!(view["mfe_at"], T0 + 150 * MIN);

    // Filled and stopped in one bar: out at the stop, whatever the wick did.
    let mut stopped = create(long_args());
    stopped.apply_bar(&bar(0, 102.0, 102.1, 97.5, 97.8));
    assert_eq!(
        extremes(&stopped),
        (Some(-1.0), Some(0), Some(0.0), Some(0))
    );
    assert_eq!(stopped.durations().bars_to_exit, Some(0.0));

    // A bar that opens past the target was out at the open: its later low does not count.
    let mut gapped = create(long_args());
    walk(
        &mut gapped,
        &[
            bar(0, 103.0, 103.5, 102.0, 102.5),
            bar(30, 102.5, 102.6, 100.8, 101.2),
        ],
    );
    gapped.apply_bar(&bar(90, 105.0, 105.5, 99.5, 100.0));
    assert_eq!(gapped.status, Status::Won);
    assert_eq!(
        extremes(&gapped),
        (Some(-0.0667), Some(30), Some(1.0), Some(90))
    );
}

#[test]
fn excursions_of_a_short_say_which_extreme_came_first() {
    let mut r = create(short_args());
    walk(
        &mut r,
        &[
            // Filled at 100, risk 3.
            bar(0, 99.0, 100.4, 98.9, 100.2),
            bar(60, 100.2, 100.5, 98.0, 98.4),
        ],
    );
    assert_eq!(
        extremes(&r),
        (Some(-0.1667), Some(60), Some(0.6667), Some(60))
    );
    // Stopped at 103: the stop counts first, so of this bar its open went with the
    // position, not its low.
    r.apply_bar(&bar(240, 98.4, 103.5, 97.0, 103.2));
    assert_eq!(r.status, Status::Invalidated);
    assert_eq!(
        extremes(&r),
        (Some(-1.0), Some(240), Some(0.6667), Some(60))
    );
    let d = r.durations();
    assert_eq!((d.bars_to_mfe, d.bars_to_mae), (Some(1.0), Some(4.0)));
    assert_eq!((d.bars_to_fill, d.bars_to_exit), (Some(0.0), Some(4.0)));

    // A cancel closes at the next bar's open: that open counts, the rest of the bar not.
    let mut cancelled = create(short_args());
    cancelled.apply_bar(&bar(0, 99.0, 100.4, 98.9, 100.2));
    cancelled.cancel("done".into(), T0 + MIN + 1).unwrap();
    walk(
        &mut cancelled,
        &[
            bar(1, 100.2, 100.3, 99.4, 99.5),
            bar(2, 99.1, 99.2, 90.0, 91.0),
        ],
    );
    assert_eq!(cancelled.status, Status::Cancelled);
    assert_eq!(
        extremes(&cancelled),
        (Some(-0.1333), Some(0), Some(0.3), Some(2))
    );
}

#[test]
fn durations_are_in_bars_of_the_setups_own_interval() {
    let mut r = create(long_args());
    r.apply_bar(&bar(0, 100.5, 100.9, 100.2, 100.6));
    r.apply_bar(&bar(120, 100.6, 100.9, 100.2, 100.6));
    for (interval, bars) in [
        ("1h", 2.0),
        ("4h", 0.5),
        ("7m", 17.14),
        ("45m", 2.67),
        ("1m", 120.0),
        ("2D", 0.04),
    ] {
        r.interval = interval.into();
        assert_eq!(r.durations().bars_to_exit, Some(bars), "{interval}");
    }
    r.interval = "nonsense".into();
    assert_eq!(r.durations(), Durations::default());
}

#[test]
fn excursions_and_durations_wait_for_the_fill() {
    let mut r = create(long_args());
    walk(&mut r, &[bar(0, 103.0, 110.0, 102.0, 102.5)]);
    assert_eq!(r.status, Status::Watching);
    assert_eq!(extremes(&r), (None, None, None, None));
    assert_eq!(r.durations(), Durations::default());
    let view = r.view();
    for field in [
        "mae_r",
        "mfe_at",
        "bars_to_fill",
        "bars_to_exit",
        "bars_to_mae",
    ] {
        assert_eq!(view[field], Value::Null, "{field}");
    }
    // Closed without a fill, it has none either.
    r.apply_bar(&bar(1, 97.0, 97.5, 96.5, 97.2));
    assert_eq!(r.status, Status::Invalidated);
    assert_eq!(extremes(&r), (None, None, None, None));
    assert_eq!(r.durations(), Durations::default());
}

#[test]
fn closest_approach_says_how_near_an_unfilled_setup_came() {
    // Above the zone of a long: from its high edge, 101, risk 3 to 98.
    let mut r = create(long_args());
    walk(
        &mut r,
        &[
            bar(0, 103.0, 103.5, 102.0, 102.5),
            bar(1, 102.5, 102.6, 101.6, 102.0),
            bar(2, 102.0, 104.0, 101.9, 103.5),
        ],
    );
    assert_eq!(r.closest_approach_r, Some(0.2));
    r.cancel("not coming".into(), T0 + 2 * MIN + 1).unwrap();
    r.apply_bar(&bar(3, 103.5, 103.6, 100.5, 101.0));
    assert_eq!(r.status, Status::Cancelled);
    assert_eq!(
        r.closest_approach_r,
        Some(0.2),
        "the cancel bar is not scored"
    );

    // Below the zone of a long, a breakout: from its low edge, 100, risk 2.
    let mut below = create(long_args());
    below.apply_bar(&bar(0, 99.0, 99.5, 98.5, 99.2));
    assert_eq!(below.closest_approach_r, Some(0.25));
    // Below the zone of a short: from its low edge, 100, risk 3 to 103.
    let mut short = create(short_args());
    short.apply_bar(&bar(0, 98.0, 99.0, 97.5, 98.5));
    assert_eq!(short.closest_approach_r, Some(0.3333));
    // Filled, it is 0.
    short.apply_bar(&bar(1, 98.5, 100.2, 98.4, 100.1));
    assert_eq!(short.status, Status::Triggered);
    assert_eq!(short.closest_approach_r, Some(0.0));

    // Expired unfilled, it still says how near it came.
    let mut expired = create(long_args());
    walk(
        &mut expired,
        &[
            bar(0, 103.0, 103.5, 101.3, 102.5),
            bar(1440, 103.0, 103.5, 101.0, 102.5),
        ],
    );
    assert_eq!(expired.status, Status::Expired);
    assert_eq!(expired.closest_approach_r, Some(0.1));

    // New levels in effect start it again: it is to the zone the setup has.
    let mut moved = create(long_args());
    moved.apply_bar(&bar(0, 102.0, 102.0, 101.25, 101.5));
    assert_eq!(moved.closest_approach_r, Some(0.0833));
    let mut lower = moved.levels.clone();
    (lower.entry.low, lower.entry.high, lower.invalidation) = (95.0, 96.0, 93.0);
    moved
        .revise(None, None, None, Some(lower), "lower".into(), T0 + 90_000)
        .unwrap();
    walk(
        &mut moved,
        &[
            bar(2, 102.0, 102.5, 101.25, 102.0),
            bar(3, 102.0, 102.5, 101.25, 102.0),
        ],
    );
    assert_eq!(moved.closest_approach_r, Some(1.75));
}

#[test]
fn a_stop_bar_that_opens_past_the_target_counts_no_more_than_the_target() {
    let mut r = create(long_args());
    r.apply_bar(&bar(0, 100.5, 100.8, 100.2, 100.6));
    assert_eq!(r.status, Status::Triggered);
    r.apply_bar(&bar(1, 105.0, 105.0, 97.5, 98.0));
    assert_eq!(r.status, Status::Invalidated);
    // Filled at 100.5, risk 2.5: the target 104 is 1.4 R, the 105 open 1.8 R.
    assert_eq!(r.mfe_r, Some(1.4));
    assert_eq!(r.mae_r, Some(-1.0));
}

#[test]
fn a_record_written_before_tags_and_excursions_loads_without_them() {
    let mut r = create(long_args());
    r.apply_bar(&bar(0, 103.0, 103.5, 102.0, 102.5));
    let mut watching = serde_json::to_value(&r).unwrap();
    r.apply_bar(&bar(1, 100.5, 100.9, 100.2, 100.6));
    let mut filled = serde_json::to_value(&r).unwrap();
    let new = [
        "tags",
        "mae_r",
        "mfe_r",
        "mae_at",
        "mfe_at",
        "closest_approach_r",
    ];
    for old in [&mut watching, &mut filled] {
        for field in new {
            old.as_object_mut().unwrap().remove(field);
        }
    }
    let mut watching: Record = serde_json::from_value(watching).unwrap();
    assert_eq!(watching.tags, Vec::<String>::new());
    // Its first bars were scored before the approach was kept: none, rather than one
    // that may miss a closer bar.
    watching.apply_bar(&bar(1, 102.5, 102.6, 101.6, 102.0));
    assert_eq!(watching.closest_approach_r, None);

    let mut filled: Record = serde_json::from_value(filled).unwrap();
    assert_eq!(extremes(&filled), (None, None, None, None));
    // Filled before they were kept: none, rather than ones that miss its start.
    filled.apply_bar(&bar(2, 100.6, 103.0, 99.0, 102.0));
    assert_eq!(filled.status, Status::Triggered);
    assert_eq!(extremes(&filled), (None, None, None, None));
    // The durations come from times every record keeps.
    assert_eq!(filled.durations().bars_to_fill, Some(0.02));
    assert_eq!(filled.durations().bars_to_mae, None);
}

#[test]
fn tags_are_checked_and_kept_once() {
    let r = create(with(
        long_args(),
        "tags",
        json!([
            "strategy:retest-short",
            "trigger:100eyes:abnormal-volatility",
            "pattern:double-top-ee",
            "strategy:retest-short"
        ]),
    ));
    assert_eq!(
        r.tags,
        [
            "strategy:retest-short",
            "trigger:100eyes:abnormal-volatility",
            "pattern:double-top-ee"
        ]
    );
    assert_eq!(create(long_args()).tags, Vec::<String>::new());

    let bad = json!([
        "Strategy",
        "has space",
        ":lead",
        "",
        "x".repeat(MAX_TAG + 1),
        5,
        "ok/1.2_a"
    ]);
    let problems = problems_of(with(long_args(), "tags", bad));
    assert_eq!(
        fields(&problems),
        ["tags[0]", "tags[1]", "tags[2]", "tags[3]", "tags[4]", "tags[5]"]
    );
    assert!(problems.iter().all(|p| p.hint.is_some()));
    let many = json!((0..=MAX_TAGS).map(|i| format!("t{i}")).collect::<Vec<_>>());
    assert_eq!(
        fields(&problems_of(with(long_args(), "tags", many))),
        ["tags"]
    );
    assert_eq!(
        fields(&problems_of(with(long_args(), "tags", json!("a")))),
        ["tags"]
    );
}

#[test]
fn tags_can_be_revised_after_the_fill_and_are_logged() {
    let mut r = create(with(long_args(), "tags", json!(["a"])));
    r.apply_bar(&bar(0, 100.5, 100.9, 100.2, 100.6));
    assert_eq!(r.status, Status::Triggered);
    let tags = Some(vec!["a".to_string(), "review:late-entry".to_string()]);
    let revision = r
        .revise(None, None, tags, None, "tagged".into(), T0 + MIN)
        .unwrap();
    assert_eq!(r.tags, ["a", "review:late-entry"]);
    assert_eq!(
        revision.changes["tags"],
        json!({ "from": ["a"], "to": ["a", "review:late-entry"] })
    );
    let same = Some(r.tags.clone());
    assert!(r
        .revise(None, None, same, None, "again".into(), T0 + MIN)
        .is_err());
}

#[test]
fn a_closed_setup_takes_tags_alone() {
    let mut r = create(with(long_args(), "tags", json!(["a"])));
    r.apply_bar(&bar(0, 100.5, 100.9, 100.2, 100.6));
    r.apply_bar(&bar(1, 100.6, 104.5, 100.5, 104.2));
    assert_eq!(r.status, Status::Won);
    let review = || Some(vec!["a".to_string(), "review:r1".to_string()]);
    // With anything else, the whole call is refused, and the hint says what may change.
    for (thesis, drawings) in [(Some("later".to_string()), None), (None, Some(vec![]))] {
        let refused = r
            .revise(thesis, drawings, review(), None, "r".into(), T0 + 2 * MIN)
            .unwrap_err();
        assert_eq!(fields(&refused), ["id"]);
        assert!(refused[0]
            .hint
            .as_deref()
            .unwrap()
            .contains("only its tags"));
    }
    let levels = Some(r.levels.clone());
    assert!(r
        .revise(None, None, review(), levels, "r".into(), T0 + 2 * MIN)
        .is_err());
    assert_eq!(r.tags, ["a"]);
    let revision = r
        .revise(None, None, review(), None, "reviewed".into(), T0 + 2 * MIN)
        .unwrap();
    assert_eq!(r.tags, ["a", "review:r1"]);
    assert_eq!(
        revision.changes["tags"],
        json!({ "from": ["a"], "to": ["a", "review:r1"] })
    );
    assert_eq!(r.status, Status::Won, "nothing else moves");

    // Being cancelled, the same.
    let mut cancelling = create(long_args());
    cancelling.cancel("done".into(), T0 + 1).unwrap();
    assert!(cancelling
        .revise(None, None, review(), None, "r".into(), T0 + 2)
        .is_ok());
    assert!(cancelling
        .revise(Some("x".into()), None, None, None, "r".into(), T0 + 3)
        .is_err());
}

// Shadow exit variants (ADR-0066). A long in at 101 and wrong at 98 risks 3: +1R is
// 104, +0.5R 102.5, T1 105 and T2 107. Its short twin mirrors every price about 100.5,
// so that one walk of bars tests both: in at 100, wrong at 103, T1 96 and T2 94.

fn shadow_setup(direction: Direction, targets: &[f64], interval: &str, days: i64) -> Record {
    let args = match direction {
        Direction::Long => long_args(),
        Direction::Short => short_args(),
    };
    let targets: Vec<f64> = targets.iter().map(|t| px(direction, *t)).collect();
    let args = with(args, "targets", json!(targets));
    let args = with(args, "interval", json!(interval));
    create(with(args, "expires_at", json!(T0 + days * 86_400_000)))
}

/// A long's price, or its short twin's.
fn px(direction: Direction, p: f64) -> f64 {
    match direction {
        Direction::Long => p,
        Direction::Short => 201.0 - p,
    }
}

/// A bar of the long's walk, or its short twin's.
fn twin(direction: Direction, b: MinuteBar) -> MinuteBar {
    match direction {
        Direction::Long => b,
        Direction::Short => MinuteBar {
            time: b.time,
            open: 201.0 - b.open,
            high: 201.0 - b.low,
            low: 201.0 - b.high,
            close: 201.0 - b.close,
        },
    }
}

/// Filled at 101 (100 for the short) by a bar coming down into the zone, then `bars`.
fn shadow_walk(r: &mut Record, bars: &[MinuteBar]) {
    let d = r.direction;
    r.apply_bar(&twin(d, bar(0, 102.0, 102.1, 100.8, 101.2)));
    assert_eq!(r.fill.unwrap().price, px(d, 101.0));
    shadow_walk_on(r, bars);
}

/// More bars after the fill bar.
fn shadow_walk_on(r: &mut Record, bars: &[MinuteBar]) {
    let d = r.direction;
    for b in bars {
        r.apply_bar(&twin(d, *b));
    }
}

fn shadows(r: &Record) -> &Shadows {
    r.shadow_results
        .as_ref()
        .expect("filled, so scored with its shadows")
}

fn all(r: &Record) -> [&Shadow; 3] {
    let s = shadows(r);
    [&s.v1_half_t1_be, &s.v2_be_at_1r, &s.v3_time_stop]
}

fn rounded(r: f64) -> Option<f64> {
    Some((r * 10_000.0).round() / 10_000.0)
}

const BOTH: [Direction; 2] = [Direction::Long, Direction::Short];

#[test]
fn v1_takes_half_at_t1_and_the_rest_at_t2_after_the_setup_closed() {
    for d in BOTH {
        let mut r = shadow_setup(d, &[105.0, 107.0], "1h", 1);
        shadow_walk(&mut r, &[bar(1, 101.2, 105.2, 102.0, 105.0)]);
        // The setup itself is out at T1; v1's other half runs on, its stop at
        // break-even after costs.
        assert_eq!(r.status, Status::Won);
        let v1 = &shadows(&r).v1_half_t1_be;
        assert_eq!(v1.status, ShadowStatus::Open);
        assert_eq!(v1.result_r, None);
        let half = ShadowExit {
            price: px(d, 105.0),
            fraction: 0.5,
            bar_time: T0 + MIN,
        };
        assert_eq!(v1.exits, [half]);
        let fill = r.fill.unwrap().price;
        assert_eq!(v1.stop, r.break_even(fill));
        assert_eq!(r.score(fill, v1.stop), 0.0, "{d:?}: even after costs");
        assert!(r.is_scoring());

        shadow_walk_on(&mut r, &[bar(2, 105.0, 107.5, 104.0, 107.0)]);
        let v1 = &shadows(&r).v1_half_t1_be;
        assert_eq!(v1.status, ShadowStatus::Won);
        assert_eq!(v1.exits[1].price, px(d, 107.0));
        assert_eq!(v1.exited_at, Some(T0 + 2 * MIN));
        let expected = 0.5 * r.score(fill, px(d, 105.0)) + 0.5 * r.score(fill, px(d, 107.0));
        assert_eq!(v1.result_r, rounded(expected));
        // The canonical result is not touched by its shadows.
        assert_eq!(r.result_r, Some(r.score(fill, px(d, 105.0))));
        assert_eq!(r.exit.unwrap().bar_time, T0 + MIN);
        assert!(!r.is_scoring());
        // v2 and v3 took all at T1 with the setup.
        for s in &all(&r)[1..] {
            assert_eq!(s.status, ShadowStatus::Won);
            assert_eq!(s.result_r, r.result_r);
        }
    }
}

#[test]
fn v1s_runner_is_stopped_at_break_even_from_the_bar_after_t1() {
    for d in BOTH {
        let mut r = shadow_setup(d, &[105.0, 107.0], "1h", 1);
        // Back below break-even in the bar that took T1: the moved stop is not in force
        // yet.
        shadow_walk(&mut r, &[bar(1, 101.2, 105.2, 100.5, 101.5)]);
        assert_eq!(shadows(&r).v1_half_t1_be.status, ShadowStatus::Open);
        shadow_walk_on(&mut r, &[bar(2, 101.5, 101.6, 101.0, 101.2)]);
        let v1 = &shadows(&r).v1_half_t1_be;
        let fill = r.fill.unwrap().price;
        assert_eq!(v1.status, ShadowStatus::Breakeven);
        assert_eq!(v1.exits[1].price, r.break_even(fill));
        assert_eq!(v1.exits[1].fraction, 0.5);
        assert_eq!(v1.result_r, rounded(0.5 * r.score(fill, px(d, 105.0))));
    }
}

#[test]
fn v1_with_one_target_runs_the_rest_to_its_stop_or_the_expiry() {
    for d in BOTH {
        let mut r = shadow_setup(d, &[105.0], "1h", 1);
        shadow_walk(
            &mut r,
            &[
                bar(1, 101.2, 105.2, 102.0, 105.0),
                bar(600, 108.0, 109.0, 107.0, 108.5),
                bar(1440, 110.0, 111.0, 101.0, 102.0),
            ],
        );
        let v1 = &shadows(&r).v1_half_t1_be;
        assert_eq!(v1.status, ShadowStatus::Expired);
        // Out at the open of the first bar past the expiry, as the setup would be.
        assert_eq!(v1.exits[1].price, px(d, 110.0));
        assert_eq!(v1.exited_at, Some(T0 + 1440 * MIN));
        let fill = r.fill.unwrap().price;
        let expected = 0.5 * r.score(fill, px(d, 105.0)) + 0.5 * r.score(fill, px(d, 110.0));
        assert_eq!(v1.result_r, rounded(expected));
        assert!(!r.is_scoring());
        let through = r.scored_through;
        assert!(r
            .apply_bar(&bar(1441, 100.0, 100.0, 100.0, 100.0))
            .is_empty());
        assert_eq!(r.scored_through, through, "nothing is left to score");
    }
}

#[test]
fn v2_moves_its_stop_at_1r_and_is_stopped_at_break_even() {
    for d in BOTH {
        let mut r = shadow_setup(d, &[105.0, 107.0], "1h", 1);
        // +1R reached, and back below break-even in the same bar: in force from the next.
        shadow_walk(&mut r, &[bar(1, 101.2, 104.1, 101.0, 103.8)]);
        let v2 = &shadows(&r).v2_be_at_1r;
        assert_eq!(v2.status, ShadowStatus::Open);
        assert_eq!(v2.stop, r.break_even(r.fill.unwrap().price));
        // v1 keeps the setup's stop until T1.
        assert_eq!(shadows(&r).v1_half_t1_be.stop, px(d, 98.0));
        shadow_walk_on(&mut r, &[bar(2, 103.8, 103.9, 101.0, 101.2)]);
        let v2 = &shadows(&r).v2_be_at_1r;
        assert_eq!(v2.status, ShadowStatus::Breakeven);
        assert_eq!(v2.result_r, Some(0.0));
        assert_eq!(v2.exited_at, Some(T0 + 2 * MIN));
        // The setup and the other variants are still in.
        assert_eq!(r.status, Status::Triggered);
        assert_eq!(shadows(&r).v1_half_t1_be.status, ShadowStatus::Open);
        assert_eq!(shadows(&r).v3_time_stop.status, ShadowStatus::Open);
    }
}

#[test]
fn a_gap_through_the_invalidation_past_a_moved_stop_is_stopped_not_breakeven() {
    for d in BOTH {
        let mut r = shadow_setup(d, &[105.0, 107.0], "1h", 1);
        shadow_walk(
            &mut r,
            &[
                bar(1, 101.2, 104.1, 102.0, 103.8),
                bar(2, 97.0, 97.5, 96.5, 97.2),
            ],
        );
        let v2 = &shadows(&r).v2_be_at_1r;
        assert_ne!(v2.stop, px(d, 98.0), "{d:?}: its stop had moved");
        assert_eq!(v2.status, ShadowStatus::Stopped);
        assert_eq!(v2.exits[0].price, px(d, 97.0));
        assert_eq!(v2.result_r, r.result_r);
        assert!(v2.result_r.unwrap() < -1.0);
    }
}

#[test]
fn v1s_moved_stop_is_never_beyond_a_t1_nearer_than_the_costs() {
    for d in BOTH {
        // T1 is 5 bps from the fill, less than break-even after 14 bps of costs.
        let mut r = shadow_setup(d, &[101.05, 106.0], "1h", 1);
        shadow_walk(&mut r, &[bar(1, 101.0, 101.06, 100.95, 101.0)]);
        let fill = r.fill.unwrap().price;
        assert!(d.sign() * (r.break_even(fill) - px(d, 101.05)) > 0.0);
        let v1 = &shadows(&r).v1_half_t1_be;
        assert_eq!(v1.stop, px(d, 101.05), "{d:?}: capped at T1");
        shadow_walk_on(&mut r, &[bar(2, 101.06, 101.07, 101.0, 101.02)]);
        let v1 = &shadows(&r).v1_half_t1_be;
        assert_eq!(v1.status, ShadowStatus::Breakeven);
        assert_eq!(v1.exits[1].price, px(d, 101.05));
        // Both halves out at T1: a little under 0 R after costs.
        assert_eq!(v1.result_r, Some(r.score(fill, px(d, 101.05))));
        assert!(v1.result_r.unwrap() < 0.0);
    }
}

#[test]
fn v2_wins_at_t1_after_moving_its_stop() {
    for d in BOTH {
        let mut r = shadow_setup(d, &[105.0, 107.0], "1h", 1);
        shadow_walk(
            &mut r,
            &[
                bar(1, 101.2, 104.1, 102.0, 103.8),
                bar(2, 103.8, 105.2, 102.0, 105.0),
            ],
        );
        let v2 = &shadows(&r).v2_be_at_1r;
        assert_eq!(v2.status, ShadowStatus::Won);
        assert_eq!(v2.exits.len(), 1);
        assert_eq!(v2.exits[0].price, px(d, 105.0));
        assert_eq!(v2.result_r, r.result_r);
    }
}

/// The open time (ms) of the 1m bar that closes the `n`th bar of `hours` after the one
/// holding T0, on bars counted from the epoch.
fn time_stop_bar(hours: i64, n: i64) -> i64 {
    let secs = hours * 3600;
    let start = (T0 / 1000).div_euclid(secs) * secs;
    (start + (n + 1) * secs) * 1000 - MIN
}

#[test]
fn v3_times_out_at_the_close_of_the_12th_hour_bar_below_half_an_r() {
    for d in BOTH {
        let mut r = shadow_setup(d, &[105.0], "1h", 1);
        let judged = time_stop_bar(1, 12);
        let minute = (judged - T0) / MIN;
        shadow_walk(&mut r, &[bar(minute - 1, 101.5, 102.0, 101.0, 101.5)]);
        let v3 = &shadows(&r).v3_time_stop;
        assert_eq!(v3.status, ShadowStatus::Open);
        assert_eq!(v3.time_stop_at, Some(judged + MIN));
        shadow_walk_on(&mut r, &[bar(minute, 101.5, 102.4, 101.2, 102.0)]);
        let v3 = &shadows(&r).v3_time_stop;
        assert_eq!(v3.status, ShadowStatus::TimedOut);
        assert_eq!(v3.exits[0].price, px(d, 102.0));
        assert_eq!(v3.exited_at, Some(judged));
        assert_eq!(v3.time_stop_at, None);
        let fill = r.fill.unwrap().price;
        assert_eq!(v3.result_r, Some(r.score(fill, px(d, 102.0))));
        assert_eq!(r.status, Status::Triggered);
    }
}

#[test]
fn v3_on_4h_judges_the_6th_bar() {
    for d in BOTH {
        let mut r = shadow_setup(d, &[105.0], "4h", 3);
        let judged = time_stop_bar(4, 6);
        let minute = (judged - T0) / MIN;
        shadow_walk(&mut r, &[bar(minute - 1, 101.5, 102.0, 101.0, 101.5)]);
        assert_eq!(shadows(&r).v3_time_stop.status, ShadowStatus::Open);
        shadow_walk_on(&mut r, &[bar(minute, 101.5, 102.0, 100.0, 100.5)]);
        let v3 = &shadows(&r).v3_time_stop;
        assert_eq!(v3.status, ShadowStatus::TimedOut);
        assert_eq!(v3.exits[0].price, px(d, 100.5));
        assert_eq!(v3.exited_at, Some(judged));
    }
}

#[test]
fn v3_keeps_a_position_at_half_an_r_or_more_and_judges_it_once() {
    for d in BOTH {
        let mut r = shadow_setup(d, &[105.0], "1h", 1);
        let minute = (time_stop_bar(1, 12) - T0) / MIN;
        // 102.5 is +0.5R exactly: kept.
        shadow_walk(&mut r, &[bar(minute, 102.0, 102.8, 101.8, 102.5)]);
        let v3 = &shadows(&r).v3_time_stop;
        assert_eq!(v3.status, ShadowStatus::Open);
        assert_eq!(v3.time_stop_at, None, "judged");
        // Later closes below +0.5R do not time it out: it is the canonical setup now.
        shadow_walk_on(
            &mut r,
            &[
                bar(minute + 60, 102.0, 102.1, 101.0, 101.2),
                bar(minute + 61, 101.2, 105.5, 101.0, 105.2),
            ],
        );
        let v3 = &shadows(&r).v3_time_stop;
        assert_eq!(v3.status, ShadowStatus::Won);
        assert_eq!(v3.result_r, r.result_r);
    }
}

#[test]
fn the_fill_bar_can_stop_the_shadows_but_cannot_move_or_win_them() {
    for d in BOTH {
        // Past T1 and +1R in the fill bar: none of it counts.
        let mut r = shadow_setup(d, &[105.0, 107.0], "1h", 1);
        r.apply_bar(&twin(d, bar(0, 102.0, 105.5, 100.8, 105.2)));
        assert_eq!(r.status, Status::Triggered);
        for s in all(&r) {
            assert_eq!(s.status, ShadowStatus::Open);
            assert_eq!(s.stop, px(d, 98.0));
            assert!(s.exits.is_empty());
        }

        let mut r = shadow_setup(d, &[105.0, 107.0], "1h", 1);
        r.apply_bar(&twin(d, bar(0, 102.0, 102.1, 97.5, 97.8)));
        assert_eq!(r.status, Status::Invalidated);
        for s in all(&r) {
            assert_eq!(s.status, ShadowStatus::Stopped);
            assert_eq!(s.exits[0].price, px(d, 98.0));
            assert_eq!(s.result_r, r.result_r);
        }
    }
}

#[test]
fn a_stop_and_a_target_in_one_bar_and_a_gap_are_read_as_the_setup_reads_them() {
    for d in BOTH {
        // Stop and T1 in one bar: the stop, for every variant.
        let mut r = shadow_setup(d, &[105.0, 107.0], "1h", 1);
        shadow_walk(&mut r, &[bar(1, 101.2, 105.5, 97.9, 104.0)]);
        for s in all(&r) {
            assert_eq!(s.status, ShadowStatus::Stopped);
            assert_eq!(s.result_r, r.result_r);
        }

        // A bar that opens past T1 and reaches the stop: the stop too, as the setup
        // reads it (ADR-0066 leaves changing that open).
        let mut r = shadow_setup(d, &[105.0, 107.0], "1h", 1);
        shadow_walk(&mut r, &[bar(1, 105.5, 106.0, 97.5, 98.5)]);
        assert_eq!(r.status, Status::Invalidated);
        for s in all(&r) {
            assert_eq!(s.status, ShadowStatus::Stopped);
            assert_eq!((s.exits.len(), s.exits[0].price), (1, px(d, 98.0)));
            assert_eq!(s.result_r, r.result_r);
        }

        // A gap through the setup's stop: out at the open.
        let mut r = shadow_setup(d, &[105.0, 107.0], "1h", 1);
        shadow_walk(&mut r, &[bar(1, 96.0, 96.5, 95.0, 96.2)]);
        assert_eq!(r.exit.unwrap().price, px(d, 96.0));
        for s in all(&r) {
            assert_eq!(s.exits[0].price, px(d, 96.0));
            assert_eq!(s.result_r, r.result_r);
        }

        // A gap through v1's break-even stop: out at the open, which is worse.
        let mut r = shadow_setup(d, &[105.0, 107.0], "1h", 1);
        shadow_walk(
            &mut r,
            &[
                bar(1, 101.2, 105.2, 102.0, 105.0),
                bar(2, 100.0, 100.5, 99.5, 100.2),
            ],
        );
        let v1 = &shadows(&r).v1_half_t1_be;
        assert_eq!(v1.status, ShadowStatus::Breakeven);
        assert_eq!(v1.exits[1].price, px(d, 100.0));
        assert!(r.score(r.fill.unwrap().price, px(d, 100.0)) < 0.0);
    }
}

#[test]
fn a_cancel_closes_the_shadows_at_the_open_as_it_does_the_setup() {
    let mut r = shadow_setup(Direction::Long, &[105.0, 107.0], "1h", 1);
    shadow_walk(&mut r, &[bar(1, 101.2, 104.1, 102.0, 103.8)]);
    r.cancel("done".into(), T0 + MIN + 1).unwrap();
    shadow_walk_on(&mut r, &[bar(2, 103.5, 103.6, 101.0, 101.2)]);
    assert_eq!(r.status, Status::Cancelled);
    for s in all(&r) {
        assert_eq!(s.status, ShadowStatus::Cancelled);
        assert_eq!(s.exits[0].price, 103.5);
        assert_eq!(s.result_r, r.result_r);
    }
}

#[test]
fn an_open_shadow_that_runs_out_unscored_ends_with_no_result() {
    let mut r = shadow_setup(Direction::Long, &[105.0], "1h", 1);
    shadow_walk(&mut r, &[bar(1, 101.2, 105.2, 102.0, 105.0)]);
    let result = r.result_r;
    let log = r.expire_unscored(T0 + 2 * 86_400_000);
    assert_eq!(log.iter().map(|e| e.kind).collect::<Vec<_>>(), [Kind::Gap]);
    let v1 = &shadows(&r).v1_half_t1_be;
    assert_eq!(v1.status, ShadowStatus::Expired);
    assert_eq!((v1.result_r, v1.exits.len()), (None, 1));
    // The setup itself was scored to its end: its result and gap are as they were.
    assert_eq!((r.status, r.result_r, r.gap), (Status::Won, result, false));
    assert!(!r.is_scoring());
}

#[test]
fn a_record_filled_before_shadows_loads_and_gets_none() {
    let mut r = shadow_setup(Direction::Long, &[105.0, 107.0], "1h", 1);
    let mut watching = serde_json::to_value(&r).unwrap();
    shadow_walk(&mut r, &[]);
    let mut filled = serde_json::to_value(&r).unwrap();
    assert!(filled["shadow_results"].is_object());
    for old in [&mut watching, &mut filled] {
        old.as_object_mut().unwrap().remove("shadow_results");
    }
    let mut filled: Record = serde_json::from_value(filled).unwrap();
    assert_eq!(filled.shadow_results, None);
    shadow_walk_on(&mut filled, &[bar(1, 101.2, 105.2, 102.0, 105.0)]);
    assert_eq!(filled.status, Status::Won);
    assert_eq!(filled.shadow_results, None);
    assert!(!filled.is_scoring());
    assert_eq!(filled.view()["shadow_results"], Value::Null);
    // Stored while watching, it is filled here and gets them.
    let mut watching: Record = serde_json::from_value(watching).unwrap();
    shadow_walk(&mut watching, &[]);
    assert!(watching.shadow_results.is_some());
}

// Stock setups (ADR-0070). Times are New York's, around the weekend of 2026-11-07.

fn ny(day: u32, hour: u32, minute: u32) -> i64 {
    use chrono::TimeZone;
    chrono_tz::America::New_York
        .with_ymd_and_hms(2026, 11, day, hour, minute, 0)
        .single()
        .unwrap()
        .timestamp_millis()
}

fn at(time: i64, open: f64, high: f64, low: f64, close: f64) -> MinuteBar {
    MinuteBar {
        time,
        open,
        high,
        low,
        close,
    }
}

/// A long on a stock at 180 to 181, wrong at 176, made on Friday at 14:00, as the
/// service makes it; and its twin on a 24/7 symbol.
fn stock_long(targets: Value, expires_at: i64) -> (Record, Record) {
    let args = json!({
        "symbol": "NVDA", "interval": "1h", "direction": "long", "thesis": "example thesis",
        "entry": { "low": 180.0, "high": 181.0 }, "invalidation": 176.0,
        "targets": targets, "expires_at": expires_at,
    });
    let made = ny(6, 14, 0);
    let make = |market: Option<Market>| {
        let mut problems = Vec::new();
        let draft = Draft::parse_for(&args, market, made, &mut problems).unwrap();
        let levels = draft.levels.clone().unwrap();
        let (mut r, _) = Record::new("s".into(), "o", "NVDA".into(), draft, levels, vec![], made);
        r.market = market;
        r.costs = market.map_or_else(Costs::default, Market::costs);
        r
    };
    (make(Some(Market::UsEquity)), make(None))
}

/// Friday's bars: a fill at 180.5 at 15:00, and the session's last minute closing at
/// 182.5.
fn friday() -> [MinuteBar; 2] {
    [
        at(ny(6, 15, 0), 180.5, 181.0, 180.0, 180.6),
        at(ny(6, 15, 59), 182.0, 183.0, 181.5, 182.5),
    ]
}

#[test]
fn a_stock_setup_expiring_on_a_weekend_ends_at_fridays_close() {
    let (mut stock, mut crypto) = stock_long(json!([190.0]), ny(7, 12, 0));
    // Monday opens far below the stop.
    let monday = at(ny(9, 9, 30), 172.0, 173.0, 171.0, 172.5);
    for b in friday().iter().chain([&monday]) {
        stock.apply_bar(b);
        crypto.apply_bar(b);
    }
    assert_eq!(stock.status, Status::Expired);
    let friday_close = Mark {
        price: 182.5,
        bar_time: ny(6, 15, 59),
    };
    assert_eq!(stock.exit, Some(friday_close));
    assert_eq!(stock.result_r, Some(stock.score(180.5, 182.5)));
    assert!(stock.result_r.unwrap() > 0.0);
    // The status entry is logged at the expiry.
    let log = stock.expire_by(i64::MAX);
    assert!(log.is_empty(), "closed already");
    // A 24/7 setup expires at the first bar at or after the expiry, at its open.
    assert_eq!(crypto.status, Status::Expired);
    assert_eq!(crypto.exit.map(|m| m.price), Some(172.0));

    // With no bar after the expiry, the server ends it once it has read past it.
    let (mut quiet, _) = stock_long(json!([190.0]), ny(7, 12, 0));
    walk(&mut quiet, &friday());
    assert!(quiet.expire_by(ny(7, 11, 59)).is_empty());
    let log = quiet.expire_by(ny(7, 12, 0));
    assert_eq!(log[0].at, ny(7, 12, 0));
    assert_eq!(quiet.status, Status::Expired);
    assert_eq!(quiet.exit, Some(friday_close));
    // So do its shadows, at the same close.
    for s in all(&quiet) {
        assert_eq!((s.status, s.exits[0].price), (ShadowStatus::Expired, 182.5));
    }
}

#[test]
fn a_stock_setup_ends_at_the_last_close_before_its_expiry_within_a_session() {
    let (mut r, _) = stock_long(json!([190.0]), ny(6, 15, 30));
    walk(
        &mut r,
        &[friday()[0], at(ny(6, 15, 29), 181.0, 181.2, 180.9, 181.1)],
    );
    assert_eq!(r.status, Status::Triggered);
    // The 15:30 bar closes after the expiry: the 15:29 close counts.
    r.apply_bar(&at(ny(6, 15, 30), 181.1, 185.0, 181.0, 184.0));
    assert_eq!(r.status, Status::Expired);
    assert_eq!(
        r.exit.map(|m| (m.price, m.bar_time)),
        Some((181.1, ny(6, 15, 29)))
    );
}

#[test]
fn an_unfilled_stock_setup_expires_at_zero_and_new_levels_count_in_a_closed_market() {
    let (mut r, _) = stock_long(json!([190.0]), ny(7, 12, 0));
    // Revised on Saturday morning, before the expiry: there is no bar to wait for.
    let mut later = r.levels.clone();
    later.expires_at = ny(10, 16, 0);
    r.revise(
        None,
        None,
        None,
        Some(later),
        "more time".into(),
        ny(7, 10, 0),
    )
    .unwrap();
    assert!(r.expire_by(ny(7, 13, 0)).is_empty());
    assert_eq!(r.status, Status::Watching);
    let (mut unrevised, _) = stock_long(json!([190.0]), ny(7, 12, 0));
    unrevised.expire_by(ny(7, 13, 0));
    assert_eq!(
        (unrevised.status, unrevised.result_r, unrevised.exit),
        (Status::Expired, Some(0.0), None)
    );
}

#[test]
fn a_stock_target_gapped_past_exits_at_the_open_and_a_binance_one_at_the_target() {
    let (mut stock, mut crypto) = stock_long(json!([186.0, 195.0]), ny(20, 16, 0));
    let monday = at(ny(9, 9, 30), 190.0, 191.0, 189.0, 190.5);
    for r in [&mut stock, &mut crypto] {
        walk(r, &friday());
        r.apply_bar(&monday);
        assert_eq!(r.status, Status::Won);
    }
    assert_eq!(stock.exit.map(|m| m.price), Some(190.0));
    assert_eq!(crypto.exit.map(|m| m.price), Some(186.0));
    assert_eq!(
        stock.mfe_r,
        crypto
            .mfe_r
            .map(|_| ((190.0 - 180.5) / 4.5 * 10_000.0_f64).round() / 10_000.0)
    );
    // The shadows take their targets at the gap's open too: T1 for v2 and v3, and
    // v1's half at T1.
    let s = shadows(&stock);
    assert_eq!(s.v2_be_at_1r.exits[0].price, 190.0);
    assert_eq!(s.v3_time_stop.exits[0].price, 190.0);
    assert_eq!(s.v1_half_t1_be.exits[0].price, 190.0);
    let c = shadows(&crypto);
    assert_eq!(c.v2_be_at_1r.exits[0].price, 186.0);
    assert_eq!(c.v1_half_t1_be.exits[0].price, 186.0);
}

#[test]
fn a_stock_setup_has_its_own_costs_and_least_risk() {
    // 30 bps of risk: under Binance's 42 bps floor, over the stocks' 24 bps.
    let args = with(
        with(long_args(), "entry", json!({ "low": 100.0, "high": 100.5 })),
        "invalidation",
        json!(99.7),
    );
    let mut problems = Vec::new();
    assert!(Draft::parse(&args, T0, &mut problems).is_none());
    assert_eq!(fields(&problems), ["invalidation"]);
    let mut problems = Vec::new();
    assert!(Draft::parse_for(&args, Some(Market::UsEquity), T0, &mut problems).is_some());
    assert_eq!(problems, vec![]);
    let costs = Market::UsEquity.costs();
    assert_eq!(
        (costs.fee_bps_per_side, costs.slippage_bps_per_side),
        (1.0, 3.0)
    );
    assert!((costs.round_trip() - 0.0008).abs() < 1e-12);
}

#[test]
fn a_split_divides_every_price_once_and_the_result_in_r_does_not_move() {
    let (mut split, _) = stock_long(json!([186.0, 195.0]), ny(20, 16, 0));
    let (mut plain, _) = stock_long(json!([186.0, 195.0]), ny(20, 16, 0));
    walk(&mut split, &friday());
    walk(&mut plain, &friday());
    let action = Adjustment::Split {
        at: ny(9, 9, 30),
        ratio: 10.0,
    };
    let entry = split.adjust(action).unwrap();
    assert_eq!(entry.kind, Kind::Adjusted);
    assert!(entry.reason.as_deref().unwrap().contains("divided by 10"));
    assert!(split.adjust(action).is_none(), "once");
    assert_eq!(split.levels.invalidation, 17.6);
    assert_eq!(split.levels.targets, [18.6, 19.5]);
    assert_eq!(split.fill.map(|m| m.price), Some(18.05));
    assert_eq!(split.last_close.map(|m| m.price), Some(18.25));
    assert_eq!(shadows(&split).v1_half_t1_be.stop, 17.6);
    // The same day in the old terms and in the new.
    split.apply_bar(&at(ny(9, 9, 30), 18.3, 18.7, 18.2, 18.6));
    plain.apply_bar(&at(ny(9, 9, 30), 183.0, 187.0, 182.0, 186.0));
    assert_eq!(split.status, Status::Won);
    assert_eq!(split.result_r, plain.result_r);
    // A split from before the setup was made: its prices were in the new terms.
    let (mut late, _) = stock_long(json!([190.0]), ny(20, 16, 0));
    let earlier = Adjustment::Split {
        at: ny(6, 9, 30),
        ratio: 10.0,
    };
    assert!(late.adjust(earlier).is_none());
    // A Binance setup takes none.
    let (_, mut crypto) = stock_long(json!([190.0]), ny(20, 16, 0));
    assert!(crypto.adjust(action).is_none());
}

#[test]
fn a_dividend_is_paid_to_a_long_and_by_a_short_from_the_exit_on() {
    let ex = ny(9, 9, 30);
    let dividend = Adjustment::Dividend {
        at: ex,
        amount: 0.5,
    };
    let (mut long, _) = stock_long(json!([186.0]), ny(20, 16, 0));
    // Not filled yet: nothing to pay.
    assert!(long.adjust(dividend).is_none());
    walk(&mut long, &friday());
    assert_eq!(long.adjust(dividend).unwrap().kind, Kind::Adjusted);
    long.apply_bar(&at(ex, 183.0, 187.0, 182.0, 186.0));
    assert_eq!(long.status, Status::Won);
    let costs = (180.5 + 186.0) * long.costs.round_trip() / 2.0 / 4.5;
    let paid = ((186.0 - 180.5 + 0.5) / 4.5 - costs) * 10_000.0;
    assert_eq!(long.result_r, Some(paid.round() / 10_000.0));
    // Exited before the ex-date: not paid.
    assert_eq!(
        long.score_at(180.5, 186.0, ex - 1),
        long.score(180.5, 186.0)
    );
    assert!(long.score_at(180.5, 186.0, ex) > long.score(180.5, 186.0));

    let mut short = create(short_args());
    short.market = Some(Market::UsEquity);
    short.apply_bar(&bar(1, 100.5, 101.0, 100.2, 100.6));
    assert_eq!(short.status, Status::Triggered);
    let at = T0 + 5 * MIN;
    short
        .adjust(Adjustment::Dividend { at, amount: 0.3 })
        .unwrap();
    let fill = short.fill.unwrap().price;
    assert!(short.score_at(fill, 99.0, at) < short.score_at(fill, 99.0, at - 1));
}

#[test]
fn a_stock_setups_time_stop_and_durations_count_session_time() {
    let (mut r, _) = stock_long(json!([190.0]), ny(20, 16, 0));
    walk(&mut r, &friday());
    // The fill at 15:00 is in Friday's 14:30 bar; 12 bars on is Tuesday's 12:30 bar.
    assert_eq!(shadows(&r).v3_time_stop.time_stop_at, Some(ny(10, 13, 30)));
    r.apply_bar(&at(ny(9, 9, 59), 182.0, 191.0, 181.0, 190.5));
    assert_eq!(r.status, Status::Won);
    // Made at 14:00 and filled at 15:00; out at 09:59 on Monday: an hour on Friday and
    // 29 minutes on Monday.
    let durations = r.durations();
    assert_eq!(durations.bars_to_fill, Some(1.0));
    assert_eq!(durations.bars_to_exit, Some(1.48));
}

#[test]
fn a_skip_on_a_stock_runs_out_after_its_bars_of_session_time() {
    use chrono::TimeZone;
    let wednesday = chrono_tz::America::New_York
        .with_ymd_and_hms(2026, 11, 25, 15, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let (levels, rule) = stock_skip_bracket(
        Market::UsEquity,
        Direction::Long,
        &hourly(40),
        "1h",
        wednesday,
    )
    .unwrap();
    assert_eq!(rule, Bracket::Atr);
    let next_wednesday_close = chrono_tz::America::New_York
        .with_ymd_and_hms(2026, 12, 2, 16, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    assert_eq!(levels.expires_at, next_wednesday_close);
    // On a weekly chart, at most 90 days, as for Binance.
    let (levels, _) = stock_skip_bracket(
        Market::UsEquity,
        Direction::Long,
        &hourly(40),
        "1w",
        wednesday,
    )
    .unwrap();
    assert_eq!(levels.expires_at, wednesday + MAX_EXPIRY_MS);
}

#[test]
fn a_binance_record_is_written_without_the_stock_fields() {
    let r = create(long_args());
    let text = serde_json::to_string(&r).unwrap();
    for field in ["market", "last_close", "adjustments"] {
        assert!(!text.contains(field), "{field} in {text}");
    }
    let (stock, _) = stock_long(json!([190.0]), ny(20, 16, 0));
    let view = stock.view();
    assert_eq!(view["market"], "us_equity");
    let back: Record = serde_json::from_str(&serde_json::to_string(&stock).unwrap()).unwrap();
    assert_eq!(back, stock);
}
