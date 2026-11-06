//! Real-time chart push (ADR-0021).
//!
//! `subscribe_chart` ties a subscription to one SSE stream (`GET /mcp`).
//! Subscriptions that ask for the same chart (ticker, timeframe,
//! indicators, chart style, warmup) share a group. A group keeps a rolling
//! window of `TAIL_BARS + warmup` raw bars: seeded through the normal fetch
//! path, then updated from a feed. Groups on the same symbol and interval
//! share one feed, so one Binance WebSocket serves every chart of it. A chart
//! of an interval with no stream (7m, ADR-0038) holds the bars of the stream
//! it is built from and resamples them on each recompute, with the code a fetch
//! uses, so a live bar is the bar a reload shows.
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
use chartgen::interval::{Interval, Unit};
use chartgen::price_bars::{Kind, Params};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::fetch::{self, DataSource};

/// Bars per pushed tail: the last closed bar and the forming one.
pub const TAIL_BARS: usize = 2;
/// One closed source bar can confirm several price-based bars, so their tails are
/// longer (ADR-0047). A jump past this reloads the chart.
pub const PRICE_TAIL_BARS: usize = 64;
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
/// Prefix of errors for too many feeds starting. Anyone's churn can cause this and
/// it passes within a minute, so unlike `PUSH_LIMIT` clients try again.
pub const FEEDS_BUSY: &str = "feeds busy";
/// Cadence of the sample source's synthetic ticks.
const SAMPLE_TICK: Duration = Duration::from_millis(500);
/// Feed events buffered per group before it lags (and skips some).
const FEED_BUFFER: usize = 1024;
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
    /// Shortest gap between two pushes of a group whose window holds at most 302
    /// bars, native ones included; twice that above (see `push_interval`).
    pub min_push_interval: Duration,
    /// How long a feed without groups stays connected, so a quick
    /// unsubscribe/subscribe does not reconnect the WebSocket.
    pub feed_linger: Duration,
    /// New WebSockets per minute, for charts and alerts together: Binance bans an
    /// IP that connects too often, which would stop every chart.
    pub new_feeds_per_minute: usize,
    /// Feeds connected at once, the lingering ones included.
    pub feeds_total: usize,
    /// Shortest gap between two reads of klines a group is missing.
    pub reread_pause: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            per_token: 4,
            // Eight 4-chart grids (ADR-0043). No forwarding header is trusted, so
            // everyone behind one proxy shares this; `total` bounds the server.
            per_client: 32,
            total: 64,
            subscribes_per_minute: 10,
            min_push_interval: Duration::from_millis(250),
            feed_linger: Duration::from_secs(30),
            new_feeds_per_minute: 40,
            feeds_total: 96,
            reread_pause: Duration::from_secs(5),
        }
    }
}

/// The recompute costs about linearly in the bars of its window, native ones
/// included, so large windows push half as often.
fn push_interval(base: Duration, window: usize) -> Duration {
    if window <= TAIL_BARS + 300 {
        base
    } else {
        base * 2
    }
}

/// Bars of a feed that one bar of the chart takes: more than 1 when the chart has
/// no stream of its own, and the window then holds native bars.
fn per_bar(timeframe: &str, feed_interval: &str) -> usize {
    match (
        data::interval_secs(timeframe),
        data::interval_secs(feed_interval),
    ) {
        (Some(chart), Some(feed)) if feed < chart => (chart / feed) as usize,
        _ => 1,
    }
}

