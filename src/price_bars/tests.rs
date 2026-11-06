use super::*;

const HOUR: i64 = 3_600;
const FAR_FUTURE_MS: i64 = 4_000_000_000_000;

fn iv() -> Interval {
    Interval::parse("1h").unwrap()
}

fn opts() -> BuildOpts {
    BuildOpts {
        max_base: 20_000,
        end_time_ms: None,
        now_ms: FAR_FUTURE_MS,
        session: false,
    }
}

fn row(i: i64, o: f64, h: f64, l: f64, c: f64) -> Bar {
    Bar {
        open: o,
        high: h,
        low: l,
        close: c,
        volume: 1.0,
        date: (i * HOUR).to_string(),
    }
}

fn closes(cs: &[f64]) -> Vec<Bar> {
    cs.iter()
        .enumerate()
        .map(|(i, &c)| row(i as i64, c, c, c, c))
        .collect()
}

fn params(kind: Kind, size: f64) -> Params {
    Params {
        kind,
        mode: BoxMode::Fixed,
        atr_length: 14,
        percent: 1.0,
        box_size: Some(size),
        reversal: 3,
        lines: 3,
    }
}

fn run(source: &[Bar], p: &Params) -> Vec<Bar> {
    build(source, &iv(), p, &opts()).unwrap().bars
}

fn oc(bars: &[Bar]) -> Vec<(f64, f64)> {
    bars.iter().map(|b| (b.open, b.close)).collect()
}

fn times(bars: &[Bar]) -> Vec<i64> {
    bars.iter().map(|b| b.date.parse().unwrap()).collect()
}

/// A wandering price from a fixed-seed generator.
fn walk(n: i64, start: i64) -> Vec<Bar> {
    let mut x = 42u64;
    let mut price = 1000.0_f64;
    (0..n)
        .map(|i| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let step = ((x >> 32) as f64 / u32::MAX as f64 - 0.5) * 16.0;
            let open = price;
            price = (price + step).max(10.0);
            row(
                start + i,
                open,
                open.max(price) + 1.5,
                open.min(price) - 1.5,
                price,
            )
        })
        .collect()
}

fn all_kinds() -> [Params; 5] {
    [
        params(Kind::Renko, 10.0),
        params(Kind::Kagi, 10.0),
        params(Kind::PointFigure, 5.0),
        params(Kind::LineBreak, 10.0),
        params(Kind::Range, 10.0),
    ]
}

#[test]
fn renko_makes_the_worked_bricks() {
    // Seed cell [100, 110]. 125 passes 120; 99 is a box under 110's brick start,
    // and 70 is two more boxes in one source bar.
    let bars = run(
        &closes(&[100.0, 105.0, 112.0, 125.0, 99.0, 89.0, 70.0]),
        &params(Kind::Renko, 10.0),
    );
    assert_eq!(
        oc(&bars),
        vec![
            (110.0, 120.0),
            (110.0, 100.0),
            (100.0, 90.0),
            (90.0, 80.0),
            (80.0, 70.0)
        ]
    );
    assert_eq!(bars[0].high, 120.0);
    assert_eq!(bars[0].low, 110.0);
    // Opened with the first source bar, then at each bar that confirmed it; the
    // second brick of source bar 6 takes the next second.
    assert_eq!(
        times(&bars),
        vec![0, 4 * HOUR, 5 * HOUR, 6 * HOUR, 6 * HOUR + 1]
    );
}

#[test]
fn renko_needs_a_whole_box_past_the_grid_to_reverse() {
    // After 110 to 120, 101 is not yet a box under 110, and 100 is.
    let bars = run(&closes(&[100.0, 125.0, 101.0]), &params(Kind::Renko, 10.0));
    assert_eq!(oc(&bars)[0], (110.0, 120.0));
    assert_eq!(bars.len(), 2, "the open bar holds what is unconfirmed");
    assert_eq!((bars[1].open, bars[1].close), (120.0, 101.0));
    let bars = run(&closes(&[100.0, 125.0, 100.0]), &params(Kind::Renko, 10.0));
    assert_eq!(oc(&bars), vec![(110.0, 120.0), (110.0, 100.0)]);
}

