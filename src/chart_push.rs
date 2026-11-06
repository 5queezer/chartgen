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
//! `generate_chart(format=series, bars=TAIL_BARS, warmup)`, on the blocking
//! pool, and pushes it as `notifications/chart_tick`, at most once per
//! `push_interval`. The last subscriber leaving (unsubscribe, or its SSE
//! stream closing or falling behind) stops the group; a feed without groups
//! lingers for `Limits::feed_linger`, then closes.
//!
//! Independent of the trading engine, so it also runs under plain `--serve`.

use std::collections::{HashMap, HashSet, VecDeque};
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
/// Most indicators one subscription may ask for.
pub const MAX_INDICATORS: usize = 10;
/// Frames buffered per SSE stream. A stream that falls this far behind is
/// unlinked (see `ChartHub::send`).
pub const STREAM_BUFFER: usize = 256;
/// Prefix of errors for a chart the server will not push; clients poll.
pub const NOT_PUSHABLE: &str = "not pushable";
/// Prefix of errors for a cap or rate limit; clients poll this chart.
pub const PUSH_LIMIT: &str = "push limit reached";
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
    /// Subscriptions one client address may hold, across its tokens.
    pub per_client: usize,
    /// Subscriptions the server holds in total.
    pub total: usize,
    /// `subscribe_chart` calls one token may make per minute.
    pub subscribes_per_minute: usize,
    /// Shortest gap between two pushes of a group with warmup ≤ 300; twice
    /// that above (see `push_interval`).
    pub min_push_interval: Duration,
    /// How long a feed without groups stays connected, so a quick
    /// unsubscribe/subscribe does not reconnect the WebSocket.
    pub feed_linger: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            per_token: 4,
            per_client: 8,
            total: 64,
            subscribes_per_minute: 10,
            min_push_interval: Duration::from_millis(250),
            feed_linger: Duration::from_secs(30),
        }
    }
}

/// The recompute costs about linearly in `TAIL_BARS + warmup`, so large
/// windows push half as often.
fn push_interval(base: Duration, warmup: usize) -> Duration {
    if warmup <= 300 {
        base
    } else {
        base * 2
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
            "{NOT_PUSHABLE}: {symbol} is not a Binance symbol; poll generate_chart instead"
        ))
    }
}

