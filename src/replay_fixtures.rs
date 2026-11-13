//! `chartgen replay-fixtures`: replay fixtures from real Binance history (ADR-0068).
//!
//! Reads the 1m klines of a window over Binance REST, through the spot gate, and writes
//! one `<SYMBOL>.json` per symbol in the format `CHARTGEN_REPLAY_FIXTURES` loads
//! (`replay.rs`). A minute Binance has no kline for stays missing: no bar is invented.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chartgen::data::Bar;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

const MINUTE_MS: i64 = 60_000;
const DAY_S: i64 = 86_400;

/// Most symbol-days one run may write: 5 symbols over a 92-day quarter. A replay server
/// holds every 1m bar in memory, about 100 bytes each, so this is about 66 MB, which
/// fits beside the live services on a small host (ADR-0068).
const MAX_SYMBOL_DAYS: i64 = 460;
/// Pause before each request: at most 10 a second, 1200 of the 6000 weight a minute
/// Binance allows an IP, so a live server on the same address keeps the rest.
const PACE: Duration = Duration::from_millis(100);
/// Runs of missing minutes longer than this are reported.
const REPORTED_GAP_MIN: i64 = 5;
/// The most gaps printed per symbol.
const GAPS_PRINTED: usize = 20;
/// Furthest `--shift-days` may move a window: a century.
const MAX_SHIFT_DAYS: i64 = 36_500;
/// Reads of one page while Binance's limits hold requests back.
const ATTEMPTS: u32 = 5;
/// The longest wait for those limits before reading again; a longer one ends the run.
const MAX_WAIT: Duration = Duration::from_secs(300);

#[derive(clap::Args)]
pub struct Args {
    /// Binance spot symbols, comma-separated (e.g. BTCUSDT,ETHUSDT)
    #[arg(long, value_delimiter = ',', required = true)]
    symbols: Vec<String>,
    /// First day of the window, UTC (YYYY-MM-DD)
    #[arg(long)]
    from: NaiveDate,
    /// Last day of the window, UTC, included (YYYY-MM-DD)
    #[arg(long)]
    to: NaiveDate,
    /// Directory to write one <SYMBOL>.json per symbol into
    #[arg(long)]
    out: PathBuf,
    /// Move every timestamp by this many days, so the fixtures do not carry the real dates
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    shift_days: i64,
    /// Allow a --shift-days that is not a multiple of 7, though weekdays and weekly bars change
    #[arg(long)]
    shift_any: bool,
}

/// What a fixture was built from, kept in its file: a rerun over the same window keeps
/// the file instead of reading it again, and a run can be audited afterwards. The
/// replay server does not read it.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Window {
    from: NaiveDate,
    to: NaiveDate,
    shift_days: i64,
}

/// A fixture as written: `replay.rs` reads `symbol` and `bars`.
#[derive(Serialize)]
struct Written<'a> {
    symbol: &'a str,
    window: &'a Window,
    bars: Vec<(i64, f64, f64, f64, f64, f64)>,
}

/// The part of a `*.json` in `--out` that says which window it holds.
#[derive(Deserialize)]
struct Existing {
    window: Option<Window>,
}

struct Plan {
    symbols: Vec<String>,
    window: Window,
    /// The window in ms, `end_ms` excluded.
    from_ms: i64,
    end_ms: i64,
}

/// Write the fixtures `args` asks for.
pub fn run(args: &Args) -> Result<(), String> {
    build(
        args,
        chrono::Utc::now().timestamp_millis(),
        &std::thread::sleep,
    )
}