#[test]
fn the_open_brick_keeps_its_time_when_it_is_confirmed() {
    let p = params(Kind::Renko, 10.0);
    // Source bar 3 is forming at the first look.
    let mut o = opts();
    o.now_ms = (3 * HOUR + 10) * 1000;
    let source = closes(&[100.0, 125.0, 127.0, 129.0]);
    let early = build(&source, &iv(), &p, &o).unwrap().bars;
    assert_eq!(times(&early), vec![0, 2 * HOUR]);
    assert_eq!(
        (early[1].open, early[1].close),
        (120.0, 129.0),
        "the last price, not a level"
    );
    // A forming bar that spans boxes confirms nothing: one open bar, no extra bricks.
    let spike = closes(&[100.0, 125.0, 127.0, 175.0]);
    let spiked = build(&spike, &iv(), &p, &o).unwrap().bars;
    assert_eq!(spiked.len(), 2);
    assert_eq!(spiked[1].close, 175.0);
    // It closes above the next level: the same time, now a brick.
    o.now_ms = (4 * HOUR + 1) * 1000;
    let source = closes(&[100.0, 125.0, 127.0, 131.0]);
    let later = build(&source, &iv(), &p, &o).unwrap().bars;
    assert_eq!(times(&later), vec![0, 2 * HOUR]);
    assert_eq!((later[1].open, later[1].close), (120.0, 130.0));
}

#[test]
fn volume_is_kept_across_bricks() {
    let source = closes(&[100.0, 105.0, 150.0, 151.0]);
    let bars = run(&source, &params(Kind::Renko, 10.0));
    // Bars 0 to 2 confirm 4 bricks (110..150) that share 3 of volume.
    assert_eq!(bars.len(), 5);
    assert_eq!(bars[0].volume, 0.75);
    assert_eq!(bars[3].volume, 0.75);
    assert_eq!(bars[4].volume, 1.0, "the open bar has what is left");
    let total: f64 = bars.iter().map(|b| b.volume).sum();
    assert_eq!(total, 4.0);
}

#[test]
fn line_break_follows_the_three_line_rule() {
    let source = closes(&[10.0, 11.0, 12.0, 13.0, 12.5, 14.0, 10.5, 10.0, 11.0, 15.0]);
    let bars = run(&source, &params(Kind::LineBreak, 0.0));
    assert_eq!(
        oc(&bars),
        vec![
            (10.0, 11.0),
            (11.0, 12.0),
            (12.0, 13.0),
            (13.0, 14.0),
            // Under the lowest of the last three lines: from the last line's bottom.
            (13.0, 10.5),
            (10.5, 10.0),
            // Over the highest of the last three: 14. From the last line's top.
            (10.5, 15.0),
        ]
    );
    // 12.5 and 11 drew nothing, so they are in the next line's time.
    let t = times(&bars);
    assert_eq!(t[0], 0);
    assert_eq!(t[3], 4 * HOUR);
}

#[test]
fn line_break_count_changes_what_reverses() {
    let source = closes(&[10.0, 11.0, 12.0, 10.5]);
    let mut p = params(Kind::LineBreak, 0.0);
    p.lines = 1;
    assert_eq!(
        oc(&run(&source, &p))[2],
        (11.0, 10.5),
        "under the last line"
    );
    p.lines = 2;
    let bars = run(&source, &p);
    assert_eq!(bars.len(), 3, "two lines, then the open bar");
    assert_eq!((bars[2].open, bars[2].close), (12.0, 10.5));
}

#[test]
fn kagi_turns_at_the_extreme_after_the_reversal_amount() {
    let source = closes(&[100.0, 105.0, 112.0, 108.0, 101.0, 95.0, 110.0]);
    let bars = run(&source, &params(Kind::Kagi, 10.0));
    assert_eq!(
        oc(&bars),
        vec![(100.0, 112.0), (112.0, 95.0), (95.0, 110.0)]
    );
    assert_eq!(times(&bars), vec![0, 4 * HOUR, 6 * HOUR]);
    assert_eq!(bars[1].high, 112.0);
    assert_eq!(bars[1].low, 95.0);
}

#[test]
fn kagi_shows_the_last_price_without_turning_on_a_forming_bar() {
    let mut o = opts();
    o.now_ms = (3 * HOUR + 1) * 1000;
    let source = closes(&[100.0, 112.0, 113.0, 90.0]);
    let bars = build(&source, &iv(), &params(Kind::Kagi, 10.0), &o)
        .unwrap()
        .bars;
    assert_eq!(bars.len(), 1, "a forming bar turns nothing");
    assert_eq!(bars[0].close, 90.0);
    assert_eq!(bars[0].high, 113.0, "the extreme stays");
}

