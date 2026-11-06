//! Alerts that run on the server, with no browser open (ADR-0042).
//!
//! Off unless `CHARTGEN_SERVER_ALERTS=1`, and registration being open, webhooks need
//! `CHARTGEN_SERVER_ALERT_CLIENTS` too (`Access`). An alert watches one Binance
//! symbol's price and belongs to the OAuth client that created it. Prices come from the
//! chart hub's feeds, one WebSocket per symbol and interval shared with every
//! chart of it, so alerts add no REST polling: only the one fetch that checks a
//! symbol new to the server, on the path `generate_chart` uses.
//!
//! The fire rules are the browser's (`web/src/lib/alerts.ts`); both run
//! `tests/fixtures/alert_semantics.json`. A fire is written before anything is
//! sent, together with its delivery. So a restart neither fires it again (`once`
//! is off, `every` greater/less is disarmed, a crossing has no previous reading)
//! nor loses the delivery, which resumes under the same event id.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use chartgen::data;
use chartgen::feed::FeedEvent;
use chartgen::scripting::persistence::{self, Store, StoredEvent};
use chartgen::webhook::{self, Policy, Target, Timing};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, Semaphore};
use tokio::task::JoinHandle;

use crate::chart_push::{ChartHub, FeedKey, FeedLease};
use crate::fetch;
use crate::mcp::{normalize_quote_symbol, tool_err, tool_ok};

const MAX_MESSAGE: usize = 200;
/// Prices are the same on every kline stream, so live alerts share this one.
const LIVE_FEED_INTERVAL: &str = "1m";
/// Pause before a feed that ended is joined again.
const FEED_RETRY: Duration = Duration::from_secs(30);
/// No price for this long and the alert shows as waiting.
const FEED_QUIET: Duration = Duration::from_secs(120);
/// A delivery still pending after this long, across a restart, is dropped.
const EVENT_MAX_AGE_SECS: i64 = 3_600;
const MINUTE: Duration = Duration::from_secs(60);
/// How often waiting deliveries are looked at. The budget is per tick, so the total
/// per minute is spread out instead of being spent by whoever is first.
const DISPATCH_EVERY: Duration = Duration::from_millis(500);
const DISPATCHES_PER_MINUTE: usize = 120;
/// How often expired alerts are collected and feeds refused earlier are tried again.
const UPKEEP_EVERY: Duration = Duration::from_secs(60);
/// A fired `once` alert is deleted this long after it fired.
const SPENT_TTL_MS: i64 = 7 * 86_400 * 1000;
/// An alert is deleted once its owner has had no token for this long, unless the
/// operator listed that client: alerts can outlive the people who made them.
const IDLE_OWNER_TTL_MS: i64 = 30 * 86_400 * 1000;

/// `CHARTGEN_SERVER_ALERTS=1` turns server alerts on.
pub fn enabled() -> bool {
    std::env::var("CHARTGEN_SERVER_ALERTS").is_ok_and(|v| v == "1")
}

/// Who may do what (`CHARTGEN_SERVER_ALERT_CLIENTS`). Client ids are free, so no
/// per-client limit can stand in for this.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    /// Unset: anyone may create alerts, nobody gets webhooks. An alert in a panel
    /// sends nothing anywhere; a webhook is a request made for a stranger.
    NoWebhooks,
    /// Only these clients may create alerts, and they get webhooks.
    Clients(HashSet<String>),
    /// `*`: anyone, with webhooks.
    Open,
}

impl Access {
    fn parse(raw: Option<&str>) -> Self {
        let ids: HashSet<String> = raw
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect();
        if ids.is_empty() {
            Self::NoWebhooks
        } else if ids.contains("*") {
            Self::Open
        } else {
            Self::Clients(ids)
        }
    }

    fn may_create(&self, owner: &str) -> bool {
        match self {
            Self::Clients(ids) => ids.contains(owner),
            _ => true,
        }
    }

    fn may_use_webhooks(&self, owner: &str) -> bool {
        match self {
            Self::Clients(ids) => ids.contains(owner),
            Self::Open => true,
            Self::NoWebhooks => false,
        }
    }

    fn keeps_idle_alerts(&self, owner: &str) -> bool {
        matches!(self, Self::Clients(ids) if ids.contains(owner))
    }
}

/// Registration is open (ADR-0036), so these assume a stranger.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub alerts_per_client: usize,
    /// Alerts not yet spent.
    pub alerts_total: usize,
    /// Rows, spent ones included, until they expire.
    pub rows_total: usize,
    /// Symbol and interval pairs watched: each is a WebSocket to Binance.
    pub feeds: usize,
    pub pairs_per_client: usize,
    /// `create_server_alert` and `update_server_alert` calls per client.
    pub changes_per_minute: usize,
    /// Fires of one `every` alert; a `once` alert cannot repeat.
    pub fires_per_alert_per_minute: usize,
    pub webhooks_per_client_per_minute: usize,
    pub webhooks_total_per_minute: usize,
    pub pending_deliveries: usize,
    /// So that a few clients cannot fill `pending_deliveries`.
    pub pending_deliveries_per_client: usize,
    pub deliveries_in_flight: usize,
    pub deliveries_in_flight_per_client: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            alerts_per_client: 20,
            alerts_total: 200,
            rows_total: 1000,
            feeds: 16,
            pairs_per_client: 2,
            changes_per_minute: 10,
            fires_per_alert_per_minute: 6,
            webhooks_per_client_per_minute: 30,
            webhooks_total_per_minute: 120,
            pending_deliveries: 2000,
            pending_deliveries_per_client: 50,
            deliveries_in_flight: 16,
            deliveries_in_flight_per_client: 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    Crossing,
    CrossingUp,
    CrossingDown,
    Greater,
    Less,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Once,
    Every,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluateOn {
    Live,
    Close,
}

fn crossed(condition: Condition, prev: f64, d: f64) -> bool {
    match condition {
        Condition::CrossingUp => prev < 0.0 && d >= 0.0,
        Condition::CrossingDown => prev > 0.0 && d <= 0.0,
        _ => (prev < 0.0 && d >= 0.0) || (prev > 0.0 && d <= 0.0),
    }
}

