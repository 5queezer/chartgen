//! Real-time chart push (ADR-0021).
//!
//! `subscribe_chart` ties a subscription to one SSE stream (`GET /mcp`).
//! Subscriptions that ask for the same chart (ticker, timeframe,
//! indicators, chart style, warmup) share a group. A group keeps a rolling
//! window of `TAIL_BARS + warmup` raw bars: seeded through the normal fetch
//! path, then updated from a feed. Groups on the same symbol and interval
//! share one feed, so one Binance WebSocket serves every chart of it.
//!
//! After a feed event the group recomputes the tail exactly like
//! `generate_chart(format=series, bars=TAIL_BARS, warmup)` and pushes it as
//! `notifications/chart_tick`, at most once per `Limits::min_push_interval`.
//! The last subscriber leaving (unsubscribe, or its SSE stream closing)
//! stops the group; the last group leaving stops the feed.
//!
//! Independent of the trading engine, so it also runs under plain `--serve`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use chartgen::data::{self, Bar, OhlcvData};
use chartgen::feed::{BinanceFeed, FeedEvent};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::fetch::{self, DataSource};

/// Bars per pushed tail: the last closed bar and the forming one.
pub const TAIL_BARS: usize = 2;
/// JSON-RPC method of a pushed tail.
pub const CHART_TICK: &str = "notifications/chart_tick";
/// Cadence of the sample source's synthetic ticks.
const SAMPLE_TICK: Duration = Duration::from_millis(500);
/// Feed events buffered per group before it lags (and skips some).
const FEED_BUFFER: usize = 1024;
/// Binance kline stream intervals.
const BINANCE_KLINES: [&str; 15] = [
    "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h", "1d", "3d", "1w", "1M",
];

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Subscriptions one bearer token may hold, across its streams.
    pub per_token: usize,
    /// Subscriptions the server holds in total.
    pub total: usize,
    /// Shortest gap between two pushes of one group.
    pub min_push_interval: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            per_token: 4,
            total: 64,
            min_push_interval: Duration::from_millis(250),
        }
    }
}

/// Where pushed candles come from.
pub trait PushSource: Send + Sync + 'static {
    /// The feed interval for a pushable chart, or why it is not pushable.
    fn feed_interval(&self, symbol: &str, timeframe: &str) -> Result<String, String>;
    /// The newest `count` bars, from the same path `generate_chart` uses.
    /// Blocking.
    fn seed(&self, symbol: &str, timeframe: &str, count: usize) -> Result<OhlcvData, String>;
    /// Send feed events into `tx` until it closes. Dropping the future
    /// ends the feed.
    fn run(
        &self,
        symbol: String,
        interval: String,
        tx: mpsc::Sender<FeedEvent>,
    ) -> BoxFuture<'static, ()>;
}

fn binance_only(symbol: &str) -> Result<(), String> {
    if fetch::detect_source(symbol) == "binance" {
        Ok(())
    } else {
        Err(format!(
            "not pushable: {symbol} is not a Binance symbol; poll generate_chart instead"
        ))
    }
}

/// Live source: Binance kline + aggTrade WebSocket.
pub struct BinanceSource;

impl PushSource for BinanceSource {
    fn feed_interval(&self, symbol: &str, timeframe: &str) -> Result<String, String> {
        binance_only(symbol)?;
        let interval = fetch::binance_interval(timeframe);
        if BINANCE_KLINES.contains(&interval) {
            Ok(interval.to_string())
        } else {
            Err(format!(
                "not pushable: Binance has no {timeframe} kline stream; poll generate_chart instead"
            ))
        }
    }

    fn seed(&self, symbol: &str, timeframe: &str, count: usize) -> Result<OhlcvData, String> {
        fetch::fetch_ohlcv(DataSource::Live, symbol, timeframe, count, None).map(|f| f.data)
    }

    fn run(
        &self,
        symbol: String,
        interval: String,
        tx: mpsc::Sender<FeedEvent>,
    ) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let feed = BinanceFeed::new(&symbol, &interval).with_trades();
            if let Err(e) = feed.run(tx).await {
                eprintln!("[push] feed {symbol}@{interval} ended: {e}");
            }
        })
    }
}