fn plan(args: &Args, now_ms: i64) -> Result<Plan, String> {
    let mut symbols: Vec<String> = Vec::new();
    for given in &args.symbols {
        let symbol = given.trim().to_uppercase();
        // It names a file, so nothing but letters and digits.
        if symbol.is_empty()
            || symbol.len() > 20
            || !symbol.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return Err(format!("{given:?} is not a Binance spot symbol"));
        }
        if symbols.contains(&symbol) {
            return Err(format!("{symbol} is named twice"));
        }
        symbols.push(symbol);
    }
    if symbols.is_empty() {
        return Err("--symbols names no symbol".into());
    }
    if args.to < args.from {
        return Err("--to is before --from".into());
    }
    let days = (args.to - args.from).num_days() + 1;
    let symbol_days = days * symbols.len() as i64;
    if symbol_days > MAX_SYMBOL_DAYS {
        return Err(format!(
            "{} symbols over {days} days is {symbol_days} symbol-days, and the most is {MAX_SYMBOL_DAYS} \
             (5 symbols over a 92-day quarter): a replay server holds every 1m bar in memory, \
             about 100 bytes each, so this bounds it to about 66 MB",
            symbols.len()
        ));
    }
    let from_s = args
        .from
        .and_time(chrono::NaiveTime::MIN)
        .and_utc()
        .timestamp();
    let end_ms = (from_s + days * DAY_S) * 1000;
    if end_ms > now_ms {
        return Err(format!(
            "{} has not ended yet: a fixture holds closed bars only, so --to must be a day before today (UTC)",
            args.to
        ));
    }
    if args.shift_days.abs() > MAX_SHIFT_DAYS || from_s + args.shift_days * DAY_S < 0 {
        return Err(format!(
            "--shift-days {} moves the window before 1970 or by more than {MAX_SHIFT_DAYS} days",
            args.shift_days
        ));
    }
    if args.shift_days % 7 != 0 && !args.shift_any {
        return Err(format!(
            "--shift-days {} is not a multiple of 7: the weekdays would move, so the agent would see \
             a weekend's quiet on a weekday, and each weekly bar (which opens on Monday) would hold \
             other days than it did. Use a multiple of 7, or add --shift-any to accept that",
            args.shift_days
        ));
    }
    Ok(Plan {
        symbols,
        window: Window {
            from: args.from,
            to: args.to,
            shift_days: args.shift_days,
        },
        from_ms: from_s * 1000,
        end_ms,
    })
}

/// `sleep` is `std::thread::sleep` but in tests.
fn build(args: &Args, now_ms: i64, sleep: &dyn Fn(Duration)) -> Result<(), String> {
    let plan = plan(args, now_ms)?;
    std::fs::create_dir_all(&args.out)
        .map_err(|e| format!("cannot create {}: {e}", args.out.display()))?;
    one_window(&args.out, &plan.window)?;
    for symbol in &plan.symbols {
        let path = args.out.join(format!("{symbol}.json"));
        if path.exists() {
            println!("{symbol}: kept {}, built for this window", path.display());
            continue;
        }
        let began = Instant::now();
        let bars = minutes(plan.from_ms, plan.end_ms, |start, last| {
            retried(
                || {
                    sleep(PACE);
                    crate::fetch::fetch_binance_minutes(symbol, start, last)
                },
                crate::fetch::binance_wait,
                sleep,
            )
        })
        .map_err(|e| {
            format!("{symbol}: {e}. The fixtures already written are kept, and a rerun reads only the rest")
        })?;
        if bars.is_empty() {
            return Err(format!(
                "{symbol}: Binance has no 1m bars in this window. Is it a spot symbol, listed by then?"
            ));
        }
        let bytes = write(&path, symbol, &plan.window, &bars)?;
        let found = gaps(&bars, plan.from_ms, plan.end_ms);
        let missing = (plan.end_ms - plan.from_ms) / MINUTE_MS - bars.len() as i64;
        println!(
            "{symbol}: {} bars, {missing} minutes missing, {} gaps over {REPORTED_GAP_MIN} min, {:.1} MB, {:.1} s -> {}",
            bars.len(),
            found.len(),
            bytes as f64 / 1e6,
            began.elapsed().as_secs_f64(),
            path.display()
        );
        for (start, length) in found.iter().take(GAPS_PRINTED) {
            println!(
                "  no bars for {length} min from {} (real time)",
                chartgen::drawings::iso(*start)
            );
        }
        if found.len() > GAPS_PRINTED {
            println!("  and {} more gaps", found.len() - GAPS_PRINTED);
        }
    }
    Ok(())
}

