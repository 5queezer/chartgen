use super::*;
use crate::chart_push::{Limits as HubLimits, PushSource};
use chartgen::data::{Bar, OhlcvData};
use futures_util::future::BoxFuture;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

const BASE: i64 = 1_700_000_040;

/// A source that sends only what a test tells it to, through the real hub. Most
/// tests call `on_event` directly.
#[derive(Default)]
struct Fake {
    seeds: AtomicUsize,
    feeds: Mutex<Vec<mpsc::Sender<FeedEvent>>>,
}

impl Fake {
    async fn send(&self, event: FeedEvent) {
        let feeds = self.feeds.lock().unwrap().clone();
        for tx in feeds {
            tx.send(event.clone()).await.unwrap();
        }
    }
}

impl PushSource for Fake {
    fn feed_interval(&self, symbol: &str, timeframe: &str) -> Result<String, String> {
        if timeframe == "2m" || !symbol.ends_with("USDT") {
            return Err("not pushable".into());
        }
        // An interval with no stream of its own is built from a finer one.
        Ok(if timeframe == "7m" { "1m" } else { timeframe }.to_string())
    }
    fn seed(&self, symbol: &str, timeframe: &str, count: usize) -> Result<OhlcvData, String> {
        self.seeds.fetch_add(1, Ordering::SeqCst);
        if symbol == "NOSUCHUSDT" {
            return Err("unknown symbol".into());
        }
        data::sample_ohlcv(symbol, timeframe, count, None, 1_704_067_200_000)
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

fn store_at(name: &str) -> (&'static Store, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "chartgen-server-alerts-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("store.db");
    (reopen(&path), path)
}

fn reopen(path: &std::path::Path) -> &'static Store {
    Box::leak(Box::new(Store::open(path).unwrap()))
}

fn quick_timing() -> Timing {
    Timing {
        connect: Duration::from_millis(300),
        total: Duration::from_millis(500),
        backoff: [Duration::from_millis(10), Duration::from_millis(10)],
    }
}

fn hub_with(fake: Arc<Fake>) -> Arc<ChartHub> {
    ChartHub::new(fake, HubLimits::default())
}

fn alerts_on(store: &'static Store, policy: Policy, limits: Limits) -> Arc<ServerAlerts> {
    let hub = hub_with(Arc::new(Fake::default()));
    ServerAlerts::new(
        store,
        hub,
        policy,
        Access::Open,
        None,
        limits,
        quick_timing(),
    )
}

fn plain(store: &'static Store) -> Arc<ServerAlerts> {
    alerts_on(store, Policy::default(), Limits::default())
}

/// `call_tool` blocks on the runtime, so it runs where a real call does.
async fn call(
    a: &Arc<ServerAlerts>,
    tool: &str,
    args: Value,
    owner: &str,
) -> Result<Value, String> {
    let (a, tool, owner) = (a.clone(), tool.to_string(), owner.to_string());
    let resp = tokio::task::spawn_blocking(move || a.call_tool(&tool, None, &args, Some(&owner)))
        .await
        .unwrap();
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

fn spec(condition: &str, trigger: &str, level: f64) -> Value {
    json!({ "symbol": "BTCUSDT", "condition": condition, "trigger": trigger, "level": level })
}

async fn create(a: &Arc<ServerAlerts>, args: Value, owner: &str) -> String {
    call(a, "create_server_alert", args, owner).await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn bar(t: i64, close: f64) -> Bar {
    Bar {
        open: close,
        high: close,
        low: close,
        close,
        volume: 1.0,
        date: t.to_string(),
    }
}

fn key(interval: &str) -> FeedKey {
    ("BTCUSDT".to_string(), interval.to_string())
}

fn tick(a: &ServerAlerts, t: i64, price: f64) {
    a.on_event(&key("1m"), FeedEvent::Tick(bar(t, price)));
}

async fn get(a: &Arc<ServerAlerts>, owner: &str, id: &str) -> Value {
    let list = call(a, "list_server_alerts", json!({}), owner)
        .await
        .unwrap();
    list["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == id)
        .cloned()
        .unwrap_or(Value::Null)
}

fn fixture() -> Value {
    let raw = include_str!("../../tests/fixtures/alert_semantics.json");
    serde_json::from_str(raw).unwrap()
}

/// The browser runs these same cases (`web/src/stores/alerts.test.ts`).
#[tokio::test(flavor = "multi_thread")]
async fn the_fire_rules_match_the_browser_fixture() {
    for (i, case) in fixture()["cases"].as_array().unwrap().iter().enumerate() {
        let name = case["name"].as_str().unwrap();
        let (store, _) = store_at(&format!("fixture-{i}"));
        let a = plain(store);
        let id = create(
            &a,
            spec(
                case["condition"].as_str().unwrap(),
                case["trigger"].as_str().unwrap(),
                case["level"].as_f64().unwrap(),
            ),
            "me",
        )
        .await;
        let mut fired = Vec::new();
        let mut count = 0;
        for (n, price) in case["prices"].as_array().unwrap().iter().enumerate() {
            match price.as_f64() {
                Some(p) => tick(&a, BASE, p),
                None => ServerAlerts::forget_baselines(&mut a.lock(), &key("1m")),
            }
            let now = a.lock().alerts[&id].fire_count;
            if now > count {
                fired.push(json!(n));
                count = now;
            }
        }
        assert_eq!(fired, case["fires"].as_array().unwrap().clone(), "{name}");
        assert_eq!(a.lock().alerts[&id].active, case["active"], "{name}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_once_alert_does_not_fire_again_across_a_restart() {
    let (store, path) = store_at("restart-once");
    let a = plain(store);
    let id = create(&a, spec("greater", "once", 100.0), "me").await;
    tick(&a, BASE, 101.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 1);
    drop(a);

    let a = plain(reopen(&path));
    assert!(!a.lock().alerts[&id].active);
    assert!(a.lock().watches.is_empty(), "a spent alert holds no feed");
    assert_eq!(a.lock().alerts[&id].fire_count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_every_alert_stays_disarmed_across_a_restart() {
    let (store, path) = store_at("restart-every");
    let a = plain(store);
    let id = create(&a, spec("greater", "every", 100.0), "me").await;
    tick(&a, BASE, 101.0);
    assert!(a.lock().alerts[&id].disarmed);
    drop(a);

    let a = plain(reopen(&path));
    tick(&a, BASE, 102.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 1, "still on that side");
    tick(&a, BASE, 99.0);
    tick(&a, BASE, 103.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 2, "re-armed and fired");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crossing_needs_a_reading_taken_since_the_restart() {
    let (store, path) = store_at("restart-cross");
    let a = plain(store);
    let id = create(&a, spec("crossing", "every", 100.0), "me").await;
    tick(&a, BASE, 99.0);
    drop(a);

    let a = plain(reopen(&path));
    tick(&a, BASE, 101.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 0, "99 is from before");
    tick(&a, BASE, 99.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 1);
}

/// A replay is a bar at or before the last close: the backfill after a reconnect.
#[tokio::test(flavor = "multi_thread")]
async fn replayed_bars_do_not_fire_and_restart_the_baseline() {
    let (store, _) = store_at("replay");
    let a = plain(store);
    let id = create(&a, spec("crossing", "every", 100.0), "me").await;
    tick(&a, BASE, 99.0);
    a.on_event(&key("1m"), FeedEvent::Bar(bar(BASE, 99.0)));
    tick(&a, BASE + 60, 99.0);
    a.on_event(&key("1m"), FeedEvent::Bar(bar(BASE - 60, 101.0)));
    a.on_event(&key("1m"), FeedEvent::Bar(bar(BASE, 101.0)));
    assert_eq!(a.lock().alerts[&id].fire_count, 0);
    // The next price is only a baseline, like a browser after a gap.
    tick(&a, BASE + 60, 101.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 0);
    tick(&a, BASE + 60, 99.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_close_alert_reads_only_finished_bars_once_each() {
    let (store, _) = store_at("close");
    let a = plain(store);
    let mut args = spec("crossing", "every", 100.0);
    args["interval"] = json!("1h");
    args["evaluate_on"] = json!("close");
    let id = create(&a, args, "me").await;
    let k = key("1h");
    assert_eq!(a.lock().alerts[&id].feed_interval, "1h");

    a.on_event(&k, FeedEvent::Bar(bar(BASE - 7200, 99.0)));
    // Ticks cross the level but no bar closed there.
    a.on_event(&k, FeedEvent::Tick(bar(BASE - 3600, 101.0)));
    assert_eq!(a.lock().alerts[&id].fire_count, 0);
    a.on_event(&k, FeedEvent::Bar(bar(BASE - 3600, 101.0)));
    assert_eq!(a.lock().alerts[&id].fire_count, 1);
    // The same bar again, as a backfill sends it, changes nothing.
    a.on_event(&k, FeedEvent::Bar(bar(BASE - 3600, 99.0)));
    assert_eq!(a.lock().alerts[&id].fire_count, 1);
}

/// Busy pairs roll the candle on a trade before the closing kline arrives.
#[tokio::test(flavor = "multi_thread")]
async fn closes_fire_when_trades_roll_the_candle_first() {
    let (store, _) = store_at("late-close");
    let fake = Arc::new(Fake::default());
    let a = ServerAlerts::new(
        store,
        hub_with(fake.clone()),
        Policy::default(),
        Access::Open,
        None,
        Limits::default(),
        quick_timing(),
    );
    let mut crossing = spec("crossing", "every", 100.0);
    crossing["interval"] = json!("1h");
    crossing["evaluate_on"] = json!("close");
    let crossing = create(&a, crossing, "me").await;
    let mut greater = spec("greater", "once", 50.0);
    greater["interval"] = json!("1h");
    greater["evaluate_on"] = json!("close");
    let greater = create(&a, greater, "me").await;

    // Through the hub, as the real feed sends them.
    let closes = [99.0, 101.0, 99.0, 101.0];
    for (i, close) in closes.into_iter().enumerate() {
        let open = BASE + 3600 * i as i64;
        fake.send(FeedEvent::Tick(bar(open, close))).await;
        fake.send(FeedEvent::Tick(bar(open + 3600, close))).await;
        fake.send(FeedEvent::Bar(bar(open, close))).await;
    }
    until("every close to be read", || {
        a.lock().alerts[&crossing].fire_count == 3
    })
    .await;
    assert_eq!(a.lock().alerts[&greater].fire_count, 1);
    // The newest price is the forming candle's, not the late close's.
    let watch_last = a.lock().watches[&key("1h")].last_bar;
    assert_eq!(watch_last, Some(BASE + 3600 * 4));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_late_close_leaves_live_alerts_alone() {
    let (store, _) = store_at("late-live");
    let a = plain(store);
    let id = create(&a, spec("crossing", "every", 100.0), "me").await;
    tick(&a, BASE, 99.0);
    tick(&a, BASE + 60, 99.5);
    // Above the level, but late: not a price now, and the baseline stays.
    a.on_event(&key("1m"), FeedEvent::Bar(bar(BASE, 101.0)));
    assert_eq!(a.lock().alerts[&id].fire_count, 0);
    let last = a.lock().watches[&key("1m")].last_price;
    assert_eq!(last, Some(99.5));
    tick(&a, BASE + 60, 101.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_late_tick_is_ignored_without_touching_the_baseline() {
    let (store, _) = store_at("late-tick");
    let a = plain(store);
    let id = create(&a, spec("crossing", "every", 100.0), "me").await;
    tick(&a, BASE + 60, 99.0);
    tick(&a, BASE, 101.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 0);
    assert_eq!(a.lock().watches[&key("1m")].last_price, Some(99.0));
    tick(&a, BASE + 60, 101.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 1);
}

/// Closes from before the watch began are history, not news.
#[tokio::test(flavor = "multi_thread")]
async fn closes_older_than_the_watch_do_not_fire() {
    let (store, _) = store_at("old-closes");
    let a = plain(store);
    let mut args = spec("greater", "once", 50.0);
    args["interval"] = json!("1h");
    args["evaluate_on"] = json!("close");
    let id = create(&a, args, "me").await;
    let k = key("1h");
    a.on_event(&k, FeedEvent::Tick(bar(BASE, 60.0)));
    a.on_event(&k, FeedEvent::Bar(bar(BASE - 7200, 60.0)));
    a.on_event(&k, FeedEvent::Bar(bar(BASE - 3600, 60.0)));
    assert_eq!(a.lock().alerts[&id].fire_count, 0);
    a.on_event(&k, FeedEvent::Bar(bar(BASE, 60.0)));
    assert_eq!(a.lock().alerts[&id].fire_count, 1);
}

/// The stream's closed flag decides, not the clock: a 28-day month is over when
/// the kline says so, and so is an hour on a clock running behind.
#[tokio::test(flavor = "multi_thread")]
async fn a_close_is_trusted_whatever_the_clock_says() {
    let now = chrono::Utc::now().timestamp();
    for (interval, age) in [("1M", 28 * 86_400), ("1M", 29 * 86_400), ("1h", 3598)] {
        let (store, _) = store_at(&format!("clock-{interval}-{age}"));
        let a = plain(store);
        let mut args = spec("greater", "once", 50.0);
        args["interval"] = json!(interval);
        args["evaluate_on"] = json!("close");
        let id = create(&a, args, "me").await;
        a.on_event(&key(interval), FeedEvent::Bar(bar(now - age, 60.0)));
        assert_eq!(a.lock().alerts[&id].fire_count, 1, "{interval} {age}");
    }
}

/// `2m` and `90m` have no stream, but a live alert only reads the 1m feed.
#[tokio::test(flavor = "multi_thread")]
async fn a_live_alert_takes_any_interval_as_its_label() {
    let (store, _) = store_at("label");
    let a = plain(store);
    for interval in ["2m", "90m", "5d"] {
        let mut args = spec("less", "once", 1.0);
        args["interval"] = json!(interval);
        let id = create(&a, args, "me").await;
        assert_eq!(a.lock().alerts[&id].feed_interval, "1m");
    }
    let (store, _) = store_at("label-bad");
    let a = plain(store);
    for bad in ["", "m", "12", "1h/../x", "123456789m", "1 h"] {
        let mut args = spec("less", "once", 1.0);
        args["interval"] = json!(bad);
        let e = call(&a, "create_server_alert", args, "me")
            .await
            .unwrap_err();
        assert!(e.contains("not an interval"), "{bad}: {e}");
        a.lock().changes = Window::default();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn alerts_share_one_feed_per_symbol_when_live() {
    let (store, _) = store_at("feeds");
    let a = plain(store);
    for interval in ["1m", "15m", "1h"] {
        let mut args = spec("less", "every", 1.0);
        args["interval"] = json!(interval);
        create(&a, args, "me").await;
    }
    assert_eq!(
        a.lock().watches.len(),
        1,
        "live alerts all read the 1m feed"
    );
    let mut args = spec("less", "every", 1.0);
    args["interval"] = json!("1h");
    args["evaluate_on"] = json!("close");
    create(&a, args, "me").await;
    assert_eq!(a.lock().watches.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paused_or_spent_alert_lets_go_of_its_feed() {
    let (store, _) = store_at("release");
    let a = plain(store);
    let id = create(&a, spec("greater", "once", 100.0), "me").await;
    assert_eq!(a.lock().watches.len(), 1);
    call(
        &a,
        "update_server_alert",
        json!({"alert_id": id, "active": false}),
        "me",
    )
    .await
    .unwrap();
    assert_eq!(a.lock().watches.len(), 0);
    call(
        &a,
        "update_server_alert",
        json!({"alert_id": id, "active": true}),
        "me",
    )
    .await
    .unwrap();
    assert_eq!(a.lock().watches.len(), 1);
    tick(&a, BASE, 101.0);
    assert_eq!(a.lock().watches.len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn update_and_resume_follow_the_browser() {
    let (store, _) = store_at("update");
    let a = plain(store);
    let id = create(&a, spec("crossing", "once", 100.0), "me").await;
    tick(&a, BASE, 99.0);
    tick(&a, BASE, 101.0);
    let view = get(&a, "me", &id).await;
    assert_eq!(view["active"], false);
    assert!(view["last_triggered_at"].is_i64());

    // Resuming keeps when it last fired, re-arms and needs a new reading.
    let args = json!({"alert_id": id, "active": true});
    let view = call(&a, "update_server_alert", args, "me").await.unwrap();
    assert_eq!(view["active"], true);
    assert!(view["last_triggered_at"].is_i64());
    tick(&a, BASE, 99.0);
    assert_eq!(
        a.lock().alerts[&id].fire_count,
        1,
        "the first is a baseline"
    );

    // Pausing forgets the past fire, so it reads as paused.
    let args = json!({"alert_id": id, "active": false});
    let view = call(&a, "update_server_alert", args, "me").await.unwrap();
    assert!(view["last_triggered_at"].is_null());

    // A new definition switches it on, without a fire behind it.
    let args = json!({"alert_id": id, "level": 200.0, "message": " note "});
    let view = call(&a, "update_server_alert", args, "me").await.unwrap();
    assert_eq!(
        (&view["active"], &view["level"]),
        (&json!(true), &json!(200.0))
    );
    assert_eq!(view["message"], "note");
    let args = json!({"alert_id": id, "message": ""});
    let view = call(&a, "update_server_alert", args, "me").await.unwrap();
    assert!(view["message"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn another_client_sees_and_changes_nothing() {
    let (store, _) = store_at("owner");
    let a = plain(store);
    let id = create(&a, spec("less", "once", 1.0), "me").await;
    let theirs = call(&a, "list_server_alerts", json!({}), "them")
        .await
        .unwrap();
    assert_eq!(theirs["alerts"], json!([]));
    let args = json!({"alert_id": id, "active": false});
    assert_eq!(
        call(&a, "update_server_alert", args, "them")
            .await
            .unwrap_err(),
        "No such alert"
    );
    let args = json!({"alert_id": id});
    assert_eq!(
        call(&a, "delete_server_alert", args.clone(), "them")
            .await
            .unwrap()["deleted"],
        false
    );
    assert_eq!(get(&a, "me", &id).await["active"], true);
    assert_eq!(
        call(&a, "delete_server_alert", args, "me").await.unwrap()["deleted"],
        true
    );
    assert!(a.lock().alerts.is_empty());
    assert_eq!(a.lock().watches.len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_without_a_client_is_refused() {
    let (store, _) = store_at("no-owner");
    let a = plain(store);
    let args = json!({});
    let b = a.clone();
    let resp =
        tokio::task::spawn_blocking(move || b.call_tool("list_server_alerts", None, &args, None))
            .await
            .unwrap();
    assert_eq!(resp["result"]["isError"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_input_is_refused() {
    let (store, _) = store_at("input");
    let limits = Limits {
        changes_per_minute: 100,
        ..Limits::default()
    };
    let a = alerts_on(store, Policy::default(), limits);
    let check = |args: Value, needle: &'static str| {
        let a = a.clone();
        async move {
            let e = call(&a, "create_server_alert", args, "me")
                .await
                .unwrap_err();
            assert!(e.contains(needle), "{e}");
        }
    };
    check(
        json!({"condition": "crossing", "trigger": "once", "level": 1}),
        "symbol",
    )
    .await;
    check(spec("sideways", "once", 1.0), "condition").await;
    check(spec("crossing", "twice", 1.0), "trigger").await;
    check(spec("crossing", "once", 0.0), "level").await;
    check(spec("crossing", "once", -5.0), "level").await;
    let mut args = spec("crossing", "once", 1.0);
    args["symbol"] = json!("AAPL");
    check(args, "Binance").await;
    let mut args = spec("crossing", "once", 1.0);
    args["sign"] = json!(true);
    check(args, "sign needs webhook_url").await;
    let mut args = spec("crossing", "once", 1.0);
    args["evaluate_on"] = json!("close");
    args["interval"] = json!("2m");
    check(args, "cannot watch").await;
    let mut args = spec("crossing", "once", 1.0);
    args["evaluate_on"] = json!("close");
    args["interval"] = json!("7m");
    check(args, "kline stream").await;
    assert!(a.lock().alerts.is_empty());
}

/// `create` on the real source, with the gate shut. The gate is one per process, so this
/// runs in a process of its own, with a dead proxy so that a request past the gate fails
/// here and never reaches Binance.
#[tokio::test(flavor = "multi_thread")]
async fn on_the_real_source_a_new_symbol_asks_the_binance_gate() {
    const CHILD: &str = "CHARTGEN_TEST_GATE_SHUT";
    if std::env::var_os(CHILD).is_none() {
        let test = "server_alerts::tests::on_the_real_source_a_new_symbol_asks_the_binance_gate";
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env(CHILD, "1")
            .env("HTTPS_PROXY", "http://127.0.0.1:9")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success() && text.contains("1 passed"), "{text}");
        return;
    }
    let gate = chartgen::binance_gate::spot();
    gate.observe(gate.admit().unwrap(), 429, Some("600"), None);
    let (store, _) = store_at("gate");
    let hub = ChartHub::new(
        Arc::new(crate::chart_push::BinanceSource),
        HubLimits::default(),
    );
    let a = ServerAlerts::new(
        store,
        hub,
        Policy::default(),
        Access::Open,
        None,
        Limits::default(),
        quick_timing(),
    );
    let refused = |interval: &str, on: &str| {
        let mut args = spec("crossing", "once", 1.0);
        args["interval"] = json!(interval);
        args["evaluate_on"] = json!(on);
        let a = a.clone();
        async move {
            call(&a, "create_server_alert", args, "me")
                .await
                .unwrap_err()
        }
    };
    // A native close, and a custom interval as a live alert's label (ADR-0038).
    for (interval, on) in [("1h", "close"), ("7m", "live"), ("2wk", "live")] {
        let e = refused(interval, on).await;
        assert!(
            e.starts_with("Cannot watch BTCUSDT: Binance rate limit"),
            "{interval} {on}: {e}"
        );
    }
    // A custom interval's closes are built from a finer stream, so there are none to read.
    for interval in ["2m", "7m", "45m", "5d", "2wk"] {
        let e = refused(interval, "close").await;
        assert!(e.contains("kline stream"), "{interval}: {e}");
    }
    assert!(refused("61m", "close").await.contains("cannot watch"));
    assert!(a.lock().alerts.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_symbol_new_to_the_server_is_checked_once() {
    let (store, _) = store_at("symbol");
    let fake = Arc::new(Fake::default());
    let a = ServerAlerts::new(
        store,
        hub_with(fake.clone()),
        Policy::default(),
        Access::Open,
        None,
        Limits::default(),
        quick_timing(),
    );
    let mut args = spec("less", "once", 1.0);
    args["symbol"] = json!("NOSUCHUSDT");
    let e = call(&a, "create_server_alert", args, "me")
        .await
        .unwrap_err();
    assert!(e.contains("Cannot watch NOSUCHUSDT"), "{e}");
    assert_eq!(fake.seeds.load(Ordering::SeqCst), 1);
    create(&a, spec("less", "once", 1.0), "me").await;
    assert_eq!(fake.seeds.load(Ordering::SeqCst), 2);
    create(&a, spec("less", "once", 2.0), "me").await;
    assert_eq!(
        fake.seeds.load(Ordering::SeqCst),
        2,
        "that feed already runs"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn each_ssrf_refusal_reaches_the_caller_and_stores_nothing() {
    let (store, _) = store_at("ssrf");
    let limits = Limits {
        changes_per_minute: 1000,
        ..Limits::default()
    };
    let a = alerts_on(store, Policy::default(), limits);
    for url in [
        "http://example.com/",
        "https://127.0.0.1/",
        "https://10.0.0.1/",
        "https://100.64.0.1/",
        "https://169.254.169.254/latest/meta-data/",
        "https://[::1]/",
        "https://[::ffff:127.0.0.1]/",
        "https://[fd00::1]/",
        "https://localhost/",
        "https://2130706433/",
        "https://example.com:8443/",
        "https://u:p@example.com/",
    ] {
        let mut args = spec("less", "once", 1.0);
        args["webhook_url"] = json!(url);
        let e = call(&a, "create_server_alert", args, "me")
            .await
            .unwrap_err();
        assert!(
            e.starts_with("webhook") || e.contains("webhook URL"),
            "{url}: {e}"
        );
    }
    assert!(a.lock().alerts.is_empty());
    assert!(store.server_alerts().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn update_refuses_what_create_refuses_and_keeps_the_webhook() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("ssrf-update");
    let limits = Limits {
        changes_per_minute: 1000,
        ..Limits::default()
    };
    let a = alerts_on(store, local(port), limits);
    let id = create(&a, hooked(port, "less", "once"), "me").await;
    for url in [
        "http://example.com/",
        "https://127.0.0.1/",
        "https://169.254.169.254/latest/meta-data/",
        "https://[::ffff:127.0.0.1]/",
        "https://redis/",
        "https://localhost/",
        "https://api.binance.com/",
        "https://example.com:8443/",
    ] {
        let args = json!({"alert_id": id, "webhook_url": url});
        let e = call(&a, "update_server_alert", args, "me")
            .await
            .unwrap_err();
        assert!(e.starts_with("webhook"), "{url}: {e}");
    }
    let stored = a.lock().alerts[&id].webhook.as_ref().unwrap().url.clone();
    assert!(stored.contains("TOPSECRET"), "the old webhook stays");
}

/// An id you do not own gets the same answer whatever the URL is, without a lookup.
#[tokio::test(flavor = "multi_thread")]
async fn ownership_is_checked_before_the_url() {
    let (store, _) = store_at("oracle");
    let limits = Limits {
        changes_per_minute: 1000,
        ..Limits::default()
    };
    let a = alerts_on(store, Policy::default(), limits);
    let id = create(&a, spec("less", "once", 1.0), "me").await;
    for alert_id in ["made-up", id.as_str()] {
        for url in [
            "https://redis/",
            "https://localhost/",
            "not a url",
            "https://1.1.1.1/",
        ] {
            let args = json!({"alert_id": alert_id, "webhook_url": url});
            let e = call(&a, "update_server_alert", args, "them")
                .await
                .unwrap_err();
            assert_eq!(e, "No such alert", "{alert_id} {url}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_limits_hold() {
    let (store, _) = store_at("limits");
    let limits = Limits {
        alerts_per_client: 2,
        alerts_total: 3,
        feeds: 2,
        pairs_per_client: 1,
        changes_per_minute: 1000,
        ..Limits::default()
    };
    let a = alerts_on(store, Policy::default(), limits);
    create(&a, spec("less", "once", 1.0), "a").await;
    create(&a, spec("less", "once", 2.0), "a").await;
    let e = call(&a, "create_server_alert", spec("less", "once", 3.0), "a")
        .await
        .unwrap_err();
    assert!(e.contains("per client"), "{e}");

    // A second symbol is a second feed; the third is over the limit.
    let mut other = spec("less", "once", 1.0);
    other["symbol"] = json!("ETHUSDT");
    create(&a, other, "b").await;
    let e = call(&a, "create_server_alert", spec("less", "once", 3.0), "b")
        .await
        .unwrap_err();
    assert!(e.contains("maximum of 3"), "{e}");

    let (store, _) = store_at("limits-feeds");
    let a = alerts_on(store, Policy::default(), limits);
    create(&a, spec("less", "once", 1.0), "a").await;
    let mut other = spec("less", "once", 1.0);
    other["symbol"] = json!("ETHUSDT");
    create(&a, other, "b").await;
    let mut third = spec("less", "once", 1.0);
    third["symbol"] = json!("SOLUSDT");
    let e = call(&a, "create_server_alert", third, "c")
        .await
        .unwrap_err();
    assert!(e.contains("at most 2 symbol"), "{e}");
    // On a symbol already watched there is room.
    create(&a, spec("less", "once", 2.0), "c").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn changes_per_minute_are_limited_per_client() {
    let (store, _) = store_at("changes");
    let limits = Limits {
        changes_per_minute: 2,
        ..Limits::default()
    };
    let a = alerts_on(store, Policy::default(), limits);
    create(&a, spec("less", "once", 1.0), "a").await;
    create(&a, spec("less", "once", 2.0), "a").await;
    let e = call(&a, "create_server_alert", spec("less", "once", 3.0), "a")
        .await
        .unwrap_err();
    assert!(e.contains("per minute"), "{e}");
    create(&a, spec("less", "once", 1.0), "b").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_flapping_every_alert_is_held_to_its_rate_and_says_so() {
    let (store, _) = store_at("flap");
    let limits = Limits {
        fires_per_alert_per_minute: 3,
        ..Limits::default()
    };
    let a = alerts_on(store, Policy::default(), limits);
    let id = create(&a, spec("crossing", "every", 100.0), "me").await;
    tick(&a, BASE, 99.0);
    for i in 0..10 {
        tick(&a, BASE, if i % 2 == 0 { 101.0 } else { 99.0 });
    }
    assert_eq!(a.lock().alerts[&id].fire_count, 3);
    let view = get(&a, "me", &id).await;
    assert_eq!(view["last_error"], "fires limited to 3 a minute");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_held_back_state_alert_fires_when_its_rate_allows() {
    let (store, _) = store_at("held");
    let limits = Limits {
        fires_per_alert_per_minute: 1,
        ..Limits::default()
    };
    let a = alerts_on(store, Policy::default(), limits);
    let id = create(&a, spec("greater", "every", 100.0), "me").await;
    tick(&a, BASE, 101.0);
    tick(&a, BASE, 99.0);
    tick(&a, BASE, 101.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 1);
    assert!(
        !a.lock().alerts[&id].disarmed,
        "it stays armed while held back"
    );
    a.lock().fires = Window::default();
    tick(&a, BASE, 102.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 2);
    assert!(a.lock().alerts[&id].disarmed);
}

/// Answers each request with `reply`; records raw requests.
async fn receiver(reply: &'static str) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let mut buf = vec![0u8; 16384];
            let n = s.read(&mut buf).await.unwrap_or(0);
            // Other programs on this host probe local ports: only a POST is a delivery.
            if buf.starts_with(b"POST ") {
                seen2
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).to_string());
            }
            let _ = s.write_all(reply.as_bytes()).await;
            let _ = s.shutdown().await;
        }
    });
    (port, seen)
}

const OK: &str = "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

fn local(port: u16) -> Policy {
    Policy {
        test_origin: Some(format!("http://127.0.0.1:{port}")),
        ..Policy::default()
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

fn hooked(port: u16, condition: &str, trigger: &str) -> Value {
    let mut args = spec(condition, trigger, 100.0);
    args["webhook_url"] = json!(format!("http://127.0.0.1:{port}/hook?token=TOPSECRET"));
    args["message"] = json!("hello");
    args
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fire_posts_the_documented_payload_and_records_the_status() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("payload");
    let a = alerts_on(store, local(port), Limits::default());
    let id = create(&a, hooked(port, "crossing", "once"), "me").await;
    tick(&a, BASE, 99.0);
    tick(&a, BASE + 60, 101.0);
    until("the delivery", || !seen.lock().unwrap().is_empty()).await;
    until("its record", || {
        a.lock().alerts[&id].last_delivery.is_some()
    })
    .await;

    let raw = seen.lock().unwrap()[0].clone();
    assert!(raw.starts_with("POST /hook?token=TOPSECRET"), "{raw}");
    let body: Value = serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
    let event_id = body["id"].as_str().unwrap();
    assert_eq!(body["type"], "alert.fired");
    assert_eq!(body["alert_id"], id.as_str());
    assert_eq!(body["symbol"], "BTCUSDT");
    assert_eq!(body["interval"], "1m");
    assert_eq!(body["evaluate_on"], "live");
    assert_eq!(body["condition"], "crossing");
    assert_eq!(body["trigger"], "once");
    assert_eq!(body["level"], 100.0);
    assert_eq!(body["price"], 101.0);
    assert_eq!(body["message"], "hello");
    assert_eq!(body["bar_time"], BASE + 60);
    assert!(body["fired_at"].is_i64());
    assert!(raw
        .to_lowercase()
        .contains(&format!("x-chartgen-event: {event_id}")));
    assert!(!raw.to_lowercase().contains("x-chartgen-signature"));

    let view = get(&a, "me", &id).await;
    assert_eq!(view["last_status"], 200);
    assert!(view["last_error"].is_null());
    assert!(view["last_delivery_at"].is_i64());
    assert!(store.server_alert_events().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_secret_is_shown_once_and_signs_deliveries() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("secret");
    let a = alerts_on(store, local(port), Limits::default());
    let mut args = hooked(port, "greater", "once");
    args["sign"] = json!(true);
    let created = call(&a, "create_server_alert", args, "me").await.unwrap();
    let secret = created["webhook_secret"].as_str().unwrap().to_string();
    assert_eq!(secret.len(), 64);
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["webhook"]["signed"], true);
    assert_eq!(created["webhook"]["host"], "127.0.0.1");

    // Nothing later returns the secret or the URL.
    let list = call(&a, "list_server_alerts", json!({}), "me")
        .await
        .unwrap();
    let text = list.to_string();
    for hidden in [secret.as_str(), "TOPSECRET", "/hook"] {
        assert!(!text.contains(hidden), "{hidden} leaked: {text}");
    }
    assert!(list["alerts"][0].get("webhook_secret").is_none());
    let args = json!({"alert_id": id, "message": "x"});
    let updated = call(&a, "update_server_alert", args, "me").await.unwrap();
    assert!(updated.get("webhook_secret").is_none());

    tick(&a, BASE, 101.0);
    until("the delivery", || !seen.lock().unwrap().is_empty()).await;
    let raw = seen.lock().unwrap()[0].clone();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    let head = head.to_lowercase();
    let header = |name: &str| {
        head.lines()
            .find_map(|l| l.strip_prefix(name))
            .unwrap()
            .trim()
            .to_string()
    };
    let ts: i64 = header("x-chartgen-timestamp: ").parse().unwrap();
    assert_eq!(
        header("x-chartgen-signature: "),
        webhook::signature(&secret, ts, body)
    );

    // Rotating gives a new one, once; turning it off leaves none.
    let args = json!({"alert_id": id, "sign": true});
    let rotated = call(&a, "update_server_alert", args, "me").await.unwrap();
    assert_ne!(rotated["webhook_secret"].as_str().unwrap(), secret);
    let args = json!({"alert_id": id, "sign": false});
    let off = call(&a, "update_server_alert", args, "me").await.unwrap();
    assert_eq!(off["webhook"]["signed"], false);
    assert!(off.get("webhook_secret").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_delivery_shows_the_status_only() {
    let reply = "HTTP/1.1 500 No\r\ncontent-length: 9\r\nconnection: close\r\n\r\nBODYLEAKS";
    let (port, seen) = receiver(reply).await;
    let (store, _) = store_at("failure");
    let a = alerts_on(store, local(port), Limits::default());
    let id = create(&a, hooked(port, "greater", "once"), "me").await;
    tick(&a, BASE, 101.0);
    until("the failure", || {
        a.lock().alerts[&id].last_delivery.is_some()
    })
    .await;
    assert_eq!(seen.lock().unwrap().len(), 3, "bounded retries");
    let view = get(&a, "me", &id).await;
    assert_eq!(view["last_status"], 500);
    assert_eq!(view["last_error"], "HTTP 500");
    assert!(!view.to_string().contains("BODYLEAKS"));
    assert!(!view.to_string().contains("TOPSECRET"));
    assert!(store.server_alert_events().unwrap().is_empty(), "given up");
}

/// Over its budget a delivery waits, so a `once` alert keeps its webhook.
#[tokio::test(flavor = "multi_thread")]
async fn a_delivery_over_budget_waits_and_is_not_dropped() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("budget");
    let limits = Limits {
        webhooks_per_client_per_minute: 1,
        ..Limits::default()
    };
    let a = alerts_on(store, local(port), limits);
    let first = create(&a, hooked(port, "greater", "once"), "me").await;
    let second = create(&a, hooked(port, "greater", "once"), "me").await;
    let other = create(&a, hooked(port, "greater", "once"), "you").await;
    tick(&a, BASE, 101.0);
    until("two deliveries", || seen.lock().unwrap().len() == 2).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(seen.lock().unwrap().len(), 2, "one per client");
    for id in [&first, &second, &other] {
        assert_eq!(a.lock().alerts[id].fire_count, 1, "every alert fired");
    }
    assert_eq!(a.lock().pending, 1, "the third is waiting");
    assert_eq!(store.server_alert_events().unwrap().len(), 1, "and stored");

    // Its turn comes when the window moves on.
    a.lock().webhooks = Window::default();
    until("the third", || seen.lock().unwrap().len() == 3).await;
    until("the row to clear", || {
        store.server_alert_events().unwrap().is_empty()
    })
    .await;
    assert_eq!(a.lock().pending, 0);
    for id in [&first, &second] {
        let view = get(&a, "me", id).await;
        assert!(view["last_error"].is_null(), "{view}");
    }
}

/// With no budget it never leaves the queue, so this shows the bound exactly.
#[tokio::test(flavor = "multi_thread")]
async fn a_clients_queue_is_bounded_and_the_alert_says_so() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("queue-bound");
    let limits = Limits {
        fires_per_alert_per_minute: 1000,
        webhooks_per_client_per_minute: 0,
        pending_deliveries_per_client: 5,
        ..Limits::default()
    };
    let a = alerts_on(store, local(port), limits);
    let id = create(&a, hooked(port, "crossing", "every"), "hog").await;
    tick(&a, BASE, 99.0);
    for i in 0..10 {
        tick(&a, BASE, if i % 2 == 0 { 101.0 } else { 99.0 });
    }
    assert_eq!(a.lock().alerts[&id].fire_count, 10, "every fire counts");
    assert_eq!(a.lock().queues["hog"].len(), 5);
    assert_eq!(store.server_alert_events().unwrap().len(), 5);
    let view = get(&a, "hog", &id).await;
    assert_eq!(
        view["last_error"],
        "webhook skipped: too many deliveries waiting"
    );
    assert!(seen.lock().unwrap().is_empty());
}

/// The review's case: another client has a backlog, and a victim's `once` still
/// goes out next, not behind it.
#[tokio::test(flavor = "multi_thread")]
async fn a_backlog_does_not_keep_another_clients_delivery_waiting() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("share");
    let limits = Limits {
        changes_per_minute: 1000,
        fires_per_alert_per_minute: 1000,
        webhooks_per_client_per_minute: 1000,
        deliveries_total_per_minute: 1000,
        ..Limits::default()
    };
    let a = alerts_on(store, local(port), limits);
    create(&a, hooked(port, "crossing", "every"), "hog").await;
    tick(&a, BASE, 99.0);
    for i in 0..40 {
        tick(&a, BASE, if i % 2 == 0 { 101.0 } else { 99.0 });
    }
    let victim = create(&a, hooked(port, "greater", "once"), "victim").await;
    tick(&a, BASE, 101.0);
    assert_eq!(a.lock().alerts[&victim].fire_count, 1);
    let backlog = a.lock().queues["hog"].len();
    assert!(backlog > 30, "{backlog}");
    until("the victim's delivery", || {
        seen.lock().unwrap().iter().any(|r| r.contains(&victim))
    })
    .await;
    let sent = seen.lock().unwrap().len();
    assert!(sent < 8, "it waited behind the backlog: {sent} went first");
}

/// No client gets a second turn before the others had one.
#[tokio::test(flavor = "multi_thread")]
async fn the_client_that_waited_longest_goes_first() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("turns");
    let a = alerts_on(store, local(port), Limits::default());
    let owners = ["a", "b", "c", "d"];
    for owner in owners {
        let event = |n: u32| StoredEvent {
            id: format!("{owner}-{n}"),
            alert_id: "gone".into(),
            body: "{}".into(),
            created_at: chrono::Utc::now().timestamp(),
            device: None,
        };
        let mut st = a.lock();
        ServerAlerts::enqueue(&mut st, owner.to_string(), event(1));
        ServerAlerts::enqueue(&mut st, owner.to_string(), event(2));
    }
    let left = |a: &ServerAlerts| a.lock().queues.values().map(VecDeque::len).sum::<usize>();
    let mut served = HashSet::new();
    let before = left(&a);
    let mut turns = 0;
    while served.len() < owners.len() && turns < 16 {
        let waiting: HashMap<String, usize> = a
            .lock()
            .queues
            .iter()
            .map(|(o, q)| (o.clone(), q.len()))
            .collect();
        a.dispatch(Instant::now());
        turns += 1;
        for owner in owners {
            let now = a.lock().queues.get(owner).map_or(0, VecDeque::len);
            if now < waiting.get(owner).copied().unwrap_or(0) {
                assert!(served.insert(owner), "{owner} went twice before the others");
            }
        }
    }
    assert_eq!(served.len(), owners.len());
    assert_eq!(left(&a), before - 4);
}

/// A dead host of one client holds that client's slots, not everyone's.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_is_held_to_its_share_of_the_slots() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(HashSet::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16384];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let text = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                let event = text
                    .lines()
                    .find_map(|l| l.strip_prefix("x-chartgen-event: "))
                    .map(|e| e.trim().to_string());
                if let Some(event) = event {
                    log.lock().unwrap().insert(event);
                }
                // Never answers, and holds the connection until the client gives up.
                let _ = s.read(&mut buf).await;
            });
        }
    });
    let (store, _) = store_at("slots");
    let limits = Limits {
        deliveries_in_flight_per_client: 1,
        ..Limits::default()
    };
    // One attempt outlasts the test: the slot is free again between attempts.
    let timing = Timing {
        total: Duration::from_secs(5),
        ..quick_timing()
    };
    let hub = hub_with(Arc::new(Fake::default()));
    let a = ServerAlerts::new(store, hub, local(port), Access::Open, None, limits, timing);
    for _ in 0..3 {
        create(&a, hooked(port, "greater", "once"), "me").await;
    }
    tick(&a, BASE, 101.0);
    until("the first delivery", || !seen.lock().unwrap().is_empty()).await;
    // Several dispatch ticks, while the first still waits for its silent receiver.
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert_eq!(seen.lock().unwrap().len(), 1, "a second went at once");
}

/// A receiver that accepts and never answers drains slowly, so what its client fires
/// must wait in the queue, where the cap applies, and not pile up in started tasks
/// until nobody else has room.
#[tokio::test(flavor = "multi_thread")]
async fn a_silent_receiver_cannot_fill_the_global_queue() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = listener.accept().await {
            held.push(s);
        }
    });
    let (store, _) = store_at("silent");
    let limits = Limits {
        fires_per_alert_per_minute: 100_000,
        webhooks_per_client_per_minute: 1000,
        deliveries_total_per_minute: 1000,
        deliveries_in_flight_per_client: 1,
        pending_deliveries_per_client: 5,
        pending_deliveries: 20,
        ..Limits::default()
    };
    let timing = Timing {
        total: Duration::from_secs(30),
        ..quick_timing()
    };
    let hub = hub_with(Arc::new(Fake::default()));
    let a = ServerAlerts::new(store, hub, local(port), Access::Open, None, limits, timing);
    create(&a, hooked(port, "crossing", "every"), "hog").await;
    tick(&a, BASE, 99.0);
    // Fires faster than anything drains, for several dispatch ticks.
    let mut most = 0;
    for i in 0..30 {
        tick(&a, BASE, if i % 2 == 0 { 101.0 } else { 99.0 });
        most = most.max(a.lock().pending);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(most <= 6, "the queue's 5 and the one started: {most}");

    let victim = create(&a, hooked(port, "greater", "once"), "victim").await;
    tick(&a, BASE, 101.0);
    let error = a.lock().alerts[&victim]
        .last_delivery
        .as_ref()
        .and_then(|d| d.error.clone());
    assert_eq!(error, None, "the victim's webhook was refused");
    assert_eq!(a.lock().queues["victim"].len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_total_budget_holds_across_clients() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("budget-total");
    let limits = Limits {
        deliveries_total_per_minute: 2,
        ..Limits::default()
    };
    let a = alerts_on(store, local(port), limits);
    for owner in ["a", "b", "c", "d"] {
        create(&a, hooked(port, "greater", "once"), owner).await;
    }
    tick(&a, BASE, 101.0);
    until("two deliveries", || seen.lock().unwrap().len() == 2).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(seen.lock().unwrap().len(), 2);
    until("the two to finish", || a.lock().pending == 2).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delivery_that_waited_an_hour_is_dropped() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("stale-queue");
    let limits = Limits {
        webhooks_per_client_per_minute: 1,
        ..Limits::default()
    };
    let a = alerts_on(store, local(port), limits);
    a.lock().webhooks.record("me", Instant::now());
    create(&a, hooked(port, "greater", "once"), "me").await;
    tick(&a, BASE, 101.0);
    a.lock().queues.get_mut("me").unwrap()[0].created_at -= EVENT_MAX_AGE_SECS + 10;
    a.lock().webhooks = Window::default();
    until("it to go", || a.lock().pending == 0).await;
    assert!(seen.lock().unwrap().is_empty());
    assert!(store.server_alert_events().unwrap().is_empty());
}

fn with_access(store: &'static Store, access: Access, policy: Policy) -> Arc<ServerAlerts> {
    let hub = hub_with(Arc::new(Fake::default()));
    ServerAlerts::new(
        store,
        hub,
        policy,
        access,
        None,
        Limits::default(),
        quick_timing(),
    )
}

#[test]
fn the_access_setting_reads_as_documented() {
    assert_eq!(Access::parse(None), Access::NoWebhooks);
    assert_eq!(Access::parse(Some("")), Access::NoWebhooks);
    assert_eq!(Access::parse(Some(" , ")), Access::NoWebhooks);
    assert_eq!(Access::parse(Some("*")), Access::Open);
    let ids = Access::parse(Some(" a, b ,"));
    assert_eq!(
        ids,
        Access::Clients(HashSet::from(["a".to_string(), "b".to_string()]))
    );
    assert!(ids.may_create("a") && ids.may_use_webhooks("b"));
    assert!(!ids.may_create("c") && !ids.may_use_webhooks("c"));
    assert!(Access::NoWebhooks.may_create("c") && !Access::NoWebhooks.may_use_webhooks("c"));
}

/// Unset, as an operator who only set the flag gets: alerts for anyone, webhooks
/// for nobody, and no lookup of the URL either.
#[tokio::test(flavor = "multi_thread")]
async fn without_an_allowlist_nobody_gets_webhooks() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("no-webhooks");
    let a = with_access(store, Access::NoWebhooks, local(port));
    let e = call(
        &a,
        "create_server_alert",
        hooked(port, "greater", "once"),
        "me",
    )
    .await
    .unwrap_err();
    assert!(e.contains("operator allowed"), "{e}");
    let id = create(&a, spec("greater", "once", 100.0), "me").await;
    let args = json!({"alert_id": id, "webhook_url": format!("http://127.0.0.1:{port}/h")});
    let e = call(&a, "update_server_alert", args, "me")
        .await
        .unwrap_err();
    assert!(e.contains("operator allowed"), "{e}");
    let list = call(&a, "list_server_alerts", json!({}), "me")
        .await
        .unwrap();
    assert_eq!(
        (&list["can_create"], &list["webhooks"]),
        (&json!(true), &json!(false))
    );
    tick(&a, BASE, 101.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 1, "it still runs");
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_allowlist_keeps_everyone_else_out() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("allowlist");
    let a = with_access(store, Access::parse(Some("friend")), local(port));
    let e = call(
        &a,
        "create_server_alert",
        spec("less", "once", 1.0),
        "stranger",
    )
    .await
    .unwrap_err();
    assert!(e.contains("operator allowed"), "{e}");
    let list = call(&a, "list_server_alerts", json!({}), "stranger")
        .await
        .unwrap();
    assert_eq!(
        (&list["can_create"], &list["webhooks"]),
        (&json!(false), &json!(false))
    );
    let id = create(&a, hooked(port, "greater", "once"), "friend").await;
    tick(&a, BASE, 101.0);
    until("the delivery", || seen.lock().unwrap().len() == 1).await;
    assert_eq!(a.lock().alerts[&id].fire_count, 1);
}

/// Taking a client off the list stops its alerts and its webhooks, stored or not.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_taken_off_the_list_stops_running() {
    let (port, seen) = receiver(OK).await;
    let (store, path) = store_at("revoked");
    let a = with_access(store, Access::Open, local(port));
    let id = create(&a, hooked(port, "greater", "once"), "gone").await;
    drop(a);
    let a = with_access(reopen(&path), Access::parse(Some("other")), local(port));
    assert!(a.lock().watches.is_empty(), "holds no feed");
    assert_eq!(a.lock().alerts.len(), 1, "the row stays until it expires");
    assert!(a.lock().alerts[&id].active);
    // Even a tick for that feed, had another alert kept it open, passes it by.
    let other = create(&a, spec("less", "once", 1.0), "other").await;
    tick(&a, BASE, 101.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 0);
    assert!(a.lock().alerts.contains_key(&other));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(seen.lock().unwrap().is_empty());
}

/// A delivery waiting in the store when the server restarted, for a client the new
/// setting no longer allows: it is dropped, not sent.
#[tokio::test(flavor = "multi_thread")]
async fn a_queued_delivery_of_a_client_no_longer_allowed_is_dropped_on_restart() {
    for (name, access) in [
        ("off-list", Access::parse(Some("someone-else"))),
        ("unset", Access::NoWebhooks),
    ] {
        let (port, seen) = receiver(OK).await;
        let (store, path) = store_at(&format!("revoked-queue-{name}"));
        let a = with_access(store, Access::Open, local(port));
        let id = create(&a, hooked(port, "greater", "once"), "gone").await;
        let record = a.lock().alerts[&id].json();
        drop(a);
        let event = StoredEvent {
            id: "queued".into(),
            alert_id: id.clone(),
            body: "{}".into(),
            created_at: chrono::Utc::now().timestamp(),
            device: None,
        };
        store
            .record_server_alert_fire(&id, "gone", &record, std::slice::from_ref(&event))
            .unwrap();

        let store = reopen(&path);
        let a = with_access(store, access, local(port));
        assert_eq!(a.lock().pending, 0, "{name}");
        assert!(store.server_alert_events().unwrap().is_empty(), "{name}");
        // A dispatch or two, in case it was queued anyway.
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert!(seen.lock().unwrap().is_empty(), "{name}");
    }
}

/// The same event for a client still allowed is sent, so the drop above is the rule
/// at work and not something else losing it.
#[tokio::test(flavor = "multi_thread")]
async fn a_queued_delivery_of_a_client_still_allowed_is_sent_after_a_restart() {
    let (port, seen) = receiver(OK).await;
    let (store, path) = store_at("allowed-queue");
    let a = with_access(store, Access::parse(Some("friend")), local(port));
    let id = create(&a, hooked(port, "greater", "once"), "friend").await;
    let record = a.lock().alerts[&id].json();
    drop(a);
    let event = StoredEvent {
        id: "queued".into(),
        alert_id: id.clone(),
        body: "{}".into(),
        created_at: chrono::Utc::now().timestamp(),
        device: None,
    };
    store
        .record_server_alert_fire(&id, "friend", &record, std::slice::from_ref(&event))
        .unwrap();
    let _a = with_access(reopen(&path), Access::parse(Some("friend")), local(port));
    until("the delivery", || seen.lock().unwrap().len() == 1).await;
}

/// Going from `*` to unset: alerts run on, and what was stored sends nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_stored_webhook_stops_when_the_setting_no_longer_allows_it() {
    let (port, seen) = receiver(OK).await;
    let (store, path) = store_at("webhooks-gone");
    let a = with_access(store, Access::Open, local(port));
    let id = create(&a, hooked(port, "greater", "once"), "me").await;
    drop(a);
    let a = with_access(reopen(&path), Access::NoWebhooks, local(port));
    tick(&a, BASE, 101.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 1);
    assert_eq!(a.lock().pending, 0);
    let view = get(&a, "me", &id).await;
    assert_eq!(
        view["last_error"],
        "webhook skipped: this client may not use webhooks"
    );
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn spent_alerts_do_not_count_toward_the_total_but_rows_stay_bounded() {
    let (store, _) = store_at("spent-total");
    let limits = Limits {
        alerts_total: 2,
        rows_total: 3,
        changes_per_minute: 1000,
        ..Limits::default()
    };
    let a = alerts_on(store, Policy::default(), limits);
    create(&a, spec("greater", "once", 1.0), "a").await;
    create(&a, spec("greater", "once", 1.0), "b").await;
    let e = call(&a, "create_server_alert", spec("less", "once", 1.0), "c")
        .await
        .unwrap_err();
    assert!(e.contains("maximum of 2"), "{e}");
    tick(&a, BASE, 101.0);
    create(&a, spec("less", "once", 1.0), "c").await;
    let e = call(&a, "create_server_alert", spec("less", "once", 1.0), "d")
        .await
        .unwrap_err();
    assert!(e.contains("maximum"), "rows: {e}");
}

/// Bringing a spent alert back takes a place among the live ones, as creating does.
#[tokio::test(flavor = "multi_thread")]
async fn re_arming_a_spent_alert_is_held_to_the_total() {
    let (store, _) = store_at("rearm-total");
    let limits = Limits {
        alerts_total: 2,
        changes_per_minute: 1000,
        ..Limits::default()
    };
    let a = alerts_on(store, Policy::default(), limits);
    let by_level = create(&a, spec("greater", "once", 1.0), "a").await;
    let by_resume = create(&a, spec("greater", "once", 1.0), "b").await;
    tick(&a, BASE, 101.0);
    create(&a, spec("less", "once", 1.0), "c").await;
    let live = create(&a, spec("less", "once", 2.0), "d").await;

    // Two are live, which is the total, and two are spent.
    let by_level_args = json!({"alert_id": by_level, "level": 50.0});
    let e = call(&a, "update_server_alert", by_level_args.clone(), "a")
        .await
        .unwrap_err();
    assert!(e.contains("maximum of 2"), "{e}");
    let by_resume_args = json!({"alert_id": by_resume, "active": true});
    let e = call(&a, "update_server_alert", by_resume_args, "b")
        .await
        .unwrap_err();
    assert!(e.contains("maximum of 2"), "{e}");
    let active = a.lock().alerts.values().filter(|r| r.active).count();
    assert_eq!(active, 2, "the refused ones stay spent");

    // What keeps a spent alert spent is still free, and so is changing a live one.
    let args = json!({"alert_id": by_level, "message": "note"});
    call(&a, "update_server_alert", args, "a").await.unwrap();
    let args = json!({"alert_id": live, "level": 3.0});
    call(&a, "update_server_alert", args, "d").await.unwrap();

    // With a place free it comes back.
    call(&a, "delete_server_alert", json!({"alert_id": live}), "d")
        .await
        .unwrap();
    call(&a, "update_server_alert", by_level_args, "a")
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paused_alert_counts_toward_the_total() {
    let (store, _) = store_at("paused");
    let limits = Limits {
        alerts_total: 1,
        changes_per_minute: 1000,
        ..Limits::default()
    };
    let a = alerts_on(store, Policy::default(), limits);
    let id = create(&a, spec("less", "once", 1.0), "a").await;
    let args = json!({"alert_id": id, "active": false});
    call(&a, "update_server_alert", args, "a").await.unwrap();
    let e = call(&a, "create_server_alert", spec("less", "once", 1.0), "b")
        .await
        .unwrap_err();
    assert!(e.contains("maximum of 1"), "{e}");
}

#[tokio::test(flavor = "multi_thread")]
async fn pairs_are_limited_per_client_and_resuming_checks_it() {
    let (store, _) = store_at("pairs");
    let limits = Limits {
        changes_per_minute: 1000,
        ..Limits::default()
    };
    let a = alerts_on(store, Policy::default(), limits);
    let on = |symbol: &str| {
        let mut args = spec("less", "once", 1.0);
        args["symbol"] = json!(symbol);
        args
    };
    let first = create(&a, on("BTCUSDT"), "a").await;
    create(&a, on("ETHUSDT"), "a").await;
    let e = call(&a, "create_server_alert", on("SOLUSDT"), "a")
        .await
        .unwrap_err();
    assert!(
        e.contains("at most 2 symbol and interval pairs per client"),
        "{e}"
    );
    // More alerts on a pair it has is fine, and so is another client.
    let second = create(&a, on("BTCUSDT"), "a").await;
    create(&a, on("SOLUSDT"), "b").await;

    // A paused alert frees its pair, and resuming needs room again.
    for id in [&first, &second] {
        let args = json!({"alert_id": id, "active": false});
        call(&a, "update_server_alert", args, "a").await.unwrap();
    }
    create(&a, on("SOLUSDT"), "a").await;
    let args = json!({"alert_id": first, "active": true});
    let e = call(&a, "update_server_alert", args, "a")
        .await
        .unwrap_err();
    assert!(e.contains("per client"), "{e}");
}

/// Churn: deleting frees the pair at once, while the WebSocket lingers.
#[tokio::test(flavor = "multi_thread")]
async fn new_feeds_are_limited_however_fast_alerts_come_and_go() {
    let (store, _) = store_at("churn");
    let fake = Arc::new(Fake::default());
    let hub = ChartHub::new(
        fake.clone(),
        HubLimits {
            new_feeds_per_minute: 3,
            ..HubLimits::default()
        },
    );
    let limits = Limits {
        changes_per_minute: 1000,
        ..Limits::default()
    };
    let a = ServerAlerts::new(
        store,
        hub,
        Policy::default(),
        Access::Open,
        None,
        limits,
        quick_timing(),
    );
    let on = |symbol: &str| {
        let mut args = spec("less", "once", 1.0);
        args["symbol"] = json!(symbol);
        args
    };
    for symbol in ["AAAUSDT", "BBBUSDT", "CCCUSDT"] {
        let id = create(&a, on(symbol), "me").await;
        let args = json!({"alert_id": id});
        call(&a, "delete_server_alert", args, "me").await.unwrap();
    }
    let seeds = fake.seeds.load(Ordering::SeqCst);
    let e = call(&a, "create_server_alert", on("DDDUSDT"), "me")
        .await
        .unwrap_err();
    assert!(e.contains("too many feeds"), "{e}");
    assert_eq!(
        fake.seeds.load(Ordering::SeqCst),
        seeds,
        "and no fetch for it"
    );
    // A feed still lingering is not a new connection.
    create(&a, on("AAAUSDT"), "me").await;
}

/// A feed refused at startup or on a resume comes back with the upkeep.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_feed_is_tried_again() {
    let (store, path) = store_at("feed-retry");
    let a = plain(store);
    let id = create(&a, spec("less", "once", 1.0), "me").await;
    drop(a);
    let hub = ChartHub::new(
        Arc::new(Fake::default()),
        HubLimits {
            new_feeds_per_minute: 0,
            ..HubLimits::default()
        },
    );
    let a = ServerAlerts::new(
        reopen(&path),
        hub,
        Policy::default(),
        Access::Open,
        None,
        Limits::default(),
        quick_timing(),
    );
    assert!(a.lock().watches.is_empty());
    assert_eq!(get(&a, "me", &id).await["feed"], "waiting");
    assert!(a.lock().alerts[&id].active, "it stays on");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spent_alert_expires_and_a_recent_one_stays() {
    let (store, _) = store_at("expire-spent");
    store.save_oauth_client("me", &[]).unwrap();
    store.mark_oauth_client_used("me").unwrap();
    let a = plain(store);
    let spent = create(&a, spec("greater", "once", 1.0), "me").await;
    tick(&a, BASE, 2.0);
    let waiting = create(&a, spec("less", "once", 1.0), "me").await;
    let now = now_ms();
    a.sweep(&mut a.lock(), now + SPENT_TTL_MS - 60_000);
    assert_eq!(a.lock().alerts.len(), 2, "not yet");
    a.sweep(&mut a.lock(), now + SPENT_TTL_MS + 60_000);
    assert!(!a.lock().alerts.contains_key(&spent));
    assert!(a.lock().alerts.contains_key(&waiting));
    assert_eq!(
        store.server_alerts().unwrap().len(),
        1,
        "the row is gone too"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_owners_alerts_expire_unless_the_operator_listed_it() {
    let (store, _) = store_at("expire-idle");
    for owner in ["idle", "listed"] {
        store.save_oauth_client(owner, &[]).unwrap();
        store.mark_oauth_client_used(owner).unwrap();
    }
    let a = with_access(store, Access::Open, Policy::default());
    let idle = create(&a, spec("less", "once", 1.0), "idle").await;
    let unknown = create(&a, spec("less", "once", 1.0), "not-in-the-table").await;
    drop(a);
    let a = with_access(store, Access::parse(Some("listed")), Policy::default());
    let listed = create(&a, spec("less", "once", 1.0), "listed").await;
    let now = now_ms();
    a.sweep(&mut a.lock(), now + IDLE_OWNER_TTL_MS - 60_000);
    assert_eq!(a.lock().alerts.len(), 3, "not yet");
    a.sweep(&mut a.lock(), now + IDLE_OWNER_TTL_MS + 60_000);
    for gone in [&idle, &unknown] {
        assert!(!a.lock().alerts.contains_key(gone));
    }
    assert!(
        a.lock().alerts.contains_key(&listed),
        "the operator keeps those"
    );
    assert_eq!(
        store.server_alert_owners().unwrap().len(),
        1,
        "pins go with the rows"
    );
}

/// It is the token that counts, not when the alert was made.
#[tokio::test(flavor = "multi_thread")]
async fn a_recent_token_keeps_an_old_alert() {
    let (store, _) = store_at("expire-renewed");
    store.save_oauth_client("active", &[]).unwrap();
    store.mark_oauth_client_used("active").unwrap();
    let a = plain(store);
    let kept = create(&a, spec("less", "once", 1.0), "active").await;
    let gone = create(&a, spec("less", "once", 1.0), "silent").await;
    let now = now_ms();
    let long_ago = now - IDLE_OWNER_TTL_MS - 60_000;
    for id in [&kept, &gone] {
        a.lock().alerts.get_mut(id).unwrap().created_at = long_ago;
    }
    a.sweep(&mut a.lock(), now);
    assert!(a.lock().alerts.contains_key(&kept));
    assert!(!a.lock().alerts.contains_key(&gone));
}

/// Only what decides when it fires brings a spent alert back.
#[tokio::test(flavor = "multi_thread")]
async fn a_note_or_webhook_edit_does_not_re_arm_a_spent_alert() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("no-rearm");
    let limits = Limits {
        changes_per_minute: 1000,
        ..Limits::default()
    };
    let a = alerts_on(store, local(port), limits);
    let id = create(&a, hooked(port, "greater", "once"), "me").await;
    tick(&a, BASE, 101.0);
    until("the delivery", || seen.lock().unwrap().len() == 1).await;
    for args in [
        json!({"alert_id": id, "message": "note"}),
        json!({"alert_id": id, "webhook_url": format!("http://127.0.0.1:{port}/other")}),
        json!({"alert_id": id, "sign": true}),
        // What the dialog sends for those: the same definition again.
        json!({"alert_id": id, "condition": "greater", "trigger": "once", "level": 100.0, "message": "x"}),
    ] {
        let view = call(&a, "update_server_alert", args.clone(), "me")
            .await
            .unwrap();
        assert_eq!(view["active"], false, "{args}");
        assert!(view["last_triggered_at"].is_i64(), "{args}");
        tick(&a, BASE, 102.0);
        assert_eq!(a.lock().alerts[&id].fire_count, 1, "{args}");
    }
    // A new level does.
    let args = json!({"alert_id": id, "level": 50.0});
    let view = call(&a, "update_server_alert", args, "me").await.unwrap();
    assert_eq!(view["active"], true);
    tick(&a, BASE, 102.0);
    assert_eq!(a.lock().alerts[&id].fire_count, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_url_does_not_inherit_the_secret() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("secret-url");
    let limits = Limits {
        changes_per_minute: 1000,
        ..Limits::default()
    };
    let a = alerts_on(store, local(port), limits);
    let mut args = hooked(port, "less", "once");
    args["sign"] = json!(true);
    let created = call(&a, "create_server_alert", args, "me").await.unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let same = a.lock().alerts[&id].webhook.as_ref().unwrap().url.clone();

    let args = json!({"alert_id": id, "webhook_url": same});
    let view = call(&a, "update_server_alert", args, "me").await.unwrap();
    assert_eq!(view["webhook"]["signed"], true, "the same URL keeps it");

    let new = format!("http://127.0.0.1:{port}/other");
    let args = json!({"alert_id": id, "webhook_url": new});
    let view = call(&a, "update_server_alert", args, "me").await.unwrap();
    assert_eq!(view["webhook"]["signed"], false, "a new one clears it");
    assert!(view.get("webhook_secret").is_none());

    let args = json!({"alert_id": id, "webhook_url": format!("{new}2"), "sign": true});
    let view = call(&a, "update_server_alert", args, "me").await.unwrap();
    assert_eq!(view["webhook"]["signed"], true);
    assert!(view["webhook_secret"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delivery_interrupted_by_a_restart_resumes_under_the_same_id() {
    let (port, seen) = receiver(OK).await;
    let (store, path) = store_at("resume");
    let a = alerts_on(store, local(port), Limits::default());
    let id = create(&a, hooked(port, "greater", "once"), "me").await;
    // As if the process died right after writing the fire.
    let mut fired = a.lock().alerts[&id].clone();
    fired.active = false;
    fired.fire_count = 1;
    let event = StoredEvent {
        id: "event-1".into(),
        alert_id: id.clone(),
        body: r#"{"id":"event-1"}"#.into(),
        created_at: chrono::Utc::now().timestamp(),
        device: None,
    };
    store
        .record_server_alert_fire(&id, "me", &fired.json(), std::slice::from_ref(&event))
        .unwrap();
    drop(a);

    let a = alerts_on(reopen(&path), local(port), Limits::default());
    until("the delivery", || seen.lock().unwrap().len() == 1).await;
    assert!(seen.lock().unwrap()[0].contains(r#"{"id":"event-1"}"#));
    until("the row to clear", || {
        a.store.server_alert_events().unwrap().is_empty()
    })
    .await;
    assert_eq!(a.lock().alerts[&id].fire_count, 1, "no second fire");

    // A later restart has nothing left to send.
    let a = alerts_on(reopen(&path), local(port), Limits::default());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(a.lock().pending, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stale_or_orphaned_delivery_is_dropped_on_restart() {
    let (port, seen) = receiver(OK).await;
    let (store, path) = store_at("stale");
    let a = alerts_on(store, local(port), Limits::default());
    let id = create(&a, hooked(port, "less", "once"), "me").await;
    let record = a.lock().alerts[&id].json();
    let at = chrono::Utc::now().timestamp();
    let event = |eid: &str, alert: &str, created_at| StoredEvent {
        id: eid.into(),
        alert_id: alert.into(),
        body: "{}".into(),
        created_at,
        device: None,
    };
    let old = event("old", &id, at - EVENT_MAX_AGE_SECS - 10);
    let orphan = event("orphan", "gone", at);
    store
        .record_server_alert_fire(&id, "me", &record, std::slice::from_ref(&old))
        .unwrap();
    store
        .record_server_alert_fire(&id, "me", &record, std::slice::from_ref(&orphan))
        .unwrap();
    drop(a);
    let a = alerts_on(reopen(&path), local(port), Limits::default());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(seen.lock().unwrap().is_empty());
    assert!(a.store.server_alert_events().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_alert_sends_nothing_more() {
    let (port, seen) = receiver(OK).await;
    let (store, path) = store_at("delete");
    let a = alerts_on(store, local(port), Limits::default());
    let id = create(&a, hooked(port, "less", "once"), "me").await;
    let record = a.lock().alerts[&id].json();
    let event = StoredEvent {
        id: "e".into(),
        alert_id: id.clone(),
        body: "{}".into(),
        created_at: chrono::Utc::now().timestamp(),
        device: None,
    };
    store
        .record_server_alert_fire(&id, "me", &record, std::slice::from_ref(&event))
        .unwrap();
    call(&a, "delete_server_alert", json!({"alert_id": id}), "me")
        .await
        .unwrap();
    assert!(store.server_alert_events().unwrap().is_empty());
    drop(a);
    let _a = alerts_on(reopen(&path), local(port), Limits::default());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owner_has_a_token_so_collecting_unused_clients_passes_it_by() {
    let (store, _) = store_at("owners");
    store.save_oauth_client("owner", &[]).unwrap();
    store.mark_oauth_client_used("owner").unwrap();
    store.save_oauth_client("nobody", &[]).unwrap();
    let a = plain(store);
    create(&a, spec("less", "once", 1.0), "owner").await;
    assert!(store.server_alert_owners().unwrap().contains("owner"));
    assert_eq!(store.gc_oauth_clients(-10).unwrap(), 1, "only nobody");
    let left: Vec<String> = store
        .oauth_clients()
        .unwrap()
        .into_iter()
        .map(|c| c.client_id)
        .collect();
    assert_eq!(left, ["owner"]);
}

#[test]
fn a_stored_alert_reads_back_with_its_secret() {
    let record: Record = serde_json::from_value(json!({
        "id": "x", "owner": "o", "symbol": "BTCUSDT", "interval": "1h",
        "evaluate_on": "close", "feed_interval": "1h", "condition": "crossing_up",
        "level": 1.5, "trigger": "every", "active": true, "created_at": 1,
        "webhook": { "url": "https://example.com/", "secret": "s" }
    }))
    .unwrap();
    assert_eq!(record.condition, Condition::CrossingUp);
    assert_eq!(
        record.webhook.as_ref().unwrap().secret.as_deref(),
        Some("s")
    );
    assert_eq!(record.fire_count, 0);
    let again: Record = serde_json::from_str(&record.json()).unwrap();
    assert_eq!(again.webhook.unwrap().url, "https://example.com/");
}

/// ADR-0044: a user is not in `oauth_clients`, and without their own activity every
/// signed-in owner's alerts would look idle.
#[tokio::test(flavor = "multi_thread")]
async fn a_users_alerts_live_on_that_users_activity() {
    let (store, _) = store_at("expire-user");
    store.save_user("github:1", "active").unwrap();
    store.mark_user_used("github:1").unwrap();
    let a = plain(store);
    let kept = create(&a, spec("less", "once", 1.0), "github:1").await;
    let gone = create(&a, spec("less", "once", 1.0), "github:2").await;
    let now = now_ms();
    let long_ago = now - IDLE_OWNER_TTL_MS - 60_000;
    for id in [&kept, &gone] {
        a.lock().alerts.get_mut(id).unwrap().created_at = long_ago;
    }
    a.sweep(&mut a.lock(), now);
    assert!(a.lock().alerts.contains_key(&kept), "they are active");
    assert!(!a.lock().alerts.contains_key(&gone), "they are not");
    // And they do expire, like anyone's, once that activity is old.
    a.sweep(&mut a.lock(), now + IDLE_OWNER_TTL_MS + 60_000);
    assert!(!a.lock().alerts.contains_key(&kept));
}

const CLIENT_1: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1";
const CLIENT_2: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa2";

/// ADR-0044: what clients owned goes to the account named, from every client, and
/// survives a restart there. Other accounts' alerts are not touched, nor any owner that
/// is not shaped like a client id.
#[tokio::test(flavor = "multi_thread")]
async fn the_named_owner_collects_every_clients_alerts() {
    let (store, _) = store_at("claim-alerts");
    let a = plain(store);
    let first = create(&a, spec("less", "once", 1.0), CLIENT_1).await;
    let second = create(&a, spec("less", "once", 1.0), CLIENT_2).await;
    let other = create(&a, spec("less", "once", 1.0), "google:2").await;
    let device = create(&a, spec("less", "once", 1.0), "device:9").await;

    assert_eq!(a.claim_waiting("github:1"), (2, 0));
    assert_eq!(a.lock().alerts[&first].owner, "github:1");
    assert_eq!(a.lock().alerts[&second].owner, "github:1");
    assert_eq!(a.lock().alerts[&other].owner, "google:2");
    assert_eq!(a.lock().alerts[&device].owner, "device:9");
    assert_eq!(a.claim_waiting("github:1"), (0, 0), "once");
    let owners = store.server_alert_owners().unwrap();
    assert!(owners.contains("github:1") && !owners.contains(CLIENT_1));
    let stored = store.server_alerts().unwrap();
    assert_eq!(
        stored.iter().filter(|row| row.contains("github:1")).count(),
        2,
        "the record says so too, as a restart reads it"
    );
}

/// They are that person's already, so the limit for new alerts takes nothing away.
#[tokio::test(flavor = "multi_thread")]
async fn collecting_passes_the_limit_and_leaves_nothing_behind() {
    let (store, _) = store_at("claim-limit");
    let a = plain(store);
    let limit = a.limits.alerts_per_client;
    let made = create(&a, spec("less", "once", 1.0), CLIENT_1).await;
    let template = a.lock().alerts[&made].clone();
    for i in 0..limit + 2 {
        let mut record = template.clone();
        record.id = format!("copy-{i}");
        a.lock().alerts.insert(record.id.clone(), record);
    }
    assert_eq!(a.claim_waiting("github:1"), (limit + 3, 0));
    let count = |owner: &str| {
        a.lock()
            .alerts
            .values()
            .filter(|r| r.owner == owner)
            .count()
    };
    assert_eq!((count("github:1"), count(CLIENT_1)), (limit + 3, 0));
}

/// A delivery waiting when its alert moves goes along, signed as it was. One of an
/// alert that did not move stays where it is.
#[tokio::test(flavor = "multi_thread")]
async fn a_collected_alert_takes_its_waiting_deliveries_along() {
    let (store, _) = store_at("claim-queue");
    let a = plain(store);
    let moved = create(&a, spec("less", "once", 1.0), CLIENT_1).await;
    let event = |id: &str, alert: &str| StoredEvent {
        id: id.to_string(),
        alert_id: alert.to_string(),
        body: "{}".to_string(),
        created_at: 0,
        device: None,
    };
    {
        let mut st = a.lock();
        let queue = st.queues.entry(CLIENT_1.to_string()).or_default();
        queue.push_back(event("e1", &moved));
        queue.push_back(event("e2", "an-alert-not-in-the-table"));
    }
    assert_eq!(a.claim_waiting("github:1"), (1, 0));
    let st = a.lock();
    assert_eq!(st.queues["github:1"].len(), 1);
    assert_eq!(st.queues["github:1"][0].id, "e1");
    assert_eq!(st.queues[CLIENT_1].len(), 1);
    assert_eq!(st.queues[CLIENT_1][0].id, "e2");
}

// --- Push (ADR-0045) ---

const UA_PUBLIC: &str =
    "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
const UA_AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";
const GONE: &str = "HTTP/1.1 410 Gone\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
const NOT_FOUND: &str = "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
const BROKEN: &str = "HTTP/1.1 500 Oops\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

fn vapid() -> Option<Vapid> {
    Some(Vapid::from_secret(&Vapid::new_secret().unwrap(), "https://x.example").unwrap())
}

fn push_alerts(
    store: &'static Store,
    port: u16,
    access: Access,
    vapid: Option<Vapid>,
    limits: Limits,
) -> Arc<ServerAlerts> {
    let hub = hub_with(Arc::new(Fake::default()));
    ServerAlerts::new(
        store,
        hub,
        local(port),
        access,
        vapid,
        limits,
        quick_timing(),
    )
}

fn device(port: u16, name: &str) -> Value {
    json!({
        "endpoint": format!("http://127.0.0.1:{port}/push/{name}"),
        "p256dh": UA_PUBLIC,
        "auth": UA_AUTH,
    })
}

async fn subscribe(a: &Arc<ServerAlerts>, port: u16, name: &str, owner: &str) -> Value {
    call(a, "subscribe_push", device(port, name), owner)
        .await
        .unwrap()
}

fn pushed(condition: &str, trigger: &str) -> Value {
    let mut args = spec(condition, trigger, 100.0);
    args["push"] = json!(true);
    args
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_reaches_the_service_encrypted_with_a_vapid_token() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("push-send");
    let a = push_alerts(store, port, Access::Open, vapid(), Limits::default());
    subscribe(&a, port, "phone", "me").await;
    let id = create(&a, pushed("greater", "once"), "me").await;
    tick(&a, BASE, 101.0);
    until("the push", || !seen.lock().unwrap().is_empty()).await;
    until("its record", || a.lock().alerts[&id].last_push.is_some()).await;

    let raw = seen.lock().unwrap()[0].clone();
    assert!(raw.starts_with("POST /push/phone"), "{raw}");
    let lower = raw.to_lowercase();
    assert!(lower.contains("content-encoding: aes128gcm"), "{raw}");
    assert!(lower.contains("authorization: vapid t="), "{raw}");
    assert!(lower.contains("ttl: 3600"), "{raw}");
    assert!(!raw.contains("BTCUSDT"), "the service must not read it");
    assert!(!lower.contains("x-chartgen"), "no webhook headers: {raw}");

    let view = get(&a, "me", &id).await;
    assert_eq!(view["push"], true);
    assert_eq!(view["last_push_status"], 200);
    assert!(view["last_push_error"].is_null(), "{view}");
    assert!(
        view["last_delivery_at"].is_null(),
        "not the webhook's: {view}"
    );
    until("the row to go", || {
        store.server_alert_events().unwrap().is_empty()
    })
    .await;
}

/// All of a fire's rows are written with it, and carry one id, so that a device told
/// twice and a webhook receiver can both recognise it.
#[tokio::test(flavor = "multi_thread")]
async fn a_fire_writes_one_row_per_device_and_webhook_with_one_id() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("push-rows");
    let limits = Limits {
        webhooks_per_client_per_minute: 0,
        pushes_per_client_per_minute: 0,
        ..Limits::default()
    };
    let a = push_alerts(store, port, Access::Open, vapid(), limits);
    subscribe(&a, port, "phone", "me").await;
    subscribe(&a, port, "laptop", "me").await;
    subscribe(&a, port, "not-mine", "other").await;
    let mut args = hooked(port, "greater", "once");
    args["push"] = json!(true);
    create(&a, args, "me").await;
    tick(&a, BASE, 101.0);

    let rows = store.server_alert_events().unwrap();
    assert_eq!(rows.len(), 3, "a webhook and two devices");
    assert_eq!(rows.iter().filter(|e| e.device.is_none()).count(), 1);
    let bodies: HashSet<&str> = rows.iter().map(|e| e.body.as_str()).collect();
    assert_eq!(bodies.len(), 1, "one body");
    let body: Value = serde_json::from_str(rows[0].body.as_str()).unwrap();
    assert_eq!(body["fire_count"], 1);
    let ids: HashSet<&str> = rows.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids.len(), 3, "each its own row");
    assert_eq!(a.lock().pending, 3);
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_404_or_410_forgets_the_device() {
    for reply in [GONE, NOT_FOUND] {
        let (port, seen) = receiver(reply).await;
        let (store, _) = store_at(&format!("push-gone-{}", &reply[9..12]));
        let a = push_alerts(store, port, Access::Open, vapid(), Limits::default());
        subscribe(&a, port, "phone", "me").await;
        let id = create(&a, pushed("greater", "once"), "me").await;
        tick(&a, BASE, 101.0);
        until("the record", || a.lock().alerts[&id].last_push.is_some()).await;
        assert_eq!(seen.lock().unwrap().len(), 1, "final, not retried");
        assert!(a.lock().devices.is_empty());
        assert!(store.push_devices().unwrap().is_empty());
        let view = get(&a, "me", &id).await;
        assert!(view["last_push_error"]
            .as_str()
            .unwrap()
            .starts_with("HTTP 4"));
        let list = call(&a, "list_server_alerts", json!({}), "me")
            .await
            .unwrap();
        assert_eq!(list["push"]["devices"], 0);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn other_failures_retry_and_keep_the_device() {
    let (port, seen) = receiver(BROKEN).await;
    let (store, _) = store_at("push-retry");
    let a = push_alerts(store, port, Access::Open, vapid(), Limits::default());
    subscribe(&a, port, "phone", "me").await;
    let id = create(&a, pushed("greater", "once"), "me").await;
    tick(&a, BASE, 101.0);
    until("the record", || a.lock().alerts[&id].last_push.is_some()).await;
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "the two retries of a webhook"
    );
    assert_eq!(a.lock().devices.len(), 1);
    let view = get(&a, "me", &id).await;
    assert_eq!(view["last_push_error"], "HTTP 500");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_push_resumes_after_a_restart_unless_its_device_or_client_is_gone() {
    let (port, seen) = receiver(OK).await;
    let (store, path) = store_at("push-restart");
    let limits = Limits {
        pushes_per_client_per_minute: 0,
        ..Limits::default()
    };
    let a = push_alerts(store, port, Access::Open, vapid(), limits);
    subscribe(&a, port, "phone", "me").await;
    let id = create(&a, pushed("greater", "once"), "me").await;
    tick(&a, BASE, 101.0);
    assert_eq!(store.server_alert_events().unwrap().len(), 1);
    drop(a);

    // Off the list now: dropped, as a webhook's is.
    let (off_store, _) = store_at("push-restart-off");
    let off = push_alerts(off_store, port, Access::Open, vapid(), limits);
    subscribe(&off, port, "phone", "me").await;
    create(&off, pushed("greater", "once"), "me").await;
    tick(&off, BASE, 101.0);
    drop(off);
    let listed = Access::Clients(["someone-else".to_string()].into());
    let off = push_alerts(off_store, port, listed, vapid(), limits);
    assert!(off.lock().queues.is_empty());
    assert!(off_store.server_alert_events().unwrap().is_empty());

    // Its device gone: nothing to send it to.
    let (gone_store, _) = store_at("push-restart-gone");
    let gone = push_alerts(gone_store, port, Access::Open, vapid(), limits);
    subscribe(&gone, port, "phone", "me").await;
    create(&gone, pushed("greater", "once"), "me").await;
    tick(&gone, BASE, 101.0);
    drop(gone);
    let device = gone_store.push_devices().unwrap().remove(0);
    gone_store.delete_push_device(&device.id).unwrap();
    assert!(gone_store.server_alert_events().unwrap().is_empty());

    let store = reopen(&path);
    let a = push_alerts(store, port, Access::Open, vapid(), Limits::default());
    until("the resumed push", || !seen.lock().unwrap().is_empty()).await;
    until("its record", || a.lock().alerts[&id].last_push.is_some()).await;
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(seen.lock().unwrap()[0].starts_with("POST /push/phone"));
    assert_eq!(a.lock().alerts[&id].fire_count, 1, "not fired again");
}

#[tokio::test(flavor = "multi_thread")]
async fn push_is_allowed_where_webhooks_are_refused() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("push-access");
    let a = push_alerts(store, port, Access::NoWebhooks, vapid(), Limits::default());
    subscribe(&a, port, "phone", "stranger").await;
    create(&a, pushed("greater", "once"), "stranger").await;
    let e = call(
        &a,
        "create_server_alert",
        hooked(port, "greater", "once"),
        "stranger",
    )
    .await
    .unwrap_err();
    assert!(e.contains("operator allowed"), "{e}");
    let list = call(&a, "list_server_alerts", json!({}), "stranger")
        .await
        .unwrap();
    assert_eq!(list["webhooks"], false);
    assert_eq!(list["push"]["devices"], 1);
    assert!(list["push"]["public_key"].is_string());

    // With a list, only those clients: push goes with being allowed alerts at all.
    let (store, _) = store_at("push-access-list");
    let listed = Access::Clients(["friend".to_string()].into());
    let a = push_alerts(store, port, listed, vapid(), Limits::default());
    subscribe(&a, port, "phone", "friend").await;
    let e = call(&a, "subscribe_push", device(port, "phone"), "stranger")
        .await
        .unwrap_err();
    assert!(e.contains("does not send push"), "{e}");
    let list = call(&a, "list_server_alerts", json!({}), "stranger")
        .await
        .unwrap();
    assert!(list["push"].is_null(), "{list}");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_key_push_is_off_and_the_rest_runs() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("push-no-key");
    let a = push_alerts(store, port, Access::Open, None, Limits::default());
    let list = call(&a, "list_server_alerts", json!({}), "me")
        .await
        .unwrap();
    assert!(list["push"].is_null());
    for (tool, args) in [
        ("subscribe_push", device(port, "phone")),
        ("create_server_alert", pushed("greater", "once")),
    ] {
        let e = call(&a, tool, args, "me").await.unwrap_err();
        assert!(e.contains("does not send push"), "{tool}: {e}");
    }
    create(&a, spec("greater", "once", 100.0), "me").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_subscription_is_checked_and_stays_its_owners() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("push-check");
    let limits = Limits {
        changes_per_minute: 1000,
        ..Limits::default()
    };
    let a = push_alerts(store, port, Access::Open, vapid(), limits);
    let refused = |args: Value, needle: &'static str| {
        let a = a.clone();
        async move {
            let e = call(&a, "subscribe_push", args, "me").await.unwrap_err();
            assert!(e.contains(needle), "{e}");
        }
    };
    for bad in [
        "https://example.com/hook",
        "https://evil.example/fcm.googleapis.com",
        "https://169.254.169.254/latest",
        "http://127.0.0.1:9/elsewhere",
        "http://fcm.googleapis.com/fcm/send/a",
    ] {
        let mut args = device(port, "x");
        args["endpoint"] = json!(bad);
        refused(args, "not a known push service").await;
    }
    let mut args = device(port, "x");
    args["p256dh"] = json!("AAAA");
    refused(args, "push keys").await;
    let mut args = device(port, "x");
    args["auth"] = json!("AAAA");
    refused(args, "push keys").await;
    refused(json!({ "endpoint": "x" }), "p256dh is required").await;
    assert!(store.push_devices().unwrap().is_empty());

    // The same browser under a second sign-in: its keys match, so the device moves
    // (ADR-0044). A stranger who only names the endpoint still removes nothing.
    subscribe(&a, port, "shared", "me").await;
    assert_eq!(
        subscribe(&a, port, "shared", "me").await["devices"],
        1,
        "same device"
    );
    let endpoint = json!({ "endpoint": device(port, "shared")["endpoint"] });
    let out = call(&a, "unsubscribe_push", endpoint.clone(), "stranger")
        .await
        .unwrap();
    assert_eq!(out["removed"], false);
    assert_eq!(store.push_devices().unwrap().len(), 1, "still there");
    subscribe(&a, port, "shared", "you").await;
    let out = call(&a, "unsubscribe_push", endpoint.clone(), "me")
        .await
        .unwrap();
    assert_eq!(
        (&out["removed"], &out["devices"]),
        (&json!(false), &json!(0)),
        "it is no longer mine to remove"
    );
    let owners: Vec<String> = store
        .push_devices()
        .unwrap()
        .into_iter()
        .map(|d| d.owner)
        .collect();
    assert_eq!(owners, ["you"]);
    // Nothing in what a client reads gives another's endpoint or keys.
    let list = call(&a, "list_server_alerts", json!({}), "me")
        .await
        .unwrap();
    assert!(!list.to_string().contains("/push/"), "{list}");
}

#[tokio::test(flavor = "multi_thread")]
async fn devices_are_limited_per_client_and_in_total() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("push-limits");
    let limits = Limits {
        changes_per_minute: 1000,
        devices_per_client: 2,
        devices_total: 3,
        pushes_per_client_per_minute: 0,
        ..Limits::default()
    };
    let a = push_alerts(store, port, Access::Open, vapid(), limits);
    subscribe(&a, port, "one", "me").await;
    tokio::time::sleep(Duration::from_millis(3)).await;
    subscribe(&a, port, "two", "me").await;
    // What the oldest was waiting for goes with it.
    create(&a, pushed("greater", "once"), "me").await;
    tick(&a, BASE, 101.0);
    assert_eq!(a.lock().pending, 2);
    tokio::time::sleep(Duration::from_millis(3)).await;
    assert_eq!(subscribe(&a, port, "three", "me").await["devices"], 2);
    let ends: HashSet<String> = store
        .push_devices()
        .unwrap()
        .into_iter()
        .map(|d| d.endpoint)
        .collect();
    assert!(!ends.iter().any(|e| e.ends_with("/one")), "{ends:?}");
    assert_eq!(a.lock().pending, 1);
    assert_eq!(store.server_alert_events().unwrap().len(), 1);

    subscribe(&a, port, "theirs", "you").await;
    // Replacing one's own needs no room, and leaves everyone else's alone.
    subscribe(&a, port, "four", "me").await;
    let ends: HashSet<String> = store
        .push_devices()
        .unwrap()
        .into_iter()
        .map(|d| d.endpoint)
        .collect();
    assert!(ends.iter().any(|e| e.ends_with("/theirs")), "{ends:?}");
    assert!(!ends.iter().any(|e| e.ends_with("/two")), "{ends:?}");
}

fn full_table() -> Limits {
    Limits {
        changes_per_minute: 1000,
        devices_total: 3,
        pushes_per_client_per_minute: 0,
        ..Limits::default()
    }
}

fn endpoints(store: &Store) -> HashSet<String> {
    store
        .push_devices()
        .unwrap()
        .into_iter()
        .map(|d| d.endpoint.rsplit('/').next().unwrap().to_string())
        .collect()
}

async fn subscribe_in_turn(a: &Arc<ServerAlerts>, port: u16, owners: &[&str]) {
    for owner in owners {
        subscribe(a, port, owner, owner).await;
        tokio::time::sleep(Duration::from_millis(3)).await;
    }
}

/// Full, a stranger's device replaces the one whose owner was idle longest.
#[tokio::test(flavor = "multi_thread")]
async fn a_full_table_replaces_the_owner_idle_longest() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("push-full-idle");
    let a = push_alerts(store, port, Access::Open, vapid(), full_table());
    subscribe_in_turn(&a, port, &["old", "middle", "young"]).await;
    subscribe(&a, port, "new", "new").await;
    assert_eq!(
        endpoints(store),
        ["middle", "young", "new"].map(String::from).into()
    );

    // A token since then makes an owner the less idle one, however old the device.
    let (store, _) = store_at("push-full-token");
    let a = push_alerts(store, port, Access::Open, vapid(), full_table());
    store.save_oauth_client("old", &[]).unwrap();
    subscribe_in_turn(&a, port, &["old", "middle", "young"]).await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    store.mark_oauth_client_used("old").unwrap();
    subscribe(&a, port, "new", "new").await;
    assert_eq!(
        endpoints(store),
        ["old", "young", "new"].map(String::from).into()
    );
}

/// A flood needs no alert, and its clients hold fresh tokens: it must not push out
/// someone whose alert would notify them.
#[tokio::test(flavor = "multi_thread")]
async fn a_flood_of_new_devices_replaces_itself_before_a_real_users() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("push-full-flood");
    let a = push_alerts(store, port, Access::Open, vapid(), full_table());
    subscribe(&a, port, "real", "real").await;
    create(&a, pushed("greater", "every"), "real").await;
    tokio::time::sleep(Duration::from_millis(3)).await;
    for i in 0..10 {
        let owner = format!("junk{i}");
        subscribe(&a, port, &owner, &owner).await;
        tokio::time::sleep(Duration::from_millis(3)).await;
    }
    assert!(endpoints(store).contains("real"), "{:?}", endpoints(store));
    assert_eq!(store.push_devices().unwrap().len(), 3);

    // Once it no longer notifies anyone, it is one more idle device.
    let id = call(&a, "list_server_alerts", json!({}), "real")
        .await
        .unwrap()["alerts"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let args = json!({ "alert_id": id, "active": false });
    call(&a, "update_server_alert", args, "real").await.unwrap();
    subscribe(&a, port, "last", "last").await;
    assert!(!endpoints(store).contains("real"), "{:?}", endpoints(store));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listed_clients_device_is_never_replaced() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("push-full-listed");
    let listed = Access::Clients(["a", "b", "c", "d"].map(String::from).into());
    let a = push_alerts(store, port, listed, vapid(), full_table());
    subscribe_in_turn(&a, port, &["a", "b", "c"]).await;
    let e = call(&a, "subscribe_push", device(port, "d"), "d")
        .await
        .unwrap_err();
    assert!(e.contains("maximum of 3"), "{e}");
    assert_eq!(endpoints(store), ["a", "b", "c"].map(String::from).into());
}

/// The caller is under its own limit here, so its own device is not what makes room.
#[tokio::test(flavor = "multi_thread")]
async fn a_full_table_does_not_replace_the_callers_own_device() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("push-full-own");
    let a = push_alerts(store, port, Access::Open, vapid(), full_table());
    subscribe_in_turn(&a, port, &["me", "other", "another"]).await;
    subscribe(&a, port, "second", "me").await;
    assert_eq!(
        endpoints(store),
        ["me", "second", "another"].map(String::from).into()
    );
}

/// The review's measurement: this got 25 devices of 1.9 MB each past the old check.
#[tokio::test(flavor = "multi_thread")]
async fn padded_keys_are_refused_and_nothing_is_stored() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("push-padding");
    let a = push_alerts(store, port, Access::Open, vapid(), Limits::default());
    for (key, value) in [
        ("p256dh", format!("{UA_PUBLIC}{}", "=".repeat(1_900_000))),
        ("auth", format!("{UA_AUTH}{}", "=".repeat(1_900_000))),
    ] {
        let mut args = device(port, "x");
        args[key] = json!(value);
        let e = call(&a, "subscribe_push", args, "me").await.unwrap_err();
        assert!(e.contains("push keys"), "{e}");
    }
    assert!(store.push_devices().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn one_endpoint_written_two_ways_is_one_device_and_removes_as_one() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("push-normal");
    let a = push_alerts(store, port, Access::Open, vapid(), Limits::default());
    for endpoint in [
        "https://fcm.googleapis.com/fcm/send/a",
        "https://FCM.googleapis.com:443/fcm/send/a",
    ] {
        let mut args = device(port, "x");
        args["endpoint"] = json!(endpoint);
        let out = call(&a, "subscribe_push", args, "me").await.unwrap();
        assert_eq!(out["devices"], 1, "{endpoint}");
    }
    let args = json!({ "endpoint": "https://FCM.GOOGLEAPIS.com/fcm/send/a" });
    let out = call(&a, "unsubscribe_push", args, "me").await.unwrap();
    assert_eq!(
        (&out["removed"], &out["devices"]),
        (&json!(true), &json!(0))
    );
}

/// Each client's budget is its own, and a webhook's does not apply to a push.
#[tokio::test(flavor = "multi_thread")]
async fn pushes_have_a_budget_per_client() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("push-budget");
    let limits = Limits {
        changes_per_minute: 1000,
        fires_per_alert_per_minute: 1000,
        pushes_per_client_per_minute: 2,
        webhooks_per_client_per_minute: 0,
        deliveries_total_per_minute: 1000,
        ..Limits::default()
    };
    let a = push_alerts(store, port, Access::Open, vapid(), limits);
    subscribe(&a, port, "phone", "me").await;
    subscribe(&a, port, "theirs", "you").await;
    let mine = create(&a, pushed("crossing", "every"), "me").await;
    create(&a, pushed("crossing", "every"), "you").await;
    tick(&a, BASE, 99.0);
    for i in 0..10 {
        tick(&a, BASE, if i % 2 == 0 { 101.0 } else { 99.0 });
    }
    assert_eq!(a.lock().alerts[&mine].fire_count, 10);
    until("both budgets' worth", || seen.lock().unwrap().len() == 4).await;
    tokio::time::sleep(DISPATCH_EVERY * 3).await;
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 4, "the rest wait");
    let to = |path: &str| seen.iter().filter(|r| r.starts_with(path)).count();
    assert_eq!((to("POST /push/phone"), to("POST /push/theirs")), (2, 2));
    assert_eq!(a.lock().queues["me"].len(), 8);
}

/// As the ADR says: a client's deliveries stay in order, so its webhook over budget holds
/// its own pushes back, and nobody else's.
#[tokio::test(flavor = "multi_thread")]
async fn a_webhook_over_budget_holds_back_only_its_own_clients_push() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("push-order");
    let limits = Limits {
        webhooks_per_client_per_minute: 0,
        ..Limits::default()
    };
    let a = push_alerts(store, port, Access::Open, vapid(), limits);
    subscribe(&a, port, "phone", "me").await;
    subscribe(&a, port, "theirs", "you").await;
    let mut both = hooked(port, "greater", "once");
    both["push"] = json!(true);
    create(&a, both, "me").await;
    create(&a, pushed("greater", "once"), "you").await;
    tick(&a, BASE, 101.0);
    until("the other client's push", || {
        !seen.lock().unwrap().is_empty()
    })
    .await;
    tokio::time::sleep(DISPATCH_EVERY * 3).await;
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(seen[0].starts_with("POST /push/theirs"));
    assert_eq!(a.lock().queues["me"].len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_queue_or_no_device_says_why_and_the_fire_counts() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("push-full");
    let limits = Limits {
        fires_per_alert_per_minute: 1000,
        pushes_per_client_per_minute: 0,
        pending_deliveries_per_client: 3,
        ..Limits::default()
    };
    let a = push_alerts(store, port, Access::Open, vapid(), limits);
    let id = create(&a, pushed("crossing_up", "every"), "me").await;
    tick(&a, BASE, 99.0);
    tick(&a, BASE, 101.0);
    let view = get(&a, "me", &id).await;
    assert_eq!(view["fire_count"], 1);
    assert_eq!(
        view["last_push_error"],
        "push skipped: no device has notifications on"
    );

    subscribe(&a, port, "phone", "me").await;
    subscribe(&a, port, "laptop", "me").await;
    tick(&a, BASE, 99.0);
    tick(&a, BASE, 101.0);
    assert_eq!(a.lock().queues["me"].len(), 2);
    tick(&a, BASE, 99.0);
    tick(&a, BASE, 101.0);
    assert_eq!(
        a.lock().queues["me"].len(),
        3,
        "bounded, for the one that fits"
    );
    assert_eq!(store.server_alert_events().unwrap().len(), 3);
    let view = get(&a, "me", &id).await;
    assert_eq!(view["fire_count"], 3);
    assert_eq!(
        view["last_push_error"],
        "push skipped: too many deliveries waiting"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn turning_push_on_or_off_does_not_re_arm_a_spent_alert() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("push-edit");
    let a = push_alerts(store, port, Access::Open, vapid(), Limits::default());
    subscribe(&a, port, "phone", "me").await;
    let id = create(&a, spec("greater", "once", 100.0), "me").await;
    tick(&a, BASE, 101.0);
    assert_eq!(get(&a, "me", &id).await["active"], false);
    let args = json!({ "alert_id": id, "push": true });
    let view = call(&a, "update_server_alert", args, "me").await.unwrap();
    assert_eq!(
        (&view["active"], &view["push"]),
        (&json!(false), &json!(true))
    );
    let args = json!({ "alert_id": id, "push": false });
    let view = call(&a, "update_server_alert", args, "me").await.unwrap();
    assert_eq!(view["push"], false);
    assert!(seen.lock().unwrap().is_empty());
    let e = call(
        &a,
        "update_server_alert",
        json!({ "alert_id": id, "push": 1 }),
        "me",
    )
    .await
    .unwrap_err();
    assert!(e.contains("push must be"), "{e}");
}

/// A row from before push reads as a webhook-only alert.
#[test]
fn an_alert_stored_before_push_reads_without_it() {
    let old = r#"{"id":"a","owner":"o","symbol":"BTCUSDT","interval":"1m","evaluate_on":"live",
        "feed_interval":"1m","condition":"greater","level":1.0,"trigger":"once","active":true,
        "created_at":1}"#;
    let r: Record = serde_json::from_str(old).unwrap();
    assert!(!r.push && r.last_push.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_outlives_a_sign_in_but_not_the_idle_limit() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("push-idle");
    let listed = Access::Clients(["friend".to_string()].into());
    let a = push_alerts(store, port, listed.clone(), vapid(), Limits::default());
    subscribe(&a, port, "phone", "friend").await;
    let later = now_ms() + IDLE_OWNER_TTL_MS + 1000;
    a.sweep(&mut a.lock(), later);
    assert_eq!(a.lock().devices.len(), 1, "the operator listed it");

    let (store, _) = store_at("push-idle-open");
    let a = push_alerts(store, port, Access::Open, vapid(), Limits::default());
    subscribe(&a, port, "phone", "stranger").await;
    a.sweep(&mut a.lock(), now_ms() + 1000);
    assert_eq!(a.lock().devices.len(), 1, "no token today is not idle");
    a.sweep(&mut a.lock(), later);
    assert!(a.lock().devices.is_empty());
    assert!(store.push_devices().unwrap().is_empty());
}

// --- Push with sign-in (ADR-0044) ---

/// What clients held goes to the account named, from every client, and the rows follow.
/// An endpoint the owner already holds stays put, and other kinds of owner are not clients.
#[tokio::test(flavor = "multi_thread")]
async fn the_named_owner_collects_every_clients_devices() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("claim-devices");
    let a = push_alerts(store, port, Access::Open, vapid(), Limits::default());
    subscribe(&a, port, "laptop", CLIENT_1).await;
    subscribe(&a, port, "phone", CLIENT_2).await;
    subscribe(&a, port, "tablet", "device:9").await;
    subscribe(&a, port, "shared", "github:1").await;
    // Rows from before an endpoint had one owner: the same subscription under a client,
    // and one under the same endpoint with other keys, which may not be that browser.
    let row = |name: &str, id: &str, p256dh: &str| {
        let device = StoredDevice {
            id: id.to_string(),
            owner: CLIENT_1.to_string(),
            endpoint: format!("http://127.0.0.1:{port}/push/{name}"),
            p256dh: p256dh.to_string(),
            auth: UA_AUTH.to_string(),
            created_at: 1,
        };
        store.save_push_device(&device).unwrap();
        a.lock().devices.insert(id.to_string(), device);
    };
    row("shared", "duplicate", UA_PUBLIC);
    subscribe(&a, port, "other-keys", "github:1").await;
    row("other-keys", "unproven", "other");

    assert_eq!(a.claim_waiting("github:1"), (0, 2));

    let count = |owner: &str| {
        a.lock()
            .devices
            .values()
            .filter(|d| d.owner == owner)
            .count()
    };
    assert_eq!(count("github:1"), 4, "the two, and the two it held");
    assert!(
        !a.lock().devices.contains_key("duplicate"),
        "one endpoint, one owner"
    );
    assert!(
        a.lock().devices.contains_key("unproven"),
        "not provably the same"
    );
    assert_eq!(count(CLIENT_1), 1);
    assert_eq!(count(CLIENT_2), 0);
    assert_eq!(count("device:9"), 1);
    let stored = store.push_devices().unwrap();
    assert_eq!(
        stored.iter().filter(|d| d.owner == "github:1").count(),
        4,
        "as a restart reads them"
    );
    assert_eq!(a.claim_waiting("github:1"), (0, 0), "once");
}

/// The bound stays for what comes next: the owner's next device trims to the limit.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_device_trims_what_collecting_passed_the_limit_with() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("claim-devices-limit");
    let limits = Limits::default();
    let cap = limits.devices_per_client;
    let a = push_alerts(store, port, Access::Open, vapid(), limits);
    for i in 0..cap + 2 {
        subscribe(&a, port, &format!("d{i}"), CLIENT_1).await;
        // Five is a client's limit too, so these come from several.
        let id = a
            .lock()
            .devices
            .values()
            .find(|d| d.endpoint.ends_with(&format!("d{i}")))
            .unwrap()
            .id
            .clone();
        a.lock().devices.get_mut(&id).unwrap().owner = format!("aaaaaaaa-aaaa-4aaa-8aaa-{i:012}");
    }
    assert_eq!(a.claim_waiting("github:1"), (0, cap + 2));
    subscribe(&a, port, "new", "github:1").await;
    let mine = a
        .lock()
        .devices
        .values()
        .filter(|d| d.owner == "github:1")
        .count();
    assert_eq!(mine, cap);
}

/// A user is not in `oauth_clients`, so a device of theirs lives on their activity, or
/// every signed-in device would look idle and be forgotten.
#[tokio::test(flavor = "multi_thread")]
async fn a_users_devices_live_on_that_users_activity() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("device-user");
    store.save_user("github:1", "active").unwrap();
    store.mark_user_used("github:1").unwrap();
    let a = push_alerts(store, port, Access::Open, vapid(), Limits::default());
    subscribe(&a, port, "phone", "github:1").await;
    subscribe(&a, port, "other", "github:2").await;
    let long_ago = now_ms() - IDLE_OWNER_TTL_MS - 60_000;
    for d in a.lock().devices.values_mut() {
        d.created_at = long_ago;
    }
    let now = now_ms();
    a.sweep(&mut a.lock(), now);
    let owners: Vec<String> = a.lock().devices.values().map(|d| d.owner.clone()).collect();
    assert_eq!(owners, ["github:1"], "active, and not");
    a.sweep(&mut a.lock(), now + IDLE_OWNER_TTL_MS + 60_000);
    assert!(a.lock().devices.is_empty(), "and it does expire");
}

/// Each device's outcome is its own: named pending when the fire goes out, so a tab can
/// tell "not yet" from "not sent here", and kept across a restart.
#[tokio::test(flavor = "multi_thread")]
async fn the_last_fire_is_recorded_per_device() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("push-per-device");
    let limits = Limits {
        pushes_per_client_per_minute: 0,
        ..Limits::default()
    };
    let a = push_alerts(store, port, Access::Open, vapid(), limits);
    let phone = subscribe(&a, port, "phone", "me").await["device"]
        .as_str()
        .unwrap()
        .to_string();
    let id = create(&a, pushed("greater", "once"), "me").await;
    tick(&a, BASE, 101.0);
    until("the fire", || a.lock().alerts[&id].fire_count == 1).await;
    let late = subscribe(&a, port, "late", "me").await["device"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(phone, late);

    let view = get(&a, "me", &id).await;
    let devices = view["last_push_devices"].as_object().unwrap();
    assert!(devices[&phone].is_null(), "pending, not missing: {view}");
    assert!(
        !devices.contains_key(&late),
        "it was not sent this one: {view}"
    );

    let stored = store.server_alerts().unwrap();
    assert!(
        stored[0].contains(&phone),
        "it survives a restart: {}",
        stored[0]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn each_devices_outcome_is_kept_with_a_status() {
    let (port, _) = receiver(OK).await;
    let (store, _) = store_at("push-per-device-ok");
    let a = push_alerts(store, port, Access::Open, vapid(), Limits::default());
    let phone = subscribe(&a, port, "phone", "me").await["device"]
        .as_str()
        .unwrap()
        .to_string();
    let id = create(&a, pushed("greater", "once"), "me").await;
    tick(&a, BASE, 101.0);
    until("the record", || a.lock().alerts[&id].last_push.is_some()).await;
    let view = get(&a, "me", &id).await;
    assert_eq!(view["last_push_devices"][&phone]["status"], 200, "{view}");
    assert!(
        view["last_push_devices"][&phone]["error"].is_null(),
        "{view}"
    );
}

// --- One endpoint, one owner (ADR-0044) ---

/// The browser's next owner has what only that browser has, so the device is theirs. The
/// earlier owner loses that device, and its alerts run on and reach its other ones.
#[tokio::test(flavor = "multi_thread")]
async fn an_account_subscribing_an_endpoint_takes_it_over() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("takeover");
    let a = push_alerts(store, port, Access::Open, vapid(), Limits::default());
    subscribe(&a, port, "shared", "github:1").await;
    subscribe(&a, port, "laptop", "github:1").await;
    let id = create(&a, pushed("greater", "every"), "github:1").await;

    let answer = subscribe(&a, port, "shared", "github:2").await;
    assert_eq!(answer["devices"], 1);

    let owners = |endpoint: &str| -> Vec<String> {
        store
            .push_devices()
            .unwrap()
            .into_iter()
            .filter(|d| d.endpoint.ends_with(endpoint))
            .map(|d| d.owner)
            .collect()
    };
    assert_eq!(owners("/shared"), ["github:2"], "one row, the latest");
    assert_eq!(owners("/laptop"), ["github:1"], "its other device stays");
    assert_eq!(a.lock().devices.len(), 2, "and memory agrees");
    assert_eq!(
        call(&a, "list_server_alerts", json!({}), "github:1")
            .await
            .unwrap()["push"]["devices"],
        1
    );

    // Its alert goes on, to the device it has left, and not to this one.
    tick(&a, BASE, 101.0);
    until("the record", || a.lock().alerts[&id].last_push.is_some()).await;
    let view = get(&a, "github:1", &id).await;
    assert_eq!(view["fire_count"], 1, "it keeps running");
    let got = seen.lock().unwrap().clone();
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].starts_with("POST /push/laptop"), "{}", got[0]);

    // Whoever subscribes last has it: taking it back works the same way.
    subscribe(&a, port, "shared", "github:1").await;
    assert_eq!(owners("/shared"), ["github:1"]);
}

/// Knowing someone's endpoint is not enough: without its keys nothing moves, and that
/// device goes on working. Neither half of the keys will do.
#[tokio::test(flavor = "multi_thread")]
async fn an_endpoint_with_other_keys_evicts_nobody() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("no-eviction");
    let a = push_alerts(store, port, Access::Open, vapid(), Limits::default());
    subscribe(&a, port, "phone", "github:1").await;
    let rows = || -> Vec<(String, String, String, String)> {
        store
            .push_devices()
            .unwrap()
            .into_iter()
            .map(|d| (d.id, d.owner, d.p256dh, d.auth))
            .collect()
    };
    let before = rows();
    let endpoint = format!("http://127.0.0.1:{port}/push/phone");

    const OTHER_PUBLIC: &str =
        "BNToM-qH15gF5XbSbPRmBJUPnlZkIr_9kq1unPg0TkipOEABC6FQhKZi96_K2_TnLc-JPX8fT5cqzs56rxlUWSk";
    const OTHER_AUTH: &str = "AAECAwQFBgcICQoLDA0ODw";
    for (p256dh, auth) in [
        (OTHER_PUBLIC, OTHER_AUTH),
        (UA_PUBLIC, OTHER_AUTH),
        (OTHER_PUBLIC, UA_AUTH),
    ] {
        let forged = json!({"endpoint": endpoint, "p256dh": p256dh, "auth": auth});
        let refused = call(&a, "subscribe_push", forged, "github:2")
            .await
            .unwrap_err();
        assert!(refused.contains("other keys"), "{refused}");
    }
    assert_eq!(rows(), before, "the row is as it was");
    assert_eq!(a.lock().devices.len(), 1);

    let id = create(&a, pushed("greater", "once"), "github:1").await;
    tick(&a, BASE, 101.0);
    until("the record", || a.lock().alerts[&id].last_push.is_some()).await;
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "and it still gets its alerts"
    );

    // The same keys padded are the same keys, and its owner may change its own.
    let padded = json!({
        "endpoint": endpoint,
        "p256dh": format!("{UA_PUBLIC}="),
        "auth": format!("{UA_AUTH}=="),
    });
    call(&a, "subscribe_push", padded, "github:2")
        .await
        .unwrap();
    let own = json!({"endpoint": endpoint, "p256dh": OTHER_PUBLIC, "auth": OTHER_AUTH});
    call(&a, "subscribe_push", own, "github:2").await.unwrap();
    assert_eq!(store.push_devices().unwrap().len(), 1);
}