/// `CHARTGEN_DATA_SOURCE=sample`: synthetic ticks every `SAMPLE_TICK`, no
/// network. The same symbols as live are pushable.
pub struct SampleSource;

impl PushSource for SampleSource {
    fn feed_interval(&self, symbol: &str, timeframe: &str) -> Result<String, String> {
        binance_only(symbol)?;
        data::interval_secs(timeframe)
            .map(|_| timeframe.to_string())
            .ok_or_else(|| format!("Unsupported timeframe: {timeframe}"))
    }

    fn seed(&self, symbol: &str, timeframe: &str, count: usize) -> Result<OhlcvData, String> {
        fetch::fetch_ohlcv(DataSource::Sample, symbol, timeframe, count, None).map(|f| f.data)
    }

    fn run(
        &self,
        symbol: String,
        interval: String,
        tx: mpsc::Sender<FeedEvent>,
    ) -> BoxFuture<'static, ()> {
        Box::pin(sample_feed(symbol, interval, tx))
    }
}

/// The sample source's forming bar with a sub-second wobble on its close,
/// and the completed bar when the interval rolls over.
async fn sample_feed(symbol: String, interval: String, tx: mpsc::Sender<FeedEvent>) {
    let mut forming: Option<String> = None;
    let mut every = tokio::time::interval(SAMPLE_TICK);
    loop {
        every.tick().await;
        let now = fetch::now_ms();
        let Ok(d) = data::sample_ohlcv(&symbol, &interval, 2, None, now) else {
            return;
        };
        let Some(mut bar) = d.bars.last().cloned() else {
            return;
        };
        let rolled = forming.as_ref().is_some_and(|f| *f != bar.date);
        if rolled && d.bars.len() == 2 && tx.send(FeedEvent::Bar(d.bars[0].clone())).await.is_err()
        {
            return;
        }
        forming = Some(bar.date.clone());
        bar.close += bar.open * 0.002 * (now as f64 / 350.0).sin();
        bar.high = bar.high.max(bar.close);
        bar.low = bar.low.min(bar.close);
        if tx.send(FeedEvent::Tick(bar)).await.is_err() {
            return;
        }
    }
}

/// One chart, as subscribed. Equal requests share a group.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChartRequest {
    /// As the client sent it: echoed as the tail's `symbol`.
    pub ticker: String,
    pub timeframe: String,
    /// The `indicators` array as JSON (object keys sorted, so canonical).
    pub indicators: String,
    pub chart_style: String,
    pub warmup: usize,
}

/// What `subscribe_chart`/`unsubscribe_chart` need from the HTTP request.
pub struct PushCtx {
    pub hub: Arc<ChartHub>,
    /// Identifies the caller's SSE stream; see `server::stream_key`.
    pub stream: String,
}

type FeedKey = (String, String);

struct StreamSlot {
    id: u64,
    tx: mpsc::UnboundedSender<String>,
}

struct Sub {
    token: String,
    stream: String,
    group: ChartRequest,
}

struct Group {
    subs: HashSet<String>,
    feed: FeedKey,
    task: JoinHandle<()>,
}

struct Feed {
    groups: usize,
    events: broadcast::Sender<FeedEvent>,
    task: JoinHandle<()>,
}

#[derive(Default)]
struct State {
    streams: HashMap<String, StreamSlot>,
    subs: HashMap<String, Sub>,
    groups: HashMap<ChartRequest, Group>,
    feeds: HashMap<FeedKey, Feed>,
}

impl State {
    /// Remove a subscription, then its group and feed if it was their last.
    fn remove_sub(&mut self, id: &str) {
        let Some(sub) = self.subs.remove(id) else {
            return;
        };
        let Some(group) = self.groups.get_mut(&sub.group) else {
            return;
        };
        group.subs.remove(id);
        if !group.subs.is_empty() {
            return;
        }
        let Some(group) = self.groups.remove(&sub.group) else {
            return;
        };
        group.task.abort();
        let Some(feed) = self.feeds.get_mut(&group.feed) else {
            return;
        };
        feed.groups -= 1;
        if feed.groups == 0 {
            if let Some(feed) = self.feeds.remove(&group.feed) {
                feed.task.abort();
            }
        }
    }
}