/// Refuse an `--out` holding a `*.json` of another window, or of none recorded: a replay
/// server loads every `*.json` in its directory as one run, and a file of another
/// window may be a run someone means to audit.
fn one_window(out: &Path, window: &Window) -> Result<(), String> {
    let entries = std::fs::read_dir(out).map_err(|e| format!("{}: {e}", out.display()))?;
    for path in entries
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
    {
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let existing = serde_json::from_str::<Existing>(&text).ok();
        if existing.and_then(|e| e.window).as_ref() != Some(window) {
            return Err(format!(
                "{} is not a fixture of this window, and a replay server loads every *.json in its \
                 directory as one run: write to a fresh --out directory",
                path.display()
            ));
        }
    }
    Ok(())
}

/// Read a page, and while Binance's limits hold requests back (`wait`), wait and read it
/// again, at most `ATTEMPTS` times and never longer than `MAX_WAIT` at once. Failing
/// anyway ends the run, saying how long to wait: a rerun at once would meet the same
/// limit, and requests that keep coming get the IP banned (418), which a live server on
/// the same address would share.
fn retried(
    read: impl Fn() -> Result<Vec<Bar>, String>,
    wait: impl Fn() -> Option<Duration>,
    sleep: &dyn Fn(Duration),
) -> Result<Vec<Bar>, String> {
    let mut attempt = 1;
    loop {
        let error = match read() {
            Ok(bars) => return Ok(bars),
            Err(e) => e,
        };
        let Some(left) = wait() else {
            return Err(error);
        };
        let secs = left.as_millis().div_ceil(1000);
        if attempt == ATTEMPTS || left > MAX_WAIT {
            return Err(format!("{error}; wait {secs} s before running again"));
        }
        eprintln!("{error}; reading the page again in {secs} s");
        sleep(left);
        attempt += 1;
    }
}

fn open_ms(bar: &Bar) -> i64 {
    bar.date.parse::<i64>().unwrap_or(0) * 1000
}

/// The 1m bars opening in [`from_ms`, `end_ms`), oldest first, read forward one page
/// at a time. `page(start, last)` reads the bars opening from `start` to `last`.
fn minutes(
    from_ms: i64,
    end_ms: i64,
    mut page: impl FnMut(i64, i64) -> Result<Vec<Bar>, String>,
) -> Result<Vec<Bar>, String> {
    let mut bars = Vec::new();
    let mut start = from_ms;
    while start < end_ms {
        let chunk = page(start, end_ms - 1)?;
        let Some(next) = chunk.last().map(|b| open_ms(b) + MINUTE_MS) else {
            break;
        };
        // A page that does not move on would be asked for again and again.
        if next <= start {
            return Err("Binance answered a page that does not move forward".into());
        }
        bars.extend(
            chunk
                .into_iter()
                .filter(|b| (start..end_ms).contains(&open_ms(b))),
        );
        start = next;
    }
    Ok(bars)
}

/// The runs of more than `REPORTED_GAP_MIN` missing minutes in [`from_ms`, `end_ms`):
/// (the first missing minute in ms, how many minutes).
fn gaps(bars: &[Bar], from_ms: i64, end_ms: i64) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let mut expected = from_ms;
    for t in bars.iter().map(open_ms).chain([end_ms]) {
        let missing = (t - expected) / MINUTE_MS;
        if missing > REPORTED_GAP_MIN {
            out.push((expected, missing));
        }
        expected = t + MINUTE_MS;
    }
    out
}

