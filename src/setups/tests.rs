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
    let levels = skip_bracket(draft.direction, &hourly(40), 3600, T0).unwrap();
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
    assert!(r.revise(None, None, levels, "tighter".into(), T0).is_err());
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
    assert!(r.revise(None, None, same, "no change".into(), T0).is_err());
    let bad = r
        .revise(None, None, Some(revised_levels(100.5)), "closer".into(), T0)
        .unwrap_err();
    assert_eq!(fields(&bad), ["invalidation"]);
    assert_eq!(r.revisions, 1);
}

#[test]
fn the_log_numbers_every_entry_in_order() {
    let mut r = create(long_args());
    let mut log = vec![];
    log.push(
        r.revise(Some("second".into()), None, None, "why".into(), T0 + 1)
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
        .revise(Some("late".into()), None, None, "late".into(), T0 + MIN)
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
        let levels = skip_bracket(Direction::Long, &hourly(40), interval_secs, T0).unwrap();
        assert_eq!(
            levels.expires_at - T0,
            expected_ms,
            "{interval_secs} s bars"
        );
    }
}
