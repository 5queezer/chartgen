use super::*;
use crate::chart_push::{Limits as HubLimits, PushSource};
use chartgen::data::{self, OhlcvData};
use chartgen::webhook::{Policy, Timing};
use futures_util::future::BoxFuture;
use std::collections::BTreeMap;
use tokio::sync::mpsc;

/// A hub source that connects to nothing.
struct Quiet;

impl PushSource for Quiet {
    fn feed_interval(&self, symbol: &str, timeframe: &str) -> Result<String, String> {
        if !symbol.ends_with("USDT") {
            return Err("not pushable".into());
        }
        Ok(timeframe.to_string())
    }
    fn seed(&self, symbol: &str, timeframe: &str, count: usize) -> Result<OhlcvData, String> {
        data::sample_ohlcv(symbol, timeframe, count, None, fetch::now_ms())
    }
    fn run(
        &self,
        _symbol: String,
        _interval: String,
        tx: mpsc::Sender<FeedEvent>,
    ) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            tx.closed().await;
        })
    }
}

/// A hub source that sends what a test tells it to, as the real feed would.
#[derive(Default)]
struct Told {
    feeds: std::sync::Mutex<Vec<mpsc::Sender<FeedEvent>>>,
}

impl Told {
    async fn send(&self, event: FeedEvent) {
        let feeds = self.feeds.lock().unwrap().clone();
        for tx in feeds {
            tx.send(event.clone()).await.unwrap();
        }
    }
}

impl PushSource for Told {
    fn feed_interval(&self, _symbol: &str, timeframe: &str) -> Result<String, String> {
        Ok(timeframe.to_string())
    }
    fn seed(&self, symbol: &str, timeframe: &str, count: usize) -> Result<OhlcvData, String> {
        data::sample_ohlcv(symbol, timeframe, count, None, fetch::now_ms())
    }
    fn run(
        &self,
        _symbol: String,
        _interval: String,
        tx: mpsc::Sender<FeedEvent>,
    ) -> BoxFuture<'static, ()> {
        self.feeds.lock().unwrap().push(tx.clone());
        Box::pin(async move {
            tx.closed().await;
        })
    }
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..600 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting: {what}");
}

/// A directory for a test's store file, removed when the test ends.
struct Dir(std::path::PathBuf);