/// Registry of SSE streams and chart subscriptions, with their groups and
/// feeds.
pub struct ChartHub {
    source: Arc<dyn PushSource>,
    limits: Limits,
    runtime: tokio::runtime::Handle,
    state: Mutex<State>,
    next_stream: AtomicU64,
    me: Weak<ChartHub>,
}

impl ChartHub {
    /// Must be called inside a Tokio runtime; groups and feeds run on it.
    pub fn new(source: Arc<dyn PushSource>, limits: Limits) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            source,
            limits,
            runtime: tokio::runtime::Handle::current(),
            state: Mutex::default(),
            next_stream: AtomicU64::new(1),
            me: me.clone(),
        })
    }

    /// The sample source under `CHARTGEN_DATA_SOURCE=sample`, else Binance.
    pub fn from_env() -> Arc<Self> {
        let source: Arc<dyn PushSource> = match DataSource::from_env() {
            DataSource::Sample => Arc::new(SampleSource),
            _ => Arc::new(BinanceSource),
        };
        Self::new(source, Limits::default())
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Attach an SSE stream under `key` and return its id. A stream that
    /// already had the key (a client reconnect) is replaced, and its
    /// subscriptions now push to the new one.
    pub fn link_stream(&self, key: &str, tx: mpsc::UnboundedSender<String>) -> u64 {
        let id = self.next_stream.fetch_add(1, Ordering::Relaxed);
        self.lock()
            .streams
            .insert(key.to_string(), StreamSlot { id, tx });
        id
    }

    /// Detach stream `id` and drop its subscriptions, unless a newer stream
    /// has replaced it.
    pub fn unlink_stream(&self, key: &str, id: u64) {
        let mut st = self.lock();
        if !st.streams.get(key).is_some_and(|s| s.id == id) {
            return;
        }
        st.streams.remove(key);
        let ids: Vec<String> = st
            .subs
            .iter()
            .filter(|(_, s)| s.stream == key)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            st.remove_sub(&id);
        }
    }

    /// Subscribe `stream` (owned by `token`) to pushed tails of `req`.
    /// `req` must already be validated as a `generate_chart` request.
    pub fn subscribe(
        &self,
        token: &str,
        stream: &str,
        req: ChartRequest,
    ) -> Result<String, String> {
        let interval = self.source.feed_interval(&req.ticker, &req.timeframe)?;
        let mut guard = self.lock();
        let st = &mut *guard;
        if !st.streams.contains_key(stream) {
            return Err("no open SSE stream: open GET /mcp (same token and Mcp-Session-Id) before subscribe_chart".into());
        }
        if st.subs.values().filter(|s| s.token == token).count() >= self.limits.per_token {
            return Err(format!(
                "too many chart subscriptions: at most {} per token",
                self.limits.per_token
            ));
        }
        if st.subs.len() >= self.limits.total {
            return Err(
                "the server is at its chart subscription limit; poll generate_chart instead".into(),
            );
        }
        if !st.groups.contains_key(&req) {
            let feed = (req.ticker.to_lowercase(), interval);
            let events = match st.feeds.get_mut(&feed) {
                Some(f) => {
                    f.groups += 1;
                    f.events.subscribe()
                }
                None => {
                    let (events, rx) = broadcast::channel(FEED_BUFFER);
                    let task = self.spawn_feed(&feed, events.clone());
                    st.feeds.insert(
                        feed.clone(),
                        Feed {
                            groups: 1,
                            events,
                            task,
                        },
                    );
                    rx
                }
            };
            let task = self.runtime.spawn(run_group(
                self.me.clone(),
                self.source.clone(),
                req.clone(),
                events,
                self.limits.min_push_interval,
            ));
            st.groups.insert(
                req.clone(),
                Group {
                    subs: HashSet::new(),
                    feed,
                    task,
                },
            );
        }
        let id = format!("cs-{}", uuid::Uuid::new_v4());
        if let Some(group) = st.groups.get_mut(&req) {
            group.subs.insert(id.clone());
        }
        st.subs.insert(
            id.clone(),
            Sub {
                token: token.to_string(),
                stream: stream.to_string(),
                group: req,
            },
        );
        Ok(id)
    }

    /// Drop subscription `id` if `token` owns it. True if it did.
    pub fn unsubscribe(&self, token: &str, id: &str) -> bool {
        let mut st = self.lock();
        if !st.subs.get(id).is_some_and(|s| s.token == token) {
            return false;
        }
        st.remove_sub(id);
        true
    }

    #[cfg(test)]
    pub fn counts(&self) -> (usize, usize, usize) {
        let st = self.lock();
        (st.subs.len(), st.groups.len(), st.feeds.len())
    }

    fn spawn_feed(&self, key: &FeedKey, events: broadcast::Sender<FeedEvent>) -> JoinHandle<()> {
        let (tx, mut rx) = mpsc::channel(FEED_BUFFER);
        let run = self.source.run(key.0.clone(), key.1.clone(), tx);
        self.runtime.spawn(async move {
            let forward = async {
                while let Some(event) = rx.recv().await {
                    // No receivers only while the last group is going away.
                    let _ = events.send(event);
                }
            };
            tokio::join!(run, forward);
        })
    }

    /// End a group that cannot push (seeding failed, feed gone). Its
    /// subscribers stop receiving ticks and fall back to polling.
    fn drop_group(&self, req: &ChartRequest) {
        let mut st = self.lock();
        let ids: Vec<String> = st
            .groups
            .get(req)
            .map(|g| g.subs.iter().cloned().collect())
            .unwrap_or_default();
        for id in ids {
            st.remove_sub(&id);
        }
    }

    /// Compute the tail of `window` and send it to every subscriber.
    fn push(&self, req: &ChartRequest, window: &[Bar], indicators: &Value) {
        let raw = OhlcvData {
            bars: window.to_vec(),
            symbol: Some(req.ticker.clone()),
            interval: Some(req.timeframe.clone()),
        };
        let series = match crate::mcp::series_tail(&raw, indicators, &req.chart_style, TAIL_BARS) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[push] {} {}: {e}", req.ticker, req.timeframe);
                return;
            }
        };
        let st = self.lock();
        let Some(group) = st.groups.get(req) else {
            return;
        };
        for id in &group.subs {
            let Some(stream) = st.subs.get(id).and_then(|s| st.streams.get(&s.stream)) else {
                continue;
            };
            let note = json!({
                "jsonrpc": "2.0",
                "method": CHART_TICK,
                "params": { "subscription_id": id, "series": &series },
            });
            // A closed stream is unlinked by its own cleanup.
            let _ = stream.tx.send(note.to_string());
        }
    }
}