#[test]
fn point_and_figure_columns_reverse_by_boxes() {
    let source = closes(&[10.0, 11.0, 13.0, 11.0, 10.0, 8.0, 10.0, 11.0]);
    let bars = run(&source, &params(Kind::PointFigure, 1.0));
    // Xs 10..13; 3 boxes under 13 starts Os a box under it, 12..10, on to 8; 3 boxes
    // over 8 starts Xs a box over it, 9..11.
    assert_eq!(oc(&bars), vec![(10.0, 13.0), (12.0, 8.0), (9.0, 11.0)]);
    assert_eq!(bars[1].high, 12.0);
    assert_eq!(bars[1].low, 8.0);
    assert_eq!(times(&bars), vec![0, 4 * HOUR, 7 * HOUR]);
}

#[test]
fn point_and_figure_reversal_is_an_input() {
    let source = closes(&[10.0, 13.0, 12.0, 11.0]);
    let mut p = params(Kind::PointFigure, 1.0);
    p.reversal = 2;
    assert_eq!(oc(&run(&source, &p)), vec![(10.0, 13.0), (12.0, 11.0)]);
    p.reversal = 3;
    assert_eq!(oc(&run(&source, &p)), vec![(10.0, 13.0)]);
}

#[test]
fn point_and_figure_rounds_each_way_to_the_grid() {
    // 13.9 fills no box above 13, and 10.1 still falls short of 10.
    let source = closes(&[10.0, 13.9, 10.1]);
    let bars = run(&source, &params(Kind::PointFigure, 1.0));
    assert_eq!(oc(&bars), vec![(10.0, 13.0)]);
}

#[test]
fn range_bars_split_at_the_range_and_through_a_gap() {
    let source = vec![
        row(0, 100.0, 104.0, 98.0, 102.0),
        row(1, 102.0, 112.0, 101.0, 110.0),
        // A gap of 40: as many bars as it spans.
        row(2, 150.0, 151.0, 149.0, 150.0),
    ];
    let bars = run(&source, &params(Kind::Range, 10.0));
    assert_eq!(
        oc(&bars),
        vec![
            (100.0, 108.0),
            (108.0, 118.0),
            (118.0, 128.0),
            (128.0, 138.0),
            (138.0, 148.0),
        ]
    );
    for b in &bars {
        assert_eq!(b.high - b.low, 10.0, "{b:?}");
    }
    assert_eq!(bars[0].low, 98.0);
    assert_eq!(
        times(&bars),
        vec![0, 2 * HOUR, 2 * HOUR + 1, 2 * HOUR + 2, 2 * HOUR + 3]
    );
}

#[test]
fn a_range_bar_closes_downward_too() {
    // Falls: open, high, low, close.
    let bars = run(
        &[row(0, 100.0, 103.0, 90.0, 95.0)],
        &params(Kind::Range, 10.0),
    );
    assert_eq!(oc(&bars), vec![(100.0, 93.0)]);
    assert_eq!((bars[0].high, bars[0].low), (103.0, 93.0));
}

#[test]
fn a_bar_time_is_never_repeated_even_when_a_gap_outruns_its_interval() {
    let iv = Interval::parse("1s").unwrap();
    let source: Vec<Bar> = (0..3)
        .map(|i| Bar {
            open: 100.0 + 50.0 * i as f64,
            high: 100.0 + 50.0 * i as f64,
            low: 100.0 + 50.0 * i as f64,
            close: 100.0 + 50.0 * i as f64,
            volume: 1.0,
            date: (1_000 + i).to_string(),
        })
        .collect();
    let built = build(&source, &iv, &params(Kind::Renko, 10.0), &opts()).unwrap();
    let t = times(&built.bars);
    assert!(t.len() > 6);
    assert!(t.windows(2).all(|w| w[1] > w[0]), "{t:?}");
}

#[test]
fn a_box_too_small_is_refused_before_it_costs_anything() {
    let source = closes(&[100.0, 1_000_000.0]);
    for kind in [Kind::Renko, Kind::Range] {
        let e = build(&source, &iv(), &params(kind, 0.001), &opts()).unwrap_err();
        assert!(e.contains("too small"), "{e}");
    }
}

