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
    ServerAlerts::new(store, hub, policy, Access::Open, limits, quick_timing())
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

#[tokio::test(flavor = "multi_thread")]
async fn a_symbol_new_to_the_server_is_checked_once() {
    let (store, _) = store_at("symbol");
    let fake = Arc::new(Fake::default());
    let a = ServerAlerts::new(
        store,
        hub_with(fake.clone()),
        Policy::default(),
        Access::Open,
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
            seen2
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf[..n]).to_string());
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
        webhooks_total_per_minute: 1000,
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
    let a = ServerAlerts::new(store, hub, local(port), Access::Open, limits, timing);
    for _ in 0..3 {
        create(&a, hooked(port, "greater", "once"), "me").await;
    }
    tick(&a, BASE, 101.0);
    until("the first delivery", || !seen.lock().unwrap().is_empty()).await;
    // Several dispatch ticks, while the first still waits for its silent receiver.
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert_eq!(seen.lock().unwrap().len(), 1, "a second went at once");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_total_budget_holds_across_clients() {
    let (port, seen) = receiver(OK).await;
    let (store, _) = store_at("budget-total");
    let limits = Limits {
        webhooks_total_per_minute: 2,
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
    };
    store
        .record_server_alert_fire(&id, "me", &fired.json(), Some(&event))
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
    };
    let old = event("old", &id, at - EVENT_MAX_AGE_SECS - 10);
    let orphan = event("orphan", "gone", at);
    store
        .record_server_alert_fire(&id, "me", &record, Some(&old))
        .unwrap();
    store
        .record_server_alert_fire(&id, "me", &record, Some(&orphan))
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
    };
    store
        .record_server_alert_fire(&id, "me", &record, Some(&event))
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
