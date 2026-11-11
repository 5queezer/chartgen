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
//! nor loses the delivery, which resumes under the same event id. A delivery is a
//! webhook or a push to one device (ADR-0045): the same row, queue and budgets.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use chartgen::data;
use chartgen::feed::FeedEvent;
use chartgen::push::{DeviceKeys, Vapid};
use chartgen::scripting::persistence::{self, Store, StoredDevice, StoredEvent};
use chartgen::webhook::{self, Policy, Target, Timing};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
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
    pub(crate) fn parse(raw: Option<&str>) -> Self {
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

    pub(crate) fn may_create(&self, owner: &str) -> bool {
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

    /// Push has no URL of the user's own: it only reaches known push services, so it
    /// goes with being allowed alerts, where a webhook needs more.
    fn may_use_push(&self, owner: &str) -> bool {
        self.may_create(owner)
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
    pub pushes_per_client_per_minute: usize,
    /// Webhooks and pushes together.
    pub deliveries_total_per_minute: usize,
    /// Push devices per client; a new one over this replaces its oldest.
    pub devices_per_client: usize,
    /// When full, a new device replaces someone else's (`device_to_replace`).
    pub devices_total: usize,
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
            pushes_per_client_per_minute: 30,
            deliveries_total_per_minute: 120,
            devices_per_client: 5,
            devices_total: 2000,
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
    /// Notify the owner's devices when it fires.
    #[serde(default)]
    push: bool,
    #[serde(default)]
    last_push: Option<Delivery>,
    /// The devices the last fire went to, with each one's outcome once known. A device
    /// not here was not sent that fire, and each tab reads its own: one user's several
    /// browsers can fare differently (ADR-0044).
    #[serde(default)]
    device_pushes: HashMap<String, Option<Delivery>>,
    /// The fire `device_pushes` is about. An earlier fire's push can finish after a later
    /// fire, and its outcome says nothing about that one.
    #[serde(default)]
    device_pushes_fire: Option<String>,
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
pub(crate) struct Window(HashMap<String, VecDeque<Instant>>);

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

    pub(crate) fn admit(&mut self, key: &str, max: usize, now: Instant) -> bool {
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
    /// Per client, and under "" every delivery of either kind.
    webhooks: Window,
    pushes: Window,
    /// The push devices, by id.
    devices: HashMap<String, StoredDevice>,
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
    /// `None` leaves push off; everything else still runs.
    vapid: Option<Vapid>,
    slots: Arc<Semaphore>,
    /// Setups watch feeds too, and theirs count against `limits.feeds` (ADR-0059).
    setups: OnceLock<Weak<crate::setup_service::Setups>>,
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
    let vapid = load_vapid(store);
    let alerts = ServerAlerts::new(
        store,
        hub.clone(),
        policy,
        access,
        vapid,
        limits,
        Timing::default(),
    );
    println!(
        "Server alerts enabled ({} restored; {who})",
        alerts.lock().alerts.len()
    );
    let _ = GLOBAL.set(alerts);
}

/// The application server key, from `CHARTGEN_VAPID_PRIVATE_KEY` or made once and kept
/// in the store: a browser's subscription is tied to it, so it must outlive restarts.
/// Push services want a way to reach the operator, so without one push stays off.
fn load_vapid(store: &Store) -> Option<Vapid> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
    let subject = var("CHARTGEN_VAPID_SUBJECT")
        .or_else(|| var("CHARTGEN_BASE_URL").filter(|u| u.starts_with("https://")));
    let Some(subject) = subject else {
        eprintln!("[alerts] push is off: set CHARTGEN_VAPID_SUBJECT or an https CHARTGEN_BASE_URL");
        return None;
    };
    let secret = match var("CHARTGEN_VAPID_PRIVATE_KEY") {
        Some(secret) => Ok(secret),
        None => Vapid::new_secret().and_then(|new| {
            store
                .setting_or_insert("vapid_private_key", &new)
                .map_err(|e| format!("cannot store the key: {e}"))
        }),
    };
    match secret.and_then(|secret| Vapid::from_secret(&secret, &subject)) {
        Ok(vapid) => Some(vapid),
        Err(e) => {
            eprintln!("[alerts] push is off: {e}");
            None
        }
    }
}

/// Whether a device has these keys, as one subscription would, however they are padded.
/// `auth` is a secret, so this does not compare it byte by byte.
fn same_keys(device: &StoredDevice, p256dh: &str, auth: &str) -> bool {
    let print = |text: &str| Sha256::digest(text.trim_end_matches('='));
    print(&device.p256dh) == print(p256dh) && print(&device.auth) == print(auth)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl ServerAlerts {
    /// Load the stored alerts, join their feeds, resume unfinished deliveries.
    pub(crate) fn new(
        store: &'static Store,
        hub: Arc<ChartHub>,
        policy: Policy,
        access: Access,
        vapid: Option<Vapid>,
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
            vapid,
            slots: Arc::new(Semaphore::new(limits.deliveries_in_flight)),
            setups: OnceLock::new(),
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
        match self.store.push_devices() {
            Ok(devices) => st.devices = devices.into_iter().map(|d| (d.id.clone(), d)).collect(),
            Err(e) => eprintln!("[alerts] cannot read push devices: {e}"),
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
            let may_send = |owner: &String| {
                event.created_at >= oldest
                    && match &event.device {
                        None => self.access.may_use_webhooks(owner),
                        Some(device) => {
                            self.access.may_use_push(owner) && st.devices.contains_key(device)
                        }
                    }
            };
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

    /// The feeds active alerts hold, for the limit setups share (ADR-0059).
    pub fn pairs(&self) -> HashSet<FeedKey> {
        self.wanted_feeds(&self.lock())
    }

    /// Count the feeds `setups` hold against the server's limit on watched pairs.
    pub fn count_pairs_of(&self, setups: &Arc<crate::setup_service::Setups>) {
        let _ = self.setups.set(Arc::downgrade(setups));
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
        let events = self.prepare_deliveries(st, &mut updated, price, bar_time, at);
        if let Err(e) =
            self.store
                .record_server_alert_fire(id, &updated.owner, &updated.json(), &events)
        {
            // Memory stays as the store has it, so a restart cannot disagree.
            eprintln!("[alerts] cannot store a fire: {e}");
            return;
        }
        st.alerts.insert(id.to_string(), updated);
        for event in events {
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

    /// The deliveries of a fire: its webhook and one push per device, those allowed to
    /// wait. They wait for their turn however busy the budget is, so a `once` alert keeps
    /// them. Only with a full queue does the fire count without one, and the alert says
    /// why. All of them carry one body, and so one `id`, however many rows that takes.
    fn prepare_deliveries(
        &self,
        st: &State,
        alert: &mut Record,
        price: f64,
        bar_time: i64,
        at: i64,
    ) -> Vec<StoredEvent> {
        alert.device_pushes.clear();
        alert.device_pushes_fire = None;
        if alert.webhook.is_none() && !alert.push {
            return Vec::new();
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
            "fire_count": alert.fire_count,
        })
        .to_string();
        let limits = &self.limits;
        let waiting = st.queues.get(&alert.owner).map_or(0, VecDeque::len);
        let mut events: Vec<StoredEvent> = Vec::new();
        let has_room = |events: &[StoredEvent]| {
            st.pending + events.len() < limits.pending_deliveries
                && waiting + events.len() < limits.pending_deliveries_per_client
        };
        let refused = |error: &str| Delivery {
            at,
            status: None,
            error: Some(error.to_string()),
        };
        let event = |id: String, device: Option<String>| StoredEvent {
            id,
            alert_id: alert.id.clone(),
            body: body.clone(),
            created_at: at / 1000,
            device,
        };
        let mut hook_refusal = None;
        if alert.webhook.is_some() {
            if !self.access.may_use_webhooks(&alert.owner) {
                hook_refusal = Some(refused("webhook skipped: this client may not use webhooks"));
            } else if has_room(&events) {
                events.push(event(id.clone(), None));
            } else {
                hook_refusal = Some(refused("webhook skipped: too many deliveries waiting"));
            }
        }
        let mut push_refusal = None;
        if alert.push {
            alert.device_pushes_fire = Some(id.clone());
            let mut devices: Vec<&StoredDevice> = st
                .devices
                .values()
                .filter(|d| d.owner == alert.owner)
                .collect();
            devices.sort_by_key(|d| d.created_at);
            if self.vapid.is_none() || !self.access.may_use_push(&alert.owner) {
                push_refusal = Some(refused("push skipped: not available on this server"));
            } else if devices.is_empty() {
                push_refusal = Some(refused("push skipped: no device has notifications on"));
            }
            if push_refusal.is_none() {
                for device in devices {
                    if has_room(&events) {
                        events.push(event(
                            format!("{id}:{}", device.id),
                            Some(device.id.clone()),
                        ));
                        alert.device_pushes.insert(device.id.clone(), None);
                    } else {
                        push_refusal = Some(refused("push skipped: too many deliveries waiting"));
                    }
                }
            }
        }
        if hook_refusal.is_some() {
            alert.last_delivery = hook_refusal;
        }
        if push_refusal.is_some() {
            alert.last_push = push_refusal;
        }
        events
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
        let mut left = (limits.deliveries_total_per_minute / DISPATCHES_PER_MINUTE).max(1);
        for owner in owners {
            if left == 0
                || !st
                    .webhooks
                    .has_room("", limits.deliveries_total_per_minute, now)
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
            // Each kind has its budget; a client's own deliveries stay in order.
            let is_push = queue.front().is_some_and(|e| e.device.is_some());
            let has_room = if is_push {
                st.pushes
                    .has_room(&owner, limits.pushes_per_client_per_minute, now)
            } else {
                st.webhooks
                    .has_room(&owner, limits.webhooks_per_client_per_minute, now)
            };
            if !has_room {
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
            if is_push {
                st.pushes.record(&owner, now);
            } else {
                st.webhooks.record(&owner, now);
            }
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
        // As for alerts: a device outlives a sign-in, not 30 days without one.
        if let Some(tokens) = &last_tokens {
            let idle: Vec<String> = st
                .devices
                .values()
                .filter(|d| {
                    let last = tokens.get(&d.owner).map_or(0, |secs| secs * 1000);
                    !self.access.keeps_idle_alerts(&d.owner)
                        && now - last.max(d.created_at) > IDLE_OWNER_TTL_MS
                })
                .map(|d| d.id.clone())
                .collect();
            for id in idle {
                self.forget_device(st, &id);
            }
        }
        self.sync_feeds(st);
    }

    /// Every alert and push device owned by an OAuth client goes to `user`, the owner the
    /// operator named, with waiting deliveries, whatever limit that passes: they are that
    /// person's already (ADR-0044). A webhook's secret moves with its alert. Nothing is
    /// deleted but a device that would be a second row for one subscription. Returns how
    /// many alerts and devices moved. Blocking.
    pub fn claim_waiting(&self, user: &str) -> (usize, usize) {
        let mut guard = self.lock();
        let st = &mut *guard;
        let ids: Vec<String> = st
            .alerts
            .values()
            .filter(|a| persistence::is_client_id(&a.owner))
            .map(|a| a.id.clone())
            .collect();
        let mut moved = HashSet::new();
        for id in ids {
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

        let mut held: HashMap<String, StoredDevice> = st
            .devices
            .values()
            .filter(|d| d.owner == user)
            .map(|d| (d.endpoint.clone(), d.clone()))
            .collect();
        let mut waiting: Vec<(i64, String)> = st
            .devices
            .values()
            .filter(|d| persistence::is_client_id(&d.owner))
            .map(|d| (d.created_at, d.id.clone()))
            .collect();
        waiting.sort();
        let mut devices_moved = 0;
        let mut duplicates = Vec::new();
        for (_, id) in waiting {
            let Some(device) = st.devices.get_mut(&id) else {
                continue;
            };
            // One endpoint, one owner. The same subscription under another client is a
            // duplicate; other keys are not provably the same browser, so they stay.
            if let Some(kept) = held.get(&device.endpoint) {
                if same_keys(kept, &device.p256dh, &device.auth) {
                    duplicates.push(id);
                }
                continue;
            }
            let mut updated = device.clone();
            updated.owner = user.to_string();
            match self.store.save_push_device(&updated) {
                Ok(()) => {
                    held.insert(updated.endpoint.clone(), updated.clone());
                    *device = updated;
                    devices_moved += 1;
                }
                Err(e) => eprintln!("[alerts] cannot move a push device to its user: {e}"),
            }
        }
        for id in duplicates {
            self.forget_device(st, &id);
        }

        if !moved.is_empty() {
            let mut theirs = VecDeque::new();
            for (owner, queue) in st.queues.iter_mut() {
                if persistence::is_client_id(owner) {
                    let (took, stays): (VecDeque<_>, VecDeque<_>) = std::mem::take(queue)
                        .into_iter()
                        .partition(|e| moved.contains(&e.alert_id));
                    *queue = stays;
                    theirs.extend(took);
                }
            }
            st.queues
                .entry(user.to_string())
                .or_default()
                .extend(theirs);
            self.sync_feeds(st);
        }
        (moved.len(), devices_moved)
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

    /// Drop a device, with what it still had to receive.
    fn forget_device(&self, st: &mut State, id: &str) {
        if let Err(e) = self.store.delete_push_device(id) {
            eprintln!("[alerts] cannot delete a push device: {e}");
            return;
        }
        st.devices.remove(id);
        let mut dropped = 0;
        for queue in st.queues.values_mut() {
            let before = queue.len();
            queue.retain(|e| e.device.as_deref() != Some(id));
            dropped += before - queue.len();
        }
        st.pending = st.pending.saturating_sub(dropped);
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
        let (hook, device, client) = {
            let mut st = self.lock();
            let alert = st.alerts.get(&event.alert_id);
            let hook = alert.and_then(|a| a.webhook.clone());
            let owner = alert.map(|a| a.owner.clone()).unwrap_or_default();
            let device = event
                .device
                .as_ref()
                .and_then(|d| st.devices.get(d))
                .cloned();
            (hook, device, self.client_slot(&mut st, &owner))
        };
        // One client's dead host cannot hold every slot.
        let outcome = match (&event.device, hook, &device) {
            (None, Some(hook), _) => Some(self.send(&hook, &event, &client).await),
            (Some(_), _, Some(device)) => Some(self.send_push(device, &event, &client).await),
            _ => None,
        };
        let mut st = self.lock();
        st.pending = st.pending.saturating_sub(1);
        if let Some(outcome) = outcome {
            // The push service says this subscription is gone for good.
            if let (Some(device), Some(404 | 410)) = (&device, outcome.status) {
                self.forget_device(&mut st, &device.id);
            }
            if let Some(alert) = st.alerts.get_mut(&event.alert_id) {
                if let Some(device) = &event.device {
                    let this_fire = alert
                        .device_pushes_fire
                        .as_ref()
                        .is_some_and(|fire| event.id == format!("{fire}:{device}"));
                    if this_fire {
                        alert
                            .device_pushes
                            .insert(device.clone(), Some(outcome.clone()));
                    }
                    alert.last_push = Some(outcome);
                } else {
                    alert.last_delivery = Some(outcome);
                }
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

    /// One push, through the webhook path's pinning, retries and slots.
    async fn send_push(
        &self,
        device: &StoredDevice,
        event: &StoredEvent,
        client: &Semaphore,
    ) -> Delivery {
        let prepared = (|| {
            let vapid = self
                .vapid
                .as_ref()
                .ok_or("push is not available on this server")?;
            let target = Target::parse_push(&device.endpoint, &self.policy)?;
            let keys = DeviceKeys::parse(&device.p256dh, &device.auth)?;
            let now = chrono::Utc::now().timestamp();
            let request = vapid.request(&target.origin(), &keys, event.body.as_bytes(), now)?;
            Ok::<_, String>((target, request))
        })();
        let outcome = match prepared {
            Ok((target, request)) => {
                let slots = [client, self.slots.as_ref()];
                let make = || request.clone();
                webhook::deliver_request(&target, &self.policy, &make, &self.timing, &slots).await
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
            "subscribe_push" => self.subscribe_push(args, owner),
            "unsubscribe_push" => self.unsubscribe_push(args, owner),
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

    /// Can this client's alerts notify devices at all?
    fn check_push(&self, owner: &str) -> Result<(), String> {
        if self.vapid.is_none() || !self.access.may_use_push(owner) {
            return Err("This server does not send push notifications for this client".into());
        }
        Ok(())
    }

    /// Remember this browser's subscription. The endpoint and keys come from the browser,
    /// so they are checked here. A device over the limit replaces the client's oldest:
    /// a phone reinstalled or a site cleared leaves a dead one behind, nobody can name it,
    /// and the limit is only there to bound the fan-out.
    ///
    /// An endpoint has one owner, whoever subscribed it last, so a device never shows two
    /// people's alerts. Taking it over needs the keys too, which only that browser has:
    /// knowing an endpoint is not enough to remove someone's device (ADR-0044).
    fn subscribe_push(&self, args: &Value, owner: &str) -> Result<Value, String> {
        self.check_push(owner)?;
        self.admit_change(owner)?;
        let text = |name: &str| {
            args.get(name)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{name} is required"))
        };
        let (p256dh, auth) = (text("p256dh")?, text("auth")?);
        let target = Target::parse_push(text("endpoint")?, &self.policy)?;
        let endpoint = target.as_str();
        DeviceKeys::parse(p256dh, auth)?;

        let mut st = self.lock();
        let others: Vec<&StoredDevice> = st
            .devices
            .values()
            .filter(|d| d.owner != owner && d.endpoint == endpoint)
            .collect();
        if !others.iter().all(|d| same_keys(d, p256dh, auth)) {
            return Err("This push subscription is registered with other keys".into());
        }
        let taken_over: Vec<String> = others.iter().map(|d| d.id.clone()).collect();
        let existing = st
            .devices
            .values()
            .find(|d| d.owner == owner && d.endpoint == endpoint)
            .map(|d| (d.id.clone(), d.created_at));
        if existing.is_none() {
            let mut mine: Vec<(i64, String)> = st
                .devices
                .values()
                .filter(|d| d.owner == owner)
                .map(|d| (d.created_at, d.id.clone()))
                .collect();
            mine.sort();
            let over = (mine.len() + 1).saturating_sub(self.limits.devices_per_client);
            // Replacing one of its own, or taking one over, adds nothing to the total.
            if over == 0 && taken_over.is_empty() && st.devices.len() >= self.limits.devices_total {
                let Some(id) = self.device_to_replace(&st, owner) else {
                    return Err(format!(
                        "Limit reached: this server holds its maximum of {} push devices",
                        self.limits.devices_total
                    ));
                };
                self.forget_device(&mut st, &id);
            }
            for (_, id) in mine.into_iter().take(over) {
                self.forget_device(&mut st, &id);
            }
        }
        let (id, created_at) =
            existing.unwrap_or_else(|| (uuid::Uuid::new_v4().to_string(), now_ms()));
        let device = StoredDevice {
            id: id.clone(),
            owner: owner.to_string(),
            endpoint: endpoint.to_string(),
            p256dh: p256dh.to_string(),
            auth: auth.to_string(),
            created_at,
        };
        self.store.save_push_device(&device).map_err(|e| {
            eprintln!("[alerts] cannot store a push device: {e}");
            "The alert store is unavailable".to_string()
        })?;
        st.devices.insert(id.clone(), device);
        for other in taken_over {
            self.forget_device(&mut st, &other);
        }
        Ok(json!({
            "devices": st.devices.values().filter(|d| d.owner == owner).count(),
            "device": id,
        }))
    }

    /// With the table full, whose device makes room: never a client the operator listed,
    /// and those with no alert that would notify them go first, the owner idle longest
    /// among them. Junk devices need no alert, and their owners hold fresh tokens, so going
    /// by idleness alone would let a flood push out real users; this way it mostly replaces
    /// itself. An unknown owner is not an idle one, as in `sweep`.
    fn device_to_replace(&self, st: &State, caller: &str) -> Option<String> {
        let tokens: HashMap<String, i64> = match self.store.oauth_clients() {
            Ok(clients) => clients
                .into_iter()
                .filter_map(|c| Some((c.client_id, c.last_token_at?)))
                .collect(),
            Err(e) => {
                eprintln!("[alerts] cannot read clients: {e}");
                return None;
            }
        };
        let notified: HashSet<&str> = st
            .alerts
            .values()
            .filter(|a| a.push && a.active)
            .map(|a| a.owner.as_str())
            .collect();
        st.devices
            .values()
            .filter(|d| d.owner != caller && !self.access.keeps_idle_alerts(&d.owner))
            .min_by_key(|d| {
                let last = tokens.get(&d.owner).map_or(0, |secs| secs * 1000);
                (notified.contains(d.owner.as_str()), last.max(d.created_at))
            })
            .map(|d| d.id.clone())
    }

    fn unsubscribe_push(&self, args: &Value, owner: &str) -> Result<Value, String> {
        let raw = args
            .get("endpoint")
            .and_then(Value::as_str)
            .ok_or("endpoint is required")?;
        // As it was stored. One that does not parse was never stored, and removes nothing.
        let target = Target::parse_push(raw, &self.policy);
        let endpoint = target.as_ref().map_or(raw, Target::as_str);
        let mut st = self.lock();
        let id = st
            .devices
            .values()
            .find(|d| d.owner == owner && d.endpoint == endpoint)
            .map(|d| d.id.clone());
        if let Some(id) = &id {
            self.forget_device(&mut st, id);
        }
        Ok(json!({
            "removed": id.is_some(),
            "devices": st.devices.values().filter(|d| d.owner == owner).count(),
        }))
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
        // Setups never ask alerts for anything while they hold their own lock.
        let mut watched = wanted.clone();
        if let Some(setups) = self.setups.get().and_then(Weak::upgrade) {
            watched.extend(setups.pairs());
        }
        if !watched.contains(feed) && watched.len() >= self.limits.feeds {
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
        let push = push_arg(args)?.unwrap_or(false);
        if push {
            self.check_push(owner)?;
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
            push,
            last_push: None,
            device_pushes: HashMap::new(),
            device_pushes_fire: None,
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
        let push = push_arg(args)?;
        if push == Some(true) {
            self.check_push(owner)?;
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
        next.push = push.unwrap_or(next.push);
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
            // Null where this client's alerts cannot notify a device.
            "push": self.vapid.as_ref().filter(|_| self.access.may_use_push(owner)).map(|v| json!({
                "public_key": v.public_key(),
                "devices": st.devices.values().filter(|d| d.owner == owner).count(),
            })),
            "limits": {
                "alerts_per_client": limits.alerts_per_client,
                "pairs_per_client": limits.pairs_per_client,
                "fires_per_alert_per_minute": limits.fires_per_alert_per_minute,
                "webhooks_per_client_per_minute": limits.webhooks_per_client_per_minute,
                "pushes_per_client_per_minute": limits.pushes_per_client_per_minute,
                "devices_per_client": limits.devices_per_client,
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
            "push": a.push,
            "last_push_at": a.last_push.as_ref().map(|d| d.at),
            "last_push_status": a.last_push.as_ref().and_then(|d| d.status),
            "last_push_error": a.last_push.as_ref().and_then(|d| d.error.clone()),
            "last_push_devices": a.device_pushes,
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

fn push_arg(args: &Value) -> Result<Option<bool>, String> {
    match args.get("push") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => Ok(Some(v.as_bool().ok_or("push must be true or false")?)),
    }
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