#[test]
fn percent_box_is_rounded_so_a_small_move_keeps_it() {
    assert_eq!(round_box(672.34), 670.0);
    assert_eq!(round_box(668.0), 670.0);
    assert_eq!(round_box(0.012345), 0.012);
    assert_eq!(round_box(1_234_567.0), 1_200_000.0);
    let mut p = params(Kind::Renko, 1.0);
    p.mode = BoxMode::Percent;
    p.box_size = None;
    let at = |p: &Params, price: f64| {
        build(&closes(&[price, price]), &iv(), p, &opts())
            .unwrap()
            .box_size
    };
    assert_eq!(at(&p, 67_234.0), 670.0);
    assert_eq!(at(&p, 67_100.0), 670.0, "the same size after a small move");
    assert_eq!(at(&p, 67_490.0), 670.0);
    p.percent = 2.0;
    assert_eq!(at(&p, 67_234.0), 1_300.0);
}

#[test]
fn atr_box_takes_wilders_average_of_the_closed_bars() {
    let source = vec![
        row(0, 10.0, 12.0, 10.0, 11.0),
        row(1, 11.0, 13.0, 11.0, 12.0),
        row(2, 12.0, 14.0, 11.0, 13.0),
        row(3, 13.0, 20.0, 13.0, 19.0),
    ];
    let mut p = params(Kind::Renko, 1.0);
    p.mode = BoxMode::Atr;
    p.box_size = None;
    p.atr_length = 3;
    // True ranges 2, 2, 3, 7: a mean of 2.33 for the first three, then (2.33 * 2 + 7) / 3.
    assert_eq!(build(&source, &iv(), &p, &opts()).unwrap().box_size, 3.9);
    // The forming bar does not count.
    let mut o = opts();
    o.now_ms = (3 * HOUR + 1) * 1000;
    assert_eq!(build(&source, &iv(), &p, &o).unwrap().box_size, 2.3);
}

#[test]
fn a_pinned_box_outlives_the_price_it_came_from() {
    let mut p = params(Kind::Renko, 1.0);
    p.mode = BoxMode::Percent;
    p.box_size = None;
    let pinned = p
        .pinned(&closes(&[50_000.0, 50_000.0]), &iv(), FAR_FUTURE_MS)
        .unwrap();
    assert_eq!(pinned.box_size, Some(500.0));
    let later = build(&closes(&[90_000.0, 90_000.0]), &iv(), &pinned, &opts()).unwrap();
    assert_eq!(later.box_size, 500.0);
}

#[test]
fn flat_prices_give_no_box_and_say_so() {
    let mut p = params(Kind::Renko, 1.0);
    p.mode = BoxMode::Atr;
    p.box_size = None;
    let e = build(&closes(&[5.0, 5.0, 5.0]), &iv(), &p, &opts()).unwrap_err();
    assert!(e.contains("fixed box"), "{e}");
}

/// Every look along the way, as a poll or a push sees it, must agree with the
/// finished build: what was confirmed never changes, and the open bar keeps its time.
#[test]
fn a_live_tail_shows_the_bars_a_reload_shows() {
    // On a chunk edge, so all of it is one anchor.
    let start = 1_000 * 10;
    let source = walk(900, start);
    for p in all_kinds() {
        let full = run(&source, &p);
        assert!(full.len() > 10, "{:?}: {}", p.kind, full.len());
        for k in (2..source.len()).step_by(7) {
            let mut o = opts();
            // Row k-1 has just opened.
            o.now_ms = (source[k - 1].date.parse::<i64>().unwrap() + 5) * 1000;
            let live = build(&source[..k], &iv(), &p, &o).unwrap().bars;
            let confirmed = live.len().saturating_sub(1);
            assert_eq!(
                times(&live[..confirmed]),
                times(&full[..confirmed]),
                "{:?} at {k}",
                p.kind
            );
            assert_eq!(
                oc(&live[..confirmed]),
                oc(&full[..confirmed]),
                "{:?} at {k}",
                p.kind
            );
            if let Some(open) = live.last() {
                assert_eq!(
                    times(std::slice::from_ref(open))[0],
                    times(&full[confirmed..=confirmed])[0],
                    "{:?} open bar at {k}",
                    p.kind
                );
            }
        }
    }
}

#[test]
fn reads_of_different_lengths_in_one_chunk_give_the_same_bars() {
    // The anchor cuts both at the same bar.
    let chunk = 1_000;
    let start = chunk * 20 - 5_400;
    let source = walk(5_500, start);
    let last = start + 5_499;
    let from = last.div_euclid(chunk) * chunk - 4 * chunk;
    let shorter: Vec<Bar> = source
        .iter()
        .filter(|b| b.date.parse::<i64>().unwrap() / HOUR >= from)
        .cloned()
        .collect();
    assert!(shorter.len() < source.len());
    for p in all_kinds() {
        let a = run(&source, &p);
        let b = run(&shorter, &p);
        assert_eq!(times(&a), times(&b), "{:?}", p.kind);
    }
}