/// Write the fixture through a `.part` file, so an interrupted write leaves no fixture.
/// Returns its size in bytes.
fn write(path: &Path, symbol: &str, window: &Window, bars: &[Bar]) -> Result<u64, String> {
    let shift_s = window.shift_days * DAY_S;
    let fixture = Written {
        symbol,
        window,
        bars: bars
            .iter()
            .map(|b| {
                let t = open_ms(b) / 1000 + shift_s;
                (t, b.open, b.high, b.low, b.close, b.volume)
            })
            .collect(),
    };
    let part = path.with_extension("json.part");
    let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", part.display());
    let mut out = std::io::BufWriter::new(std::fs::File::create(&part).map_err(|e| fail(&e))?);
    serde_json::to_writer(&mut out, &fixture).map_err(|e| fail(&e))?;
    out.flush().map_err(|e| fail(&e))?;
    drop(out);
    std::fs::rename(&part, path).map_err(|e| fail(&e))?;
    std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::Replay;
    use clap::Parser;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// 2026-01-01T00:00:00Z.
    const JAN_1: i64 = 1_767_225_600;
    const NOW_MS: i64 = (JAN_1 + 30 * DAY_S) * 1000;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        args: Args,
    }

    fn args(line: &str) -> Args {
        Cli::try_parse_from(line.split_whitespace())
            .unwrap_or_else(|e| panic!("{line}: {e}"))
            .args
    }

    fn planned(line: &str) -> Result<Plan, String> {
        plan(&args(&format!("x --out o {line}")), NOW_MS)
    }

    #[test]
    fn arguments_are_checked() {
        let p = planned("--symbols btcusdt,ETHUSDT --from 2026-01-01 --to 2026-01-02").unwrap();
        assert_eq!(p.symbols, ["BTCUSDT", "ETHUSDT"]);
        assert_eq!(
            (p.from_ms, p.end_ms),
            (JAN_1 * 1000, (JAN_1 + 2 * DAY_S) * 1000)
        );
        let p = planned("--symbols BTCUSDT --from 2026-01-01 --to 2026-01-01 --shift-days -7");
        assert_eq!(p.unwrap().window.shift_days, -7);

        for (line, why) in [
            (
                "--symbols BTCUSDT --from 2026-01-02 --to 2026-01-01",
                "before --from",
            ),
            (
                "--symbols ../x --from 2026-01-01 --to 2026-01-01",
                "not a Binance",
            ),
            (
                "--symbols BTCUSDT,btcusdt --from 2026-01-01 --to 2026-01-01",
                "twice",
            ),
            // The last day must have ended: here "now" is the start of Jan 31.
            (
                "--symbols BTCUSDT --from 2026-01-01 --to 2026-01-31",
                "not ended",
            ),
            (
                "--symbols BTCUSDT --from 2024-01-01 --to 2025-12-31",
                "symbol-days",
            ),
            (
                "--symbols A1,A2,A3,A4,A5,A6 --from 2025-10-01 --to 2025-12-31",
                "symbol-days",
            ),
            (
                "--symbols BTCUSDT --from 2026-01-01 --to 2026-01-01 --shift-days -30000",
                "1970",
            ),
            (
                "--symbols BTCUSDT --from 2026-01-01 --to 2026-01-01 --shift-days 10",
                "multiple of 7",
            ),
        ] {
            let err = planned(line).err().unwrap_or_else(|| panic!("{line}"));
            assert!(err.contains(why), "{line}: {err}");
        }
        let p = "--symbols BTCUSDT --from 2026-01-01 --to 2026-01-01 --shift-days 10 --shift-any";
        assert_eq!(planned(p).unwrap().window.shift_days, 10);
        // 5 symbols over a 92-day quarter is the most there is.
        planned("--symbols A1,A2,A3,A4,A5 --from 2025-08-01 --to 2025-10-31").unwrap();
        assert!(
            Cli::try_parse_from(["x", "--out", "o", "--from", "2026-01-01", "--to", "x"]).is_err()
        );
    }

    /// What the pretend Binance answers instead of klines to its `n`th request (from 1):
    /// a status and `Retry-After`.
    type Trouble = fn(usize) -> Option<(u16, Option<&'static str>)>;

    /// What a test's sleep was asked for.
    type Slept = Arc<std::sync::Mutex<Vec<Duration>>>;

    /// A pretend Binance with 1m klines from `JAN_1`, without the minutes in `hole`,
    /// answering as the real one does to `startTime`, `endTime` and `limit`. Returns its
    /// count of requests, and a sleep that moves the gate's clock on instead of waiting
    /// and keeps what it was asked for.
    fn pretend(
        hole: std::ops::Range<i64>,
        trouble: Trouble,
    ) -> (Arc<AtomicUsize>, impl Fn(Duration), Slept) {
        let requests = Arc::new(AtomicUsize::new(0));
        let counted = requests.clone();
        let ahead = crate::fetch::pretend_binance(move |url| {
            let n = counted.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some((status, retry_after)) = trouble(n) {
                return (
                    status,
                    retry_after,
                    r#"{"code":-1003,"msg":"Too many"}"#.into(),
                );
            }
            let param = |name: &str| -> i64 {
                url.split(['?', '&'])
                    .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
                    .unwrap()
                    .parse()
                    .unwrap()
            };
            let (start, last, limit) = (param("startTime"), param("endTime"), param("limit"));
            assert!(url.contains("symbol=BTCUSDT&interval=1m&"), "{url}");
            let rows: Vec<String> = (start.max(JAN_1 * 1000).div_euclid(MINUTE_MS)..)
                .map(|m| m * MINUTE_MS)
                .filter(|t| !hole.contains(&((t / 1000 - JAN_1) / 60)))
                .take_while(|t| *t <= last)
                .take(limit as usize)
                .map(|t| {
                    let p = 100.0 + ((t / 1000 - JAN_1) / 60 % 50) as f64 * 0.25;
                    format!(
                        r#"[{t},"{p:.8}","{:.8}","{:.8}","{:.8}","12.5",{},"1250",9,"6","600","0"]"#,
                        p + 1.0,
                        p - 1.0,
                        p + 0.5,
                        t + MINUTE_MS - 1
                    )
                })
                .collect();
            (200, None, format!("[{}]", rows.join(",")))
        });
        let slept = Arc::new(std::sync::Mutex::new(Vec::new()));
        let kept = slept.clone();
        let sleep = move |d: Duration| {
            ahead.fetch_add(d.as_millis() as u64, Ordering::SeqCst);
            kept.lock().unwrap().push(d);
        };
        (requests, sleep, slept)
    }

    fn no_trouble(_: usize) -> Option<(u16, Option<&'static str>)> {
        None
    }

    fn one_day(dir: &Path) -> Args {
        args(&format!(
            "x --symbols BTCUSDT --from 2026-01-01 --to 2026-01-01 --out {}",
            dir.display()
        ))
    }

    #[test]
    fn written_fixtures_load_back_shifted_and_are_kept_on_a_rerun() {
        // Two days, with no bars from 01:00 to 04:00 on the first.
        let (requests, sleep, _) = pretend(60..240, no_trouble);
        let dir = crate::test_dir::new("replay_fixtures");
        let line = |shift: i64| {
            format!(
                "x --symbols BTCUSDT --from 2026-01-01 --to 2026-01-02 --out {} --shift-days {shift}",
                dir.display()
            )
        };
        build(&args(&line(7)), NOW_MS, &sleep).unwrap();
        // 2880 minutes less 180 is 2700 bars: 3 pages of 1000.
        assert_eq!(requests.load(Ordering::SeqCst), 3);

        let replay = Replay::load(&dir).unwrap();
        let bars = replay.minute_bars("BTCUSDT", i64::MIN, i64::MAX / 2);
        assert_eq!(bars.len(), 2700);
        let week = 7 * DAY_S;
        assert_eq!(bars[0].date, (JAN_1 + week).to_string());
        assert_eq!(bars[60].date, (JAN_1 + week + 4 * 3600).to_string());
        assert_eq!(bars[2699].date, (JAN_1 + week + 2 * DAY_S - 60).to_string());
        assert_eq!(
            (
                bars[1].open,
                bars[1].high,
                bars[1].low,
                bars[1].close,
                bars[1].volume
            ),
            (100.25, 101.25, 99.25, 100.75, 12.5)
        );
        assert_eq!(
            gaps(
                bars,
                (JAN_1 + week) * 1000,
                (JAN_1 + week + 2 * DAY_S) * 1000
            ),
            [((JAN_1 + week + 3600) * 1000, 180)]
        );

        // The same window again reads nothing; another is not written beside this one.
        build(&args(&line(7)), NOW_MS, &sleep).unwrap();
        assert_eq!(requests.load(Ordering::SeqCst), 3);
        let err = build(&args(&line(0)), NOW_MS, &sleep).unwrap_err();
        assert!(err.contains("fresh --out"), "{err}");
        assert_eq!(requests.load(Ordering::SeqCst), 3);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no .part left");
    }

    #[test]
    fn a_directory_with_a_fixture_of_no_recorded_window_is_refused() {
        let (requests, sleep, _) = pretend(0..0, no_trouble);
        let dir = crate::test_dir::new("replay_fixtures_hand_made");
        std::fs::write(
            dir.join("ETHUSDT.json"),
            r#"{"symbol":"ETHUSDT","bars":[[1767225600,1.0,2.0,0.5,1.5,3.0]]}"#,
        )
        .unwrap();
        let err = build(&one_day(&dir), NOW_MS, &sleep).unwrap_err();
        assert!(
            err.contains("ETHUSDT.json") && err.contains("fresh"),
            "{err}"
        );
        assert_eq!(requests.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_429_reads_the_same_page_again_after_retry_after() {
        let (requests, sleep, slept) = pretend(0..0, |n| (n == 2).then_some((429, Some("7"))));
        let dir = crate::test_dir::new("replay_fixtures_429");
        build(&one_day(&dir), NOW_MS, &sleep).unwrap();
        // Two pages for 1440 bars, and the second read twice.
        assert_eq!(requests.load(Ordering::SeqCst), 3);
        // The gate's cooldown, a hair under 7 s by the time it is asked.
        let waited = slept.lock().unwrap().iter().copied().max().unwrap();
        assert!(
            waited > Duration::from_secs(6) && waited <= Duration::from_secs(7),
            "{waited:?}"
        );
        let replay = Replay::load(&dir).unwrap();
        assert_eq!(
            replay.minute_bars("BTCUSDT", i64::MIN, i64::MAX / 2).len(),
            1440
        );
    }

    #[test]
    fn a_ban_or_a_limit_that_stays_ends_the_run_saying_how_long_to_wait() {
        // A ban longer than one is worth waiting for: no second request.
        let (requests, sleep, _) = pretend(0..0, |_| Some((418, Some("86400"))));
        let dir = crate::test_dir::new("replay_fixtures_418");
        let err = build(&one_day(&dir), NOW_MS, &sleep).unwrap_err();
        assert!(err.contains("wait 86400 s before running again"), "{err}");
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);

        // 429 after 429: as many reads as there are attempts, then the run ends.
        let (requests, sleep, _) = pretend(0..0, |_| Some((429, Some("2"))));
        let dir = crate::test_dir::new("replay_fixtures_429s");
        let err = build(&one_day(&dir), NOW_MS, &sleep).unwrap_err();
        assert!(err.contains("wait 2 s before running again"), "{err}");
        assert_eq!(requests.load(Ordering::SeqCst), ATTEMPTS as usize);
    }

    #[test]
    fn a_failed_read_writes_nothing_and_is_not_read_again() {
        let (requests, sleep, _) = pretend(0..0, |_| Some((400, None)));
        let dir = crate::test_dir::new("replay_fixtures_failed");
        let err = build(&one_day(&dir), NOW_MS, &sleep).unwrap_err();
        assert!(
            err.contains("BTCUSDT") && err.contains("status 400"),
            "{err}"
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn gaps_count_the_edges_of_the_window() {
        let at = |m: i64| Bar {
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 0.0,
            date: (JAN_1 + m * 60).to_string(),
        };
        let bars = [at(10), at(11), at(15), at(30)];
        let ms = |m: i64| (JAN_1 + m * 60) * 1000;
        // 10 before the first bar, 3 between 11 and 15 (not reported), 14, then 9 to the end.
        assert_eq!(
            gaps(&bars, ms(0), ms(40)),
            [(ms(0), 10), (ms(16), 14), (ms(31), 9)]
        );
    }
}