impl Dir {
    fn db(&self) -> std::path::PathBuf {
        self.0.join("store.db")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A store in a file, for a test that restarts the service on it. Keep the `Dir` for
/// as long as the store is used.
fn store_at(name: &str) -> (&'static Store, Dir) {
    let dir = crate::test_dir::new(format!("chartgen-setups-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = Dir(dir);
    (reopen(&dir.db()), dir)
}

/// A store in memory, for every test that does not restart: it leaves no file.
fn mem() -> &'static Store {
    Box::leak(Box::new(Store::in_memory().unwrap()))
}

fn reopen(path: &std::path::Path) -> &'static Store {
    Box::leak(Box::new(Store::open(path).unwrap()))
}

fn hub() -> Arc<ChartHub> {
    ChartHub::new(Arc::new(Quiet), HubLimits::default())
}

fn live_on(
    store: &'static Store,
    hub: Arc<ChartHub>,
    access: Access,
    limits: Limits,
    alerts: Option<Arc<ServerAlerts>>,
) -> Arc<Setups> {
    Setups::new(
        store,
        Some(hub),
        None,
        DataSource::Sample,
        access,
        limits,
        alerts,
    )
}

fn live() -> Arc<Setups> {
    live_on(mem(), hub(), Access::parse(None), Limits::default(), None)
}

fn call(s: &Setups, tool: &str, args: Value, owner: &str) -> Result<Value, String> {
    let resp = s.call_tool(tool, None, &args, Some(owner));
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    if resp["result"]["isError"] == json!(true) {
        Err(text)
    } else {
        Ok(serde_json::from_str(&text).unwrap())
    }
}

/// The last sample price of a symbol, to put levels around.
fn price(symbol: &str) -> f64 {
    data::sample_ohlcv(symbol, "1m", 1, None, fetch::now_ms())
        .unwrap()
        .bars[0]
        .close
}

/// A long below the market: entry 2 to 3 % under `p`, wrong 5 % under, target 5 % over.
fn long_on(symbol: &str) -> Value {
    let p = price(symbol);
    json!({
        "symbol": symbol, "interval": "1h", "direction": "long", "thesis": "example thesis",
        "entry": { "low": p * 0.97, "high": p * 0.98 },
        "invalidation": p * 0.95, "targets": [p * 1.05],
        "expires_at": fetch::now_ms() + 86_400_000,
    })
}

fn long() -> Value {
    long_on("BTCUSDT")
}

/// The price `long_on` built a setup around. Bars a test sends are placed against
/// this, never against a second reading of the sample series, which moves by the second.
fn mid(setup: &Value) -> f64 {
    setup["entry"]["high"].as_f64().unwrap() / 0.98
}

fn created(s: &Setups, owner: &str) -> Value {
    call(s, "create_setup", long(), owner).unwrap()
}

fn id_of(setup: &Value) -> String {
    setup["id"].as_str().unwrap().to_string()
}

fn key() -> FeedKey {
    ("BTCUSDT".to_string(), "1m".to_string())
}

/// The open time of the first bar a setup takes.
fn first_bar(setup: &Value) -> i64 {
    (setup["created_at"].as_i64().unwrap() + MINUTE_MS - 1).div_euclid(MINUTE_MS) * MINUTE_MS
}

fn bar(time: i64, open: f64, high: f64, low: f64, close: f64) -> MinuteBar {
    MinuteBar {
        time,
        open,
        high,
        low,
        close,
    }
}

fn errors(text: &str) -> Vec<String> {
    let v: Value = serde_json::from_str(text).unwrap_or_else(|_| panic!("not problems: {text}"));
    v["setup_errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap().to_string())
        .collect()
}

fn kinds(setup: &Value) -> Vec<&str> {
    setup["log"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect()
}

/// On a server without alerts the setups' own count is what the operator page shows
/// (ADR-0063).
#[tokio::test]
async fn the_pairs_open_setups_watch_are_counted_against_the_limit() {
    let s = live();
    assert_eq!(s.watched_pairs(), (0, Limits::default().pairs_total));
    created(&s, "owner");
    assert_eq!(s.watched_pairs(), (1, Limits::default().pairs_total));
}

#[tokio::test]
async fn a_setup_is_created_read_and_listed() {
    let s = live();
    let setup = created(&s, "owner");
    assert_eq!(setup["status"], "watching");
    assert_eq!(setup["symbol"], "BTCUSDT");
    assert_eq!(
        setup["costs"],
        json!({ "fee_bps_per_side": 5.0, "slippage_bps_per_side": 2.0 })
    );
    assert!(setup.get("owner").is_none());
    assert_eq!(kinds(&setup), ["created"]);

    let got = call(&s, "get_setup", json!({ "id": id_of(&setup) }), "owner").unwrap();
    assert_eq!(got, setup);
    let list = call(&s, "list_setups", json!({}), "owner").unwrap();
    assert_eq!(list["enabled"], true);
    assert_eq!(list["total"], 1);
    assert_eq!(list["setups"][0]["id"], setup["id"]);
    assert!(list["setups"][0].get("log").is_none());
    // The symbol's 1m feed is watched from now on.
    assert_eq!(s.pairs(), HashSet::from([key()]));
}

#[tokio::test]
async fn every_problem_is_reported_at_once_and_nothing_is_stored() {
    let s = live();
    let mut args = long();
    args.as_object_mut().unwrap().remove("invalidation");
    args["symbol"] = json!("AAPL");
    args["status"] = json!("won");
    args["drawings"] = json!([{ "tool": "no_such_tool", "points": [] }]);
    let text = call(&s, "create_setup", args, "owner").unwrap_err();
    assert_eq!(
        errors(&text),
        ["status", "invalidation", "symbol", "drawings[0]"]
    );
    assert!(text.contains("nothing was stored"));
    assert_eq!(
        call(&s, "list_setups", json!({}), "owner").unwrap()["total"],
        0
    );
    assert!(s.store.setups().unwrap().is_empty());
}

#[tokio::test]
async fn drawings_are_checked_by_the_chart_validator() {
    let s = live();
    let now = fetch::now_ms();
    // A wrong point count, and a time before the first bar read.
    for bad in [
        json!([{ "tool": "segment", "points": [{ "time": now, "price": 1.0 }] }]),
        json!([{ "tool": "horizontalStraightLine", "points": [{ "time": "2001-01-01", "price": 1.0 }] }]),
    ] {
        let mut args = long();
        args["drawings"] = bad.clone();
        let text = call(&s, "create_setup", args, "owner").unwrap_err();
        let ours: Value = serde_json::from_str(&text).unwrap();
        let bars = fetch::fetch_ohlcv(DataSource::Sample, "BTCUSDT", "1h", CONTEXT_BARS, None)
            .unwrap()
            .data
            .bars;
        let theirs = match drawings::parse(&bad, now) {
            Err(problems) => problems,
            Ok(requests) => drawings::resolve(requests, &bars).err().unwrap(),
        };
        assert_eq!(ours["setup_errors"][0]["message"], json!(theirs[0].message));
        assert_eq!(ours["setup_errors"][0]["field"], "drawings[0]");
    }

    let mut args = long();
    let p = price("BTCUSDT");
    args["drawings"] = json!([{ "tool": "horizontalStraightLine", "points": [{ "time": now - 3_600_000, "price": p }] }]);
    let setup = call(&s, "create_setup", args, "owner").unwrap();
    assert_eq!(setup["drawings"][0]["tool"], "horizontalStraightLine");
    assert_eq!(setup["drawings"][0]["points"][0]["price"], json!(p));
}

#[tokio::test]
async fn another_owner_cannot_read_revise_or_cancel() {
    let s = live();
    let id = id_of(&created(&s, "owner"));
    let missing = "00000000-0000-4000-8000-000000000000";
    for (tool, extra) in [
        ("get_setup", json!({})),
        (
            "revise_setup",
            json!({ "reason": "mine now", "thesis": "taken" }),
        ),
        ("cancel_setup", json!({ "reason": "mine now" })),
    ] {
        let args = |id: &str| {
            let mut a = extra.clone();
            a["id"] = json!(id);
            a
        };
        let foreign = call(&s, tool, args(&id), "stranger").unwrap_err();
        let absent = call(&s, tool, args(missing), "stranger").unwrap_err();
        assert_eq!(
            foreign.replace(&id, "X"),
            absent.replace(missing, "X"),
            "{tool}"
        );
    }
    assert_eq!(
        call(&s, "list_setups", json!({}), "stranger").unwrap()["total"],
        0
    );
    let mine = call(&s, "get_setup", json!({ "id": id }), "owner").unwrap();
    assert_eq!(mine["thesis"], "example thesis");
    assert_eq!(mine["pending"], json!([]));
    assert!(
        s.call_tool("list_setups", None, &json!({}), None)["result"]["isError"]
            .as_bool()
            .unwrap()
    );
}

#[tokio::test]
async fn non_binance_symbol_is_refused() {
    let s = live();
    let text = call(&s, "create_setup", long_on("AAPL"), "owner").unwrap_err();
    assert_eq!(errors(&text), ["symbol"]);
}

#[tokio::test]
async fn limits_hold_per_owner() {
    let limits = Limits {
        open_per_owner: 3,
        symbols_per_owner: 2,
        agent_revisions_per_setup: 1,
        changes_per_minute: 7,
        ..Limits::default()
    };
    let s = live_on(mem(), hub(), Access::parse(None), limits, None);
    let first = id_of(&created(&s, "owner"));
    call(&s, "create_setup", long_on("ETHUSDT"), "owner").unwrap();
    // A third symbol is one too many; a second setup on a watched one is not.
    let symbols = call(&s, "create_setup", long_on("SOLUSDT"), "owner").unwrap_err();
    assert!(symbols.contains("at most 2 symbols per owner"), "{symbols}");
    created(&s, "owner");
    let open = created_err(&s, "owner");
    assert!(open.contains("at most 3 open setups per owner"), "{open}");
    // Another owner has its own.
    call(&s, "create_setup", long_on("SOLUSDT"), "other").unwrap();

    let revise = |reason: &str| {
        let args = json!({ "id": first, "reason": reason, "thesis": reason });
        call(&s, "revise_setup", args, "owner")
    };
    revise("first change").unwrap();
    let revisions = revise("second change").unwrap_err();
    assert!(
        revisions.contains("at most 1 revisions per setup"),
        "{revisions}"
    );
    // Seven changes this minute: five creates and two revisions, refused ones included.
    let rate = revise("third change").unwrap_err();
    assert!(
        rate.contains("at most 7 setup changes per minute"),
        "{rate}"
    );
}

fn created_err(s: &Setups, owner: &str) -> String {
    call(s, "create_setup", long(), owner).unwrap_err()
}

#[tokio::test]
async fn totals_hold_and_keep_closed_setups() {
    let limits = Limits {
        setups_per_owner: 1,
        ..Limits::default()
    };
    let s = live_on(mem(), hub(), Access::parse(None), limits, None);
    let setup = created(&s, "owner");
    let p = mid(&setup);
    // Opens beyond the invalidation: closed, unfilled.
    s.on_bar(
        &key(),
        bar(first_bar(&setup), p * 0.9, p * 0.91, p * 0.89, p * 0.9),
    );
    let closed = call(&s, "get_setup", json!({ "id": id_of(&setup) }), "owner").unwrap();
    assert_eq!(closed["status"], "invalidated");
    // The closed one still counts, and is not dropped to make room.
    let full = created_err(&s, "owner");
    assert!(
        full.contains("at most 1 setups per owner, closed ones included"),
        "{full}"
    );
    assert_eq!(
        call(&s, "list_setups", json!({}), "owner").unwrap()["total"],
        1
    );
}

#[tokio::test]
async fn bytes_are_bounded_per_record_revision_and_owner() {
    let small = |limits: Limits| live_on(mem(), hub(), Access::parse(None), limits, None);
    let record = small(Limits {
        record_bytes: 300,
        ..Limits::default()
    });
    let text = created_err(&record, "owner");
    assert!(text.contains("a setup holds at most 300 bytes"), "{text}");

    let owner = small(Limits {
        bytes_per_owner: 1200,
        ..Limits::default()
    });
    created(&owner, "owner");
    let text = created_err(&owner, "owner");
    assert!(text.contains("hold at most 1200 bytes"), "{text}");
    created(&owner, "other");

    let revision = small(Limits {
        revision_bytes: 200,
        ..Limits::default()
    });
    let id = id_of(&created(&revision, "owner"));
    let long_thesis = "x".repeat(300);
    let args = json!({ "id": id, "reason": "more words", "thesis": long_thesis });
    let text = call(&revision, "revise_setup", args, "owner").unwrap_err();
    assert!(
        text.contains("one revision holds at most 200 bytes"),
        "{text}"
    );
    // Refused whole: the thesis is as it was, and so is the log.
    let got = call(&revision, "get_setup", json!({ "id": id }), "owner").unwrap();
    assert_eq!(got["thesis"], "example thesis");
    assert_eq!(kinds(&got), ["created"]);
}

#[tokio::test]
async fn an_owner_the_operator_did_not_list_cannot_create() {
    let access = Access::parse(Some("github:1"));
    let s = live_on(mem(), hub(), access, Limits::default(), None);
    let text = created_err(&s, "github:2");
    assert!(text.contains("its operator allowed"), "{text}");
    assert_eq!(
        call(&s, "list_setups", json!({}), "github:2").unwrap()["can_create"],
        false
    );
    created(&s, "github:1");
}

/// The operator's list is read at startup. An owner taken off it keeps its setups, but
/// they hold no feed and are not scored, as ADR-0042 stops such an owner's alerts.
#[tokio::test]
async fn setups_of_an_owner_taken_off_the_list_hold_no_feed_and_are_not_scored() {
    let (store, dir) = store_at("unlisted");
    let both = Access::parse(Some("github:1,github:2"));
    let s = live_on(store, hub(), both, Limits::default(), None);
    let kept = created(&s, "github:1");
    let dropped = call(&s, "create_setup", long_on("ETHUSDT"), "github:2").unwrap();
    assert_eq!(s.pairs().len(), 2);
    drop(s);

    let only_one = Access::parse(Some("github:1"));
    let s = live_on(reopen(&dir.db()), hub(), only_one, Limits::default(), None);
    assert_eq!(s.pairs(), HashSet::from([key()]));
    // A bar through its entry and its invalidation leaves it as it was.
    let (t, p) = (first_bar(&dropped), mid(&dropped));
    let eth = ("ETHUSDT".to_string(), "1m".to_string());
    s.on_bar(&eth, bar(t, p, p * 1.2, p * 0.5, p));
    let still = call(
        &s,
        "get_setup",
        json!({ "id": id_of(&dropped) }),
        "github:2",
    )
    .unwrap();
    assert_eq!(still, dropped, "readable by its owner, and untouched");
    // The listed owner's setup runs as before.
    let (t, p) = (first_bar(&kept), mid(&kept));
    s.on_bar(&key(), bar(t, p, p * 1.001, p * 0.999, p));
    let moved = call(&s, "get_setup", json!({ "id": id_of(&kept) }), "github:1").unwrap();
    assert_eq!(moved["scored_through"], json!(t));
    drop(s);

    // Back on the list, it is watched again and is behind like any setup the server was
    // away from: the bars it missed are read if they are in reach, else named as a gap.
    let s = live_on(
        reopen(&dir.db()),
        hub(),
        Access::parse(Some("github:1,github:2")),
        Limits::default(),
        None,
    );
    assert_eq!(s.pairs().len(), 2);
}

/// An expiry is a time, not a bar. A setup whose owner is off the list gets no bars, so
/// without this it would stay open for ever and keep one of the server's open setups.
#[tokio::test]
async fn an_unlisted_owners_setup_still_expires() {
    let (store, dir) = store_at("unlisted-expiry");
    let both = Access::parse(Some("github:1,github:2"));
    let s = live_on(store, hub(), both, Limits::default(), None);
    let watching = call(&s, "create_setup", long_on("ETHUSDT"), "github:2").unwrap();
    let filled = created(&s, "github:2");
    let listed = created(&s, "github:1");
    let (t, p) = (first_bar(&filled), mid(&filled));
    s.on_bar(&key(), bar(t, p, p, p * 0.975, p * 0.976));
    drop(s);

    let s = live_on(
        reopen(&dir.db()),
        hub(),
        Access::parse(Some("github:1")),
        Limits::default(),
        None,
    );
    let get = |id: &Value, owner: &str| call(&s, "get_setup", json!({ "id": id }), owner).unwrap();
    // Before their expiry nothing happens to them.
    s.expire_unscored(fetch::now_ms());
    assert_eq!(get(&watching["id"], "github:2")["status"], "watching");
    assert_eq!(get(&filled["id"], "github:2")["status"], "triggered");

    let after = fetch::now_ms() + 2 * 86_400_000;
    s.expire_unscored(after);
    // Never filled: expired at 0 R, marked as not scored for the time it was unlisted.
    let expired = get(&watching["id"], "github:2");
    assert_eq!(expired["status"], "expired");
    assert_eq!(expired["result_r"], 0.0);
    assert_eq!(expired["gap"], true);
    assert_eq!(kinds(&expired), ["created", "gap", "status"]);
    assert!(expired["log"][2]["reason"]
        .as_str()
        .unwrap()
        .contains("not on the operator's list"));
    // Filled: no bar was read, so there is no exit price and no result. None is invented.
    let unknown = get(&filled["id"], "github:2");
    assert_eq!(unknown["status"], "expired");
    assert_eq!(unknown["fill"]["price"], filled["entry"]["high"]);
    assert_eq!(unknown["exit"], json!(null));
    assert_eq!(unknown["result_r"], json!(null));
    assert_eq!(unknown["gap"], true);
    // A listed owner's setup, filled by the same bar, is left to its bars: only a bar
    // expires it.
    assert_eq!(get(&listed["id"], "github:1")["status"], "triggered");
    // They no longer count as open, and a second pass changes nothing.
    let open = call(&s, "list_setups", json!({ "status": "open" }), "github:2").unwrap();
    assert_eq!(open["total"], 0);
    s.expire_unscored(after);
    assert_eq!(get(&watching["id"], "github:2"), expired);
    // The closed-without-a-result row is read back as it is after a restart.
    drop(s);
    let s = live_on(
        reopen(&dir.db()),
        hub(),
        Access::parse(Some("github:1")),
        Limits::default(),
        None,
    );
    let again = call(&s, "get_setup", json!({ "id": filled["id"] }), "github:2").unwrap();
    assert_eq!(again, unknown);
}

fn alerts_on(store: &'static Store, hub: Arc<ChartHub>, feeds: usize) -> Arc<ServerAlerts> {
    let limits = crate::server_alerts::Limits {
        feeds,
        ..Default::default()
    };
    ServerAlerts::new(
        store,
        hub,
        Policy::default(),
        Access::Open,
        None,
        limits,
        Timing::default(),
    )
}

fn alert(a: &Arc<ServerAlerts>, symbol: &str) -> Value {
    let args =
        json!({ "symbol": symbol, "condition": "greater", "trigger": "once", "level": 1e12 });
    a.call_tool("create_server_alert", None, &args, Some("owner"))
}

#[tokio::test(flavor = "multi_thread")]
async fn alerts_and_setups_share_the_servers_watched_pairs() {
    let (store, hub) = (mem(), hub());
    let alerts = alerts_on(store, hub.clone(), 2);
    let limits = Limits {
        pairs_total: 2,
        ..Limits::default()
    };
    let s = live_on(
        store,
        hub,
        Access::parse(None),
        limits,
        Some(alerts.clone()),
    );
    let (a, s) = tokio::task::spawn_blocking(move || {
        // An alert and a setup on one symbol are one pair.
        assert_eq!(alert(&alerts, "BTCUSDT")["result"]["isError"], json!(null));
        created(&s, "owner");
        call(&s, "create_setup", long_on("ETHUSDT"), "owner").unwrap();
        (alerts, s)
    })
    .await
    .unwrap();
    assert_eq!(s.pairs().len(), 2);
    tokio::task::spawn_blocking(move || {
        // Two pairs are held, one of them by a setup alone: no third for either.
        let text = call(&s, "create_setup", long_on("SOLUSDT"), "other").unwrap_err();
        assert!(
            text.contains("at most 2 symbols for alerts and setups"),
            "{text}"
        );
        let refused = alert(&a, "SOLUSDT");
        assert_eq!(refused["result"]["isError"], true);
        let text = refused["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("at most 2 symbol and interval pairs"),
            "{text}"
        );
        // Existing ones keep their feeds.
        assert_eq!(alert(&a, "ETHUSDT")["result"]["isError"], json!(null));
        assert_eq!(s.pairs().len(), 2);
    })
    .await
    .unwrap();
}

/// Alerts and setups each check the shared limit and then store. An alert created
/// exactly between a setup's check and its write must not get the last pair too.
/// The alert's create is started at that point and given time to finish: with the gate
/// it cannot, because the setup holds the gate; without the gate it does, and the
/// server ends up over its limit. So the outcome does not depend on luck.
#[tokio::test(flavor = "multi_thread")]
async fn a_create_between_anothers_check_and_write_cannot_take_the_last_pair() {
    let (store, hub) = (mem(), hub());
    let alerts = alerts_on(store, hub.clone(), 1);
    let limits = Limits {
        pairs_total: 1,
        ..Limits::default()
    };
    let s = live_on(
        store,
        hub,
        Access::parse(None),
        limits,
        Some(alerts.clone()),
    );
    let racing = Arc::new(std::sync::Mutex::new(None));
    let (hooked_alerts, started) = (alerts.clone(), racing.clone());
    *s.before_final_check.lock().unwrap() = Some(Box::new(move || {
        let alerts = hooked_alerts.clone();
        let racer = std::thread::spawn(move || alert(&alerts, "ETHUSDT"));
        // Long enough for an unblocked create to finish many times over.
        let waited = std::time::Instant::now();
        while !racer.is_finished() && waited.elapsed() < Duration::from_millis(500) {
            std::thread::sleep(Duration::from_millis(5));
        }
        *started.lock().unwrap() = Some(racer);
    }));
    let (setup, s) =
        tokio::task::spawn_blocking(move || (call(&s, "create_setup", long(), "owner"), s))
            .await
            .unwrap();
    let racer = racing.lock().unwrap().take().expect("the hook ran");
    let alert_answer = racer.join().unwrap();

    // The setup was first to the check and got the pair; the alert was refused.
    assert_eq!(setup.unwrap()["symbol"], "BTCUSDT");
    assert_eq!(alert_answer["result"]["isError"], true);
    let mut pairs = s.pairs();
    pairs.extend(alerts.pairs());
    assert_eq!(pairs, HashSet::from([key()]));
}

#[tokio::test]
async fn bars_move_a_setup_and_its_log_is_kept() {
    let (store, dir) = store_at("bars");
    let s = live_on(store, hub(), Access::parse(None), Limits::default(), None);
    let setup = created(&s, "owner");
    let (id, t, p) = (id_of(&setup), first_bar(&setup), mid(&setup));
    let get = |s: &Setups| call(s, "get_setup", json!({ "id": id }), "owner").unwrap();

    s.on_bar(&key(), bar(t, p, p * 1.001, p * 0.999, p));
    assert_eq!(get(&s)["status"], "watching");
    s.on_bar(&key(), bar(t + MINUTE_MS, p, p, p * 0.975, p * 0.976));
    let filled = get(&s);
    assert_eq!(filled["status"], "triggered");
    assert_eq!(filled["fill"]["price"], setup["entry"]["high"]);
    // The same bar again, as the feed sends after a reconnect, moves nothing.
    s.on_bar(&key(), bar(t + MINUTE_MS, p, p * 1.2, p * 0.5, p));
    assert_eq!(get(&s), filled);

    // A restart: the setup, its fill and its log are as they were, and scoring goes on.
    drop(s);
    let s = live_on(
        reopen(&dir.db()),
        hub(),
        Access::parse(None),
        Limits::default(),
        None,
    );
    assert_eq!(get(&s), filled);
    assert_eq!(s.pairs(), HashSet::from([key()]));
    s.on_bar(
        &key(),
        bar(t + 2 * MINUTE_MS, p * 0.976, p * 1.06, p * 0.975, p * 1.05),
    );
    let won = get(&s);
    assert_eq!(won["status"], "won");
    assert!(won["result_r"].as_f64().unwrap() > 1.0);
    assert_eq!(kinds(&won), ["created", "status", "status"]);
    assert_eq!(won["log"][2]["by"], "server");
    // Closed, but v1's runner is still in (ADR-0066): watched until it is out, here at
    // its break-even stop. Then its feed is let go.
    assert_eq!(s.pairs(), HashSet::from([key()]));
    s.on_bar(&key(), bar(t + 3 * MINUTE_MS, p, p, p * 0.97, p * 0.975));
    assert_eq!(
        get(&s)["shadow_results"]["v1_half_t1_be"]["status"],
        "breakeven"
    );
    assert!(s.pairs().is_empty());
}

/// What setups rely on in the hub's feed (ADR-0042), pinned through the real hub: a
/// closed bar arrives as `Bar`, usually after the next candle's first tick; ticks and
/// the bars a reconnect sends again move nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_setup_moves_on_the_hubs_closed_bars_only() {
    let told = Arc::new(Told::default());
    let hub = ChartHub::new(told.clone(), HubLimits::default());
    let s = live_on(mem(), hub, Access::parse(None), Limits::default(), None);
    let setup = {
        let s = s.clone();
        tokio::task::spawn_blocking(move || created(&s, "owner"))
            .await
            .unwrap()
    };
    let (id, t, p) = (id_of(&setup), first_bar(&setup) / 1000, mid(&setup));
    let status = |s: &Setups| {
        let st = s.lock();
        let record = &st.setups[&("owner".to_string(), id.clone())].record;
        (record.status, record.scored_through)
    };
    let feed_bar = |minute: i64, low: f64, high: f64| Bar {
        open: p,
        high,
        low,
        close: p,
        volume: 1.0,
        date: (t + minute * 60).to_string(),
    };
    until("the feed to be joined", || {
        !told.feeds.lock().unwrap().is_empty()
    })
    .await;

    // A tick through the entry zone and the invalidation is not a bar.
    told.send(FeedEvent::Tick(feed_bar(0, p * 0.5, p))).await;
    // The next candle's first tick comes before the close of the one before it.
    told.send(FeedEvent::Tick(feed_bar(1, p, p))).await;
    told.send(FeedEvent::Bar(feed_bar(0, p * 0.999, p * 1.001)))
        .await;
    until("the first close to be read", || {
        status(&s).1 == Some(t * 1000)
    })
    .await;
    assert_eq!(status(&s).0, Status::Watching);

    told.send(FeedEvent::Bar(feed_bar(1, p * 0.975, p))).await;
    until("the fill", || status(&s).0 == Status::Triggered).await;
    // After a reconnect the feed sends closed bars again, here with other prices.
    told.send(FeedEvent::Bar(feed_bar(0, p * 0.5, p * 2.0)))
        .await;
    told.send(FeedEvent::Bar(feed_bar(1, p * 0.5, p * 2.0)))
        .await;
    told.send(FeedEvent::Bar(feed_bar(2, p * 0.99, p * 1.06)))
        .await;
    until("the target", || status(&s).0 == Status::Won).await;
    let st = s.lock();
    let record = &st.setups[&("owner".to_string(), id.clone())].record;
    assert_eq!(record.exit.unwrap().bar_time, (t + 120) * 1000);
    assert_eq!(
        record.revisions, 3,
        "created, filled, won: the replayed bars added nothing"
    );
}

#[tokio::test]
async fn revise_and_cancel_go_to_the_log_and_wait_for_the_next_bar() {
    let s = live();
    let setup = created(&s, "owner");
    let (id, t, p) = (id_of(&setup), first_bar(&setup), mid(&setup));
    let args = json!({ "id": id, "reason": "more room", "invalidation": p * 0.94,
        "thesis": "second thesis" });
    let revised = call(&s, "revise_setup", args, "owner").unwrap();
    assert_eq!(revised["thesis"], "second thesis");
    assert_eq!(
        revised["invalidation"], setup["invalidation"],
        "not in effect yet"
    );
    assert_eq!(
        revised["pending"][0]["levels"]["invalidation"],
        json!(p * 0.94)
    );
    assert_eq!(revised["log"][1]["reason"], "more room");
    assert_eq!(
        revised["log"][1]["changes"]["invalidation"],
        json!({ "from": setup["invalidation"], "to": p * 0.94 })
    );
    // The entry was left out of the call and keeps its value.
    assert!(revised["log"][1]["changes"].get("entry").is_none());

    // The agent cannot set an outcome through a revision either.
    let outcome = json!({ "id": id, "reason": "it won", "status": "won", "result_r": 9 });
    let text = call(&s, "revise_setup", outcome, "owner").unwrap_err();
    assert_eq!(errors(&text), ["result_r", "status"]);

    s.on_bar(&key(), bar(t, p, p * 1.001, p * 0.999, p));
    let cancelled = call(
        &s,
        "cancel_setup",
        json!({ "id": id, "reason": "done" }),
        "owner",
    );
    assert_eq!(cancelled.unwrap()["status"], "watching");
    s.on_bar(&key(), bar(t + MINUTE_MS, p, p * 1.001, p * 0.999, p));
    let got = call(&s, "get_setup", json!({ "id": id }), "owner").unwrap();
    assert_eq!(got["status"], "cancelled");
    assert_eq!(got["invalidation"], json!(p * 0.94));
    assert_eq!(got["result_r"], 0.0);
    assert_eq!(kinds(&got), ["created", "revised", "cancelled", "status"]);
    let seqs: Vec<u64> = got["log"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_u64().unwrap())
        .collect();
    assert_eq!(seqs, [1, 2, 3, 4]);
}

/// A missing field does not hide what is wrong with the fields that are there, in
/// create_setup or in revise_setup, and nothing is stored either way.
#[tokio::test]
async fn every_check_the_present_fields_allow_is_made_in_one_answer() {
    let s = live();
    let p = price("BTCUSDT");
    // A long with its stop 1 % above the entry and no expiry.
    let mut args = long();
    args.as_object_mut().unwrap().remove("expires_at");
    args["invalidation"] = json!(p * 0.99);
    let text = call(&s, "create_setup", args, "owner").unwrap_err();
    assert_eq!(errors(&text), ["expires_at", "invalidation"]);
    // A long with no stop and no expiry, and its target 2 % below the entry.
    let mut args = long();
    args.as_object_mut().unwrap().remove("expires_at");
    args.as_object_mut().unwrap().remove("invalidation");
    args["targets"] = json!([p * 0.95]);
    let text = call(&s, "create_setup", args, "owner").unwrap_err();
    assert_eq!(errors(&text), ["invalidation", "expires_at", "targets[0]"]);
    assert_eq!(
        call(&s, "list_setups", json!({}), "owner").unwrap()["total"],
        0
    );

    // A revision without a reason whose new stop is above the entry it keeps.
    let setup = created(&s, "owner");
    let stop = setup["entry"]["high"].as_f64().unwrap() * 1.01;
    let args = json!({ "id": setup["id"], "invalidation": stop, "expires_at": "yesterday" });
    let text = call(&s, "revise_setup", args, "owner").unwrap_err();
    assert_eq!(errors(&text), ["reason", "expires_at", "invalidation"]);
    let after = call(&s, "get_setup", json!({ "id": setup["id"] }), "owner").unwrap();
    assert_eq!(after, setup);
}

/// When a setup's state forbids a change, that is the answer. Problems with levels that
/// could not be applied anyway are not reported: fixing them would only earn the
/// agent a second refusal.
#[tokio::test]
async fn a_revision_the_state_forbids_is_answered_about_the_state() {
    // More calls than a minute's allowance of changes.
    let limits = Limits {
        changes_per_minute: 100,
        ..Limits::default()
    };
    let s = live_on(mem(), hub(), Access::parse(None), limits, None);
    let setup = created(&s, "owner");
    let (id, t, p) = (id_of(&setup), first_bar(&setup), mid(&setup));
    let revise = |extra: Value| {
        let mut args = json!({ "id": id, "reason": "example reason" });
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        call(&s, "revise_setup", args, "owner")
    };
    let only = |answer: Result<Value, String>, field: &str, words: &str| {
        let text = answer.unwrap_err();
        assert_eq!(errors(&text), [field], "{text}");
        assert!(text.contains(words), "{text}");
    };
    s.on_bar(&key(), bar(t, p, p, p * 0.975, p * 0.976));

    // Filled: a stop on the wrong side, a valid target, an expiry in the past.
    let fixed = "its levels are fixed";
    only(revise(json!({ "invalidation": p * 1.5 })), "status", fixed);
    only(revise(json!({ "targets": [p * 1.07] })), "status", fixed);
    only(
        revise(json!({ "expires_at": "2001-01-01" })),
        "status",
        fixed,
    );
    // What can still change after the fill does.
    let revised = revise(json!({ "thesis": "after the fill" })).unwrap();
    assert_eq!(revised["thesis"], "after the fill");
    // Another problem of the call itself is still named beside the state.
    let no_reason = json!({ "id": id, "invalidation": p * 1.5 });
    let text = call(&s, "revise_setup", no_reason, "owner").unwrap_err();
    assert_eq!(errors(&text), ["reason", "status"]);

    // Closed: the stop alone, valid or not, and the thesis.
    s.on_bar(
        &key(),
        bar(t + MINUTE_MS, p * 0.976, p * 1.06, p * 0.975, p * 1.05),
    );
    let closed = "closed and cannot be revised";
    only(revise(json!({ "invalidation": p * 0.94 })), "id", closed);
    only(revise(json!({ "invalidation": p * 1.5 })), "id", closed);
    only(revise(json!({ "thesis": "too late" })), "id", closed);

    // A skip keeps its bracket, whatever is sent for it.
    let skip = json!({ "symbol": "BTCUSDT", "interval": "1h", "direction": "long",
        "decision": "skip", "thesis": "example reason" });
    let skip = call(&s, "create_setup", skip, "owner").unwrap();
    let args = json!({ "id": skip["id"], "reason": "example reason", "invalidation": -1 });
    let text = call(&s, "revise_setup", args, "owner").unwrap_err();
    assert_eq!(errors(&text), ["decision"], "{text}");

    // A setup being cancelled takes no revision.
    let other = created(&s, "owner");
    call(
        &s,
        "cancel_setup",
        json!({ "id": other["id"], "reason": "example" }),
        "owner",
    )
    .unwrap();
    let args = json!({ "id": other["id"], "reason": "example", "targets": [1.0] });
    let text = call(&s, "revise_setup", args, "owner").unwrap_err();
    assert_eq!(errors(&text), ["id"], "{text}");
    assert!(text.contains("being cancelled"));
}

#[tokio::test]
async fn skip_is_stored_and_scored_with_the_default_bracket() {
    let s = live();
    let args = json!({ "symbol": "BTCUSDT", "interval": "1h", "direction": "short",
        "decision": "skip", "thesis": "example reason" });
    let skip = call(&s, "create_setup", args.clone(), "owner").unwrap();
    assert_eq!(skip["decision"], "skip");
    // The server's bracket, by one of its two rules; a setup with levels is the agent's.
    assert!(["atr", "floor"].contains(&skip["bracket"].as_str().unwrap()));
    assert_eq!(created(&s, "other")["bracket"], "agent");
    let (entry, stop, target) = (
        skip["entry"]["low"].as_f64().unwrap(),
        skip["invalidation"].as_f64().unwrap(),
        skip["targets"][0].as_f64().unwrap(),
    );
    assert!(target < entry && entry < stop);
    assert!(((entry - target) - 2.0 * (stop - entry)).abs() < 1e-6 * entry);
    assert_eq!(
        skip["expires_at"].as_i64().unwrap() - skip["created_at"].as_i64().unwrap(),
        24 * 3_600_000
    );
    // In at the next bar's open, whatever the agent thinks of it now.
    s.on_bar(
        &key(),
        bar(
            first_bar(&skip),
            entry,
            entry * 1.0001,
            target * 0.999,
            target,
        ),
    );
    let filled = call(&s, "get_setup", json!({ "id": id_of(&skip) }), "owner").unwrap();
    assert_eq!(filled["status"], "triggered");
    assert_eq!(filled["fill"]["price"], json!(entry));

    let mut with_levels = args;
    with_levels["invalidation"] = json!(stop * 2.0);
    let text = call(&s, "create_setup", with_levels, "owner").unwrap_err();
    assert_eq!(errors(&text), ["invalidation"]);
}

/// 1m bars the test chooses, flat at 100 but for a dip to 90.5 in `dip`. The catch-up
/// read gets these and nothing that moves with the wall clock.
fn flat_bars(minutes: usize, dip: std::ops::Range<usize>) -> Replay {
    let bars = (0..minutes)
        .map(|i| {
            let low = if dip.contains(&i) { 90.5 } else { 99.9 };
            Bar {
                open: 100.0,
                high: 100.1,
                low,
                close: 100.0,
                volume: 1.0,
                date: (T0 + i as i64 * 60).to_string(),
            }
        })
        .collect();
    Replay::of(BTreeMap::from([("BTCUSDT".to_string(), bars)]))
}

/// A live service whose reads of missed bars come from the bars a test chose.
fn live_reading_chosen_bars() -> Arc<Setups> {
    Setups::new(
        mem(),
        Some(hub()),
        None,
        DataSource::Replay {
            as_of: Some(i64::MAX / 4),
        },
        Access::parse(None),
        Limits::default(),
        None,
    )
}

/// A long at 90 to 91, wrong at 85, made at `minute` past `T0`.
fn long_made_at(s: &Setups, minute: i64, owner: &str) -> Value {
    let then = T0_MS + minute * MINUTE_MS + 30_000;
    crate::replay::at(Some(then), || {
        let args = json!({
            "symbol": "BTCUSDT", "interval": "1h", "direction": "long",
            "thesis": "example thesis", "entry": { "low": 90.0, "high": 91.0 },
            "invalidation": 85.0, "targets": [120.0], "expires_at": then + 30 * 86_400_000,
        });
        call(s, "create_setup", args, owner).unwrap()
    })
}

#[tokio::test]
async fn bars_the_server_missed_are_read_and_scored_in_order() {
    crate::replay::on(flat_bars(3000, 50..52), |_| {
        let s = live_reading_chosen_bars();
        // Made at minute 10, with the server down until the bar of minute 100.
        let quiet = long_made_at(&s, 60, "quiet");
        let dipped = long_made_at(&s, 10, "dipped");
        s.on_bar(&key(), bar(at_minute(100), 100.0, 100.1, 99.9, 100.0));

        // Nothing came near this one: no gap, nothing moved, and it is up to date.
        let quiet = call(&s, "get_setup", json!({ "id": id_of(&quiet) }), "quiet").unwrap();
        assert_eq!(kinds(&quiet), ["created"]);
        assert_eq!(quiet["gap"], false);
        assert_eq!(quiet["status"], "watching");
        assert_eq!(quiet["scored_through"], json!(at_minute(100)));

        // This one was filled by a bar the server never saw arrive.
        let dipped = call(&s, "get_setup", json!({ "id": id_of(&dipped) }), "dipped").unwrap();
        assert_eq!(kinds(&dipped), ["created", "status"]);
        assert_eq!(dipped["status"], "triggered");
        assert_eq!(
            dipped["fill"],
            json!({ "price": 91.0, "bar_time": at_minute(50) })
        );
    });
}

#[test]
fn a_catch_up_read_is_logged_with_its_count_and_symbol() {
    assert_eq!(
        missed_note(89, "BTCUSDT"),
        "[setups] read 89 missed 1m bars of BTCUSDT"
    );
}

#[tokio::test]
async fn bars_out_of_reach_are_named_as_a_gap() {
    crate::replay::on(flat_bars(3000, 0..0), |_| {
        let s = live_reading_chosen_bars();
        let old = long_made_at(&s, 10, "owner");
        // Further back than one read reaches: the stretch not scored is in the log.
        s.on_bar(&key(), bar(at_minute(2500), 100.0, 100.1, 99.9, 100.0));
        let gapped = call(&s, "get_setup", json!({ "id": id_of(&old) }), "owner").unwrap();
        assert_eq!(kinds(&gapped), ["created", "gap"]);
        let gap = &gapped["log"][1];
        assert_eq!(gap["by"], "server");
        assert_eq!(
            gap["changes"]["not_scored"],
            json!({ "from": at_minute(11), "to": at_minute(2500 - CATCH_UP_BARS) })
        );
        assert_eq!(gapped["scored_through"], json!(at_minute(2500)));
        // The result itself says it is incomplete, in the list too, where no log is.
        assert_eq!(gapped["gap"], true);
        let list = call(&s, "list_setups", json!({}), "owner").unwrap();
        assert_eq!(list["setups"][0]["gap"], true);
    });
}

#[tokio::test]
async fn a_read_of_missed_bars_that_fails_is_a_gap_too() {
    // No bars for the symbol at all: the read fails.
    crate::replay::on(Replay::of(BTreeMap::new()), |_| {
        let s = live_reading_chosen_bars();
        let setup = long_made_at(&s, 10, "owner");
        s.on_bar(&key(), bar(at_minute(40), 100.0, 100.1, 99.9, 100.0));
        let gapped = call(&s, "get_setup", json!({ "id": id_of(&setup) }), "owner").unwrap();
        assert_eq!(
            gapped["log"][1]["changes"]["not_scored"],
            json!({ "from": at_minute(11), "to": at_minute(40) })
        );
    });
}

#[tokio::test]
async fn a_stored_row_that_cannot_be_read_is_skipped_and_never_scored() {
    let (store, dir) = store_at("bad-rows");
    let s = live_on(store, hub(), Access::parse(None), Limits::default(), None);
    let good = created(&s, "owner");
    let mut broken: Value = serde_json::from_str(&store.setups().unwrap()[0].0).unwrap();
    broken["id"] = json!("broken");
    broken["levels"]["invalidation"] = json!(-1.0);
    let row = |id: &str, record: String| StoredSetup {
        id: id.into(),
        owner: "owner".into(),
        record,
        revisions: vec![],
    };
    store
        .save_setups(&[
            row("garbage", "not json".into()),
            row("broken", broken.to_string()),
        ])
        .unwrap();
    drop(s);
    let s = live_on(
        reopen(&dir.db()),
        hub(),
        Access::parse(None),
        Limits::default(),
        None,
    );
    let list = call(&s, "list_setups", json!({}), "owner").unwrap();
    assert_eq!(list["total"], 1);
    assert_eq!(list["setups"][0]["id"], good["id"]);
    assert!(call(&s, "get_setup", json!({ "id": "broken" }), "owner").is_err());
    let p = mid(&good);
    s.on_bar(&key(), bar(first_bar(&good), p, p * 1.2, p * 0.5, p));
    // The rows are still there, untouched.
    assert_eq!(reopen(&dir.db()).setups().unwrap().len(), 3);
}

#[tokio::test]
async fn as_of_is_refused_outside_replay_mode() {
    let s = live();
    let mut args = long();
    args["as_of"] = json!(fetch::now_ms());
    let text = call(&s, "create_setup", args, "owner").unwrap_err();
    assert_eq!(text, crate::replay::NOT_A_REPLAY);
    assert_eq!(
        call(&s, "list_setups", json!({}), "owner").unwrap()["replay"],
        false
    );
}

// --- Replay ---

/// An hour boundary.
const T0: i64 = 1_700_006_400;
const T0_MS: i64 = T0 * 1000;
const HOUR_MS: i64 = 3_600_000;

/// Two symbols of 1m bars from `T0_MS`. Bars from minute `changed_from` on are lifted
/// by `lift`, so two fixtures can agree up to a time and differ after it.
fn fixture(changed_from: usize, lift: f64) -> BTreeMap<String, Vec<Bar>> {
    let wave = |i: usize, base: f64| {
        let x = i as f64;
        let c = base * (1.0 + 0.02 * (x / 40.0).sin() + 0.008 * (x / 7.0).cos())
            + if i >= changed_from { lift } else { 0.0 };
        let o = base * (1.0 + 0.02 * ((x - 1.0) / 40.0).sin() + 0.008 * ((x - 1.0) / 7.0).cos())
            + if i >= changed_from && i > 0 {
                lift
            } else {
                0.0
            };
        Bar {
            open: o,
            high: o.max(c) * 1.0005,
            low: o.min(c) * 0.9995,
            close: c,
            volume: 10.0 + (i % 7) as f64,
            date: (T0 + i as i64 * 60).to_string(),
        }
    };
    BTreeMap::from([
        (
            "BTCUSDT".to_string(),
            (0..3000).map(|i| wave(i, 100.0)).collect(),
        ),
        (
            "ETHUSDT".to_string(),
            (0..3000).map(|i| wave(i, 50.0)).collect(),
        ),
    ])
}

fn replaying(replay: &'static Replay) -> Arc<Setups> {
    Setups::new(
        Box::leak(Box::new(Store::in_memory().unwrap())),
        None,
        Some(replay),
        DataSource::Replay { as_of: None },
        Access::parse(None),
        Limits::default(),
        None,
    )
}

/// A chart tool as the server calls it on a replay: admitted, then run at its time.
fn chart_tool(replay: &Replay, tool: &str, args: Value) -> Value {
    match replay.admit(tool, &args) {
        Err(e) => json!({ "refused": e }),
        Ok(at) => crate::replay::at(at, || {
            let params = json!({ "name": tool, "arguments": args });
            let request = Some(&params);
            crate::mcp::handle_mcp_request("tools/call", None, request, None, None, None, None)
        }),
    }
}

fn at_minute(minute: i64) -> i64 {
    T0_MS + minute * MINUTE_MS
}

/// One evaluation run: every tool a replay answers, at times that only move forward.
/// Returns every answer, as text.
fn scenario(replay: &'static Replay) -> Vec<String> {
    let s = replaying(replay);
    let mut out = Vec::new();
    let mut say = |v: Value| out.push(v.to_string());
    let setup = |tool: &str, args: Value| match call(&s, tool, args, "agent") {
        Ok(v) => v,
        Err(e) => json!({ "error": e }),
    };

    let t = at_minute(1500);
    for format in ["summary", "series", "png"] {
        let args = json!({ "ticker": "BTCUSDT", "timeframe": "15m", "bars": 60,
            "format": format, "indicators": ["rsi", "ema_stack"], "as_of": t });
        say(chart_tool(replay, "generate_chart", args));
    }
    say(chart_tool(
        replay,
        "inspect_chart",
        json!({ "ticker": "BTCUSDT", "timeframe": "1h", "bars": 20, "as_of": t }),
    ));
    say(chart_tool(
        replay,
        "get_indicators",
        json!({ "symbol": "ETHUSDT", "interval": "5m", "indicators": ["rsi", "macd"], "as_of": t }),
    ));
    say(chart_tool(
        replay,
        "get_quotes",
        json!({ "symbols": ["BTCUSDT", "ETHUSDT", "SOLUSDT"], "as_of": t }),
    ));

    let p = replay.quote("BTCUSDT", t).unwrap().0;
    let long = json!({
        "symbol": "BTCUSDT", "interval": "15m", "direction": "long", "thesis": "example thesis",
        "entry": { "low": p * 0.995, "high": p * 0.999 }, "invalidation": p * 0.985,
        "targets": [p * 1.01], "expires_at": t + 12 * HOUR_MS, "as_of": t,
        "drawings": [{ "tool": "horizontalStraightLine", "points": [{ "time": t - HOUR_MS, "snap": "low" }] }],
    });
    let first = setup("create_setup", long);
    say(first.clone());
    say(setup(
        "create_setup",
        json!({ "symbol": "ETHUSDT", "interval": "15m", "direction": "short",
            "decision": "skip", "thesis": "example reason", "as_of": t + 30_000 }),
    ));
    let id = first["id"].as_str().unwrap_or_default().to_string();
    say(setup(
        "revise_setup",
        json!({ "id": id, "reason": "more room", "invalidation": p * 0.98, "as_of": at_minute(1502) }),
    ));
    for minute in [1530, 1600, 1700, 1900, 2400] {
        let t = at_minute(minute);
        say(setup("get_setup", json!({ "id": id, "as_of": t })));
        say(setup("list_setups", json!({ "as_of": t })));
        say(chart_tool(
            replay,
            "get_quotes",
            json!({ "symbols": ["BTCUSDT"], "as_of": t }),
        ));
    }
    let another = setup("create_setup", {
        let t = at_minute(2401);
        let p = replay.quote("BTCUSDT", t).unwrap().0;
        json!({ "symbol": "BTCUSDT", "interval": "1h", "direction": "short",
            "thesis": "example thesis", "entry": { "low": p * 1.001, "high": p * 1.004 },
            "invalidation": p * 1.02, "targets": [p * 0.98],
            "expires_at": t + 6 * HOUR_MS, "as_of": t })
    });
    say(another.clone());
    let id = another["id"].as_str().unwrap_or_default().to_string();
    say(setup(
        "cancel_setup",
        json!({ "id": id, "reason": "done", "as_of": at_minute(2450) }),
    ));
    say(setup(
        "list_setups",
        json!({ "status": "closed", "as_of": at_minute(2500) }),
    ));
    say(setup(
        "get_setup",
        json!({ "id": id, "as_of": at_minute(2500) }),
    ));
    out
}

#[tokio::test]
async fn the_same_scenario_gives_the_same_bytes_twice() {
    let uses = chartgen::binance_gate::http_uses();
    let first = crate::replay::on(Replay::of(fixture(usize::MAX, 0.0)), scenario);
    let second = crate::replay::on(Replay::of(fixture(usize::MAX, 0.0)), scenario);
    assert_eq!(first, second);
    // The run did something: setups were made, moved and closed, and no call failed.
    let all = first.join("\n");
    for expected in [
        "setup-000001",
        "setup-000002",
        "setup-000003",
        "\"triggered\"",
        "\"cancelled\"",
        "\"replay\":true",
    ] {
        assert!(all.contains(expected), "no {expected} in the run");
    }
    // One failure is wanted: the run asks for a quote of a symbol the fixtures do not
    // hold, and a replay refuses that symbol by name without failing the others.
    let failures: Vec<String> = first
        .iter()
        .flat_map(|answer| failures_in(answer))
        .collect();
    assert_eq!(failures, ["error: \"this replay has no bars for SOLUSDT\""]);
    let quotes: Value = serde_json::from_str(&first[5]).unwrap();
    let quotes: Value =
        serde_json::from_str(quotes["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(quotes["quotes"][0]["symbol"], "BTCUSDT");
    assert!(quotes["quotes"][0]["last"].is_number() && quotes["quotes"][1]["last"].is_number());
    assert_eq!(
        quotes["quotes"][2],
        json!({ "symbol": "SOLUSDT", "error": "this replay has no bars for SOLUSDT" })
    );
    // And it never reached for the network.
    assert_eq!(chartgen::binance_gate::http_uses(), uses);
}

/// Every failure an answer holds, however deep. A tool's payload travels as text inside
/// the JSON-RPC envelope, so it is decoded first: searching the envelope's own text for
/// `"error"` finds nothing, because there the quotes are escaped.
fn failures_in(answer: &str) -> Vec<String> {
    fn walk(v: &Value, found: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                for (key, value) in map {
                    let failed = match key.as_str() {
                        "error" | "refused" => !value.is_null(),
                        "isError" => value == &json!(true),
                        "indicator_errors" | "setup_errors" | "drawing_errors" => {
                            value.as_object().is_some_and(|m| !m.is_empty())
                                || value.as_array().is_some_and(|a| !a.is_empty())
                        }
                        _ => false,
                    };
                    if failed {
                        found.push(format!("{key}: {value}"));
                    }
                    // A text item is a payload: look inside it too.
                    match (
                        key.as_str(),
                        value.as_str().map(serde_json::from_str::<Value>),
                    ) {
                        ("text", Some(Ok(payload))) => walk(&payload, found),
                        _ => walk(value, found),
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    walk(&serde_json::from_str(answer).unwrap(), &mut found);
    found
}

/// What the replay tests compare against does count.
#[test]
fn reaching_for_the_http_client_is_counted() {
    let uses = chartgen::binance_gate::http_uses();
    chartgen::binance_gate::http();
    assert_eq!(chartgen::binance_gate::http_uses(), uses + 1);
}

#[tokio::test]
async fn no_bar_after_as_of_leaks_into_a_run() {
    // The run's last call is at minute 2500. The fixtures agree up to there.
    let same = crate::replay::on(Replay::of(fixture(usize::MAX, 0.0)), scenario);
    let later_differs = crate::replay::on(Replay::of(fixture(2500, 7.0)), scenario);
    assert_eq!(same, later_differs);
    // A difference inside the run does change it, so the comparison can fail.
    let earlier_differs = crate::replay::on(Replay::of(fixture(2000, 7.0)), scenario);
    assert_ne!(same, earlier_differs);
}

#[tokio::test]
async fn one_clock_for_every_tool_and_caller_and_it_cannot_go_backwards() {
    crate::replay::on(Replay::of(fixture(usize::MAX, 0.0)), |replay| {
        let s = replaying(replay);
        // Reading the chart at a later time closes the earlier time for setups too.
        let later = json!({ "ticker": "BTCUSDT", "timeframe": "15m", "format": "summary",
            "indicators": [], "as_of": at_minute(1600) });
        let read = chart_tool(replay, "generate_chart", later);
        assert_eq!(failures_in(&read.to_string()), Vec::<String>::new());
        let p = replay.quote("BTCUSDT", at_minute(1500)).unwrap().0;
        let mut args = json!({
            "symbol": "BTCUSDT", "interval": "15m", "direction": "long", "thesis": "example",
            "entry": { "low": p * 0.99, "high": p * 0.995 }, "invalidation": p * 0.97,
            "targets": [p * 1.02], "expires_at": at_minute(3000), "as_of": at_minute(1500),
        });
        let text = call(&s, "create_setup", args.clone(), "agent").unwrap_err();
        assert!(text.contains("only moves forward"), "{text}");
        // And the other way round.
        args["as_of"] = json!(at_minute(1700));
        let id = id_of(&call(&s, "create_setup", args, "agent").unwrap());
        let earlier = json!({ "symbols": ["BTCUSDT"], "as_of": at_minute(1650) });
        let refused = chart_tool(replay, "get_quotes", earlier);
        assert!(refused["refused"]
            .as_str()
            .unwrap()
            .contains("only moves forward"));
        let text = call(
            &s,
            "get_setup",
            json!({ "id": id, "as_of": at_minute(1699) }),
            "agent",
        );
        assert!(text.unwrap_err().contains("only moves forward"));
        // The clock is the server's, not the caller's: registration is open on a replay,
        // so a second client must not be a way to read an earlier or a later time.
        let other = call(
            &s,
            "list_setups",
            json!({ "as_of": at_minute(1400) }),
            "other",
        );
        assert!(other.unwrap_err().contains("only moves forward"));
        let earlier = json!({ "symbols": ["BTCUSDT"], "as_of": at_minute(1400) });
        let refused = chart_tool(replay, "get_quotes", earlier);
        assert!(refused["refused"]
            .as_str()
            .unwrap()
            .contains("only moves forward"));
        // At the server's time it answers, and still cannot see the first caller's setup.
        let other = call(
            &s,
            "list_setups",
            json!({ "as_of": at_minute(1700) }),
            "other",
        );
        assert_eq!(other.unwrap()["total"], 0);
        // No as_of, no answer.
        let text = call(&s, "list_setups", json!({}), "agent").unwrap_err();
        assert!(text.contains("every call needs as_of"), "{text}");
    });
}

#[tokio::test]
async fn a_replay_refuses_what_the_fixtures_do_not_hold() {
    crate::replay::on(Replay::of(fixture(usize::MAX, 0.0)), |replay| {
        let t = at_minute(1500);
        for tool in [
            "get_footprint",
            "get_tpo",
            "search_symbols",
            "subscribe_chart",
            "place_order",
        ] {
            let refused = chart_tool(replay, tool, json!({ "ticker": "BTCUSDT", "as_of": t }));
            assert!(
                refused["refused"]
                    .as_str()
                    .unwrap()
                    .contains("not available"),
                "{tool}"
            );
        }
        let missing = chart_tool(replay, "inspect_chart", json!({ "ticker": "BTCUSDT" }));
        assert!(missing["refused"]
            .as_str()
            .unwrap()
            .contains("every call needs as_of"));
        // A read that forgot the cut gets nothing, not the newest bar.
        let forgot = fetch::fetch_ohlcv(DataSource::from_env(), "BTCUSDT", "1m", 5, None);
        assert_eq!(forgot.err().unwrap(), "a replay read needs as_of");
        // Indicators that fetch a series of their own are refused by what they are.
        let args = json!({ "ticker": "BTCUSDT", "timeframe": "15m", "format": "summary",
            "indicators": ["rsi", "funding", "fear_greed"], "as_of": t });
        let text = chart_tool(replay, "generate_chart", args).to_string();
        assert!(text.contains("Not available in a replay"), "{text}");
        // Each of them, by itself: one that was let through would ask the exchange.
        for name in [
            "funding",
            "open_interest",
            "long_short",
            "top_long_short",
            "taker_ratio",
            "fear_greed",
        ] {
            let args = json!({ "ticker": "BTCUSDT", "timeframe": "15m", "format": "summary",
                "indicators": [name], "as_of": t });
            let text = chart_tool(replay, "generate_chart", args).to_string();
            assert!(text.contains("Not available in a replay"), "{name}: {text}");
        }
        let plain = chart_tool(replay, "list_drawing_tools", json!({}));
        assert!(plain["result"]["content"][0]["text"].is_string());
    });
}

fn answers_at(bars: BTreeMap<String, Vec<Bar>>, t: i64, calls: &[Value]) -> Vec<String> {
    crate::replay::on(Replay::of(bars), |replay| {
        calls
            .iter()
            .map(|args| {
                let mut args = args.clone();
                args["as_of"] = json!(t);
                chart_tool(replay, "generate_chart", args).to_string()
            })
            .collect()
    })
}

#[tokio::test]
async fn no_bar_after_as_of_leaks_into_any_indicator_or_chart_type() {
    let uses = chartgen::binance_gate::http_uses();
    let mut calls = Vec::new();
    for name in chartgen::indicators::available() {
        for format in ["series", "summary"] {
            calls.push(json!({ "ticker": "BTCUSDT", "timeframe": "5m", "bars": 120,
                "format": format, "indicators": [name] }));
        }
    }
    let styles = ["candles", "heikin_ashi"]
        .into_iter()
        .chain(chartgen::price_bars::STYLES);
    for style in styles {
        for format in ["series", "png"] {
            calls.push(json!({ "ticker": "BTCUSDT", "timeframe": "5m", "bars": 120,
                "format": format, "chart_style": style, "indicators": ["rsi", "volume_profile"] }));
        }
    }
    // Inside a 5m bar and inside a minute: the forming ones must not be there.
    let t = at_minute(2003) + 20_000;
    let truth = answers_at(fixture(usize::MAX, 0.0), t, &calls);
    // Every bar that had not closed by `t` is different here.
    let future_differs = answers_at(fixture(2003, 9.0), t, &calls);
    for ((call, a), b) in calls.iter().zip(&truth).zip(&future_differs) {
        assert_eq!(a, b, "a later bar reached {call}");
    }
    // The newest bar that had closed does reach the answers.
    let past_differs = answers_at(fixture(1999, 9.0), t, &calls);
    let moved = truth
        .iter()
        .zip(&past_differs)
        .filter(|(a, b)| a != b)
        .count();
    let refused = truth
        .iter()
        .filter(|a| a.contains("Not available in a replay"))
        .count();
    assert_eq!(moved + refused, calls.len(), "every answer reads the bars");
    // An answer that is not refused holds no failure of its own: an indicator that could
    // not be computed would still "move" with the bars around it and pass the count.
    for (call, answer) in calls.iter().zip(&truth) {
        let failures = failures_in(answer);
        match answer.contains("Not available in a replay") {
            true => assert_eq!(failures.len(), 1, "{call}: {failures:?}"),
            false => assert_eq!(failures, Vec::<String>::new(), "{call}"),
        }
    }
    assert!(refused > 0 && refused < calls.len() / 4);
    assert_eq!(chartgen::binance_gate::http_uses(), uses);
}

#[tokio::test]
async fn a_replays_store_is_its_own_and_touches_no_file() {
    crate::replay::on(Replay::of(fixture(usize::MAX, 0.0)), |replay| {
        let (a, b) = (replaying(replay), replaying(replay));
        let p = replay.quote("BTCUSDT", at_minute(1500)).unwrap().0;
        let args = json!({
            "symbol": "BTCUSDT", "interval": "15m", "direction": "long", "thesis": "example",
            "entry": { "low": p * 0.99, "high": p * 0.995 }, "invalidation": p * 0.97,
            "targets": [p * 1.02], "expires_at": at_minute(3000), "as_of": at_minute(1500),
        });
        call(&a, "create_setup", args, "agent").unwrap();
        let other = call(
            &b,
            "list_setups",
            json!({ "as_of": at_minute(1500) }),
            "agent",
        );
        assert_eq!(other.unwrap()["total"], 0);
        assert!(!std::path::Path::new(":memory:").exists());
    });
}

/// The answers of the five tools, pinned in a file the web app's types are tested
/// against (`web/src/types/responses.test.ts`). They come from a replay over the
/// example fixtures, so they are the same on every run. To write the file again after
/// a deliberate change: `CHARTGEN_WRITE_FIXTURES=1 cargo test setup_answers`.
#[tokio::test]
async fn setup_answers_match_the_file_the_web_types_are_tested_against() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let replay = Replay::load(&dir.join("replay")).unwrap();
    let answers = crate::replay::on(replay, |replay| {
        let s = replaying(replay);
        let start = 1_767_225_600_000 + 600 * MINUTE_MS;
        let at = |minute: i64| start + minute * MINUTE_MS;
        let ask = |tool: &str, args: Value| call(&s, tool, args, "agent").unwrap();
        let p = replay.quote("BTCUSDT", at(0)).unwrap().0;
        let created = ask(
            "create_setup",
            json!({
                "symbol": "BTCUSDT", "interval": "15m", "direction": "long",
                "thesis": "example thesis", "tags": ["strategy:example"],
                "entry": { "low": p * 0.997, "high": p * 0.999 }, "invalidation": p * 0.99,
                "targets": [p * 1.008, p * 1.02], "expires_at": at(600), "as_of": at(0),
                "drawings": [{ "tool": "horizontalStraightLine",
                    "points": [{ "time": at(-60), "snap": "low" }] }],
            }),
        );
        let id = created["id"].clone();
        let skipped = ask(
            "create_setup",
            json!({ "symbol": "ETHUSDT", "interval": "15m", "direction": "short",
                "decision": "skip", "thesis": "example reason", "as_of": at(0) }),
        );
        let revised = ask(
            "revise_setup",
            json!({ "id": id, "reason": "example reason", "targets": [p * 1.006],
                "thesis": "second thesis", "as_of": at(1) }),
        );
        let later = ask("get_setup", json!({ "id": id, "as_of": at(120) }));
        let q = replay.quote("ETHUSDT", at(120)).unwrap().0;
        let second = ask(
            "create_setup",
            json!({
                "symbol": "ETHUSDT", "interval": "1h", "direction": "short",
                "thesis": "example thesis",
                "entry": { "low": q * 1.001, "high": q * 1.003 }, "invalidation": q * 1.01,
                "targets": [q * 0.99], "expires_at": at(600), "as_of": at(120),
            }),
        );
        let cancelled = ask(
            "cancel_setup",
            json!({ "id": second["id"], "reason": "example reason", "as_of": at(121) }),
        );
        json!({
            "create_setup": created,
            "create_setup_skip": skipped,
            "revise_setup": revised,
            "get_setup": later,
            "cancel_setup": cancelled,
            "list_setups": ask("list_setups", json!({ "as_of": at(400) })),
        })
    });
    let path = dir.join("setup_responses.json");
    let text = serde_json::to_string_pretty(&answers).unwrap() + "\n";
    if std::env::var("CHARTGEN_WRITE_FIXTURES").is_ok() {
        std::fs::write(&path, &text).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), text);
}

/// An alert can name an open setup only. One the owner cancelled, once the next bar
/// has closed it, is refused by create and by update (ADR-0061).
#[tokio::test(flavor = "multi_thread")]
async fn an_alert_cannot_name_a_setup_that_is_closed() {
    let store = mem();
    let alerts = ServerAlerts::new(
        store,
        hub(),
        Policy::default(),
        Access::Open,
        None,
        crate::server_alerts::Limits::default(),
        Timing::default(),
    );
    let agent = chartgen::webhook::AgentReceiver::from_vars(
        |name| match name {
            "CHARTGEN_AGENT_WEBHOOK_URL" => Some("http://127.0.0.1:9/webhooks/route".to_string()),
            "CHARTGEN_AGENT_WEBHOOK_OWNER" => Some("owner".to_string()),
            "CHARTGEN_AGENT_WEBHOOK_SECRET" => {
                Some("a-secret-of-thirty-two-characters".to_string())
            }
            _ => None,
        },
        None,
    );
    alerts.set_agent(agent.unwrap().unwrap());
    let s = live_on(
        store,
        hub(),
        Access::parse(None),
        Limits::default(),
        Some(alerts.clone()),
    );
    let alert = |tool: &str, args: Value| {
        let resp = alerts.call_tool(tool, None, &args, Some("owner"));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        (resp["result"]["isError"] == json!(true), text.to_string())
    };
    let waking = |setup: &Value| {
        json!({ "symbol": "BTCUSDT", "condition": "greater", "trigger": "once",
            "level": 1.0e9, "notify_agent": true, "setup_id": setup["id"] })
    };

    let open = created(&s, "owner");
    let closed = created(&s, "owner");
    let (refused, made) = alert("create_server_alert", waking(&open));
    assert!(!refused, "{made}");
    let made: Value = serde_json::from_str(&made).unwrap();

    let (t, p) = (first_bar(&closed), mid(&closed));
    let cancel = json!({ "id": closed["id"], "reason": "example reason" });
    call(&s, "cancel_setup", cancel, "owner").unwrap();
    s.on_bar(&key(), bar(t, p, p, p, p));
    let now = call(&s, "get_setup", json!({ "id": closed["id"] }), "owner").unwrap();
    assert_eq!(now["status"], "cancelled");

    assert_eq!(
        alert("create_server_alert", waking(&closed)),
        (true, "That setup is closed".to_string())
    );
    let moved = json!({ "alert_id": made["id"], "setup_id": closed["id"] });
    assert_eq!(
        alert("update_server_alert", moved),
        (true, "That setup is closed".to_string())
    );
}

#[tokio::test]
async fn list_setups_keeps_those_with_every_tag_asked_for() {
    let s = live();
    let tagged = |tags: Value| {
        let mut args = long();
        args["tags"] = tags;
        call(&s, "create_setup", args, "owner").unwrap()
    };
    let both = tagged(json!(["strategy:retest-short", "pattern:double-top-ee"]));
    let one = tagged(json!(["strategy:retest-short"]));
    let none = tagged(json!([]));
    assert_eq!(none["tags"], json!([]));
    // In any order: two setups made in the same millisecond sort by their random ids.
    let ids = |args: Value| -> HashSet<String> {
        let list = call(&s, "list_setups", args, "owner").unwrap();
        list["setups"]
            .as_array()
            .unwrap()
            .iter()
            .map(id_of)
            .collect()
    };
    assert_eq!(
        ids(json!({ "tags": ["strategy:retest-short"] })),
        HashSet::from([id_of(&one), id_of(&both)])
    );
    assert_eq!(
        ids(json!({ "tags": ["pattern:double-top-ee", "strategy:retest-short"] })),
        HashSet::from([id_of(&both)])
    );
    assert_eq!(ids(json!({ "tags": [] })).len(), 3);
    assert!(ids(json!({ "tags": ["strategy:other"] })).is_empty());
    let refused = call(&s, "list_setups", json!({ "tags": ["Bad Tag"] }), "owner").unwrap_err();
    assert_eq!(errors(&refused), ["tags[0]"]);

    // Tags change after the fill too, and the change is in the log.
    let (t, p) = (first_bar(&one), mid(&one));
    s.on_bar(&key(), bar(t, p, p, p * 0.975, p * 0.976));
    let filled = call(&s, "get_setup", json!({ "id": one["id"] }), "owner").unwrap();
    assert_eq!(filled["status"], "triggered");
    assert_eq!(filled["mfe_r"], 0.0);
    assert_eq!(filled["bars_to_exit"], 0.0);
    assert_eq!(filled["closest_approach_r"], 0.0);
    let args = json!({ "id": one["id"], "reason": "reviewed", "tags": ["review:late-entry"] });
    let revised = call(&s, "revise_setup", args, "owner").unwrap();
    assert_eq!(revised["tags"], json!(["review:late-entry"]));
    assert_eq!(
        revised["log"].as_array().unwrap().last().unwrap()["changes"]["tags"],
        json!({ "from": ["strategy:retest-short"], "to": ["review:late-entry"] })
    );
    let bad = json!({ "id": one["id"], "reason": "r", "tags": ["x".repeat(65)] });
    assert_eq!(
        errors(&call(&s, "revise_setup", bad, "owner").unwrap_err()),
        ["tags[0]"]
    );

    // Closed, it takes tags alone.
    s.on_bar(
        &key(),
        bar(t + MINUTE_MS, p * 0.976, p * 0.977, p * 0.9, p * 0.91),
    );
    let both = json!({ "id": one["id"], "reason": "r", "tags": ["review:r1"], "thesis": "x" });
    assert_eq!(
        errors(&call(&s, "revise_setup", both, "owner").unwrap_err()),
        ["id"]
    );
    let args = json!({ "id": one["id"], "reason": "reviewed", "tags": ["review:r1"] });
    let reviewed = call(&s, "revise_setup", args, "owner").unwrap();
    assert_eq!(reviewed["status"], "invalidated");
    assert_eq!(reviewed["tags"], json!(["review:r1"]));
    assert_eq!(kinds(&reviewed).last(), Some(&"revised"));
}

/// Review tags on a closed setup do not count against its revisions.
#[tokio::test]
async fn review_tags_fit_a_closed_setup_out_of_revisions() {
    let limits = Limits {
        agent_revisions_per_setup: 1,
        ..Limits::default()
    };
    let s = live_on(mem(), hub(), Access::parse(None), limits, None);
    let one = call(&s, "create_setup", long(), "owner").unwrap();
    let thesis = json!({ "id": one["id"], "reason": "r", "thesis": "changed" });
    call(&s, "revise_setup", thesis.clone(), "owner").unwrap();
    let full = call(&s, "revise_setup", thesis, "owner").unwrap_err();
    assert!(full.contains("at most 1 revisions per setup"), "{full}");
    let (t, p) = (first_bar(&one), mid(&one));
    s.on_bar(&key(), bar(t, p, p, p * 0.975, p * 0.976));
    s.on_bar(
        &key(),
        bar(t + MINUTE_MS, p * 0.976, p * 0.977, p * 0.9, p * 0.91),
    );
    let args = json!({ "id": one["id"], "reason": "reviewed", "tags": ["review:r1"] });
    let reviewed = call(&s, "revise_setup", args, "owner").unwrap();
    assert_eq!(reviewed["status"], "invalidated");
    assert_eq!(reviewed["tags"], json!(["review:r1"]));
}

/// A row stored before tags and excursions existed loads with none, and is listed.
#[tokio::test]
async fn a_row_from_before_tags_and_excursions_loads() {
    let (store, dir) = store_at("old-rows");
    let s = live_on(store, hub(), Access::parse(None), Limits::default(), None);
    let new = created(&s, "owner");
    let mut old: Value = serde_json::from_str(&store.setups().unwrap()[0].0).unwrap();
    for field in [
        "tags",
        "mae_r",
        "mfe_r",
        "mae_at",
        "mfe_at",
        "closest_approach_r",
    ] {
        old.as_object_mut().unwrap().remove(field);
    }
    old["id"] = json!("old");
    let row = StoredSetup {
        id: "old".into(),
        owner: "owner".into(),
        record: old.to_string(),
        revisions: vec![],
    };
    store.save_setups(&[row]).unwrap();
    drop(s);
    let s = live_on(
        reopen(&dir.db()),
        hub(),
        Access::parse(None),
        Limits::default(),
        None,
    );
    let got = call(&s, "get_setup", json!({ "id": "old" }), "owner").unwrap();
    assert_eq!(got["tags"], json!([]));
    assert_eq!(got["mae_r"], Value::Null);
    assert_eq!(got["bars_to_exit"], Value::Null);
    assert_eq!(got["closest_approach_r"], Value::Null);
    let list = call(&s, "list_setups", json!({}), "owner").unwrap();
    assert_eq!(list["total"], 2);
    assert_eq!(
        call(&s, "get_setup", json!({ "id": new["id"] }), "owner").unwrap()["tags"],
        json!([])
    );
}

/// v1's runner outlives the setup's close (ADR-0066): the setup's feed is kept, across a
/// restart too, until the last shadow closes. It holds no place among the open setups
/// and no symbol of the owner's.
#[tokio::test]
async fn an_open_shadow_keeps_its_feed_after_the_setup_closed_and_counts_toward_no_limit() {
    let (store, dir) = store_at("shadows");
    let limits = Limits {
        open_per_owner: 1,
        symbols_per_owner: 1,
        ..Limits::default()
    };
    let s = live_on(store, hub(), Access::parse(None), limits, None);
    let mut args = long();
    let p = args["entry"]["high"].as_f64().unwrap() / 0.98;
    args["targets"] = json!([p * 1.05, p * 1.08]);
    let setup = call(&s, "create_setup", args, "owner").unwrap();
    assert_eq!(setup["shadow_results"], Value::Null, "none before the fill");
    let (id, t) = (id_of(&setup), first_bar(&setup));
    let get = |s: &Setups| call(s, "get_setup", json!({ "id": id }), "owner").unwrap();

    s.on_bar(&key(), bar(t, p, p, p * 0.975, p * 0.976));
    assert_eq!(get(&s)["shadow_results"]["v1_half_t1_be"]["status"], "open");
    s.on_bar(
        &key(),
        bar(t + MINUTE_MS, p * 0.976, p * 1.06, p * 0.975, p * 1.05),
    );
    let won = get(&s);
    assert_eq!(won["status"], "won");
    let v1 = &won["shadow_results"]["v1_half_t1_be"];
    assert_eq!(v1["status"], "open");
    assert_eq!(v1["exits"][0]["fraction"], 0.5);
    assert_eq!(won["shadow_results"]["v2_be_at_1r"]["status"], "won");
    // Still watched for the runner, and not open: another setup, on another symbol,
    // fits the limits of one open setup on one symbol.
    assert_eq!(s.pairs(), HashSet::from([key()]));
    let open = call(&s, "list_setups", json!({ "status": "open" }), "owner").unwrap();
    assert_eq!(open["total"], 0);
    let other = call(&s, "create_setup", long_on("ETHUSDT"), "owner").unwrap();
    call(
        &s,
        "cancel_setup",
        json!({ "id": other["id"], "reason": "example reason" }),
        "owner",
    )
    .unwrap();

    // A restart: the closed setup's open shadow joins its feed again.
    drop(s);
    let s = live_on(reopen(&dir.db()), hub(), Access::parse(None), limits, None);
    assert!(s.pairs().contains(&key()));
    s.on_bar(
        &key(),
        bar(t + 2 * MINUTE_MS, p * 1.05, p * 1.09, p * 1.04, p * 1.085),
    );
    let done = get(&s);
    let v1 = &done["shadow_results"]["v1_half_t1_be"];
    assert_eq!(v1["status"], "won");
    assert_eq!(v1["exited_at"], json!(t + 2 * MINUTE_MS));
    assert!(v1["result_r"].as_f64().unwrap() > done["result_r"].as_f64().unwrap());
    // The setup's own fields stay as its close left them, its log too.
    assert_eq!(done["result_r"], won["result_r"]);
    assert_eq!(done["exit"], won["exit"]);
    assert_eq!(kinds(&done), ["created", "status", "status"]);
    // Nothing is left to score: the feed is let go.
    assert!(!s.pairs().contains(&key()));
}

// ---- ADR-0061: a setup's status changes as events ----

fn events_of(store: &Store, owner: &str) -> Vec<persistence::LoggedEvent> {
    store.events_after(owner, 0, 100).unwrap().0
}

/// Setups beside alerts whose receiver, at `port`, wakes the agent of `owner`.
fn waking(store: &'static Store, port: u16, owner: &str) -> (Arc<Setups>, Arc<ServerAlerts>) {
    let alerts = ServerAlerts::new(
        store,
        hub(),
        Policy::default(),
        Access::Open,
        None,
        crate::server_alerts::Limits::default(),
        Timing::default(),
    );
    let url = format!("http://127.0.0.1:{port}/webhooks/route");
    let agent = chartgen::webhook::AgentReceiver::from_vars(
        |name| match name {
            "CHARTGEN_AGENT_WEBHOOK_URL" => Some(url.clone()),
            "CHARTGEN_AGENT_WEBHOOK_OWNER" => Some(owner.to_string()),
            "CHARTGEN_AGENT_WEBHOOK_SECRET" => {
                Some("a-secret-of-thirty-two-characters".to_string())
            }
            _ => None,
        },
        None,
    );
    alerts.set_agent(agent.unwrap().unwrap());
    let setups = live_on(
        store,
        hub(),
        Access::parse(None),
        Limits::default(),
        Some(alerts.clone()),
    );
    (setups, alerts)
}

/// A receiver that answers 200 and keeps the bodies it got.
async fn receiver() -> (u16, Arc<Mutex<Vec<String>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let got = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let mut buf = vec![0u8; 16384];
            let n = s.read(&mut buf).await.unwrap_or(0);
            if buf.starts_with(b"POST ") {
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                got.lock()
                    .unwrap()
                    .push(request.split_once("\r\n\r\n").unwrap().1.to_string());
            }
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                .await;
            let _ = s.shutdown().await;
        }
    });
    (port, seen)
}

/// Each status the server gives a setup is an event of its owner's log, written with
/// the entry that says it, at the bar's time. For a setup the agent took, with a
/// receiver configured for the owner, it is also a wake-up that shows the setup.
#[tokio::test(flavor = "multi_thread")]
async fn a_status_the_server_gives_a_setup_is_logged_at_the_bars_time_and_wakes_the_agent() {
    let (port, seen) = receiver().await;
    let store = mem();
    let (s, _alerts) = waking(store, port, "owner");
    let setup = created(&s, "owner");
    let (t, p) = (first_bar(&setup), mid(&setup));
    assert!(
        events_of(store, "owner").is_empty(),
        "making a setup is no event"
    );
    s.on_bar(&key(), bar(t, p, p, p * 0.975, p * 0.976));
    s.on_bar(
        &key(),
        bar(t + MINUTE_MS, p * 0.976, p * 1.06, p * 0.976, p * 1.05),
    );

    let log = events_of(store, "owner");
    let told: Vec<(String, i64, Value)> = log
        .iter()
        .map(|e| (e.kind.clone(), e.at, serde_json::from_str(&e.body).unwrap()))
        .collect();
    assert_eq!(told.len(), 2, "{told:?}");
    assert_eq!(
        (told[0].0.as_str(), told[0].1),
        ("setup.status", t),
        "the bar's time, not the write's"
    );
    assert_eq!(
        told[0].2,
        json!({ "from": "watching", "to": "triggered", "gap": false })
    );
    assert_eq!(told[1].1, t + MINUTE_MS);
    assert_eq!(
        (told[1].2["from"].clone(), told[1].2["to"].clone()),
        (json!("triggered"), json!("won"))
    );
    assert!(told[1].2["result_r"].as_f64().unwrap() > 0.0);
    assert!(log
        .iter()
        .all(|e| e.setup_id.as_deref() == Some(&id_of(&setup)) && e.alert_id.is_none()));
    // What happened, and nothing of the setup's text.
    assert!(log
        .iter()
        .all(|e| !e.body.contains("thesis") && e.body.len() < 200));

    for _ in 0..600 {
        if seen.lock().unwrap().len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let bodies: Vec<Value> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|b| serde_json::from_str(b).unwrap())
        .collect();
    assert_eq!(bodies.len(), 2, "both wake the agent");
    let filled = bodies
        .iter()
        .find(|b| b["change"]["to"] == "triggered")
        .unwrap();
    assert_eq!(
        (filled["type"].clone(), filled["at"].clone()),
        (json!("setup.status"), json!(t))
    );
    assert_eq!(filled["id"], log[0].id.as_str());
    // The event says what happened then; the setup is shown as it is when it is sent.
    assert_eq!(filled["setup"]["id"], id_of(&setup).as_str());
    assert_eq!(filled["setup"]["thesis"], "example thesis");
    // A filled setup's exit variants (ADR-0066), as get_setup has them.
    let variants = &filled["setup"]["shadow_results"];
    for v in ["v1_half_t1_be", "v2_be_at_1r", "v3_time_stop"] {
        assert!(variants[v]["status"].is_string(), "{v}: {variants}");
    }
    assert!(filled.get("alert").is_none() && filled.get("note").is_none());
    let raw = seen.lock().unwrap()[0].clone();
    assert!(raw.find("\"change\":").unwrap() < raw.find("\"setup\":").unwrap());
    assert!(raw.find("\"levels\":").unwrap() < raw.find("\"thesis\":").unwrap());
}

/// A cancellation is the agent's own act, and nobody waits on a skip: both are in the
/// log and neither wakes anyone. Nor does another owner's setup.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancellation_a_skip_and_another_owners_setup_are_logged_and_wake_nobody() {
    let (port, seen) = receiver().await;
    let store = mem();
    let (s, _alerts) = waking(store, port, "owner");

    let mine = created(&s, "owner");
    let (t, p) = (first_bar(&mine), mid(&mine));
    call(
        &s,
        "cancel_setup",
        json!({ "id": mine["id"], "reason": "example reason" }),
        "owner",
    )
    .unwrap();
    let skip = json!({ "symbol": "BTCUSDT", "interval": "1h", "direction": "long",
        "decision": "skip", "thesis": "example reason" });
    let skip = call(&s, "create_setup", skip, "owner").unwrap();
    let theirs = created(&s, "someone-else");
    // Bars that cancel the first, and fill the skip and the other owner's.
    for i in 0..3 {
        s.on_bar(&key(), bar(t + i * MINUTE_MS, p, p * 1.3, p * 0.5, p));
    }
    let log = events_of(store, "owner");
    let to: Vec<String> = log
        .iter()
        .map(|e| {
            serde_json::from_str::<Value>(&e.body).unwrap()["to"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert!(to.contains(&"cancelled".to_string()), "{to:?}");
    assert!(
        log.iter()
            .any(|e| e.setup_id.as_deref() == Some(&id_of(&skip))),
        "the skip's changes are logged: {to:?}"
    );
    assert!(
        log.iter().all(|e| e.delivery == "none"),
        "{:?}",
        log.iter().map(|e| &e.delivery).collect::<Vec<_>>()
    );
    let others = events_of(store, "someone-else");
    assert!(!others.is_empty() && others.iter().all(|e| e.delivery == "none"));
    assert!(others
        .iter()
        .all(|e| e.setup_id.as_deref() == Some(&id_of(&theirs))));
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(seen.lock().unwrap().is_empty(), "nobody was woken");
}

/// A replay keeps no event log: the same bars change the same statuses and log nothing,
/// so its answers stay what they were.
#[tokio::test(flavor = "multi_thread")]
async fn a_replay_logs_no_events() {
    crate::replay::on(Replay::of(fixture(usize::MAX, 0.0)), |replay| {
        let s = replaying(replay);
        let p = replay.quote("BTCUSDT", at_minute(1500)).unwrap().0;
        let args = json!({
            "symbol": "BTCUSDT", "interval": "15m", "direction": "long", "thesis": "example",
            "entry": { "low": p * 0.99, "high": p * 0.995 }, "invalidation": p * 0.97,
            "targets": [p * 1.02], "expires_at": at_minute(1600), "as_of": at_minute(1500),
        });
        let made = call(&s, "create_setup", args, "agent").unwrap();
        let read = json!({ "id": made["id"], "as_of": at_minute(2900) });
        let later = call(&s, "get_setup", read, "agent").unwrap();
        assert_ne!(
            later["status"], "watching",
            "the bars must move it: {later}"
        );
        assert!(s.store.events_after("agent", 0, 10).unwrap().0.is_empty());
        assert_eq!(s.store.last_event_seq("agent").unwrap(), 0);
    });
}

/// The log is kept for 7 days, and a wake-up waits for an hour, wherever events are
/// written: with setups on and server alerts off there is nobody else to see to it.
#[tokio::test(flavor = "multi_thread")]
async fn the_event_log_is_tidied_where_only_setups_run() {
    let store = mem();
    let s = live_on(store, hub(), Access::parse(None), Limits::default(), None);
    let made = created(&s, "owner");
    let (t, p) = (first_bar(&made), mid(&made));
    s.on_bar(&key(), bar(t, p, p, p * 0.975, p * 0.976));
    // A wake-up left waiting from a time when a receiver was configured.
    let waiting = NewEvent {
        id: "left-waiting".to_string(),
        owner: "owner".to_string(),
        kind: "setup.status".to_string(),
        at: t,
        alert_id: None,
        setup_id: Some(id_of(&made)),
        body: "{}".to_string(),
        deliver: true,
    };
    store.save_setups_logging(&[], &[waiting]).unwrap();
    let delivery = || {
        let log = events_of(store, "owner");
        log.iter().map(|e| e.delivery.clone()).collect::<Vec<_>>()
    };
    let now = chrono::Utc::now().timestamp();
    s.tidy_events(now);
    assert_eq!(delivery(), ["none", "waiting"], "nothing is old yet");
    s.tidy_events(now + persistence::EVENT_NEWS_SECS + 1);
    assert_eq!(
        delivery(),
        ["none", "failed"],
        "given up after the hour, and kept"
    );
    s.tidy_events(now + persistence::EVENT_TTL_SECS + 1);
    assert!(events_of(store, "owner").is_empty(), "gone after 7 days");
    assert_eq!(
        store.last_event_seq("owner").unwrap(),
        2,
        "the numbers go on"
    );
}

// ---- ADR-0064: what a setup was made with, and the record read back ----

/// `long()` with the three fields an agent may state when it makes a setup.
fn stated() -> Value {
    let mut args = long();
    args["features"] = json!({ "example_number": 1.5, "example_flag": true, "example_label": "a" });
    args["probability"] = json!(0.6);
    args["playbook_version"] = json!("v1");
    args
}

/// The three fields are stored with the setup and shown by every answer, and the first
/// entry of its log holds them too. The log is what the store keeps from being
/// rewritten, so what a setup was made with cannot be changed afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn what_a_setup_was_made_with_is_stored_shown_and_in_its_first_log_entry() {
    let (store, dir) = store_at("stated");
    let s = live_on(store, hub(), Access::parse(None), Limits::default(), None);
    let made = call(&s, "create_setup", stated(), "owner").unwrap();
    let features = json!({ "example_flag": true, "example_label": "a", "example_number": 1.5 });
    for view in [
        made.clone(),
        call(&s, "get_setup", json!({ "id": made["id"] }), "owner").unwrap(),
        call(&s, "list_setups", json!({}), "owner").unwrap()["setups"][0].clone(),
    ] {
        assert_eq!(view["features"], features);
        assert_eq!(view["probability"], 0.6);
        assert_eq!(view["playbook_version"], "v1");
    }
    let first = &made["log"][0];
    assert_eq!(first["kind"], "created");
    assert_eq!(
        first["changes"],
        json!({
            "features": { "to": features },
            "probability": { "to": 0.6 },
            "playbook_version": { "to": "v1" },
        })
    );

    // The store refuses to write that entry again with other values.
    let id = id_of(&made);
    let record = store.setups().unwrap().remove(0).0;
    let altered = first.to_string().replace("0.6", "0.9");
    let rewrite = persistence::StoredSetup {
        id: id.clone(),
        owner: "owner".to_string(),
        record,
        revisions: vec![(1, altered)],
    };
    let refused = store.save_setups(&[rewrite]).unwrap_err().to_string();
    assert!(refused.contains("append-only"), "{refused}");

    // A restart reads them back, and a status the server gives leaves them alone.
    drop(s);
    let s = live_on(
        reopen(&dir.db()),
        hub(),
        Access::parse(None),
        Limits::default(),
        None,
    );
    let (t, p) = (first_bar(&made), mid(&made));
    s.on_bar(&key(), bar(t, p, p, p * 0.975, p * 0.976));
    let later = call(&s, "get_setup", json!({ "id": id }), "owner").unwrap();
    assert_eq!(later["status"], "triggered");
    assert_eq!(later["features"], features);
    assert_eq!(
        (
            later["probability"].clone(),
            later["playbook_version"].clone()
        ),
        (json!(0.6), json!("v1"))
    );
    assert_eq!(later["log"][0], *first);
}

/// A setup made without them shows none of the three, and its log entry is as it was
/// before they existed: answers of such setups do not change by a byte.
#[tokio::test(flavor = "multi_thread")]
async fn a_setup_made_without_them_shows_none_of_the_three() {
    let s = live();
    let made = created(&s, "owner");
    let listed = call(&s, "list_setups", json!({}), "owner").unwrap()["setups"][0].clone();
    for view in [&made, &listed] {
        for field in ["features", "probability", "playbook_version"] {
            assert!(view.get(field).is_none(), "{field} in {view}");
        }
    }
    assert_eq!(made["log"][0]["changes"], json!({}));
    let stored = s.store.setups().unwrap().remove(0).0;
    assert!(
        !stored.contains("features")
            && !stored.contains("probability")
            && !stored.contains("playbook")
    );
}

/// A record that could be rewritten after the outcome is no record: no tool takes the
/// three fields after the setup was made, open or closed.
#[tokio::test(flavor = "multi_thread")]
async fn what_a_setup_was_made_with_cannot_be_revised() {
    let s = live();
    let made = call(&s, "create_setup", stated(), "owner").unwrap();
    for (field, value) in [
        ("features", json!({ "example_number": 2.5 })),
        ("features", json!(null)),
        ("probability", json!(0.9)),
        ("playbook_version", json!("v2")),
    ] {
        let mut args = json!({ "id": made["id"], "reason": "example reason", "thesis": "another example thesis" });
        args[field] = value;
        let refused = call(&s, "revise_setup", args, "owner").unwrap_err();
        assert_eq!(errors(&refused), [field], "{refused}");
        assert!(
            refused.contains("is fixed when a setup is made"),
            "{refused}"
        );
        let mut args = json!({ "id": made["id"], "reason": "example reason" });
        args[field] = json!(0.9);
        let refused = call(&s, "cancel_setup", args, "owner").unwrap_err();
        assert!(
            refused.contains("is fixed when a setup is made"),
            "{refused}"
        );
    }
    // Nothing of those calls was applied, the thesis they carried included.
    let now = call(&s, "get_setup", json!({ "id": made["id"] }), "owner").unwrap();
    assert_eq!(now, made);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_three_fields_are_checked_and_bounded() {
    let limits = Limits {
        changes_per_minute: 1000,
        ..Limits::default()
    };
    let s = live_on(mem(), hub(), Access::parse(None), limits, None);
    let long_text = "x".repeat(33);
    let many: serde_json::Map<String, Value> =
        (0..17).map(|i| (format!("key_{i}"), json!(i))).collect();
    for (field, bad) in [
        ("features", json!([1, 2])),
        ("features", json!("example")),
        ("features", json!(many)),
        ("features.Example", json!({ "Example": 1 })),
        ("features.has space", json!({ "has space": 1 })),
        ("features.", json!({ "": 1 })),
        ("features.example", json!({ "example": [1] })),
        ("features.example", json!({ "example": { "nested": 1 } })),
        ("features.example", json!({ "example": null })),
        ("features.example", json!({ "example": long_text })),
        ("probability", json!(-0.01)),
        ("probability", json!(1.01)),
        ("probability", json!("0.5")),
        ("playbook_version", json!("")),
        ("playbook_version", json!(long_text)),
        ("playbook_version", json!(3)),
    ] {
        let mut args = long();
        let top = field.split('.').next().unwrap();
        args[top] = bad.clone();
        let refused = call(&s, "create_setup", args, "owner").unwrap_err();
        assert_eq!(errors(&refused), [field], "{bad}: {refused}");
    }
    let key = "k".repeat(33);
    let mut args = long();
    args["features"] = json!({ key.clone(): 1 });
    let refused = call(&s, "create_setup", args, "owner").unwrap_err();
    assert!(refused.contains("at most 32"), "{refused}");
    assert!(s.store.setups().unwrap().is_empty(), "nothing was stored");

    // The edges are taken: 16 entries, 32 characters, 0 and 1, and null as not given.
    let mut full: serde_json::Map<String, Value> =
        (0..15).map(|i| (format!("key_{i}"), json!(i))).collect();
    full.insert("k".repeat(32), json!("x".repeat(32)));
    for (probability, features) in [
        (json!(0), json!(full)),
        (json!(1), json!({})),
        (json!(null), json!(null)),
    ] {
        let mut args = long();
        args["probability"] = probability;
        args["features"] = features;
        args["playbook_version"] = json!("x".repeat(32));
        call(&s, "create_setup", args, "owner").unwrap();
    }
}

/// The log's first entry is what says what a setup was made with. A record row that
/// lost the three fields, as a build from before them writes it, or that holds other
/// values, is read with the log's values at the next start.
#[tokio::test(flavor = "multi_thread")]
async fn what_a_setup_was_made_with_is_read_from_its_log_at_start() {
    let (store, dir) = store_at("restated");
    let s = live_on(store, hub(), Access::parse(None), Limits::default(), None);
    let made = call(&s, "create_setup", stated(), "owner").unwrap();
    let plain = created(&s, "owner");
    drop(s);

    let rewrite = |change: &dyn Fn(&mut Value)| {
        let row = store.setups().unwrap().into_iter().map(|(row, _)| row);
        let mut record: Value = row
            .map(|row| serde_json::from_str::<Value>(&row).unwrap())
            .find(|r| r["id"] == made["id"])
            .unwrap();
        change(&mut record);
        let row = persistence::StoredSetup {
            id: id_of(&made),
            owner: "owner".to_string(),
            record: record.to_string(),
            revisions: Vec::new(),
        };
        store.save_setups(&[row]).unwrap();
    };
    let read = |id: &Value| {
        let s = live_on(
            reopen(&dir.db()),
            hub(),
            Access::parse(None),
            Limits::default(),
            None,
        );
        call(&s, "get_setup", json!({ "id": id }), "owner").unwrap()
    };

    // As an older build stores the setup when it writes it: without the fields.
    rewrite(&|record| {
        for field in setups::STATED_FIELDS {
            record.as_object_mut().unwrap().remove(field);
        }
    });
    assert_eq!(read(&made["id"]), made, "all three are back, from the log");
    // And a row that says something else does not count.
    rewrite(&|record| {
        record["probability"] = json!(0.99);
        record["playbook_version"] = json!("another");
        record["features"] = json!({ "example_number": 9 });
    });
    assert_eq!(read(&made["id"]), made);
    // A setup made without them still has none.
    assert_eq!(read(&plain["id"]), plain);
}

/// A setup that the bars fill and then take to its target, or through its
/// invalidation. Returns it as it is afterwards.
fn closed(s: &Setups, args: Value, owner: &str, win: bool) -> Value {
    let made = call(s, "create_setup", args, owner).unwrap();
    let (t, p) = (first_bar(&made), mid(&made));
    s.on_bar(&key(), bar(t, p, p, p * 0.975, p * 0.976));
    let end = match win {
        true => bar(t + MINUTE_MS, p * 0.976, p * 1.06, p * 0.976, p * 1.05),
        false => bar(t + MINUTE_MS, p * 0.976, p * 0.976, p * 0.9, p * 0.9),
    };
    s.on_bar(&key(), end);
    let now = call(s, "get_setup", json!({ "id": made["id"] }), owner).unwrap();
    assert_eq!(
        now["status"],
        if win { "won" } else { "invalidated" },
        "{now}"
    );
    now
}

fn with(mut args: Value, more: Value) -> Value {
    for (k, v) in more.as_object().unwrap() {
        args[k] = v.clone();
    }
    args
}

fn keys(record: &Value) -> Vec<Value> {
    let groups = record["groups"].as_array().unwrap();
    groups.iter().map(|g| g["key"].clone()).collect()
}

/// `get_track_record` counts the caller's closed setups, in all and per group, and
/// shows the stated probability beside what happened. Open setups and other owners'
/// are not in it.
#[tokio::test(flavor = "multi_thread")]
async fn the_track_record_counts_closed_setups_in_all_and_per_group() {
    let limits = Limits {
        changes_per_minute: 1000,
        ..Limits::default()
    };
    let s = live_on(mem(), hub(), Access::parse(None), limits, None);
    let record = |args: Value| call(&s, "get_track_record", args, "owner").unwrap();
    let nothing = record(json!({}));
    assert_eq!(
        (nothing["closed"].clone(), nothing["total"]["count"].clone()),
        (json!(0), json!(0))
    );
    assert!(
        nothing["total"]["mean_r"].is_null(),
        "no mean of nothing: {nothing}"
    );
    assert!(nothing["total"]["probability"]["observed"].is_null());

    let a = closed(&s, stated(), "owner", true);
    let b_args = with(
        long(),
        json!({ "features": { "example_flag": false }, "probability": 0.25, "playbook_version": "v1" }),
    );
    let b = closed(&s, b_args, "owner", false);
    let c = closed(&s, long(), "owner", true);
    closed(&s, stated(), "someone-else", true);
    created(&s, "owner");

    let r = |v: &Value| v["result_r"].as_f64().unwrap();
    let sum = r(&a) + r(&b) + r(&c);
    let all = record(json!({}));
    assert_eq!(
        (all["closed"].clone(), all["open"].clone()),
        (json!(3), json!(1))
    );
    assert_eq!(all["truncated"], false);
    assert_eq!(all["groups"], json!([]));
    assert!(all["group_by"].is_null() && all["feature"].is_null());
    let total = &all["total"];
    assert_eq!(
        (
            total["count"].clone(),
            total["won"].clone(),
            total["invalidated"].clone()
        ),
        (json!(3), json!(2), json!(1))
    );
    assert_eq!(
        (total["expired"].clone(), total["cancelled"].clone()),
        (json!(0), json!(0))
    );
    assert_eq!(
        (total["with_result"].clone(), total["gaps"].clone()),
        (json!(3), json!(0))
    );
    assert!((total["total_r"].as_f64().unwrap() - sum).abs() < 1e-9);
    assert!((total["mean_r"].as_f64().unwrap() - sum / 3.0).abs() < 1e-9);
    // Two stated a probability: 0.6 and won, 0.25 and lost. Both were entered.
    assert_eq!(
        total["probability"],
        json!({ "stated": 2, "mean": 0.425, "won": 1, "observed": 0.5,
            "stated_entered": 2, "won_entered": 1, "observed_entered": 0.5 })
    );

    let by_version = record(json!({ "group_by": "playbook_version" }));
    assert_eq!(keys(&by_version), [json!(null), json!("v1")]);
    assert_eq!(by_version["groups"][0]["count"], 1);
    assert_eq!(by_version["groups"][1]["count"], 2);
    assert_eq!(by_version["groups"][1]["won"], 1);
    assert_eq!(
        by_version["total"], all["total"],
        "the total does not depend on the grouping"
    );
    assert_eq!(
        keys(&record(json!({ "group_by": "probability" }))),
        [json!(null), json!(0.2), json!(0.6)]
    );
    let by_flag = record(json!({ "group_by": "feature", "feature": "example_flag" }));
    assert_eq!(keys(&by_flag), [json!(null), json!(false), json!(true)]);
    assert_eq!(by_flag["feature"], "example_flag");
    assert_eq!(
        keys(&record(
            json!({ "group_by": "feature", "feature": "no_such" })
        )),
        [json!(null)]
    );
    assert_eq!(
        keys(&record(json!({ "group_by": "symbol" }))),
        [json!("BTCUSDT")]
    );
    assert_eq!(
        keys(&record(json!({ "group_by": "direction" }))),
        [json!("long")]
    );

    // Filters.
    for (filter, count) in [
        (json!({ "playbook_version": "v1" }), 2),
        (json!({ "playbook_version": "v2" }), 0),
        (json!({ "direction": "short" }), 0),
        (
            json!({ "direction": "long", "decision": "take", "symbol": "btcusdt" }),
            3,
        ),
        (json!({ "decision": "skip" }), 0),
        (json!({ "symbol": "ETHUSDT" }), 0),
        (json!({ "from": "2020-01-01" }), 3),
        (json!({ "to": "2020-01-01" }), 0),
        (
            json!({ "from": a["created_at"], "to": a["created_at"].as_i64().unwrap() + 1 }),
            1,
        ),
    ] {
        assert_eq!(record(filter.clone())["total"]["count"], count, "{filter}");
    }

    // What it does not take.
    for (bad, field) in [
        (json!({ "group_by": "thesis" }), "group_by"),
        (json!({ "group_by": "feature" }), "feature"),
        (json!({ "feature": "example_flag" }), "feature"),
        (
            json!({ "group_by": "symbol", "feature": "example_flag" }),
            "feature",
        ),
        (json!({ "direction": "up" }), "direction"),
        (json!({ "decision": "maybe" }), "decision"),
        (json!({ "from": "yesterday" }), "from"),
        (json!({ "status": "won" }), "status"),
        (
            json!({ "playbook_version": "x".repeat(33) }),
            "playbook_version",
        ),
    ] {
        let refused = call(&s, "get_track_record", bad.clone(), "owner").unwrap_err();
        assert_eq!(errors(&refused), [field], "{bad}: {refused}");
    }
    assert_eq!(
        call(&s, "get_track_record", json!({}), "a-stranger").unwrap()["closed"],
        0
    );

    // A grouping with more values than an answer holds is refused, not cut.
    let one = s.owned("owner", &id_of(&a)).unwrap();
    let many: Vec<Record> = (0..=setups::MAX_RECORD_GROUPS)
        .map(|i| {
            let mut r = one.clone();
            r.features = Some(serde_json::Map::from_iter([("n".to_string(), json!(i))]));
            r
        })
        .collect();
    let all_of_them: Vec<&Record> = many.iter().collect();
    let by = setups::GroupBy::Feature("n".to_string());
    assert!(setups::track_record(&all_of_them, Some(&by)).is_err());
    let (total, groups) = setups::track_record(&all_of_them[1..], Some(&by)).unwrap();
    assert_eq!((total["count"].clone(), groups.len()), (json!(50), 50));
    assert_eq!(
        groups[1]["key"], 2.0,
        "by number, not by text: {}",
        groups[1]
    );
}

/// A result with a gap, an expiry without a result and a cancellation are counted as
/// what they are: a mean is over the results there are.
#[tokio::test(flavor = "multi_thread")]
async fn the_track_record_counts_what_has_no_result_without_inventing_one() {
    let s = live();
    let cancelled = created(&s, "owner");
    let cancel = json!({ "id": cancelled["id"], "reason": "example reason" });
    call(&s, "cancel_setup", cancel, "owner").unwrap();
    let (t, p) = (first_bar(&cancelled), mid(&cancelled));
    s.on_bar(&key(), bar(t, p, p, p, p));
    let won = closed(&s, long(), "owner", true);
    // One that expired filled while nobody scored it: no result, and a gap.
    let mut lost = s.owned("owner", &id_of(&won)).unwrap();
    lost.id = "made-up".to_string();
    lost.status = Status::Expired;
    lost.result_r = None;
    lost.gap = true;
    let key = ("owner".to_string(), lost.id.clone());
    let held = Held {
        record: lost,
        record_bytes: 0,
        log_bytes: 0,
    };
    s.lock().setups.insert(key, held);

    let total = call(&s, "get_track_record", json!({}), "owner").unwrap()["total"].clone();
    assert_eq!(
        (
            total["count"].clone(),
            total["won"].clone(),
            total["expired"].clone(),
            total["cancelled"].clone()
        ),
        (json!(3), json!(1), json!(1), json!(1))
    );
    assert_eq!(total["gaps"], 1);
    let results = total["with_result"].as_u64().unwrap();
    assert!(results < 3, "the gap has no result: {total}");
    let mean = total["total_r"].as_f64().unwrap() / results as f64;
    assert!((total["mean_r"].as_f64().unwrap() - mean).abs() < 1e-9);
}

/// On a replay server the record is answered as of a time, as every tool there is,
/// and is the same on every run: it is what an evaluation reads its score from.
#[tokio::test]
async fn the_track_record_on_a_replay_is_the_same_on_every_run() {
    let run = || {
        crate::replay::on(Replay::of(fixture(usize::MAX, 0.0)), |replay| {
            let s = replaying(replay);
            let p = replay.quote("BTCUSDT", at_minute(1500)).unwrap().0;
            for (i, probability) in [0.3, 0.7].into_iter().enumerate() {
                let args = json!({
                    "symbol": "BTCUSDT", "interval": "15m", "direction": "long", "thesis": "example",
                    "entry": { "low": p * 0.99, "high": p * 0.995 }, "invalidation": p * 0.97,
                    "targets": [p * 1.02], "expires_at": at_minute(1700 + i as i64),
                    "as_of": at_minute(1500 + i as i64),
                    "probability": probability, "playbook_version": "v1",
                    "features": { "example_number": i },
                });
                call(&s, "create_setup", args, "agent").unwrap();
            }
            let without = call(&s, "get_track_record", json!({}), "agent").unwrap_err();
            assert!(without.contains("as_of"), "{without}");
            let early = json!({ "as_of": at_minute(1502), "group_by": "probability" });
            let early = call(&s, "get_track_record", early, "agent").unwrap();
            assert_eq!(
                (early["closed"].clone(), early["open"].clone()),
                (json!(0), json!(2))
            );
            let late = json!({ "as_of": at_minute(2900), "group_by": "probability" });
            let late = call(&s, "get_track_record", late, "agent").unwrap();
            assert_eq!(
                (late["closed"].clone(), late["replay"].clone()),
                (json!(2), json!(true))
            );
            late.to_string()
        })
    };
    assert_eq!(run(), run());
}

/// The answers that hold what a setup was made with, and the record read back, pinned
/// in a file the web app's types are tested against, as `setup_responses.json` is. From
/// a replay over the example fixtures, so the same on every run. To write the file
/// again after a deliberate change: `CHARTGEN_WRITE_FIXTURES=1 cargo test record_answers`.
#[tokio::test]
async fn record_answers_match_the_file_the_web_types_are_tested_against() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let replay = Replay::load(&dir.join("replay")).unwrap();
    let answers = crate::replay::on(replay, |replay| {
        let s = replaying(replay);
        let start = 1_767_225_600_000 + 600 * MINUTE_MS;
        let at = |minute: i64| start + minute * MINUTE_MS;
        let ask = |tool: &str, args: Value| call(&s, tool, args, "agent").unwrap();
        let p = replay.quote("BTCUSDT", at(0)).unwrap().0;
        let stated = ask(
            "create_setup",
            json!({
                "symbol": "BTCUSDT", "interval": "15m", "direction": "long",
                "thesis": "example thesis",
                "entry": { "low": p * 0.997, "high": p * 0.999 }, "invalidation": p * 0.99,
                "targets": [p * 1.008], "expires_at": at(300), "as_of": at(0),
                "features": { "example_number": 1.5, "example_flag": true, "example_label": "a" },
                "probability": 0.6, "playbook_version": "v1",
            }),
        );
        ask(
            "create_setup",
            json!({ "symbol": "ETHUSDT", "interval": "15m", "direction": "short",
                "decision": "skip", "thesis": "example reason", "as_of": at(0),
                "probability": 0.4 }),
        );
        json!({
            "create_setup_stated": stated,
            "get_track_record": ask("get_track_record", json!({ "as_of": at(700) })),
            "get_track_record_grouped": ask(
                "get_track_record",
                json!({ "as_of": at(700), "group_by": "feature", "feature": "example_flag" }),
            ),
        })
    });
    assert_eq!(
        answers["get_track_record"]["closed"], 2,
        "both must have closed by then"
    );
    let path = dir.join("track_record_responses.json");
    let text = serde_json::to_string_pretty(&answers).unwrap() + "\n";
    if std::env::var("CHARTGEN_WRITE_FIXTURES").is_ok() {
        std::fs::write(&path, &text).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), text);
}

/// A setup that was cancelled or ran out before its entry was reached has a result of
/// 0 R. It is in `mean_r`, which is over every result there is; `entered` and
/// `mean_r_entered` leave it out, so both readings can be had from one answer.
#[tokio::test(flavor = "multi_thread")]
async fn the_track_record_tells_setups_that_were_entered_from_those_that_never_traded() {
    let s = live();
    let won = closed(&s, long(), "owner", true);
    let base = s.owned("owner", &id_of(&won)).unwrap();
    assert!(base.fill.is_some());
    // Four that were entered and two that never traded, with results chosen by hand.
    let made = |entered: bool, status: Status, result: f64, version: &str| {
        let mut r = base.clone();
        r.status = status;
        r.result_r = Some(result);
        r.playbook_version = Some(version.to_string());
        if !entered {
            r.fill = None;
            r.exit = None;
        }
        r
    };
    let six = [
        made(true, Status::Won, 0.984, "v1"),
        made(true, Status::Invalidated, -1.016, "v1"),
        made(true, Status::Invalidated, -1.016, "v2"),
        made(true, Status::Invalidated, -1.016, "v2"),
        made(false, Status::Expired, 0.0, "v1"),
        made(false, Status::Cancelled, 0.0, "v2"),
    ];
    let all: Vec<&Record> = six.iter().collect();
    let by = setups::GroupBy::PlaybookVersion;
    let (total, groups) = setups::track_record(&all, Some(&by)).unwrap();
    let near = |v: &Value, to: f64| (v.as_f64().unwrap() - to).abs() < 1e-9;
    assert_eq!(
        (total["count"].clone(), total["with_result"].clone()),
        (json!(6), json!(6))
    );
    assert!(near(&total["mean_r"], -0.344), "{total}");
    assert_eq!(total["entered"], 4);
    assert!(near(&total["mean_r_entered"], -0.516), "{total}");
    // Per group as well: v1 has two entered of three, v2 two of three.
    assert_eq!(
        (groups[0]["key"].clone(), groups[0]["entered"].clone()),
        (json!("v1"), json!(2))
    );
    assert!(
        near(&groups[0]["mean_r"], (0.984 - 1.016) / 3.0),
        "{}",
        groups[0]
    );
    assert!(near(&groups[0]["mean_r_entered"], (0.984 - 1.016) / 2.0));
    assert!(near(&groups[1]["mean_r_entered"], -1.016));

    // Entered, and no result: one that ran out filled while nobody scored it.
    let mut unscored = made(true, Status::Expired, 0.0, "v1");
    unscored.result_r = None;
    unscored.gap = true;
    let (total, _) = setups::track_record(&[&unscored], None).unwrap();
    assert_eq!(
        (total["entered"].clone(), total["with_result"].clone()),
        (json!(1), json!(0))
    );
    assert!(
        total["mean_r_entered"].is_null(),
        "no mean of no result: {total}"
    );
    // None entered: a count of 0 and no mean.
    let (total, _) = setups::track_record(&[&six[4], &six[5]], None).unwrap();
    assert_eq!(total["entered"], 0);
    assert!(
        total["mean_r_entered"].is_null() && near(&total["mean_r"], 0.0),
        "{total}"
    );
}

/// The probability's observed share also over the setups that were entered: one that
/// never traded cannot win, so the share over all mixes the chance of a fill into it.
#[tokio::test(flavor = "multi_thread")]
async fn the_track_record_gives_the_observed_share_over_entered_setups_too() {
    let s = live();
    let won = closed(&s, long(), "owner", true);
    let base = s.owned("owner", &id_of(&won)).unwrap();
    let made = |entered: bool, status: Status, p: Option<f64>| {
        let mut r = base.clone();
        r.status = status;
        r.probability = p;
        if !entered {
            r.fill = None;
            r.exit = None;
        }
        r
    };
    let four = [
        made(true, Status::Won, Some(0.6)),
        made(true, Status::Invalidated, Some(0.6)),
        made(false, Status::Expired, Some(0.6)),
        made(true, Status::Won, None),
    ];
    let all: Vec<&Record> = four.iter().collect();
    let (total, _) = setups::track_record(&all, None).unwrap();
    let p = &total["probability"];
    // Over the three that stated one: one won. Over the two of those entered: one won.
    assert_eq!(
        (p["stated"].clone(), p["won"].clone()),
        (json!(3), json!(1))
    );
    assert!(
        (p["observed"].as_f64().unwrap() - 1.0 / 3.0).abs() < 1e-9,
        "{p}"
    );
    assert_eq!(
        (p["stated_entered"].clone(), p["won_entered"].clone()),
        (json!(2), json!(1))
    );
    assert_eq!(p["observed_entered"], 0.5);
    // None entered: a count of 0 and no share.
    let (total, _) = setups::track_record(&[&four[2]], None).unwrap();
    assert_eq!(total["probability"]["stated_entered"], 0);
    assert!(total["probability"]["observed_entered"].is_null());
}

/// The fixed exit variants are counted per variant beside the total: the positions
/// that closed with their results, and those still open.
#[tokio::test(flavor = "multi_thread")]
async fn the_track_record_counts_each_exit_variant_and_those_still_open() {
    use chartgen::setups::ShadowStatus;
    let s = live();
    let won = closed(&s, long(), "owner", true);
    let base = s.owned("owner", &id_of(&won)).unwrap();
    assert!(base.shadow_results.is_some(), "a filled setup has variants");
    let with = |v1: Option<f64>, v2: f64, v3: Option<f64>| {
        let mut r = base.clone();
        let sh = r.shadow_results.as_mut().unwrap();
        let set = |shadow: &mut chartgen::setups::Shadow, result: Option<f64>| match result {
            Some(x) => {
                shadow.status = ShadowStatus::Won;
                shadow.result_r = Some(x);
            }
            None => {
                shadow.status = ShadowStatus::Open;
                shadow.result_r = None;
            }
        };
        set(&mut sh.v1_half_t1_be, v1);
        set(&mut sh.v2_be_at_1r, Some(v2));
        set(&mut sh.v3_time_stop, v3);
        r
    };
    let mut never = base.clone();
    never.fill = None;
    never.shadow_results = None;
    let four = [
        with(Some(1.5), 1.0, Some(0.5)),
        with(None, -1.0, Some(-0.5)),
        with(Some(0.5), 0.0, None),
        never,
    ];
    let all: Vec<&Record> = four.iter().collect();
    let v = setups::shadow_record(&all);
    assert_eq!(
        v["v1_half_t1_be"],
        json!({ "closed": 2, "with_result": 2, "total_r": 2.0, "mean_r": 1.0, "open": 1 })
    );
    assert_eq!(
        v["v2_be_at_1r"],
        json!({ "closed": 3, "with_result": 3, "total_r": 0.0, "mean_r": 0.0, "open": 0 })
    );
    assert_eq!(
        v["v3_time_stop"],
        json!({ "closed": 2, "with_result": 2, "total_r": 0.0, "mean_r": 0.0, "open": 1 })
    );
    // In the answer, beside the total.
    let answer = call(&s, "get_track_record", json!({}), "owner").unwrap();
    assert!(
        answer["shadows"]["v2_be_at_1r"]["closed"].is_u64(),
        "{answer}"
    );
}

/// A feature's number is one value however it was written: 1 and 1.0 are one group.
#[tokio::test(flavor = "multi_thread")]
async fn the_track_record_groups_a_number_by_its_value_not_by_how_it_was_written() {
    let s = live();
    let won = closed(&s, long(), "owner", true);
    let base = s.owned("owner", &id_of(&won)).unwrap();
    let with = |value: Value| {
        let mut r = base.clone();
        r.features = Some(serde_json::Map::from_iter([("n".to_string(), value)]));
        r
    };
    let four = [
        with(json!(1)),
        with(json!(1.0)),
        with(json!(2)),
        with(json!("1")),
    ];
    let all: Vec<&Record> = four.iter().collect();
    let by = setups::GroupBy::Feature("n".to_string());
    let (_, groups) = setups::track_record(&all, Some(&by)).unwrap();
    let shown: Vec<String> = groups.iter().map(|g| g["key"].to_string()).collect();
    assert_eq!(
        shown,
        ["1.0", "2.0", "\"1\""],
        "a number as a decimal, a string as it is"
    );
    assert_eq!(groups[0]["count"], 2);
}

/// A call that only reads is not answered in the words of one that stores.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_track_record_does_not_speak_of_storing() {
    let s = live();
    let refused = call(
        &s,
        "get_track_record",
        json!({ "group_by": "thesis" }),
        "owner",
    )
    .unwrap_err();
    assert_eq!(errors(&refused), ["group_by"]);
    assert!(
        !refused.contains("stored") && !refused.contains("this setup"),
        "{refused}"
    );
    assert!(refused.contains("nothing was read"), "{refused}");
}