/// `fires` of `web/src/lib/alerts.ts`, for one level.
fn fires(condition: Condition, prev: Option<f64>, d: f64) -> bool {
    match condition {
        Condition::Greater => d > 0.0,
        Condition::Less => d < 0.0,
        _ => prev.is_some_and(|p| crossed(condition, p, d)),
    }
}

/// `rearmedReading` of `web/src/lib/alerts.ts`, for one level.
fn rearmed(condition: Condition, d: f64) -> bool {
    match condition {
        Condition::Greater => d <= 0.0,
        Condition::Less => d >= 0.0,
        _ => true,
    }
}

fn needs_rearm(condition: Condition) -> bool {
    matches!(condition, Condition::Greater | Condition::Less)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Armed {
    active: bool,
    disarmed: bool,
}

/// The fire rules of one alert on one price, as the loop in `processPrice`
/// (`web/src/stores/alerts.ts`): the state after `price`, and whether it fired.
/// `allow_fire` false holds a fire back, as if the condition had not been met.
fn step(
    condition: Condition,
    trigger: Trigger,
    level: f64,
    state: Armed,
    prev_price: Option<f64>,
    price: f64,
    allow_fire: bool,
) -> (Armed, bool) {
    let d = price - level;
    let mut next = state;
    if next.disarmed {
        if !rearmed(condition, d) {
            return (next, false);
        }
        next.disarmed = false;
    }
    if !allow_fire || !fires(condition, prev_price.map(|p| p - level), d) {
        return (next, false);
    }
    next.active = trigger == Trigger::Every;
    next.disarmed = trigger == Trigger::Every && needs_rearm(condition);
    (next, true)
}

/// Not `Debug`: nothing can print the URL or the secret by accident.
#[derive(Clone, Serialize, Deserialize)]
struct Hook {
    url: String,
    #[serde(default)]
    secret: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
struct Delivery {
    at: i64,
    status: Option<u16>,
    error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    id: String,
    owner: String,
    symbol: String,
    interval: String,
    evaluate_on: EvaluateOn,
    feed_interval: String,
    condition: Condition,
    level: f64,
    trigger: Trigger,
    #[serde(default)]
    message: Option<String>,
    active: bool,
    #[serde(default)]
    disarmed: bool,
    created_at: i64,
    #[serde(default)]
    last_triggered_at: Option<i64>,
    #[serde(default)]
    fire_count: u64,
    #[serde(default)]
    webhook: Option<Hook>,
    #[serde(default)]
    last_delivery: Option<Delivery>,
}

impl Record {
    fn feed(&self) -> FeedKey {
        (self.symbol.clone(), self.feed_interval.clone())
    }

    fn json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

struct Watch {
    task: JoinHandle<()>,
    _lease: FeedLease,
    /// Open time of the newest bar seen, and of the newest closed one (or just
    /// before the first bar seen, so that an older close reads as a replay).
    last_bar: Option<i64>,
    last_closed: Option<i64>,
    last_price: Option<f64>,
    last_at: Option<Instant>,
}

/// Calls in the last minute, per key.
#[derive(Default)]
struct Window(HashMap<String, VecDeque<Instant>>);

impl Window {
    fn has_room(&mut self, key: &str, max: usize, now: Instant) -> bool {
        self.0.retain(|_, calls| {
            while calls.front().is_some_and(|t| now - *t >= MINUTE) {
                calls.pop_front();
            }
            !calls.is_empty()
        });
        self.0.get(key).map_or(0, VecDeque::len) < max
    }

    fn record(&mut self, key: &str, now: Instant) {
        self.0.entry(key.to_string()).or_default().push_back(now);
    }

    fn admit(&mut self, key: &str, max: usize, now: Instant) -> bool {
        let admitted = self.has_room(key, max, now);
        if admitted {
            self.record(key, now);
        }
        admitted
    }
}

#[derive(Default)]
struct State {
    alerts: HashMap<String, Record>,
    watches: HashMap<FeedKey, Watch>,
    /// The previous price each alert read; lost on a restart, on purpose.
    baselines: HashMap<String, f64>,
    changes: Window,
    fires: Window,
    webhooks: Window,
    /// Fires an `every` alert was refused in the last minute.
    held_back: HashMap<String, Instant>,
    /// Deliveries waiting for their turn, per client.
    queues: HashMap<String, VecDeque<StoredEvent>>,
    /// When each client last had a turn: the longest waiting goes first, so a client
    /// with a backlog cannot keep another's single delivery waiting.
    served: HashMap<String, Instant>,
    pending: usize,
    client_slots: HashMap<String, Arc<Semaphore>>,
}

pub struct ServerAlerts {
    store: &'static Store,
    hub: Arc<ChartHub>,
    policy: Policy,
    limits: Limits,
    timing: Timing,
    runtime: tokio::runtime::Handle,
    state: Mutex<State>,
    access: Access,
    slots: Arc<Semaphore>,
    me: Weak<ServerAlerts>,
}

static GLOBAL: OnceLock<Arc<ServerAlerts>> = OnceLock::new();

/// The running server alerts, when on.
pub fn global() -> Option<&'static Arc<ServerAlerts>> {
    GLOBAL.get()
}

/// Start server alerts if they are on and the store opened. Inside the runtime.
pub fn start(hub: &Arc<ChartHub>) {
    if !enabled() {
        return;
    }
    let Some(store) = persistence::global() else {
        eprintln!("[alerts] the store is unavailable; server alerts stay off");
        return;
    };
    let policy = match Policy::from_env() {
        Ok(policy) => policy,
        Err(e) => {
            eprintln!("[alerts] {e}; server alerts stay off");
            return;
        }
    };
    let access = Access::parse(
        std::env::var("CHARTGEN_SERVER_ALERT_CLIENTS")
            .ok()
            .as_deref(),
    );
    let mut limits = Limits::default();
    if let Some(pairs) = std::env::var("CHARTGEN_SERVER_ALERT_PAIRS_PER_CLIENT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
    {
        limits.pairs_per_client = pairs;
    }
    let who = match &access {
        Access::NoWebhooks => "no webhooks: CHARTGEN_SERVER_ALERT_CLIENTS is unset".to_string(),
        Access::Clients(ids) => format!("{} allowed clients or accounts", ids.len()),
        Access::Open => "open to every client, with webhooks".to_string(),
    };
    let alerts = ServerAlerts::new(
        store,
        hub.clone(),
        policy,
        access,
        limits,
        Timing::default(),
    );
    println!(
        "Server alerts enabled ({} restored; {who})",
        alerts.lock().alerts.len()
    );
    let _ = GLOBAL.set(alerts);
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl ServerAlerts {
    /// Load the stored alerts, join their feeds, resume unfinished deliveries.
    fn new(
        store: &'static Store,
        hub: Arc<ChartHub>,
        policy: Policy,
        access: Access,
        limits: Limits,
        timing: Timing,
    ) -> Arc<Self> {
        let alerts = Arc::new_cyclic(|me| Self {
            store,
            hub,
            policy,
            limits,
            timing,
            runtime: tokio::runtime::Handle::current(),
            state: Mutex::default(),
            access,
            slots: Arc::new(Semaphore::new(limits.deliveries_in_flight)),
            me: me.clone(),
        });
        alerts.restore();
        alerts.runtime.spawn(run_upkeep(alerts.me.clone()));
        alerts
    }

    fn restore(&self) {
        let mut st = self.lock();
        match self.store.server_alerts() {
            Ok(rows) => {
                for row in rows {
                    match serde_json::from_str::<Record>(&row) {
                        Ok(r) => {
                            st.alerts.insert(r.id.clone(), r);
                        }
                        // Not the error: it can quote the row, which holds a secret.
                        Err(_) => eprintln!("[alerts] skipping an unreadable alert"),
                    }
                }
            }
            Err(e) => eprintln!("[alerts] cannot read stored alerts: {e}"),
        }
        self.sweep(&mut st, now_ms());
        let events = self.store.server_alert_events().unwrap_or_else(|e| {
            eprintln!("[alerts] cannot read pending deliveries: {e}");
            Vec::new()
        });
        let oldest = chrono::Utc::now().timestamp() - EVENT_MAX_AGE_SECS;
        for event in events {
            let owner = st.alerts.get(&event.alert_id).map(|a| a.owner.clone());
            // The list is read at startup, so this is where a removed client's
            // deliveries stop, and what it queued goes with them.
            let may_send =
                |owner: &String| event.created_at >= oldest && self.access.may_use_webhooks(owner);
            match owner.filter(may_send) {
                Some(owner) => Self::enqueue(&mut st, owner, event),
                None => {
                    let _ = self.store.delete_server_alert_event(&event.id);
                }
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A client the operator took off the list does not run, from the restart that
    /// read the new list.
    fn runs(&self, a: &Record) -> bool {
        a.active && self.access.may_create(&a.owner)
    }

    fn wanted_feeds(&self, st: &State) -> HashSet<FeedKey> {
        st.alerts
            .values()
            .filter(|a| self.runs(a))
            .map(Record::feed)
            .collect()
    }

    /// Join the feeds active alerts need and leave the others.
    fn sync_feeds(&self, st: &mut State) {
        let wanted = self.wanted_feeds(st);
        st.watches.retain(|key, w| {
            let keep = wanted.contains(key);
            if !keep {
                w.task.abort();
            }
            keep
        });
        for key in wanted {
            if st.watches.contains_key(&key) {
                continue;
            }
            // Refused now: the upkeep tries again, and the alert reads as waiting.
            let (rx, lease) = match self.hub.watch_feed(&key) {
                Ok(joined) => joined,
                Err(e) => {
                    eprintln!("[alerts] feed {} {}: {e}", key.0, key.1);
                    continue;
                }
            };
            let task = self
                .runtime
                .spawn(run_watch(self.me.clone(), key.clone(), rx));
            st.watches.insert(
                key,
                Watch {
                    task,
                    _lease: lease,
                    last_bar: None,
                    last_closed: None,
                    last_price: None,
                    last_at: None,
                },
            );
        }
    }

    fn forget_baselines(st: &mut State, key: &FeedKey) {
        let ids: Vec<String> = st
            .alerts
            .values()
            .filter(|a| a.feed() == *key)
            .map(|a| a.id.clone())
            .collect();
        for id in ids {
            st.baselines.remove(&id);
        }
    }

    /// A feed event: evaluate the alerts on this feed. A feed with trades rolls
    /// the forming candle on the first trade past the boundary, so the closing
    /// kline usually arrives after the next candle's first tick. So an older bar
    /// is a late close unless it closed before, or before this watch began: those
    /// are a replay (the backfill after a reconnect), and the baselines start over,
    /// as the browser's do after a gap. An older tick is simply stale.
    fn on_event(&self, key: &FeedKey, event: FeedEvent) {
        let (is_close, bar) = match event {
            FeedEvent::Bar(bar) => (true, bar),
            FeedEvent::Tick(bar) => (false, bar),
        };
        let price = bar.close;
        let Ok(bar_time) = bar.date.parse::<i64>() else {
            return;
        };
        if !price.is_finite() || price <= 0.0 {
            return;
        }
        let now = Instant::now();
        let mut guard = self.lock();
        let st = &mut *guard;
        let Some(watch) = st.watches.get_mut(key) else {
            return;
        };
        if watch.last_bar.is_none() {
            watch.last_closed = Some(bar_time - 1);
        }
        if is_close && watch.last_closed.is_some_and(|c| bar_time <= c) {
            Self::forget_baselines(st, key);
            return;
        }
        let late = watch.last_bar.is_some_and(|last| bar_time < last);
        if !late {
            watch.last_bar = Some(bar_time);
            watch.last_price = Some(price);
            watch.last_at = Some(now);
        }
        if is_close {
            watch.last_closed = Some(bar_time);
        }

        let ids: Vec<String> = st
            .alerts
            .values()
            .filter(|a| {
                self.runs(a)
                    && a.feed() == *key
                    && match a.evaluate_on {
                        EvaluateOn::Live => !late,
                        EvaluateOn::Close => is_close,
                    }
            })
            .map(|a| a.id.clone())
            .collect();
        for id in ids {
            self.evaluate(st, &id, price, bar_time, now);
        }
    }

    fn evaluate(&self, st: &mut State, id: &str, price: f64, bar_time: i64, now: Instant) {
        let Some(alert) = st.alerts.get(id) else {
            return;
        };
        let prev = st.baselines.insert(id.to_string(), price);
        let armed = Armed {
            active: alert.active,
            disarmed: alert.disarmed,
        };
        let (condition, trigger, level) = (alert.condition, alert.trigger, alert.level);
        let (next, fired) = step(condition, trigger, level, armed, prev, price, true);
        let (next, fired) = if fired
            && trigger == Trigger::Every
            && !st
                .fires
                .admit(id, self.limits.fires_per_alert_per_minute, now)
        {
            st.held_back.insert(id.to_string(), now);
            step(condition, trigger, level, armed, prev, price, false)
        } else {
            (next, fired)
        };
        if next == armed && !fired {
            return;
        }
        let Some(alert) = st.alerts.get(id) else {
            return;
        };
        let mut updated = alert.clone();
        updated.active = next.active;
        updated.disarmed = next.disarmed;
        if !fired {
            if let Err(e) = self
                .store
                .save_server_alert(id, &updated.owner, &updated.json())
            {
                eprintln!("[alerts] cannot store a re-arm: {e}");
                return;
            }
            st.alerts.insert(id.to_string(), updated);
            return;
        }
        let at = now_ms();
        updated.last_triggered_at = Some(at);
        updated.fire_count += 1;
        let event = self.prepare_delivery(st, &mut updated, price, bar_time, at);
        if let Err(e) =
            self.store
                .record_server_alert_fire(id, &updated.owner, &updated.json(), event.as_ref())
        {
            // Memory stays as the store has it, so a restart cannot disagree.
            eprintln!("[alerts] cannot store a fire: {e}");
            return;
        }
        st.alerts.insert(id.to_string(), updated);
        if let Some(event) = event {
            let owner = st.alerts[id].owner.clone();
            Self::enqueue(st, owner, event);
        }
        if !next.active {
            self.sync_feeds(st);
        }
    }

    fn enqueue(st: &mut State, owner: String, event: StoredEvent) {
        st.pending += 1;
        st.queues.entry(owner).or_default().push_back(event);
    }

    /// The delivery of a fire, if it has a webhook and is allowed to wait for one.
    /// It waits for its turn however busy the budget is, so a `once` alert keeps its
    /// webhook. Only with a full queue does the fire count without one, and the alert
    /// says why.
    fn prepare_delivery(
        &self,
        st: &mut State,
        alert: &mut Record,
        price: f64,
        bar_time: i64,
        at: i64,
    ) -> Option<StoredEvent> {
        alert.webhook.as_ref()?;
        let limits = &self.limits;
        let refusal = if !self.access.may_use_webhooks(&alert.owner) {
            Some("webhook skipped: this client may not use webhooks")
        } else if st.pending >= limits.pending_deliveries
            || st.queues.get(&alert.owner).map_or(0, VecDeque::len)
                >= limits.pending_deliveries_per_client
        {
            Some("webhook skipped: too many deliveries waiting")
        } else {
            None
        };
        if let Some(error) = refusal {
            alert.last_delivery = Some(Delivery {
                at,
                status: None,
                error: Some(error.into()),
            });
            return None;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let body = json!({
            "id": id,
            "type": "alert.fired",
            "alert_id": alert.id,
            "symbol": alert.symbol,
            "interval": alert.interval,
            "evaluate_on": alert.evaluate_on,
            "condition": alert.condition,
            "level": alert.level,
            "price": price,
            "trigger": alert.trigger,
            "message": alert.message,
            "bar_time": bar_time,
            "fired_at": at,
        });
        Some(StoredEvent {
            id,
            alert_id: alert.id.clone(),
            body: body.to_string(),
            created_at: at / 1000,
        })
    }

    /// Start the deliveries whose turn it is: the client that waited longest first,
    /// within its own budget and the total, at an even pace.
    fn dispatch(&self, now: Instant) {
        let mut guard = self.lock();
        let st = &mut *guard;
        let limits = &self.limits;
        let oldest = chrono::Utc::now().timestamp() - EVENT_MAX_AGE_SECS;
        let mut owners: Vec<String> = st
            .queues
            .iter()
            .filter(|(_, q)| !q.is_empty())
            .map(|(owner, _)| owner.clone())
            .collect();
        owners.sort_by_key(|owner| st.served.get(owner).copied());
        let mut left = (limits.webhooks_total_per_minute / DISPATCHES_PER_MINUTE).max(1);
        for owner in owners {
            if left == 0
                || !st
                    .webhooks
                    .has_room("", limits.webhooks_total_per_minute, now)
            {
                break;
            }
            // Waited for an hour: a price that old is no news, as after a restart.
            let queue = st.queues.entry(owner.clone()).or_default();
            while queue.front().is_some_and(|e| e.created_at < oldest) {
                if let Some(stale) = queue.pop_front() {
                    st.pending = st.pending.saturating_sub(1);
                    let _ = self.store.delete_server_alert_event(&stale.id);
                }
            }
            if !st
                .webhooks
                .has_room(&owner, limits.webhooks_per_client_per_minute, now)
            {
                continue;
            }
            // Busy: it stays queued, where its cap and its age limit apply. A started
            // delivery counts toward neither, and a silent receiver drains slowly.
            if self.client_slot(st, &owner).available_permits() == 0 {
                continue;
            }
            let Some(event) = st.queues.get_mut(&owner).and_then(VecDeque::pop_front) else {
                continue;
            };
            st.webhooks.record(&owner, now);
            st.webhooks.record("", now);
            st.served.insert(owner, now);
            left -= 1;
            if let Some(this) = self.me.upgrade() {
                self.runtime.spawn(this.deliver(event));
            }
        }
        st.queues.retain(|_, q| !q.is_empty());
    }

    /// Delete what expired, and try again to join feeds that were refused. A failure
    /// to read the clients keeps their alerts: an unknown owner is not an idle one.
    /// A signed-in owner is a user, whose activity is theirs and not a client's
    /// (ADR-0044).
    fn sweep(&self, st: &mut State, now: i64) {
        let activity = self
            .store
            .oauth_clients()
            .and_then(|clients| Ok((clients, self.store.user_activity()?)));
        let last_tokens: Option<HashMap<String, i64>> = match activity {
            Ok((clients, users)) => Some(
                clients
                    .into_iter()
                    .filter_map(|c| Some((c.client_id, c.last_token_at?)))
                    .chain(users)
                    .collect(),
            ),
            Err(e) => {
                eprintln!("[alerts] cannot read clients: {e}");
                None
            }
        };
        let expired: Vec<String> = st
            .alerts
            .values()
            .filter(|a| {
                let spent =
                    !a.active && a.last_triggered_at.is_some_and(|t| now - t > SPENT_TTL_MS);
                let idle = last_tokens.as_ref().is_some_and(|tokens| {
                    let last = tokens.get(&a.owner).map_or(0, |secs| secs * 1000);
                    !self.access.keeps_idle_alerts(&a.owner)
                        && now - last.max(a.created_at) > IDLE_OWNER_TTL_MS
                });
                spent || idle
            })
            .map(|a| a.id.clone())
            .collect();
        for id in expired {
            if let Err(e) = self.remove(st, &id) {
                eprintln!("[alerts] cannot delete an expired alert: {e}");
            }
        }
        self.sync_feeds(st);
    }

    /// Alerts of a client from before sign-in go to the first user who gets a token
    /// through it, oldest first and up to that user's limit, with their deliveries. The
    /// rest stay with the client, and nothing is deleted (ADR-0044). Returns how many
    /// moved. Blocking.
    pub fn claim(&self, client: &str, user: &str) -> usize {
        let mut guard = self.lock();
        let st = &mut *guard;
        let mine = st.alerts.values().filter(|a| a.owner == user).count();
        let room = self.limits.alerts_per_client.saturating_sub(mine);
        let mut ids: Vec<(i64, String)> = st
            .alerts
            .values()
            .filter(|a| a.owner == client)
            .map(|a| (a.created_at, a.id.clone()))
            .collect();
        ids.sort();
        let mut moved = HashSet::new();
        for (_, id) in ids.into_iter().take(room) {
            let Some(alert) = st.alerts.get_mut(&id) else {
                continue;
            };
            let mut updated = alert.clone();
            updated.owner = user.to_string();
            match self.store.save_server_alert(&id, user, &updated.json()) {
                Ok(()) => {
                    *alert = updated;
                    moved.insert(id);
                }
                Err(e) => eprintln!("[alerts] cannot move an alert to its user: {e}"),
            }
        }
        if let Some(queue) = st.queues.get_mut(client) {
            let (theirs, stays): (VecDeque<_>, VecDeque<_>) = std::mem::take(queue)
                .into_iter()
                .partition(|e| moved.contains(&e.alert_id));
            *queue = stays;
            st.queues
                .entry(user.to_string())
                .or_default()
                .extend(theirs);
        }
        moved.len()
    }

    /// Delete an alert, with what it still had to deliver.
    fn remove(&self, st: &mut State, id: &str) -> rusqlite::Result<()> {
        self.store.delete_server_alert(id)?;
        st.alerts.remove(id);
        st.baselines.remove(id);
        st.held_back.remove(id);
        let mut dropped = 0;
        for queue in st.queues.values_mut() {
            let before = queue.len();
            queue.retain(|e| e.alert_id != id);
            dropped += before - queue.len();
        }
        st.pending = st.pending.saturating_sub(dropped);
        Ok(())
    }

    fn client_slot(&self, st: &mut State, owner: &str) -> Arc<Semaphore> {
        let per_client = self.limits.deliveries_in_flight_per_client;
        st.client_slots
            .entry(owner.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(per_client)))
            .clone()
    }

    /// Send one event, then record how it went and forget it.
    async fn deliver(self: Arc<Self>, event: StoredEvent) {
        let (hook, client) = {
            let mut st = self.lock();
            let alert = st.alerts.get(&event.alert_id);
            let hook = alert.and_then(|a| a.webhook.clone());
            let owner = alert.map(|a| a.owner.clone()).unwrap_or_default();
            (hook, self.client_slot(&mut st, &owner))
        };
        let outcome = match hook {
            // One client's dead host cannot hold every slot.
            Some(hook) => Some(self.send(&hook, &event, &client).await),
            None => None,
        };
        let mut st = self.lock();
        st.pending = st.pending.saturating_sub(1);
        if let Some(outcome) = outcome {
            if let Some(alert) = st.alerts.get_mut(&event.alert_id) {
                alert.last_delivery = Some(outcome);
                if let Err(e) = self
                    .store
                    .save_server_alert(&alert.id, &alert.owner, &alert.json())
                {
                    eprintln!("[alerts] cannot store a delivery: {e}");
                }
            }
        }
        if let Err(e) = self.store.delete_server_alert_event(&event.id) {
            eprintln!("[alerts] cannot clear a delivery: {e}");
        }
    }

    async fn send(&self, hook: &Hook, event: &StoredEvent, client: &Semaphore) -> Delivery {
        let outcome = match Target::parse(&hook.url, &self.policy) {
            Ok(target) => {
                let slots = [client, self.slots.as_ref()];
                webhook::deliver(
                    &target,
                    &self.policy,
                    &event.body,
                    &event.id,
                    hook.secret.as_deref(),
                    &self.timing,
                    &slots,
                )
                .await
            }
            Err(error) => webhook::Outcome {
                status: None,
                error: Some(error),
                attempts: 0,
            },
        };
        Delivery {
            at: now_ms(),
            status: outcome.status,
            error: outcome.error,
        }
    }

    /// Join a feed again after it ended.
    fn rejoin(&self, key: &FeedKey) {
        let mut st = self.lock();
        Self::forget_baselines(&mut st, key);
        st.watches.remove(key);
        self.sync_feeds(&mut st);
    }

    /// Handle a tool call. `owner` is the caller's user, or with sign-in off its OAuth
    /// client (ADR-0044). Blocking.
    pub fn call_tool(
        &self,
        tool: &str,
        id: Option<Value>,
        args: &Value,
        owner: Option<&str>,
    ) -> Value {
        let Some(owner) = owner else {
            return tool_err(id, "Server alerts need an authenticated OAuth client");
        };
        let result = match tool {
            "create_server_alert" => self.create(args, owner),
            "list_server_alerts" => Ok(self.list(owner)),
            "update_server_alert" => self.update(args, owner),
            "delete_server_alert" => self.delete(args, owner),
            _ => Err(format!("Unknown tool: {tool}")),
        };
        match result {
            Ok(payload) => tool_ok(id, payload),
            Err(message) => tool_err(id, &message),
        }
    }

    /// Check a webhook URL now, so a bad one is refused to the person typing it.
    fn check_webhook(&self, raw: &str, owner: &str) -> Result<(), String> {
        if !self.access.may_use_webhooks(owner) {
            return Err("This server only sends webhooks for sign-ins its operator allowed".into());
        }
        let target = Target::parse(raw, &self.policy)?;
        self.runtime
            .block_on(webhook::resolve(&target, &self.policy))
            .map(|_| ())
    }

    fn admit_change(&self, owner: &str) -> Result<(), String> {
        let max = self.limits.changes_per_minute;
        if self.lock().changes.admit(owner, max, Instant::now()) {
            Ok(())
        } else {
            Err(format!(
                "Limit reached: at most {max} server alert changes per minute"
            ))
        }
    }

    fn check_capacity(&self, st: &State, owner: &str, feed: &FeedKey) -> Result<(), String> {
        let limits = &self.limits;
        let mine = st.alerts.values().filter(|a| a.owner == owner).count();
        if mine >= limits.alerts_per_client {
            return Err(format!(
                "Limit reached: at most {} server alerts per client",
                limits.alerts_per_client
            ));
        }
        if st.alerts.len() >= limits.rows_total {
            return Err(self.total_reached());
        }
        self.check_live_total(st)?;
        self.check_feed_room(st, owner, feed)
    }

    fn total_reached(&self) -> String {
        format!(
            "Limit reached: this server holds its maximum of {} server alerts",
            self.limits.alerts_total
        )
    }

    /// Room for one more alert that is not spent, new or brought back?
    fn check_live_total(&self, st: &State) -> Result<(), String> {
        let live = st.alerts.values().filter(|a| !is_spent(a)).count();
        if live >= self.limits.alerts_total {
            return Err(self.total_reached());
        }
        Ok(())
    }

    fn check_feed_room(&self, st: &State, owner: &str, feed: &FeedKey) -> Result<(), String> {
        let wanted = self.wanted_feeds(st);
        if !wanted.contains(feed) && wanted.len() >= self.limits.feeds {
            return Err(format!(
                "Limit reached: this server watches at most {} symbol and interval pairs",
                self.limits.feeds
            ));
        }
        let mine: HashSet<FeedKey> = st
            .alerts
            .values()
            .filter(|a| a.owner == owner && a.active)
            .map(Record::feed)
            .collect();
        if !mine.contains(feed) && mine.len() >= self.limits.pairs_per_client {
            return Err(format!(
                "Limit reached: at most {} symbol and interval pairs per client",
                self.limits.pairs_per_client
            ));
        }
        if !st.watches.contains_key(feed) {
            self.hub.feed_room(feed).map_err(|_| {
                "Limit reached: this server is starting too many feeds; try again in a minute"
                    .to_string()
            })?;
        }
        Ok(())
    }

    fn create(&self, args: &Value, owner: &str) -> Result<Value, String> {
        if !self.access.may_create(owner) {
            return Err("This server only runs alerts for sign-ins its operator allowed".into());
        }
        self.admit_change(owner)?;
        let raw_symbol = args.get("symbol").and_then(Value::as_str).unwrap_or("");
        let symbol = normalize_quote_symbol(raw_symbol).ok_or("symbol is required")?;
        if fetch::detect_source(&symbol) != "binance" {
            return Err(format!(
                "Server alerts run on Binance symbols only; {symbol} stays a browser alert"
            ));
        }
        let interval = match args.get("interval") {
            None | Some(Value::Null) => LIVE_FEED_INTERVAL.to_string(),
            Some(v) => v.as_str().ok_or("interval must be a string")?.to_string(),
        };
        let evaluate_on = enum_arg(args, "evaluate_on")?.unwrap_or(EvaluateOn::Live);
        let condition: Condition = enum_arg(args, "condition")?.ok_or("condition is required")?;
        let trigger: Trigger = enum_arg(args, "trigger")?.ok_or("trigger is required")?;
        let level = level_arg(args)?.ok_or("level is required")?;
        let message = message_arg(args)?.filter(|m| !m.is_empty());
        let (webhook_url, sign) = webhook_args(args)?;
        let webhook_url = webhook_url.filter(|u| !u.is_empty());
        if sign == Some(true) && webhook_url.is_none() {
            return Err("sign needs webhook_url".into());
        }

        let refuse = |_| format!("Server alerts cannot watch {symbol} at {interval}");
        let feed = match evaluate_on {
            // Only a label here: the price is the same on every stream.
            EvaluateOn::Live if is_interval_label(&interval) => self
                .hub
                .feed_key(&symbol, LIVE_FEED_INTERVAL)
                .map_err(refuse)?,
            EvaluateOn::Live => return Err(format!("{interval} is not an interval")),
            EvaluateOn::Close => {
                let feed = self.hub.feed_key(&symbol, &interval).map_err(refuse)?;
                // The bars of the interval itself, not of a finer stream it is built from.
                if data::interval_secs(&feed.1)
                    .is_none_or(|s| data::interval_secs(&interval) != Some(s))
                {
                    return Err(format!(
                        "{interval} has no kline stream, so its closes cannot be watched; use evaluate_on: live"
                    ));
                }
                feed
            }
        };
        let new_feed = {
            let st = self.lock();
            self.check_capacity(&st, owner, &feed)?;
            !st.watches.contains_key(&feed)
        };
        // In this order: a refused URL costs the exchange nothing.
        if let Some(url) = &webhook_url {
            self.check_webhook(url, owner)?;
        }
        if new_feed {
            self.hub
                .check_symbol(&symbol, &feed.1)
                .map_err(|e| format!("Cannot watch {symbol}: {e}"))?;
        }
        let secret = (sign == Some(true)).then(new_secret);
        let record = Record {
            id: uuid::Uuid::new_v4().to_string(),
            owner: owner.to_string(),
            symbol,
            interval,
            evaluate_on,
            feed_interval: feed.1.clone(),
            condition,
            level,
            trigger,
            message,
            active: true,
            disarmed: false,
            created_at: now_ms(),
            last_triggered_at: None,
            fire_count: 0,
            webhook: webhook_url.map(|url| Hook {
                url,
                secret: secret.clone(),
            }),
            last_delivery: None,
        };
        let mut st = self.lock();
        // The fetch and the lookup ran unlocked: another call may have filled up.
        self.check_capacity(&st, owner, &feed)?;
        self.store
            .save_server_alert(&record.id, owner, &record.json())
            .map_err(|e| {
                eprintln!("[alerts] cannot store an alert: {e}");
                "The alert store is unavailable".to_string()
            })?;
        let mut payload = self.view(&st, &record);
        if let Some(secret) = secret {
            payload["webhook_secret"] = json!(secret);
        }
        st.alerts.insert(record.id.clone(), record);
        self.sync_feeds(&mut st);
        Ok(payload)
    }

    fn update(&self, args: &Value, owner: &str) -> Result<Value, String> {
        self.admit_change(owner)?;
        let id = args
            .get("alert_id")
            .and_then(Value::as_str)
            .ok_or("alert_id is required")?;
        // First: a stranger's URL must not cost a lookup, or answer anything about it.
        if !self.lock().alerts.get(id).is_some_and(|a| a.owner == owner) {
            return Err("No such alert".into());
        }
        let condition: Option<Condition> = enum_arg(args, "condition")?;
        let trigger: Option<Trigger> = enum_arg(args, "trigger")?;
        let level = level_arg(args)?;
        let message = message_arg(args)?;
        let active = match args.get("active") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_bool().ok_or("active must be true or false")?),
        };
        let (webhook_url, sign) = webhook_args(args)?;
        if let Some(url) = webhook_url.as_deref().filter(|u| !u.is_empty()) {
            self.check_webhook(url, owner)?;
        }

        let mut st = self.lock();
        let mut next = st
            .alerts
            .get(id)
            .filter(|a| a.owner == owner)
            .cloned()
            .ok_or("No such alert")?;
        let was_spent = is_spent(&next);
        // What decides when it fires: a note or a webhook does not bring a spent alert
        // back, and neither does the dialog sending the same values again.
        let redefined = condition.is_some_and(|c| c != next.condition)
            || trigger.is_some_and(|t| t != next.trigger)
            || level.is_some_and(|l| l != next.level);
        next.condition = condition.unwrap_or(next.condition);
        next.trigger = trigger.unwrap_or(next.trigger);
        next.level = level.unwrap_or(next.level);
        if let Some(m) = message {
            next.message = Some(m).filter(|m| !m.is_empty());
        }
        if redefined {
            next.active = true;
            next.disarmed = false;
            next.last_triggered_at = None;
        }
        match active {
            Some(true) => {
                next.active = true;
                next.disarmed = false;
            }
            Some(false) => {
                next.active = false;
                next.disarmed = false;
                next.last_triggered_at = None;
            }
            None => {}
        }
        match webhook_url {
            Some(url) if url.is_empty() => next.webhook = None,
            Some(url) => {
                // A secret signs for one receiver: a new host does not inherit it.
                let secret = next
                    .webhook
                    .take()
                    .filter(|h| h.url == url)
                    .and_then(|h| h.secret);
                next.webhook = Some(Hook { url, secret });
            }
            None => {}
        }
        let mut new_secret_text = None;
        if let Some(sign) = sign {
            let hook = next.webhook.as_mut().ok_or("sign needs a webhook")?;
            hook.secret = sign.then(new_secret);
            new_secret_text = hook.secret.clone();
        }
        if was_spent && !is_spent(&next) {
            self.check_live_total(&st)?;
        }
        if next.active {
            self.check_feed_room(&st, owner, &next.feed())?;
        }
        self.store
            .save_server_alert(id, owner, &next.json())
            .map_err(|e| {
                eprintln!("[alerts] cannot store an alert: {e}");
                "The alert store is unavailable".to_string()
            })?;
        if redefined || active.is_some() {
            st.baselines.remove(id);
        }
        let mut payload = self.view(&st, &next);
        if let Some(secret) = new_secret_text {
            payload["webhook_secret"] = json!(secret);
        }
        st.alerts.insert(id.to_string(), next);
        self.sync_feeds(&mut st);
        Ok(payload)
    }

    fn delete(&self, args: &Value, owner: &str) -> Result<Value, String> {
        let id = args
            .get("alert_id")
            .and_then(Value::as_str)
            .ok_or("alert_id is required")?;
        let mut st = self.lock();
        if !st.alerts.get(id).is_some_and(|a| a.owner == owner) {
            return Ok(json!({ "deleted": false }));
        }
        self.remove(&mut st, id).map_err(|e| {
            eprintln!("[alerts] cannot delete an alert: {e}");
            "The alert store is unavailable".to_string()
        })?;
        self.sync_feeds(&mut st);
        Ok(json!({ "deleted": true }))
    }

    fn list(&self, owner: &str) -> Value {
        let st = self.lock();
        let mut alerts: Vec<&Record> = st.alerts.values().filter(|a| a.owner == owner).collect();
        alerts.sort_by_key(|a| a.created_at);
        let limits = &self.limits;
        json!({
            "enabled": true,
            "can_create": self.access.may_create(owner),
            "webhooks": self.access.may_use_webhooks(owner),
            "limits": {
                "alerts_per_client": limits.alerts_per_client,
                "pairs_per_client": limits.pairs_per_client,
                "fires_per_alert_per_minute": limits.fires_per_alert_per_minute,
                "webhooks_per_client_per_minute": limits.webhooks_per_client_per_minute,
            },
            "alerts": alerts.into_iter().map(|a| self.view(&st, a)).collect::<Vec<_>>(),
        })
    }

    /// An alert as its owner sees it: never the webhook's URL or secret.
    fn view(&self, st: &State, a: &Record) -> Value {
        let watch = st.watches.get(&a.feed());
        let live = watch
            .and_then(|w| w.last_at)
            .is_some_and(|t| t.elapsed() < FEED_QUIET);
        let held_back = st
            .held_back
            .get(&a.id)
            .is_some_and(|t| t.elapsed() < MINUTE);
        let delivery_error = a.last_delivery.as_ref().and_then(|d| d.error.clone());
        let last_error = if held_back {
            Some(format!(
                "fires limited to {} a minute",
                self.limits.fires_per_alert_per_minute
            ))
        } else {
            delivery_error
        };
        json!({
            "id": a.id,
            "symbol": a.symbol,
            "interval": a.interval,
            "evaluate_on": a.evaluate_on,
            "condition": a.condition,
            "level": a.level,
            "trigger": a.trigger,
            "message": a.message,
            "active": a.active,
            "disarmed": a.disarmed,
            "created_at": a.created_at,
            "last_triggered_at": a.last_triggered_at,
            "fire_count": a.fire_count,
            "webhook": a.webhook.as_ref().map(|h| json!({
                "host": Target::parse(&h.url, &self.policy).map(|t| t.host().to_string()).unwrap_or_default(),
                "signed": h.secret.is_some(),
            })),
            "last_delivery_at": a.last_delivery.as_ref().map(|d| d.at),
            "last_status": a.last_delivery.as_ref().and_then(|d| d.status),
            "last_error": last_error,
            "feed": if live { "live" } else { "waiting" },
            "last_price": watch.and_then(|w| w.last_price),
        })
    }
}

/// A `once` alert that fired: it holds no place among the live ones until re-armed.
fn is_spent(a: &Record) -> bool {
    !a.active && a.last_triggered_at.is_some()
}

/// `7m`, `1h`, `1mo`: digits then letters, short enough to show.
fn is_interval_label(interval: &str) -> bool {
    let letters = interval.trim_start_matches(|c: char| c.is_ascii_digit());
    interval.len() <= 8
        && letters.len() < interval.len()
        && !letters.is_empty()
        && letters.chars().all(|c| c.is_ascii_alphabetic())
}

fn enum_arg<T: serde::de::DeserializeOwned>(args: &Value, name: &str) -> Result<Option<T>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|_| format!("{name} has an unknown value")),
    }
}

fn level_arg(args: &Value) -> Result<Option<f64>, String> {
    match args.get("level") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_f64() {
            Some(l) if l.is_finite() && l > 0.0 => Ok(Some(l)),
            _ => Err("level must be a number greater than 0".into()),
        },
    }
}

fn message_arg(args: &Value) -> Result<Option<String>, String> {
    match args.get("message") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let m = v.as_str().ok_or("message must be a string")?.trim();
            Ok(Some(m.chars().take(MAX_MESSAGE).collect()))
        }
    }
}