/// Feed bars a window holds for a tail computed over `warmup`. One bar more than
/// that when resampling, as the oldest may lack klines.
fn window_bars(warmup: usize, per_bar: usize) -> usize {
    if per_bar > 1 {
        ((TAIL_BARS + warmup + 1) * per_bar).min(fetch::MAX_BASE_BARS)
    } else {
        TAIL_BARS + warmup
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
        Ok(Interval::parse(timeframe)?.binance_base().0.to_string())
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
        // As Binance would, but sample months are a fixed 30 days (`fetch_ohlcv`).
        let iv = Interval::parse(timeframe)?;
        Ok(if iv.unit == Unit::Month {
            timeframe.to_string()
        } else {
            iv.binance_base().0.to_string()
        })
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
/// and the completed bar just after the interval rolls over.
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
        forming = Some(bar.date.clone());
        bar.close += bar.open * 0.002 * (now as f64 / 350.0).sin();
        bar.high = bar.high.max(bar.close);
        bar.low = bar.low.min(bar.close);
        if tx.send(FeedEvent::Tick(bar)).await.is_err() {
            return;
        }
        // After the next candle's first tick, as a feed with trades sends it.
        if rolled && d.bars.len() == 2 && tx.send(FeedEvent::Bar(d.bars[0].clone())).await.is_err()
        {
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
    /// Inputs of a price-based style, canonical JSON, with any pinned box (ADR-0047).
    pub chart_params: Option<String>,
    /// Warmup the group computes with: the client's, raised to what the
    /// indicators need (ADR-0032).
    pub warmup: usize,
    /// The client's `warmup` before that raise. The push cadence follows
    /// it, so the server's own raise does not slow pushes down.
    pub asked_warmup: usize,
}

impl ChartRequest {
    /// Shortest gap between two pushes of this chart's group, on this feed. It
    /// follows the client's warmup, so the server's own raise does not slow it.
    fn push_interval(&self, base: Duration, feed_interval: &str) -> Duration {
        let per_bar = per_bar(&self.timeframe, feed_interval);
        push_interval(base, window_bars(self.asked_warmup, per_bar))
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
    /// When recent feeds started.
    feed_starts: VecDeque<std::time::Instant>,
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

#[cfg(feature = "scripting")]
/// Keeps a feed running for `ChartHub::watch_feed`.
pub struct FeedLease {
    hub: Weak<ChartHub>,
    key: FeedKey,
}

#[cfg(feature = "scripting")]
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
        match DataSource::from_env() {
            // No exchange to protect, and tests start feeds quickly.
            DataSource::Sample => Self::new(
                Arc::new(SampleSource),
                Limits {
                    new_feeds_per_minute: usize::MAX,
                    ..Limits::default()
                },
            ),
            _ => Self::new(Arc::new(BinanceSource), Limits::default()),
        }
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
            let events = self.join_feed(st, &feed)?;
            let id = self.id();
            let task = self.runtime.spawn(run_group(
                self.me.clone(),
                self.source.clone(),
                req.clone(),
                id,
                events,
                feed.1.clone(),
                *limits,
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

    /// Is there room to start `feed`? Yes if it runs already, however idle.
    fn check_feed_room(&self, st: &mut State, feed: &FeedKey) -> Result<(), String> {
        if st
            .feeds
            .get(feed)
            .is_some_and(|f| f.events.upgrade().is_some())
        {
            return Ok(());
        }
        let now = std::time::Instant::now();
        let minute = Duration::from_secs(60);
        while st.feed_starts.front().is_some_and(|t| now - *t >= minute) {
            st.feed_starts.pop_front();
        }
        if st.feed_starts.len() >= self.limits.new_feeds_per_minute {
            return Err(format!(
                "{FEEDS_BUSY}: at most {} new feeds per minute; try again shortly",
                self.limits.new_feeds_per_minute
            ));
        }
        if st.feeds.len() >= self.limits.feeds_total && !st.feeds.contains_key(feed) {
            return Err(format!(
                "{FEEDS_BUSY}: the server holds its maximum of {} feeds; try again shortly",
                self.limits.feeds_total
            ));
        }
        Ok(())
    }

    /// One more user of `feed`, started if it is new or its task ended and there
    /// is room. Users still on an ended feed see `Closed` and leave.
    fn join_feed(
        &self,
        st: &mut State,
        feed: &FeedKey,
    ) -> Result<broadcast::Receiver<FeedEvent>, String> {
        self.check_feed_room(st, feed)?;
        if let Some(f) = st.feeds.get_mut(feed) {
            if let Some(events) = f.events.upgrade() {
                if f.groups == 0 {
                    f.epoch += 1;
                }
                f.groups += 1;
                return Ok(events.subscribe());
            }
        }
        st.feed_starts.push_back(std::time::Instant::now());
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
        Ok(rx)
    }

    /// Could `watch_feed` join `key` now?
    #[cfg(feature = "scripting")]
    pub fn feed_room(&self, key: &FeedKey) -> Result<(), String> {
        self.check_feed_room(&mut self.lock(), key)
    }

    #[cfg(feature = "scripting")]
    /// The feed serving `symbol` at `timeframe`, or why there is none.
    pub fn feed_key(&self, symbol: &str, timeframe: &str) -> Result<FeedKey, String> {
        let interval = self.source.feed_interval(symbol, timeframe)?;
        Ok((feed_symbol(symbol)?, interval))
    }

    #[cfg(feature = "scripting")]
    /// Does `symbol` have bars at `timeframe`? Blocking; one fetch.
    pub fn check_symbol(&self, symbol: &str, timeframe: &str) -> Result<(), String> {
        match self.source.seed(symbol, timeframe, 2) {
            Ok(d) if !d.is_empty() => Ok(()),
            Ok(_) => Err(format!("no bars for {symbol}")),
            Err(e) => Err(e),
        }
    }

    #[cfg(feature = "scripting")]
    /// Listen to a feed without a chart (server alerts, ADR-0042): it shares the
    /// WebSocket with every chart of that symbol and interval, and closes after
    /// the linger once the lease and all charts are gone.
    pub fn watch_feed(
        self: &Arc<Self>,
        key: &FeedKey,
    ) -> Result<(broadcast::Receiver<FeedEvent>, FeedLease), String> {
        let rx = self.join_feed(&mut self.lock(), key)?;
        let lease = FeedLease {
            hub: Arc::downgrade(self),
            key: key.clone(),
        };
        Ok((rx, lease))
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
/// group. The window holds bars of `feed_interval`; when that is finer than
/// the chart's, each recompute resamples it first.
async fn run_group(
    hub: Weak<ChartHub>,
    source: Arc<dyn PushSource>,
    req: ChartRequest,
    id: u64,
    mut events: broadcast::Receiver<FeedEvent>,
    feed_interval: String,
    limits: Limits,
) {
    let min_interval = req.push_interval(limits.min_push_interval, &feed_interval);
    let fail = |why: String| {
        eprintln!("[push] {} {}: {why}", req.ticker, req.timeframe);
        if let Some(hub) = hub.upgrade() {
            hub.drop_group(&req, id);
        }
    };
    let per_bar = per_bar(&req.timeframe, &feed_interval);
    let resample_to = Interval::parse(&req.timeframe).ok().filter(|_| per_bar > 1);
    let len = window_bars(req.warmup, per_bar);
    let symbol = req.ticker.clone();
    let seed_interval = match resample_to {
        Some(_) => feed_interval,
        None => req.timeframe.clone(),
    };
    // Subscribed to the feed before seeding, so no event in between is lost.
    let read = {
        let (source, seed_interval) = (source.clone(), seed_interval.clone());
        move || source.seed(&symbol, &seed_interval, len)
    };
    let seeded = tokio::task::spawn_blocking(read.clone()).await;
    let mut window = match seeded {
        Ok(Ok(d)) if !d.is_empty() => d.bars,
        Ok(Ok(_)) => return fail("seeding returned no bars".into()),
        Ok(Err(e)) => return fail(format!("seeding failed: {e}")),
        Err(e) => return fail(format!("seeding failed: {e}")),
    };
    let indicators: Value = serde_json::from_str(&req.indicators).unwrap_or(Value::Null);
    // A price-based chart keeps the box it resolves now for as long as it lives.
    let price = match Kind::from_style(&req.chart_style) {
        Some(kind) => {
            let args: Option<Value> = req
                .chart_params
                .as_deref()
                .and_then(|p| serde_json::from_str(p).ok());
            let Ok(chart_iv) = Interval::parse(&req.timeframe) else {
                return fail(format!("{} is not an interval", req.timeframe));
            };
            let source = match &resample_to {
                Some(iv) => fetch::resample(&window, iv, true),
                None => window.clone(),
            };
            match Params::parse(kind, args.as_ref())
                .and_then(|p| p.pinned(&source, &chart_iv, fetch::now_ms()))
            {
                Ok(p) => Some(p),
                Err(e) => return fail(e),
            }
        }
        None => None,
    };
    let tail_bars = if price.is_some() {
        PRICE_TAIL_BARS
    } else {
        TAIL_BARS
    };
    // Newest bar of the last tail sent: the next needs only what came since.
    let mut last_sent: Option<i64> = None;
    let price_based = price.is_some();
    // Push the seeded window right away.
    let mut dirty = true;
    let mut next = Instant::now();
    let feed_grid = Interval::parse(&seed_interval).ok();
    // Klines may be missing: a lag, or a bar that does not follow the last. A
    // resampled bar would stay wrong, so read again, which costs one page when the
    // klines are still held, and try again later if the budget says no.
    let mut missing = false;
    let mut next_read = Instant::now();
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(FeedEvent::Tick(bar) | FeedEvent::Bar(bar)) => {
                    missing |= feed_grid.is_some_and(|grid| skips_a_bar(&window, &bar, &grid));
                    if missing && Instant::now() >= next_read {
                        next_read = Instant::now() + limits.reread_pause;
                        match tokio::task::spawn_blocking(read.clone()).await {
                            Ok(Ok(d)) if !d.is_empty() => {
                                window = d.bars;
                                // REST may still be behind the stream: read again
                                // after the pause, until it has caught up.
                                missing = feed_grid
                                    .is_some_and(|grid| skips_a_bar(&window, &bar, &grid));
                                dirty = true;
                            }
                            Ok(Ok(_)) => {}
                            Ok(Err(e)) => eprintln!("[push] {} {}: {e}", req.ticker, req.timeframe),
                            Err(e) => eprintln!("[push] {} {}: {e}", req.ticker, req.timeframe),
                        }
                    }
                    dirty |= upsert_bar(&mut window, bar, len);
                }
                Err(broadcast::error::RecvError::Lagged(_)) => missing = true,
                Err(broadcast::error::RecvError::Closed) => return fail("feed ended".into()),
            },
            _ = tokio::time::sleep_until(next), if dirty => {
                let mut raw = OhlcvData {
                    bars: window.clone(),
                    symbol: Some(req.ticker.clone()),
                    interval: Some(req.timeframe.clone()),
                    session: None,
                };
                let (indicators, style) = (indicators.clone(), req.chart_style.clone());
                let price = price.clone().map(|params| crate::mcp::PriceTail {
                    params,
                    since: last_sent,
                });
                let computed = tokio::task::spawn_blocking(move || {
                    if let Some(iv) = resample_to {
                        raw.bars = fetch::resample(&raw.bars, &iv, true);
                    }
                    crate::mcp::series_tail(&raw, &indicators, &style, price.as_ref(), tail_bars)
                })
                .await;
                let series = match computed {
                    Ok(Ok(series)) => series,
                    Ok(Err(e)) => return fail(format!("recompute failed: {e}")),
                    Err(e) => return fail(format!("recompute panicked: {e}")),
                };
                if price_based {
                    last_sent = series["bars"]
                        .as_array()
                        .and_then(|b| b.last())
                        .and_then(|b| b["t"].as_i64());
                }
                let Some(hub) = hub.upgrade() else { return };
                hub.send(&req, &series);
                dirty = false;
                next = Instant::now() + min_interval;
            }
        }
    }
}

/// Does `bar` open later than the bar after the window's last one?
fn skips_a_bar(window: &[Bar], bar: &Bar, grid: &Interval) -> bool {
    let ts = |b: &Bar| b.date.parse::<i64>().ok();
    match (window.last().and_then(ts), ts(bar)) {
        (Some(last), Some(t)) => t > grid.grid_end(last),
        _ => false,
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
        // No stream of their own: built from the coarsest that divides them.
        assert_eq!(BinanceSource.feed_interval("BTCUSDT", "2m").unwrap(), "1m");
        assert_eq!(BinanceSource.feed_interval("BTCUSDT", "7m").unwrap(), "1m");
        assert_eq!(
            BinanceSource.feed_interval("BTCUSDT", "45m").unwrap(),
            "15m"
        );
        assert_eq!(BinanceSource.feed_interval("BTCUSDT", "2wk").unwrap(), "1w");
        assert_eq!(SampleSource.feed_interval("BTCUSDT", "7m").unwrap(), "1m");
        assert_eq!(SampleSource.feed_interval("BTCUSDT", "45m").unwrap(), "15m");
        assert_eq!(SampleSource.feed_interval("BTCUSDT", "3mo").unwrap(), "3mo");
        assert!(BinanceSource.feed_interval("BTCUSDT", "61m").is_err());
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
        assert_eq!(push_interval(base, TAIL_BARS + 300), base);
        assert_eq!(push_interval(base, TAIL_BARS + 301), base * 2);
        // A server-raised warmup (ADR-0032) keeps the client's cadence.
        let raised = ChartRequest {
            warmup: 891,
            asked_warmup: 300,
            ..req("BTCUSDT", r#"["ema_stack"]"#)
        };
        assert_eq!(raised.push_interval(base, "1m"), base);
        let asked = ChartRequest {
            asked_warmup: 1000,
            ..raised
        };
        assert_eq!(asked.push_interval(base, "1m"), base * 2);
        // A chart built from a finer feed pays for that feed's bars.
        let custom = ChartRequest {
            timeframe: "7m".into(),
            asked_warmup: 300,
            ..req("BTCUSDT", "[]")
        };
        assert_eq!(custom.push_interval(base, "7m"), base);
        assert_eq!(custom.push_interval(base, "1m"), base * 2);
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
        /// The feed behind every chart, when it is not the chart's interval.
        base: Option<&'static str>,
        seeds: AtomicUsize,
        /// Seeds fail while set, as a refused read does.
        refuse_seeds: AtomicBool,
        /// Seeds wait while set, so the feed can overflow behind a slow one.
        hold_seeds: AtomicBool,
        /// How far past its fixed clock REST has got, in seconds.
        rest_ahead: std::sync::atomic::AtomicI64,
    }

    struct Running(Arc<AtomicUsize>);
    impl Drop for Running {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl PushSource for Fake {
        fn feed_interval(&self, symbol: &str, timeframe: &str) -> Result<String, String> {
            binance_only(symbol).map(|_| self.base.unwrap_or(timeframe).to_string())
        }
        fn seed(&self, symbol: &str, timeframe: &str, count: usize) -> Result<OhlcvData, String> {
            assert!(!self.panic_on_seed.load(Ordering::SeqCst), "seed exploded");
            self.seeds.fetch_add(1, Ordering::SeqCst);
            while self.hold_seeds.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            if self.refuse_seeds.load(Ordering::SeqCst) {
                return Err("Binance rate limit: refused".into());
            }
            // 2024-01-01T00:00Z, so the forming bar is fixed.
            let now_ms = 1_704_067_200_000 + self.rest_ahead.load(Ordering::SeqCst) * 1000;
            data::sample_ohlcv(symbol, timeframe, count, None, now_ms)
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
            chart_params: None,
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
    async fn a_price_chart_pushes_a_long_tail_on_the_box_it_resolved_first() {
        let fake = Arc::new(Fake::default());
        let hub = hub(&fake, quick());
        let (tx, mut rx) = stream();
        hub.link_stream("s1", tx);
        let r = ChartRequest {
            chart_style: "line_break".into(),
            chart_params: Some(json!({ "lines": 2 }).to_string()),
            ..req("BTCUSDT", "[]")
        };
        hub.subscribe("tok", "ip", "s1", r.clone()).unwrap();
        let n = next(&mut rx, 2_000).await.expect("seed push");
        let series = &n["params"]["series"];
        assert_eq!(series["chart_style"], "line_break");
        assert_eq!(series["chart_params"], json!({ "lines": 2 }));
        let t: Vec<i64> = series["bars"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["t"].as_i64().unwrap())
            .collect();
        assert!(t.windows(2).all(|w| w[1] > w[0]), "{t:?}");
        assert!(t.len() > 2, "the first tail is long: {t:?}");
        let (newest, oc) = (*t.last().unwrap(), series["chart_params"].clone());

        // The next push needs only what came since the last, and an overlap.
        let now: i64 = 1_704_067_200;
        fake.send(FeedEvent::Tick(bar(now, 777.0))).await;
        let n = next(&mut rx, 2_000).await.expect("next push");
        let series = &n["params"]["series"];
        let bars = series["bars"].as_array().unwrap();
        assert!(bars.len() <= 3, "{} bars", bars.len());
        assert!(bars[0]["t"].as_i64().unwrap() <= newest);
        assert_eq!(bars.last().unwrap()["c"], 777.0);
        assert_eq!(series["chart_params"], oc);
    }

    #[tokio::test]
    async fn a_custom_interval_folds_its_feed_onto_the_grid_of_a_reload() {
        let fake = Arc::new(Fake {
            base: Some("1m"),
            ..Fake::default()
        });
        let hub = hub(&fake, quick());
        let (tx, mut rx) = stream();
        hub.link_stream("s1", tx);
        let r = ChartRequest {
            timeframe: "7m".into(),
            ..req("BTCUSDT", "[]")
        };
        hub.subscribe("tok", "ip", "s1", r).unwrap();

        // The seed's 1m bars arrive as 7m bars on the 7m grid. Fake's clock
        // stands at 2024-01-01T00:00Z.
        let now: i64 = 1_704_067_200;
        let iv = Interval::parse("7m").unwrap();
        let open = iv.grid_start(now);
        let n = next(&mut rx, 2_000).await.expect("seed push");
        let series = &n["params"]["series"];
        assert_eq!(series["interval"], "7m");
        let bars = series["bars"].as_array().unwrap();
        assert_eq!(bars.len(), TAIL_BARS);
        assert_eq!(bars[0]["t"], open - 420);
        assert_eq!(bars[1]["t"], open);
        // Fake's clock stands on the bar's last minute (now + 60 is on the grid).
        assert_eq!(open + 420, now + 60);

        // The first minute past the bar opens the next one.
        fake.send(FeedEvent::Tick(bar(now + 60, 123.0))).await;
        let n = next(&mut rx, 2_000).await.expect("rollover push");
        let bars = n["params"]["series"]["bars"].as_array().unwrap().clone();
        assert_eq!(bars[0]["t"], open);
        assert_eq!(bars[1]["t"], now + 60);
        assert_eq!(bars[1]["c"], 123.0);

        // The next minute folds into that bar.
        fake.send(FeedEvent::Tick(bar(now + 120, 124.0))).await;
        let n = next(&mut rx, 2_000).await.expect("tick push");
        let last = &n["params"]["series"]["bars"][1];
        assert_eq!(last["t"], now + 60);
        assert_eq!(last["o"], 123.0);
        assert_eq!(last["c"], 124.0);
        assert_eq!(last["v"].as_f64().unwrap(), 2.0);

        // Its update replaces that minute rather than adding to it.
        let mut update = bar(now + 120, 125.0);
        update.volume = 3.0;
        fake.send(FeedEvent::Tick(update)).await;
        let n = next(&mut rx, 2_000).await.expect("update push");
        let last = &n["params"]["series"]["bars"][1];
        assert_eq!(last["c"], 125.0);
        assert_eq!(last["v"].as_f64().unwrap(), 4.0);
    }

    /// A 7m chart on a 1m feed, past its seed push; Fake's clock is on the bar's
    /// last minute. Returns the hub too, which owns the group.
    async fn custom_chart(
        fake: &Arc<Fake>,
        limits: Limits,
    ) -> (Arc<ChartHub>, mpsc::Receiver<String>, i64) {
        let hub = hub(fake, limits);
        let (tx, mut rx) = stream();
        hub.link_stream("s1", tx);
        let r = ChartRequest {
            timeframe: "7m".into(),
            ..req("BTCUSDT", "[]")
        };
        hub.subscribe("tok", "ip", "s1", r).unwrap();
        next(&mut rx, 2_000).await.expect("seed push");
        assert_eq!(fake.seeds.load(Ordering::SeqCst), 1);
        (hub, rx, 1_704_067_200)
    }

    /// Past any `reread_pause` of the tests below.
    const PAST_PAUSE: Duration = Duration::from_millis(150);

    fn short_pause() -> Limits {
        Limits {
            reread_pause: Duration::from_millis(50),
            ..quick()
        }
    }

    #[tokio::test]
    async fn a_gap_in_the_feed_reads_the_missing_klines_once() {
        let fake = Arc::new(Fake {
            base: Some("1m"),
            ..Fake::default()
        });
        let (_hub, mut rx, now) = custom_chart(&fake, short_pause()).await;

        // The next minute is contiguous: no read.
        fake.send(FeedEvent::Tick(bar(now + 60, 1.0))).await;
        next(&mut rx, 2_000).await.expect("tick push");
        assert_eq!(fake.seeds.load(Ordering::SeqCst), 1);

        // Three minutes later: two klines were lost, so it reads them again, and
        // REST has them.
        fake.rest_ahead.store(240, Ordering::SeqCst);
        fake.send(FeedEvent::Tick(bar(now + 240, 2.0))).await;
        let n = next(&mut rx, 2_000).await.expect("push after the gap");
        assert_eq!(fake.seeds.load(Ordering::SeqCst), 2);
        let last = &n["params"]["series"]["bars"][1];
        assert_eq!(last["c"], 2.0, "the tick that showed the gap is kept");

        // Read and done: well past the pause, a contiguous tick reads nothing.
        tokio::time::sleep(PAST_PAUSE).await;
        fake.send(FeedEvent::Tick(bar(now + 300, 3.0))).await;
        next(&mut rx, 2_000).await.expect("tick push");
        assert_eq!(fake.seeds.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_read_still_behind_the_feed_is_read_again_until_it_catches_up() {
        let fake = Arc::new(Fake {
            base: Some("1m"),
            ..Fake::default()
        });
        let (_hub, mut rx, now) = custom_chart(&fake, short_pause()).await;

        // REST stays where it was, so the read leaves the hole.
        fake.send(FeedEvent::Tick(bar(now + 240, 2.0))).await;
        next(&mut rx, 2_000).await.expect("push after the gap");
        assert_eq!(fake.seeds.load(Ordering::SeqCst), 2);

        // The next tick past the pause reads again, though it follows the last.
        tokio::time::sleep(PAST_PAUSE).await;
        fake.send(FeedEvent::Tick(bar(now + 300, 3.0))).await;
        next(&mut rx, 2_000).await.expect("tick push");
        assert_eq!(fake.seeds.load(Ordering::SeqCst), 3);

        // Caught up: one more read, and then none.
        fake.rest_ahead.store(360, Ordering::SeqCst);
        tokio::time::sleep(PAST_PAUSE).await;
        fake.send(FeedEvent::Tick(bar(now + 360, 4.0))).await;
        next(&mut rx, 2_000).await.expect("tick push");
        assert_eq!(fake.seeds.load(Ordering::SeqCst), 4);
        tokio::time::sleep(PAST_PAUSE).await;
        fake.send(FeedEvent::Tick(bar(now + 420, 5.0))).await;
        next(&mut rx, 2_000).await.expect("tick push");
        assert_eq!(fake.seeds.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn a_lagging_group_reads_again_and_keeps_the_newest_close() {
        // The group cannot read the feed during a slow seed, so a burst overflows
        // its buffer and it lags past events it will never see.
        let fake = Arc::new(Fake::default());
        fake.hold_seeds.store(true, Ordering::SeqCst);
        let hub = hub(&fake, short_pause());
        let (tx, mut rx) = stream();
        hub.link_stream("s1", tx);
        hub.subscribe("tok", "ip", "s1", req("BTCUSDT", "[]"))
            .unwrap();
        settle().await;
        assert_eq!(fake.seeds.load(Ordering::SeqCst), 1);

        let now = 1_704_067_200;
        for i in 0..FEED_BUFFER + 50 {
            fake.send(FeedEvent::Tick(bar(now, i as f64))).await;
        }
        fake.send(FeedEvent::Tick(bar(now, 9999.0))).await;
        settle().await;
        fake.hold_seeds.store(false, Ordering::SeqCst);

        // Pushes settle on the newest close, after one more read.
        let mut close = Value::Null;
        for _ in 0..50 {
            let n = next(&mut rx, 2_000).await.expect("push");
            close = n["params"]["series"]["bars"][1]["c"].clone();
            if close == 9999.0 {
                break;
            }
        }
        assert_eq!(close, 9999.0);
        assert_eq!(fake.seeds.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_refused_read_after_a_gap_keeps_the_chart_alive_and_retries() {
        let fake = Arc::new(Fake {
            base: Some("1m"),
            ..Fake::default()
        });
        let (hub, mut rx, now) = custom_chart(&fake, short_pause()).await;
        fake.refuse_seeds.store(true, Ordering::SeqCst);

        fake.send(FeedEvent::Tick(bar(now + 240, 2.0))).await;
        let n = next(&mut rx, 2_000).await.expect("still pushing");
        assert_eq!(n["params"]["series"]["bars"][1]["c"], 2.0);
        assert_eq!(fake.seeds.load(Ordering::SeqCst), 2);
        assert_eq!(hub.counts().1, 1, "the group is still there");

        // Once allowed, the next tick past the pause reads again, though this tick
        // itself is contiguous: the klines are still missing.
        fake.refuse_seeds.store(false, Ordering::SeqCst);
        tokio::time::sleep(PAST_PAUSE).await;
        fake.send(FeedEvent::Tick(bar(now + 240, 3.0))).await;
        next(&mut rx, 2_000).await.expect("tick push");
        assert_eq!(fake.seeds.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn missing_klines_are_not_asked_for_on_every_tick() {
        // The default pause: seconds, against ticks many times a second.
        let fake = Arc::new(Fake {
            base: Some("1m"),
            refuse_seeds: AtomicBool::new(false),
            ..Fake::default()
        });
        let (_hub, mut rx, now) = custom_chart(&fake, quick()).await;
        fake.refuse_seeds.store(true, Ordering::SeqCst);
        for i in 0..10 {
            fake.send(FeedEvent::Tick(bar(now + 240, f64::from(i))))
                .await;
        }
        next(&mut rx, 2_000).await.expect("still pushing");
        assert_eq!(fake.seeds.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_bar_skips_only_if_one_is_missing_before_it() {
        let grid = Interval::parse("1m").unwrap();
        let window = [bar(600, 1.0)];
        assert!(!skips_a_bar(&window, &bar(600, 2.0), &grid), "the same bar");
        assert!(!skips_a_bar(&window, &bar(540, 2.0), &grid), "a late one");
        assert!(!skips_a_bar(&window, &bar(660, 2.0), &grid), "the next one");
        assert!(skips_a_bar(&window, &bar(720, 2.0), &grid));
        assert!(!skips_a_bar(&[], &bar(720, 2.0), &grid));
        // Months have calendar lengths.
        let month = Interval::parse("1mo").unwrap();
        let ts = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp();
        let jan = [bar(ts("2026-01-01T00:00:00Z"), 1.0)];
        assert!(!skips_a_bar(
            &jan,
            &bar(ts("2026-02-01T00:00:00Z"), 1.0),
            &month
        ));
        assert!(skips_a_bar(
            &jan,
            &bar(ts("2026-03-01T00:00:00Z"), 1.0),
            &month
        ));
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
    async fn new_feeds_are_limited_per_minute_and_a_running_one_is_free() {
        let fake = Arc::new(Fake::default());
        let hub = hub(
            &fake,
            Limits {
                new_feeds_per_minute: 2,
                feed_linger: Duration::from_millis(20),
                ..Limits::default()
            },
        );
        let (tx, _rx) = stream();
        hub.link_stream("s1", tx);
        let a = hub
            .subscribe("t1", "i1", "s1", req("BTCUSDT", "[]"))
            .unwrap();
        hub.subscribe("t2", "i2", "s1", req("ETHUSDT", "[]"))
            .unwrap();
        let e = hub
            .subscribe("t3", "i3", "s1", req("SOLUSDT", "[]"))
            .unwrap_err();
        assert!(e.starts_with(FEEDS_BUSY), "{e}");
        assert!(!e.contains(PUSH_LIMIT), "a client would stop retrying: {e}");
        assert_eq!(fake.started.load(Ordering::SeqCst), 2);
        // A chart on a feed that runs starts nothing.
        hub.subscribe("t3", "i3", "s1", req("ETHUSDT", r#"["rsi"]"#))
            .unwrap();
        // Closing a feed does not give its start back: churn is what this limits.
        hub.unsubscribe("t1", &a);
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(hub.counts().2, 1);
        let e = hub
            .subscribe("t1", "i1", "s1", req("BTCUSDT", "[]"))
            .unwrap_err();
        assert!(e.starts_with(FEEDS_BUSY), "{e}");
    }

    #[tokio::test]
    async fn lingering_feeds_count_toward_the_total() {
        let fake = Arc::new(Fake::default());
        let hub = hub(
            &fake,
            Limits {
                feeds_total: 1,
                feed_linger: Duration::from_secs(30),
                ..Limits::default()
            },
        );
        let (tx, _rx) = stream();
        hub.link_stream("s1", tx);
        let a = hub
            .subscribe("t1", "i1", "s1", req("BTCUSDT", "[]"))
            .unwrap();
        hub.unsubscribe("t1", &a);
        assert_eq!(hub.counts(), (0, 0, 1), "lingering");
        let e = hub
            .subscribe("t1", "i1", "s1", req("ETHUSDT", "[]"))
            .unwrap_err();
        assert!(e.starts_with(FEEDS_BUSY), "{e}");
        assert!(e.contains("maximum of 1 feeds"), "{e}");
        // The lingering one itself can be taken again.
        hub.subscribe("t1", "i1", "s1", req("BTCUSDT", "[]"))
            .unwrap();
    }

    #[cfg(feature = "scripting")]
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
        let (mut events, lease) = hub.watch_feed(&key).unwrap();
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

    #[test]
    fn default_cap_per_client_leaves_room_for_grids_behind_one_address() {
        let limits = Limits::default();
        assert_eq!(limits.per_client, 32);
        assert_eq!(limits.total, 64);
        // ADR-0043: a token holds a 4-chart grid, and a proxy's clients share an address.
        assert_eq!(limits.per_client / limits.per_token, 8);
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