/// A group's task: seed the window, then apply feed events and push the
/// recomputed tail, throttled to one push per `min_interval`.
async fn run_group(
    hub: Weak<ChartHub>,
    source: Arc<dyn PushSource>,
    req: ChartRequest,
    mut events: broadcast::Receiver<FeedEvent>,
    min_interval: Duration,
) {
    let len = TAIL_BARS + req.warmup;
    let (symbol, timeframe) = (req.ticker.clone(), req.timeframe.clone());
    // Subscribed to the feed before seeding, so no event in between is lost.
    let seeded = tokio::task::spawn_blocking(move || source.seed(&symbol, &timeframe, len)).await;
    let mut window = match seeded {
        Ok(Ok(d)) if !d.is_empty() => d.bars,
        other => {
            let why = match other {
                Ok(Err(e)) => e,
                Err(e) => e.to_string(),
                Ok(Ok(_)) => "no bars".into(),
            };
            eprintln!(
                "[push] seeding {} {} failed: {why}",
                req.ticker, req.timeframe
            );
            if let Some(hub) = hub.upgrade() {
                hub.drop_group(&req);
            }
            return;
        }
    };
    let indicators: Value = serde_json::from_str(&req.indicators).unwrap_or(Value::Null);
    // Push the seeded window right away.
    let mut dirty = true;
    let mut next = Instant::now();
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(FeedEvent::Tick(bar) | FeedEvent::Bar(bar)) => {
                    dirty |= upsert_bar(&mut window, bar, len);
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => {
                    if let Some(hub) = hub.upgrade() {
                        hub.drop_group(&req);
                    }
                    return;
                }
            },
            _ = tokio::time::sleep_until(next), if dirty => {
                let Some(hub) = hub.upgrade() else { return };
                hub.push(&req, &window, &indicators);
                dirty = false;
                next = Instant::now() + min_interval;
            }
        }
    }
}