/// When a chunk rolls over the build starts later. What it had already drawn at the
/// live end must not change, or every chart would reload.
#[test]
fn a_later_start_converges_on_the_same_bars() {
    let source = walk(3_000, 0);
    for p in all_kinds() {
        let full = run(&source, &p);
        let later = run(&source[1_000..], &p);
        // Its first bars are where it has only just started.
        let n = later.len() / 2;
        assert!(n >= 20, "{:?}: {n}", p.kind);
        assert_eq!(
            times(&full[full.len() - n..]),
            times(&later[later.len() - n..]),
            "{:?}",
            p.kind
        );
        assert_eq!(
            oc(&full[full.len() - n..]),
            oc(&later[later.len() - n..]),
            "{:?}",
            p.kind
        );
    }
}

#[test]
fn a_page_ends_before_end_time_and_only_lines_may_stay_open() {
    let source = closes(&[100.0, 125.0, 127.0, 140.0, 141.0]);
    let mut o = opts();
    o.end_time_ms = Some(5 * HOUR * 1000);
    let bricks = build(&source, &iv(), &params(Kind::Renko, 10.0), &o)
        .unwrap()
        .bars;
    assert_eq!(
        oc(&bricks),
        vec![(110.0, 120.0), (120.0, 130.0), (130.0, 140.0)]
    );
    // A line is finished by the turn at `end_time`, so the page keeps it.
    let lines = build(&source, &iv(), &params(Kind::Kagi, 10.0), &o)
        .unwrap()
        .bars;
    assert_eq!(oc(&lines), vec![(100.0, 141.0)]);
    o.end_time_ms = Some(3 * HOUR * 1000);
    let cut = build(&source, &iv(), &params(Kind::Renko, 10.0), &o)
        .unwrap()
        .bars;
    assert!(times(&cut).iter().all(|t| *t < 3 * HOUR), "{cut:?}");
}

#[test]
fn source_reads_stay_within_the_fetch_bound() {
    let at = |s: &str| {
        let iv = Interval::parse(s).unwrap();
        source_bars(&iv, 20_000) as i64 * iv.binance_base().1
    };
    assert_eq!(at("1h"), 5_000, "five pages of native klines");
    assert_eq!(at("1d"), 5_000);
    for s in ["7m", "59m", "45m", "2h", "5s", "2d", "365d", "52wk", "12mo"] {
        assert!(at(s) <= 20_000, "{s}: {}", at(s));
        assert!(at(s) > 0, "{s}");
    }
}

#[test]
fn params_are_checked_and_echoed() {
    let parse = |kind, v: Value| Params::parse(kind, Some(&v));
    let d = Params::parse(Kind::Renko, None).unwrap();
    assert_eq!((d.mode, d.atr_length), (BoxMode::Atr, 14));
    assert_eq!(d.to_json(), json!({ "box": "atr", "atr_length": 14 }));
    assert_eq!(
        parse(
            Kind::PointFigure,
            json!({ "box": "percent", "percent": 2.5 })
        )
        .unwrap()
        .to_json(),
        json!({ "box": "percent", "percent": 2.5, "reversal": 3 })
    );
    assert_eq!(
        parse(Kind::LineBreak, json!({ "lines": 5 }))
            .unwrap()
            .to_json(),
        json!({ "lines": 5 })
    );
    assert_eq!(
        parse(Kind::Renko, json!({ "box": "fixed", "box_size": 25 }))
            .unwrap()
            .to_json(),
        json!({ "box": "fixed", "box_size": 25.0 })
    );
    for (v, want) in [
        (json!({ "box": "tick" }), "atr, fixed or percent"),
        (json!({ "box": "fixed" }), "needs box_size"),
        (json!({ "box_size": 0 }), "above 0"),
        (json!({ "box_size": -2 }), "above 0"),
        (json!({ "percent": 0 }), "above 0"),
        (json!({ "percent": 101 }), "at most 100"),
        (json!({ "atr_length": 0 }), "1 to 500"),
        (json!({ "reversal": 21 }), "1 to 20"),
        (json!({ "lines": 1.5 }), "1 to 20"),
        (json!({ "brick": 3 }), "no brick"),
        (json!(3), "an object"),
    ] {
        let e = parse(Kind::Renko, v.clone()).unwrap_err();
        assert!(e.contains(want), "{v}: {e}");
    }
}