/// The ticker as it goes into a feed (and the Binance WebSocket URL):
/// uppercase ASCII letters and digits, 2 to 20 of them.
fn feed_symbol(ticker: &str) -> Result<String, String> {
    let t = ticker.to_ascii_uppercase();
    if (2..=20).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(t)
    } else {
        Err(format!(
            "{NOT_PUSHABLE}: the ticker must be 2-20 letters or digits"
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
                "{NOT_PUSHABLE}: Binance has no {timeframe} kline stream; poll generate_chart instead"
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
    /// Warmup the group computes with: the client's, raised to what the
    /// indicators need (ADR-0032).
    pub warmup: usize,
    /// The client's `warmup` before that raise. The push cadence follows
    /// it, so the server's own raise does not slow pushes down.
    pub asked_warmup: usize,
}

impl ChartRequest {
    /// Shortest gap between two pushes of this chart's group.
    fn push_interval(&self, base: Duration) -> Duration {
        push_interval(base, self.asked_warmup)
    }
}

/// What `subscribe_chart`/`unsubscribe_chart` need from the HTTP request.
pub struct PushCtx {
    pub hub: Arc<ChartHub>,
    /// Identifies the caller's SSE stream; see `server::stream_key`.
    pub stream: String,
    /// The caller's network address, for the per-client cap.
    pub client: String,
}

/// A feed: the symbol as it goes into the stream name, and its interval.
pub type FeedKey = (String, String);

struct StreamSlot {
    id: u64,
    tx: mpsc::Sender<String>,
}

struct Sub {
    token: String,
    client: String,
    stream: String,
    group: ChartRequest,
}

struct Group {
    /// Tells this group apart from a later one for the same chart.
    id: u64,
    subs: HashSet<String>,
    feed: FeedKey,
    task: JoinHandle<()>,
}

struct Feed {
    groups: usize,
    /// Only the feed task holds the sender, so its groups see `Closed` when
    /// the feed ends.
    events: broadcast::WeakSender<FeedEvent>,
    task: JoinHandle<()>,
    /// Bumped whenever `groups` drops to 0 or leaves it; a pending linger
    /// closes the feed only if it is unchanged.
    epoch: u64,
}

#[derive(Default)]
struct State {
    streams: HashMap<String, StreamSlot>,
    subs: HashMap<String, Sub>,
    groups: HashMap<ChartRequest, Group>,
    feeds: HashMap<FeedKey, Feed>,
    /// Recent `subscribe_chart` calls per token.
    attempts: HashMap<String, VecDeque<std::time::Instant>>,
}

impl State {
    /// Remove a subscription, then its group if it was the last. Returns a
    /// feed that has no groups left, with its epoch, to close after the
    /// linger.
    fn remove_sub(&mut self, id: &str) -> Option<(FeedKey, u64)> {
        let sub = self.subs.remove(id)?;
        let group = self.groups.get_mut(&sub.group)?;
        group.subs.remove(id);
        if !group.subs.is_empty() {
            return None;
        }
        let group = self.groups.remove(&sub.group)?;
        group.task.abort();
        let feed = self.feeds.get_mut(&group.feed)?;
        feed.groups -= 1;
        if feed.groups > 0 {
            return None;
        }
        feed.epoch += 1;
        Some((group.feed, feed.epoch))
    }

    fn subs_of(&self, keep: impl Fn(&Sub) -> bool) -> Vec<String> {
        self.subs
            .iter()
            .filter(|(_, s)| keep(s))
            .map(|(id, _)| id.clone())
            .collect()
    }
}

/// Keeps a feed running for `ChartHub::watch_feed`.
pub struct FeedLease {
    hub: Weak<ChartHub>,
    key: FeedKey,
}

impl Drop for FeedLease {
    fn drop(&mut self) {
        let Some(hub) = self.hub.upgrade() else {
            return;
        };
        let mut st = hub.lock();
        let Some(feed) = st.feeds.get_mut(&self.key) else {
            return;
        };
        feed.groups = feed.groups.saturating_sub(1);
        if feed.groups == 0 {
            feed.epoch += 1;
            let epoch = feed.epoch;
            hub.close_later(self.key.clone(), epoch);
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
    next_id: AtomicU64,
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
            next_id: AtomicU64::new(1),
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

    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Attach an SSE stream under `key` and return its id. A stream that
    /// already had the key (a client reconnect) is replaced, and its
    /// subscriptions now push to the new one.
    pub fn link_stream(&self, key: &str, tx: mpsc::Sender<String>) -> u64 {
        let id = self.id();
        self.lock()
            .streams
            .insert(key.to_string(), StreamSlot { id, tx });
        id
    }

    /// Detach stream `id` and drop its subscriptions, unless a newer stream
    /// has replaced it.
    pub fn unlink_stream(&self, key: &str, id: u64) {
        let mut st = self.lock();
        if st.streams.get(key).is_some_and(|s| s.id == id) {
            self.drop_stream(&mut st, key);
        }
    }

    fn drop_stream(&self, st: &mut State, key: &str) {
        st.streams.remove(key);
        for id in st.subs_of(|s| s.stream == key) {
            self.remove_sub(st, &id);
        }
    }

    fn remove_sub(&self, st: &mut State, id: &str) {
        if let Some((feed, epoch)) = st.remove_sub(id) {
            self.close_later(feed, epoch);
        }
    }

    /// Close an idle feed after the linger, unless a group took it again.
    fn close_later(&self, key: FeedKey, epoch: u64) {
        let hub = self.me.clone();
        let linger = self.limits.feed_linger;
        self.runtime.spawn(async move {
            tokio::time::sleep(linger).await;
            let Some(hub) = hub.upgrade() else { return };
            let mut st = hub.lock();
            if st
                .feeds
                .get(&key)
                .is_some_and(|f| f.groups == 0 && f.epoch == epoch)
            {
                if let Some(feed) = st.feeds.remove(&key) {
                    feed.task.abort();
                }
            }
        });
    }

    /// Subscribe `stream` (owned by `token`, from `client`) to pushed tails
    /// of `req`. `req` must already be validated as a `generate_chart`
    /// request.
    pub fn subscribe(
        &self,
        token: &str,
        client: &str,
        stream: &str,
        req: ChartRequest,
    ) -> Result<String, String> {
        let interval = self.source.feed_interval(&req.ticker, &req.timeframe)?;
        let symbol = feed_symbol(&req.ticker)?;
        let limits = &self.limits;
        let mut guard = self.lock();
        let st = &mut *guard;

        let now = std::time::Instant::now();
        let minute = Duration::from_secs(60);
        st.attempts.retain(|_, calls| {
            while calls.front().is_some_and(|t| now - *t >= minute) {
                calls.pop_front();
            }
            !calls.is_empty()
        });
        let calls = st.attempts.entry(token.to_string()).or_default();
        if calls.len() >= limits.subscribes_per_minute {
            return Err(format!(
                "{PUSH_LIMIT}: at most {} subscribe_chart calls per minute per token",
                limits.subscribes_per_minute
            ));
        }
        calls.push_back(now);

        if !st.streams.contains_key(stream) {
            return Err("no open SSE stream: open GET /mcp (same token and Mcp-Session-Id) before subscribe_chart".into());
        }
        let count = |keep: &dyn Fn(&Sub) -> bool| st.subs.values().filter(|s| keep(s)).count();
        if count(&|s| s.token == token) >= limits.per_token {
            return Err(format!(
                "{PUSH_LIMIT}: at most {} chart subscriptions per token",
                limits.per_token
            ));
        }
        if count(&|s| s.client == client) >= limits.per_client {
            return Err(format!(
                "{PUSH_LIMIT}: at most {} chart subscriptions per client address",
                limits.per_client
            ));
        }
        if st.subs.len() >= limits.total {
            return Err(format!(
                "{PUSH_LIMIT}: the server holds its maximum of {} chart subscriptions",
                limits.total
            ));
        }

        if !st.groups.contains_key(&req) {
            let feed = (symbol, interval);
            let events = self.join_feed(st, &feed);
            let id = self.id();
            let task = self.runtime.spawn(run_group(
                self.me.clone(),
                self.source.clone(),
                req.clone(),
                id,
                events,
                req.push_interval(limits.min_push_interval),
            ));
            st.groups.insert(
                req.clone(),
                Group {
                    id,
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
                client: client.to_string(),
                stream: stream.to_string(),
                group: req,
            },
        );
        Ok(id)
    }

    /// One more user of `feed`, started if it is new or its task ended. Users
    /// still on an ended feed see `Closed` and leave.
    fn join_feed(&self, st: &mut State, feed: &FeedKey) -> broadcast::Receiver<FeedEvent> {
        if let Some(f) = st.feeds.get_mut(feed) {
            if let Some(events) = f.events.upgrade() {
                if f.groups == 0 {
                    f.epoch += 1;
                }
                f.groups += 1;
                return events.subscribe();
            }
        }
        let groups = st.feeds.remove(feed).map_or(0, |f| f.groups);
        let (events, rx) = broadcast::channel(FEED_BUFFER);
        let weak = events.downgrade();
        let task = self.spawn_feed(feed, events);
        st.feeds.insert(
            feed.clone(),
            Feed {
                groups: groups + 1,
                events: weak,
                task,
                epoch: 0,
            },
        );
        rx
    }

    /// The feed serving `symbol` at `timeframe`, or why there is none.
    pub fn feed_key(&self, symbol: &str, timeframe: &str) -> Result<FeedKey, String> {
        let interval = self.source.feed_interval(symbol, timeframe)?;
        Ok((feed_symbol(symbol)?, interval))
    }

    /// Does `symbol` have bars at `timeframe`? Blocking; one fetch.
    pub fn check_symbol(&self, symbol: &str, timeframe: &str) -> Result<(), String> {
        match self.source.seed(symbol, timeframe, 2) {
            Ok(d) if !d.is_empty() => Ok(()),
            Ok(_) => Err(format!("no bars for {symbol}")),
            Err(e) => Err(e),
        }
    }

    /// Listen to a feed without a chart (server alerts, ADR-0042): it shares the
    /// WebSocket with every chart of that symbol and interval, and closes after
    /// the linger once the lease and all charts are gone.
    pub fn watch_feed(
        self: &Arc<Self>,
        key: &FeedKey,
    ) -> (broadcast::Receiver<FeedEvent>, FeedLease) {
        let rx = self.join_feed(&mut self.lock(), key);
        let lease = FeedLease {
            hub: Arc::downgrade(self),
            key: key.clone(),
        };
        (rx, lease)
    }

    /// Drop subscription `id` if `token` owns it. True if it did.
    pub fn unsubscribe(&self, token: &str, id: &str) -> bool {
        let mut st = self.lock();
        if !st.subs.get(id).is_some_and(|s| s.token == token) {
            return false;
        }
        self.remove_sub(&mut st, id);
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
                    // No receivers while the feed lingers without groups.
                    let _ = events.send(event);
                }
            };
            tokio::join!(run, forward);
        })
    }

    /// End group `id` of `req` that cannot push (seeding or a recompute
    /// failed, the feed ended). Its subscribers stop receiving ticks and
    /// fall back to polling.
    fn drop_group(&self, req: &ChartRequest, id: u64) {
        let mut st = self.lock();
        let ids: Vec<String> = match st.groups.get(req) {
            Some(g) if g.id == id => g.subs.iter().cloned().collect(),
            _ => return,
        };
        for id in ids {
            self.remove_sub(&mut st, &id);
        }
    }

    /// Send a computed tail to every subscriber of `req`. A stream whose
    /// buffer is full (a stalled or dead peer) or closed is unlinked with
    /// all its subscriptions; without the trading engine that also ends the
    /// SSE response, and a live client reconnects.
    fn send(&self, req: &ChartRequest, series: &Value) {
        let mut st = self.lock();
        let Some(group) = st.groups.get(req) else {
            return;
        };
        let mut dead = Vec::new();
        for id in &group.subs {
            let Some(sub) = st.subs.get(id) else { continue };
            let Some(stream) = st.streams.get(&sub.stream) else {
                continue;
            };
            let note = json!({
                "jsonrpc": "2.0",
                "method": CHART_TICK,
                "params": { "subscription_id": id, "series": series },
            });
            if stream.tx.try_send(note.to_string()).is_err() {
                dead.push(sub.stream.clone());
            }
        }
        dead.sort();
        dead.dedup();
        for key in dead {
            eprintln!("[push] SSE stream full or closed; dropping its subscriptions");
            self.drop_stream(&mut st, &key);
        }
    }
}

/// A group's task: seed the window, then apply feed events and push the
/// recomputed tail, throttled to one push per `min_interval`. Seeding and
/// recomputes run on the blocking pool; one that fails or panics ends the
/// group.
async fn run_group(
    hub: Weak<ChartHub>,
    source: Arc<dyn PushSource>,
    req: ChartRequest,
    id: u64,
    mut events: broadcast::Receiver<FeedEvent>,
    min_interval: Duration,
) {
    let fail = |why: String| {
        eprintln!("[push] {} {}: {why}", req.ticker, req.timeframe);
        if let Some(hub) = hub.upgrade() {
            hub.drop_group(&req, id);
        }
    };
    let len = TAIL_BARS + req.warmup;
    let (symbol, timeframe) = (req.ticker.clone(), req.timeframe.clone());
    // Subscribed to the feed before seeding, so no event in between is lost.
    let seeded = tokio::task::spawn_blocking(move || source.seed(&symbol, &timeframe, len)).await;
    let mut window = match seeded {
        Ok(Ok(d)) if !d.is_empty() => d.bars,
        Ok(Ok(_)) => return fail("seeding returned no bars".into()),
        Ok(Err(e)) => return fail(format!("seeding failed: {e}")),
        Err(e) => return fail(format!("seeding failed: {e}")),
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
                Err(broadcast::error::RecvError::Closed) => return fail("feed ended".into()),
            },
            _ = tokio::time::sleep_until(next), if dirty => {
                let raw = OhlcvData {
                    bars: window.clone(),
                    symbol: Some(req.ticker.clone()),
                    interval: Some(req.timeframe.clone()),
                };
                let (indicators, style) = (indicators.clone(), req.chart_style.clone());
                let computed = tokio::task::spawn_blocking(move || {
                    crate::mcp::series_tail(&raw, &indicators, &style, TAIL_BARS)
                })
                .await;
                let series = match computed {
                    Ok(Ok(series)) => series,
                    Ok(Err(e)) => return fail(format!("recompute failed: {e}")),
                    Err(e) => return fail(format!("recompute panicked: {e}")),
                };
                let Some(hub) = hub.upgrade() else { return };
                hub.send(&req, &series);
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
    use std::sync::atomic::{AtomicBool, AtomicUsize};

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
    fn new_intervals_are_pushable() {
        for tf in ["3m", "2h", "6h", "12h", "3d", "1wk", "1mo"] {
            assert!(BinanceSource.feed_interval("BTCUSDT", tf).is_ok(), "{tf}");
            assert!(SampleSource.feed_interval("BTCUSDT", tf).is_ok(), "{tf}");
        }
        assert_eq!(BinanceSource.feed_interval("BTCUSDT", "3d").unwrap(), "3d");
        assert_eq!(BinanceSource.feed_interval("BTCUSDT", "1mo").unwrap(), "1M");
        assert!(BinanceSource.feed_interval("BTCUSDT", "2m").is_err());
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

    #[test]
    fn tickers_and_push_intervals() {
        assert_eq!(feed_symbol("btcUSDT").unwrap(), "BTCUSDT");
        for bad in [
            "B",
            "BTC/USDT",
            "btcusdt@trade",
            "../x",
            "",
            &"A".repeat(21),
        ] {
            assert!(
                feed_symbol(bad).unwrap_err().starts_with(NOT_PUSHABLE),
                "{bad}"
            );
        }
        let base = Duration::from_millis(250);
        assert_eq!(push_interval(base, 300), base);
        assert_eq!(push_interval(base, 1000), base * 2);
        // A server-raised warmup (ADR-0032) keeps the client's cadence.
        let raised = ChartRequest {
            warmup: 891,
            asked_warmup: 300,
            ..req("BTCUSDT", r#"["ema_stack"]"#)
        };
        assert_eq!(raised.push_interval(base), base);
        let asked = ChartRequest {
            asked_warmup: 1000,
            ..raised
        };
        assert_eq!(asked.push_interval(base), base * 2);
    }

    /// Feeds that the test drives by hand, counting how many are running.
    #[derive(Default)]
    struct Fake {
        started: AtomicUsize,
        running: Arc<AtomicUsize>,
        senders: Mutex<Vec<mpsc::WeakSender<FeedEvent>>>,
        /// Ends every running feed.
        stop: Arc<tokio::sync::Notify>,
        panic_on_seed: AtomicBool,
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
            assert!(!self.panic_on_seed.load(Ordering::SeqCst), "seed exploded");
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
            self.senders.lock().unwrap().push(tx.downgrade());
            let guard = Running(self.running.clone());
            let stop = self.stop.clone();
            // The feed owns the only strong sender, like a real one: when it
            // ends, its groups see the channel close.
            Box::pin(async move {
                let _guard = guard;
                let _tx = tx;
                stop.notified().await
            })
        }
    }

    impl Fake {
        /// Send `event` on the newest feed.
        async fn send(&self, event: FeedEvent) {
            let tx = self
                .senders
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .upgrade()
                .unwrap();
            tx.send(event).await.unwrap();
        }
    }

    /// Default limits, but feeds close right after their last group.
    fn quick() -> Limits {
        Limits {
            feed_linger: Duration::from_millis(20),
            ..Limits::default()
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
            asked_warmup: 30,
        }
    }

    fn stream() -> (mpsc::Sender<String>, mpsc::Receiver<String>) {
        mpsc::channel(STREAM_BUFFER)
    }

    /// Next notification on `rx` within `ms`, parsed.
    async fn next(rx: &mut mpsc::Receiver<String>, ms: u64) -> Option<Value> {
        tokio::time::timeout(Duration::from_millis(ms), rx.recv())
            .await
            .ok()
            .flatten()
            .map(|s| serde_json::from_str(&s).unwrap())
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn pushes_tails_and_shares_one_feed() {
        let fake = Arc::new(Fake::default());
        let hub = hub(&fake, quick());
        let (tx, mut rx) = stream();
        hub.link_stream("s1", tx);

        let a = hub
            .subscribe("tok", "ip", "s1", req("BTCUSDT", r#"["rsi"]"#))
            .unwrap();
        let b = hub
            .subscribe("tok", "ip", "s1", req("BTCUSDT", r#"["macd"]"#))
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

        // Unsubscribing the last group tears the feed down after the linger.
        assert!(
            !hub.unsubscribe("other-token", &a),
            "only the owner may unsubscribe"
        );
        assert!(hub.unsubscribe("tok", &a));
        assert_eq!(hub.counts(), (1, 1, 1));
        assert!(hub.unsubscribe("tok", &b));
        assert_eq!(hub.counts(), (0, 0, 1), "the feed lingers");
        settle().await;
        assert_eq!(hub.counts(), (0, 0, 0));
        assert_eq!(fake.running.load(Ordering::SeqCst), 0, "feed task stopped");
    }

    #[tokio::test]
    async fn tails_key_each_indicator_instance() {
        // ADR-0028: two RSIs in one subscription, each under its own key.
        let fake = Arc::new(Fake::default());
        let hub = hub(&fake, quick());
        let (tx, mut rx) = stream();
        hub.link_stream("s1", tx);
        let indicators = r#"["rsi",{"id":"rsi#2","length":21,"name":"rsi"}]"#;
        hub.subscribe("tok", "ip", "s1", req("BTCUSDT", indicators))
            .unwrap();
        let seed = next(&mut rx, 2_000).await.expect("seed push");
        fake.send(FeedEvent::Tick(bar(1_704_067_200, 123.0))).await;
        let tick = next(&mut rx, 2_000).await.expect("tick push");
        for n in [seed, tick] {
            let ind = &n["params"]["series"]["indicators"];
            assert_eq!(ind.as_object().unwrap().len(), 2);
            let last = |k: &str| ind[k]["lines"][0]["values"][1].as_f64().unwrap();
            assert_ne!(last("rsi"), last("rsi#2"));
        }
    }

    #[tokio::test]
    async fn a_lingering_feed_is_reused() {
        let fake = Arc::new(Fake::default());
        let hub = hub(
            &fake,
            Limits {
                feed_linger: Duration::from_millis(300),
                ..Limits::default()
            },
        );
        let (tx, mut rx) = stream();
        hub.link_stream("s1", tx);
        let a = hub
            .subscribe("tok", "ip", "s1", req("BTCUSDT", "[]"))
            .unwrap();
        hub.unsubscribe("tok", &a);
        tokio::time::sleep(Duration::from_millis(150)).await;
        // Back within the linger: same WebSocket, and its timer is void.
        hub.subscribe("tok", "ip", "s1", req("BTCUSDT", "[]"))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(fake.started.load(Ordering::SeqCst), 1);
        assert_eq!(hub.counts(), (1, 1, 1));
        while next(&mut rx, 50).await.is_some() {}
        fake.send(FeedEvent::Tick(bar(1_704_067_200, 7.0))).await;
        assert_eq!(
            next(&mut rx, 2_000).await.unwrap()["params"]["series"]["bars"][1]["c"],
            7.0
        );
    }

    #[tokio::test]
    async fn a_watcher_shares_the_charts_feed_and_holds_it_open() {
        let fake = Arc::new(Fake::default());
        let hub = hub(
            &fake,
            Limits {
                feed_linger: Duration::from_millis(50),
                ..Limits::default()
            },
        );
        let (tx, mut rx) = stream();
        hub.link_stream("s1", tx);
        let sub = hub
            .subscribe("tok", "ip", "s1", req("BTCUSDT", "[]"))
            .unwrap();
        let key = hub.feed_key("btcusdt", "1m").unwrap();
        assert_eq!(key, ("BTCUSDT".to_string(), "1m".to_string()));
        let (mut events, lease) = hub.watch_feed(&key);
        assert_eq!(
            fake.started.load(Ordering::SeqCst),
            1,
            "no second WebSocket"
        );

        fake.send(FeedEvent::Tick(bar(1_704_067_200, 7.0))).await;
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv());
        let Ok(Ok(FeedEvent::Tick(seen))) = event.await else {
            panic!("the watcher got no tick");
        };
        assert_eq!(seen.close, 7.0);

        // The chart leaving does not close a feed that is watched.
        hub.unsubscribe("tok", &sub);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(hub.counts(), (0, 0, 1));
        assert_eq!(fake.running.load(Ordering::SeqCst), 1);

        // The lease going closes it after the linger, if nobody came back.
        drop(lease);
        assert_eq!(hub.counts(), (0, 0, 1));
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(hub.counts(), (0, 0, 0));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(fake.running.load(Ordering::SeqCst), 0);
        while next(&mut rx, 20).await.is_some() {}
    }

    #[tokio::test]
    async fn same_chart_shares_a_group() {
        let fake = Arc::new(Fake::default());
        let hub = hub(&fake, quick());
        let (t1, mut r1) = stream();
        let (t2, mut r2) = stream();
        hub.link_stream("s1", t1);
        hub.link_stream("s2", t2);
        hub.subscribe("u1", "ip1", "s1", req("BTCUSDT", "[]"))
            .unwrap();
        // Seeded before the second subscriber joins.
        assert!(next(&mut r1, 2_000).await.is_some());
        hub.subscribe("u2", "ip2", "s2", req("BTCUSDT", "[]"))
            .unwrap();
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
            ..quick()
        };
        let hub = hub(&fake, limits);
        let (tx, mut rx) = stream();
        hub.link_stream("s1", tx);
        hub.subscribe("tok", "ip", "s1", req("BTCUSDT", r#"["rsi"]"#))
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
    async fn caps_rate_limit_and_pushability() {
        let fake = Arc::new(Fake::default());
        let limits = Limits {
            per_token: 2,
            per_client: 3,
            total: 4,
            subscribes_per_minute: 4,
            ..quick()
        };
        let hub = hub(&fake, limits);
        let (tx, _rx) = stream();
        for s in ["s1", "s2", "s3", "s4"] {
            hub.link_stream(s, tx.clone());
        }

        let err = hub
            .subscribe("tok", "ip", "nope", req("BTCUSDT", "[]"))
            .unwrap_err();
        assert!(err.contains("no open SSE stream"), "{err}");
        let err = hub
            .subscribe("tok", "ip", "s1", req("AAPL", "[]"))
            .unwrap_err();
        assert!(err.starts_with(NOT_PUSHABLE), "{err}");
        let err = hub
            .subscribe("tok", "ip", "s1", req("X/BTCUSDT", "[]"))
            .unwrap_err();
        assert!(err.starts_with(NOT_PUSHABLE), "{err}");

        hub.subscribe("a", "ip1", "s1", req("BTCUSDT", "[]"))
            .unwrap();
        hub.subscribe("a", "ip1", "s1", req("ETHUSDT", "[]"))
            .unwrap();
        let err = hub
            .subscribe("a", "ip1", "s1", req("SOLUSDT", "[]"))
            .unwrap_err();
        assert!(
            err.starts_with(PUSH_LIMIT) && err.contains("per token"),
            "{err}"
        );
        // A fresh token from the same address hits the per-client cap.
        hub.subscribe("b", "ip1", "s2", req("BTCUSDT", "[]"))
            .unwrap();
        let err = hub
            .subscribe("c", "ip1", "s3", req("BTCUSDT", "[]"))
            .unwrap_err();
        assert!(err.contains("per client address"), "{err}");
        hub.subscribe("d", "ip2", "s4", req("BTCUSDT", "[]"))
            .unwrap();
        let err = hub
            .subscribe("e", "ip3", "s4", req("BTCUSDT", "[]"))
            .unwrap_err();
        assert!(err.contains("maximum of 4"), "{err}");
        assert_eq!(hub.counts(), (4, 2, 2));

        // Calls that get past the pushability check count, failed or not:
        // "tok" made one above, three more fill its minute.
        for _ in 0..3 {
            let err = hub
                .subscribe("tok", "ip9", "nope", req("BTCUSDT", "[]"))
                .unwrap_err();
            assert!(err.contains("no open SSE stream"), "{err}");
        }
        let err = hub
            .subscribe("tok", "ip9", "s1", req("BTCUSDT", "[]"))
            .unwrap_err();
        assert!(
            err.starts_with(PUSH_LIMIT) && err.contains("per minute"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn closing_a_stream_drops_its_subscriptions() {
        let fake = Arc::new(Fake::default());
        let hub = hub(&fake, quick());
        let (tx, _rx) = stream();
        let old = hub.link_stream("s1", tx.clone());
        hub.subscribe("tok", "ip", "s1", req("BTCUSDT", "[]"))
            .unwrap();

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
    async fn a_stalled_stream_is_dropped_when_its_buffer_fills() {
        let fake = Arc::new(Fake::default());
        let hub = hub(
            &fake,
            Limits {
                min_push_interval: Duration::from_millis(1),
                ..quick()
            },
        );
        // Nobody reads this stream; a 2-frame buffer fills at once.
        let (stalled, _keep) = mpsc::channel(2);
        let (tx, mut rx) = stream();
        hub.link_stream("stalled", stalled);
        hub.link_stream("ok", tx);
        hub.subscribe("u1", "ip1", "stalled", req("BTCUSDT", "[]"))
            .unwrap();
        hub.subscribe("u2", "ip2", "ok", req("BTCUSDT", "[]"))
            .unwrap();
        for i in 0..5 {
            fake.send(FeedEvent::Tick(bar(1_704_067_200, i as f64)))
                .await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(hub.counts().0, 1, "only the reading stream is left");
        assert!(next(&mut rx, 500).await.is_some());
    }

    #[tokio::test]
    async fn failed_groups_and_ended_feeds_are_removed() {
        let fake = Arc::new(Fake::default());
        let hub = hub(&fake, quick());
        let (tx, _rx) = stream();
        hub.link_stream("s1", tx);

        // A seed that panics ends its group instead of leaving a zombie.
        fake.panic_on_seed.store(true, Ordering::SeqCst);
        hub.subscribe("tok", "ip", "s1", req("BTCUSDT", "[]"))
            .unwrap();
        settle().await;
        assert_eq!(hub.counts(), (0, 0, 0));
        fake.panic_on_seed.store(false, Ordering::SeqCst);

        // The feed ending reaches its groups, which leave.
        hub.subscribe("tok", "ip", "s1", req("ETHUSDT", "[]"))
            .unwrap();
        settle().await;
        fake.stop.notify_waiters();
        settle().await;
        assert_eq!(hub.counts(), (0, 0, 0));
        // The next subscription starts the feed again.
        hub.subscribe("tok", "ip", "s1", req("ETHUSDT", "[]"))
            .unwrap();
        assert_eq!(hub.counts(), (1, 1, 1));
        assert_eq!(fake.started.load(Ordering::SeqCst), 3);
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