/// Merge a feed bar into the rolling window by open time: replace the bar
/// it updates, or append a newer one and drop the oldest beyond `cap`.
/// Bars older than the newest that match none of ours are ignored. True if
/// the window changed.
fn upsert_bar(window: &mut Vec<Bar>, bar: Bar, cap: usize) -> bool {
    let ts = |b: &Bar| b.date.parse::<i64>().ok();
    let Some(t) = ts(&bar) else {
        return false;
    };
    match window.iter().rposition(|b| ts(b).is_some_and(|x| x <= t)) {
        Some(i) if ts(&window[i]) == Some(t) => window[i] = bar,
        Some(i) if i + 1 == window.len() => {
            window.push(bar);
            if window.len() > cap {
                window.drain(..window.len() - cap);
            }
        }
        None if window.is_empty() => window.push(bar),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

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

    #[test]
    fn upsert_replaces_appends_and_rolls() {
        let mut w = vec![bar(60, 1.0), bar(120, 2.0)];
        assert!(upsert_bar(&mut w, bar(120, 2.5), 2));
        assert_eq!(w[1].close, 2.5);
        // A late close of an older bar we hold replaces it.
        assert!(upsert_bar(&mut w, bar(60, 1.5), 2));
        assert_eq!(w[0].close, 1.5);
        // A new bar appends and the window keeps its length.
        assert!(upsert_bar(&mut w, bar(180, 3.0), 2));
        assert_eq!(
            w.iter().map(|b| b.date.as_str()).collect::<Vec<_>>(),
            ["120", "180"]
        );
        // Older than the window, or untimed: ignored.
        assert!(!upsert_bar(&mut w, bar(0, 9.0), 2));
        assert!(!upsert_bar(
            &mut w,
            Bar {
                date: "x".into(),
                ..bar(0, 9.0)
            },
            2
        ));
        assert_eq!(w.len(), 2);
        let mut empty = Vec::new();
        assert!(upsert_bar(&mut empty, bar(60, 1.0), 2));
    }

    /// Feeds that the test drives by hand, counting how many are running.
    #[derive(Default)]
    struct Fake {
        started: AtomicUsize,
        running: Arc<AtomicUsize>,
        senders: Mutex<Vec<mpsc::Sender<FeedEvent>>>,
    }

    struct Running(Arc<AtomicUsize>);
    impl Drop for Running {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl PushSource for Fake {
        fn feed_interval(&self, symbol: &str, timeframe: &str) -> Result<String, String> {
            binance_only(symbol).map(|_| timeframe.to_string())
        }
        fn seed(&self, symbol: &str, timeframe: &str, count: usize) -> Result<OhlcvData, String> {
            // 2024-01-01T00:00Z, so the forming bar is fixed.
            data::sample_ohlcv(symbol, timeframe, count, None, 1_704_067_200_000)
        }
        fn run(
            &self,
            _symbol: String,
            _interval: String,
            tx: mpsc::Sender<FeedEvent>,
        ) -> BoxFuture<'static, ()> {
            self.started.fetch_add(1, Ordering::SeqCst);
            self.running.fetch_add(1, Ordering::SeqCst);
            self.senders.lock().unwrap().push(tx);
            let guard = Running(self.running.clone());
            Box::pin(async move {
                let _guard = guard;
                std::future::pending::<()>().await
            })
        }
    }

    impl Fake {
        /// Send `bar` on the newest feed.
        async fn send(&self, event: FeedEvent) {
            let tx = self.senders.lock().unwrap().last().unwrap().clone();
            tx.send(event).await.unwrap();
        }
    }

    fn hub(fake: &Arc<Fake>, limits: Limits) -> Arc<ChartHub> {
        ChartHub::new(fake.clone(), limits)
    }

    fn req(ticker: &str, indicators: &str) -> ChartRequest {
        ChartRequest {
            ticker: ticker.into(),
            timeframe: "1m".into(),
            indicators: indicators.into(),
            chart_style: "candles".into(),
            warmup: 30,
        }
    }

    /// Next notification on `rx` within `ms`, parsed.
    async fn next(rx: &mut mpsc::UnboundedReceiver<String>, ms: u64) -> Option<Value> {
        tokio::time::timeout(Duration::from_millis(ms), rx.recv())
            .await
            .ok()
            .flatten()
            .map(|s| serde_json::from_str(&s).unwrap())
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn pushes_tails_and_shares_one_feed() {
        let fake = Arc::new(Fake::default());
        let hub = hub(&fake, Limits::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        hub.link_stream("s1", tx);

        let a = hub
            .subscribe("tok", "s1", req("BTCUSDT", r#"["rsi"]"#))
            .unwrap();
        let b = hub
            .subscribe("tok", "s1", req("BTCUSDT", r#"["macd"]"#))
            .unwrap();
        assert_eq!(hub.counts(), (2, 2, 1));
        assert_eq!(
            fake.started.load(Ordering::SeqCst),
            1,
            "one feed per symbol/interval"
        );

        // Each group pushes its seeded tail first.
        let mut seen = HashSet::new();
        for _ in 0..2 {
            let n = next(&mut rx, 2_000).await.expect("seed push");
            assert_eq!(n["method"], CHART_TICK);
            let series = &n["params"]["series"];
            assert_eq!(series["bars"].as_array().unwrap().len(), TAIL_BARS);
            assert_eq!(series["symbol"], "BTCUSDT");
            assert_eq!(series["interval"], "1m");
            seen.insert(n["params"]["subscription_id"].as_str().unwrap().to_string());
        }
        assert_eq!(seen, HashSet::from([a.clone(), b.clone()]));

        // A tick on the forming bar reaches both, with indicators recomputed.
        let forming: i64 = 1_704_067_200;
        fake.send(FeedEvent::Tick(bar(forming, 123.0))).await;
        for _ in 0..2 {
            let n = next(&mut rx, 2_000).await.expect("tick push");
            let series = &n["params"]["series"];
            assert_eq!(series["bars"][1]["t"], forming);
            assert_eq!(series["bars"][1]["c"], 123.0);
            assert!(series["indicators"].as_object().unwrap().len() == 1);
        }

        // Unsubscribing the last group tears the feed down.
        assert!(
            !hub.unsubscribe("other-token", &a),
            "only the owner may unsubscribe"
        );
        assert!(hub.unsubscribe("tok", &a));
        assert_eq!(hub.counts(), (1, 1, 1));
        assert!(hub.unsubscribe("tok", &b));
        settle().await;
        assert_eq!(hub.counts(), (0, 0, 0));
        assert_eq!(fake.running.load(Ordering::SeqCst), 0, "feed task stopped");
    }

    #[tokio::test]
    async fn same_chart_shares_a_group() {
        let fake = Arc::new(Fake::default());
        let hub = hub(&fake, Limits::default());
        let (t1, mut r1) = mpsc::unbounded_channel();
        let (t2, mut r2) = mpsc::unbounded_channel();
        hub.link_stream("s1", t1);
        hub.link_stream("s2", t2);
        hub.subscribe("u1", "s1", req("BTCUSDT", "[]")).unwrap();
        // Seeded before the second subscriber joins.
        assert!(next(&mut r1, 2_000).await.is_some());
        hub.subscribe("u2", "s2", req("BTCUSDT", "[]")).unwrap();
        assert_eq!(hub.counts(), (2, 1, 1));
        fake.send(FeedEvent::Tick(bar(1_704_067_200, 5.0))).await;
        assert_eq!(
            next(&mut r1, 2_000).await.unwrap()["params"]["series"]["bars"][1]["c"],
            5.0
        );
        assert_eq!(
            next(&mut r2, 2_000).await.unwrap()["params"]["series"]["bars"][1]["c"],
            5.0
        );
    }

    #[tokio::test]
    async fn pushes_are_throttled_and_end_on_the_latest_tick() {
        let fake = Arc::new(Fake::default());
        let limits = Limits {
            min_push_interval: Duration::from_millis(200),
            ..Limits::default()
        };
        let hub = hub(&fake, limits);
        let (tx, mut rx) = mpsc::unbounded_channel();
        hub.link_stream("s1", tx);
        hub.subscribe("tok", "s1", req("BTCUSDT", r#"["rsi"]"#))
            .unwrap();
        next(&mut rx, 2_000).await.expect("seed push");

        for i in 0..50 {
            fake.send(FeedEvent::Tick(bar(1_704_067_200, 100.0 + i as f64)))
                .await;
        }
        let mut closes = Vec::new();
        while let Some(n) = next(&mut rx, 500).await {
            closes.push(n["params"]["series"]["bars"][1]["c"].as_f64().unwrap());
        }
        assert!(
            (1..=3).contains(&closes.len()),
            "50 ticks in a burst must collapse into a few pushes, got {closes:?}"
        );
        assert_eq!(closes.last(), Some(&149.0), "the newest tick is never lost");
    }

    #[tokio::test]
    async fn caps_streams_and_pushability() {
        let fake = Arc::new(Fake::default());
        let limits = Limits {
            per_token: 2,
            total: 3,
            ..Limits::default()
        };
        let hub = hub(&fake, limits);
        let (tx, _rx) = mpsc::unbounded_channel();
        hub.link_stream("s1", tx.clone());
        hub.link_stream("s2", tx);

        let err = hub
            .subscribe("tok", "nope", req("BTCUSDT", "[]"))
            .unwrap_err();
        assert!(err.contains("no open SSE stream"), "{err}");
        let err = hub.subscribe("tok", "s1", req("AAPL", "[]")).unwrap_err();
        assert!(err.starts_with("not pushable"), "{err}");

        hub.subscribe("a", "s1", req("BTCUSDT", "[]")).unwrap();
        hub.subscribe("a", "s1", req("ETHUSDT", "[]")).unwrap();
        let err = hub.subscribe("a", "s1", req("SOLUSDT", "[]")).unwrap_err();
        assert!(err.contains("per token"), "{err}");
        hub.subscribe("b", "s2", req("BTCUSDT", "[]")).unwrap();
        let err = hub.subscribe("c", "s2", req("BTCUSDT", "[]")).unwrap_err();
        assert!(err.contains("subscription limit"), "{err}");
        assert_eq!(hub.counts(), (3, 2, 2));
    }

    #[tokio::test]
    async fn closing_a_stream_drops_its_subscriptions() {
        let fake = Arc::new(Fake::default());
        let hub = hub(&fake, Limits::default());
        let (tx, _rx) = mpsc::unbounded_channel();
        let old = hub.link_stream("s1", tx.clone());
        hub.subscribe("tok", "s1", req("BTCUSDT", "[]")).unwrap();

        // A reconnect replaces the stream; the old one's cleanup is a no-op.
        let new = hub.link_stream("s1", tx);
        hub.unlink_stream("s1", old);
        assert_eq!(hub.counts(), (1, 1, 1));

        hub.unlink_stream("s1", new);
        settle().await;
        assert_eq!(hub.counts(), (0, 0, 0));
        assert_eq!(fake.running.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn sample_source_ticks_sub_second() {
        let (tx, mut rx) = mpsc::channel(16);
        let feed = tokio::spawn(SampleSource.run("BTCUSDT".into(), "1m".into(), tx));
        let mut closes = Vec::new();
        for _ in 0..3 {
            match tokio::time::timeout(Duration::from_millis(1_500), rx.recv()).await {
                Ok(Some(FeedEvent::Tick(b))) => closes.push(b.close),
                Ok(Some(FeedEvent::Bar(_))) => {}
                other => panic!("expected a tick, got {other:?}"),
            }
        }
        feed.abort();
        closes.dedup();
        assert!(closes.len() >= 2, "ticks move the close: {closes:?}");
    }
}