fn webhook_args(args: &Value) -> Result<(Option<String>, Option<bool>), String> {
    let url = match args.get("webhook_url") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_str()
                .ok_or("webhook_url must be a string")?
                .trim()
                .to_string(),
        ),
    };
    let sign = match args.get("sign") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_bool().ok_or("sign must be true or false")?),
    };
    Ok((url, sign))
}

fn new_secret() -> String {
    use rand::Rng;
    hex::encode(rand::rng().random::<[u8; 32]>())
}

/// Evaluate one feed until the alerts leave it. When the feed ends it is joined
/// again after a pause, starting a new baseline.
async fn run_watch(
    alerts: Weak<ServerAlerts>,
    key: FeedKey,
    mut events: broadcast::Receiver<FeedEvent>,
) {
    loop {
        let event = events.recv().await;
        let Some(this) = alerts.upgrade() else {
            return;
        };
        match event {
            Ok(event) => this.on_event(&key, event),
            Err(broadcast::error::RecvError::Lagged(_)) => {
                ServerAlerts::forget_baselines(&mut this.lock(), &key);
            }
            Err(broadcast::error::RecvError::Closed) => {
                eprintln!("[alerts] feed {} {} ended", key.0, key.1);
                drop(this);
                tokio::time::sleep(FEED_RETRY).await;
                if let Some(this) = alerts.upgrade() {
                    this.rejoin(&key);
                }
                return;
            }
        }
    }
}

/// Start deliveries on a steady beat, and collect what expired less often.
async fn run_upkeep(alerts: Weak<ServerAlerts>) {
    let mut last_sweep = Instant::now();
    loop {
        tokio::time::sleep(DISPATCH_EVERY).await;
        let Some(this) = alerts.upgrade() else {
            return;
        };
        let now = Instant::now();
        this.dispatch(now);
        if now - last_sweep >= UPKEEP_EVERY {
            last_sweep = now;
            this.sweep(&mut this.lock(), now_ms());
        }
    }
}

#[cfg(test)]
mod tests;
