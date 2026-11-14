//! Setups on the server: the store, the five tools and the feed (ADR-0059).
//!
//! Off unless `CHARTGEN_SETUPS=1`. A setup belongs to the token's user, or with
//! sign-in off to its OAuth client, as a server alert does. Its status moves on closed
//! 1m bars of the chart hub's feed for its symbol, the feed every chart and alert of
//! that symbol shares, so setups add no polling. On a replay server (`crate::replay`)
//! there is no feed: the bars come from fixtures, up to the time a call asks about,
//! and the setups live in a store of their own.
//!
//! A setup on a US stock (ADR-0070) has no feed: `stocks` polls Yahoo's 1m bars for it
//! and moves it with the same scorer.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use chartgen::data::Bar;
use chartgen::drawings;
use chartgen::feed::FeedEvent;
use chartgen::scripting::persistence::{self, NewEvent, Store, StoredSetup};
use chartgen::setups::{
    self, Decision, Draft, MinuteBar, Problem, Record, Revision, Status, LEVEL_FIELDS, MAX_THESIS,
};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::chart_push::{ChartHub, FeedKey, FeedLease};
use crate::fetch::{self, DataSource};
use crate::mcp::{normalize_quote_symbol, tool_err, tool_ok};
use crate::replay::Replay;
use crate::server_alerts::{Access, ServerAlerts, Window};

/// Every setup reads its symbol's 1m feed.
const FEED_INTERVAL: &str = "1m";
const MINUTE_MS: i64 = 60_000;
const FEED_RETRY: Duration = Duration::from_secs(30);
const UPKEEP_EVERY: Duration = Duration::from_secs(60);
/// Bars read to place drawings and to build a skip's bracket.
const CONTEXT_BARS: usize = 300;
/// Most 1m bars read at once to catch up after the server missed some.
const CATCH_UP_BARS: i64 = 1000;
/// Closed setups one `get_track_record` reads: the newest of them.
const TRACK_RECORD_SETUPS: usize = 500;

/// `CHARTGEN_SETUPS=1` turns setups on.
pub fn enabled() -> bool {
    std::env::var("CHARTGEN_SETUPS").is_ok_and(|v| v == "1")
}

/// Anyone with an account may sign in (ADR-0044), so these assume a stranger.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Setups watching or triggered, per owner and on the server.
    pub open_per_owner: usize,
    pub open_total: usize,
    /// Setups kept, closed ones included: the track record. At the limit a new one is
    /// refused; nothing old is dropped.
    pub setups_per_owner: usize,
    pub setups_total: usize,
    /// Symbols one owner has open setups on (`CHARTGEN_SETUP_SYMBOLS_PER_OWNER`).
    pub symbols_per_owner: usize,
    /// Symbol feeds watched on the server, alerts and setups together (ADR-0042).
    pub pairs_total: usize,
    pub agent_revisions_per_setup: u32,
    pub changes_per_minute: usize,
    pub record_bytes: usize,
    pub revision_bytes: usize,
    /// Setups and their logs.
    pub bytes_per_owner: usize,
    pub bytes_total: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            open_per_owner: 20,
            open_total: 200,
            setups_per_owner: 500,
            setups_total: 20_000,
            symbols_per_owner: 5,
            pairs_total: crate::server_alerts::Limits::default().feeds,
            agent_revisions_per_setup: 50,
            changes_per_minute: 10,
            record_bytes: 32 * 1024,
            revision_bytes: 64 * 1024,
            bytes_per_owner: 4 * 1024 * 1024,
            bytes_total: 256 * 1024 * 1024,
        }
    }
}

struct Watch {
    task: JoinHandle<()>,
    /// The hub's feed a Binance symbol holds; a stock's poll holds none.
    _lease: Option<FeedLease>,
}

/// A setup and the bytes it holds: its record and its log.
struct Held {
    record: Record,
    record_bytes: usize,
    log_bytes: usize,
}

#[derive(Default)]
struct State {
    /// By owner and id, so that a list is in a fixed order.
    setups: BTreeMap<(String, String), Held>,
    watches: HashMap<FeedKey, Watch>,
    changes: Window,
    /// Setups made so far per owner, for a replay's ids.
    made: HashMap<String, u64>,
    /// Per stock symbol, the time through which its 1m bars were read (ADR-0070).
    stock_reads: HashMap<String, i64>,
    /// Per stock symbol, a read held back for an implausible first price: that bar's
    /// time, and the polls it has been held.
    stock_holds: HashMap<String, (i64, u32)>,
}

pub struct Setups {
    store: &'static Store,
    /// `None` on a replay server, which has no feed.
    hub: Option<Arc<ChartHub>>,
    replay: Option<&'static Replay>,
    source: DataSource,
    access: Access,
    limits: Limits,
    /// Counted with ours against `pairs_total`.
    alerts: Option<Arc<ServerAlerts>>,
    runtime: tokio::runtime::Handle,
    state: Mutex<State>,
    /// Run by `create` between reading the alerts' pairs and its final check, so a test
    /// can put another create exactly there.
    #[cfg(test)]
    before_final_check: Mutex<Option<Box<dyn Fn() + Send>>>,
    me: Weak<Setups>,
}

static GLOBAL: OnceLock<Arc<Setups>> = OnceLock::new();

/// The running setups, when on.
pub fn global() -> Option<&'static Arc<Setups>> {
    GLOBAL.get()
}

/// Start setups if they are on and the store opened. Inside the runtime, after
/// server alerts.
pub fn start(hub: &Arc<ChartHub>) {
    if !enabled() {
        return;
    }
    let Some(store) = persistence::global() else {
        eprintln!("[setups] the store is unavailable; setups stay off");
        return;
    };
    let mut limits = Limits::default();
    if let Some(symbols) = std::env::var("CHARTGEN_SETUP_SYMBOLS_PER_OWNER")
        .ok()
        .and_then(|v| v.trim().parse().ok())
    {
        limits.symbols_per_owner = symbols;
    }
    let setups = Setups::new(
        store,
        Some(hub.clone()),
        None,
        DataSource::from_env(),
        allowlist(),
        limits,
        crate::server_alerts::global().cloned(),
    );
    println!("Setups enabled ({} restored)", setups.lock().setups.len());
    let _ = GLOBAL.set(setups);
}

/// Start setups on a replay server: fixture bars, and a store that lives and dies
/// with the process.
pub fn start_replay(replay: &'static Replay) -> Result<(), String> {
    let store = Store::in_memory().map_err(|e| format!("cannot open the replay's store: {e}"))?;
    let setups = Setups::new(
        Box::leak(Box::new(store)),
        None,
        Some(replay),
        DataSource::Replay { as_of: None },
        Access::parse(None),
        Limits::default(),
        None,
    );
    let _ = GLOBAL.set(setups);
    Ok(())
}

/// The operator's list of who may create alerts also says who may create setups.
fn allowlist() -> Access {
    Access::parse(
        std::env::var("CHARTGEN_SERVER_ALERT_CLIENTS")
            .ok()
            .as_deref(),
    )
}

fn field_problem(field: &str, message: impl Into<String>) -> String {
    setups::problems_text(&[Problem::new(field, message)])
}

/// `field_problem` for a call that only reads: the same shape, and words that do not
/// speak of storing.
fn read_problem(field: &str, message: impl Into<String>) -> String {
    let problem = Problem::new(field, message).to_json();
    serde_json::to_string_pretty(&json!({
        "error": "1 problem with this call; nothing was read. Fix it and send the call again.",
        "setup_errors": [problem],
    }))
    .unwrap_or_default()
}

impl Setups {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        store: &'static Store,
        hub: Option<Arc<ChartHub>>,
        replay: Option<&'static Replay>,
        source: DataSource,
        access: Access,
        limits: Limits,
        alerts: Option<Arc<ServerAlerts>>,
    ) -> Arc<Self> {
        let setups = Arc::new_cyclic(|me| Self {
            store,
            hub,
            replay,
            source,
            access,
            limits,
            alerts: alerts.clone(),
            runtime: tokio::runtime::Handle::current(),
            state: Mutex::default(),
            #[cfg(test)]
            before_final_check: Mutex::default(),
            me: me.clone(),
        });
        setups.restore();
        if let Some(alerts) = alerts {
            alerts.count_pairs_of(&setups);
        }
        if setups.hub.is_some() {
            setups.runtime.spawn(run_upkeep(setups.me.clone()));
        }
        setups
    }

    /// Load the stored setups and join their feeds. A row that cannot be read, or that
    /// could not have been written here, is left out and counted: it is never scored.
    fn restore(&self) {
        let mut st = self.lock();
        let rows = match self.store.setups() {
            Ok(rows) => rows,
            Err(e) => {
                eprintln!("[setups] cannot read stored setups: {e}");
                return;
            }
        };
        // What each setup was made with, from its log (ADR-0064).
        let made = self.store.first_setup_revisions().unwrap_or_else(|e| {
            eprintln!("[setups] cannot read the setups' first log entries: {e}");
            HashMap::new()
        });
        let mut skipped = 0;
        for (row, log_bytes) in rows {
            match serde_json::from_str::<Record>(&row).ok().filter(sound) {
                Some(mut record) => {
                    let first = made.get(&self.row_id(&record));
                    if let Some(first) = first.and_then(|b| serde_json::from_str(b).ok()) {
                        record.restate(&first);
                    }
                    // A row a build from before stock setups wrote again has lost its
                    // market (ADR-0070); a symbol read from Yahoo is only ever a stock's.
                    if record.market.is_none() {
                        record.market = fetch::market_of(&record.symbol);
                    }
                    let key = (record.owner.clone(), record.id.clone());
                    st.setups.insert(
                        key,
                        Held {
                            record_bytes: json_len(&record),
                            record,
                            log_bytes,
                        },
                    );
                }
                // Not the row: it holds someone's thesis.
                None => skipped += 1,
            }
        }
        if skipped > 0 {
            eprintln!("[setups] skipped {skipped} stored setups that cannot be read");
        }
        drop(st);
        self.expire_unscored(fetch::now_ms());
        self.tidy_events(chrono::Utc::now().timestamp());
        self.sync_feeds(&mut self.lock());
    }

    /// Keep the event log to what it promises (ADR-0061): a wake-up waits an hour, an
    /// event is kept 7 days. Here as well as in the alert service, since setups write
    /// events where no alert service runs.
    fn tidy_events(&self, unix: i64) {
        if self.replay.is_some() {
            return;
        }
        if let Err(e) = self.store.tidy_events(unix) {
            eprintln!("[setups] cannot sweep the event log: {e}");
        }
    }

    /// End the open setups that ran out while their owner was off the operator's list.
    /// They get no bars, and a setup only expires on a bar, so they would stay open, and
    /// count as open, for ever.
    fn expire_unscored(&self, now: i64) {
        let mut st = self.lock();
        let changed: Vec<(Record, Vec<Revision>)> = st
            .setups
            .values()
            .map(|h| &h.record)
            .filter(|r| r.is_scoring() && !self.runs(r))
            .filter_map(|r| {
                let mut record = r.clone();
                let log = record.expire_unscored(now);
                (!log.is_empty()).then_some((record, log))
            })
            .collect();
        let _ = self.write(&mut st, changed);
    }

    /// `owner`'s setup `id`, as it is now. For an alert that is about it (ADR-0061):
    /// called without the alerts' lock held.
    pub fn owned(&self, owner: &str, id: &str) -> Option<Record> {
        let key = (owner.to_string(), id.to_string());
        self.lock().setups.get(&key).map(|h| h.record.clone())
    }

    /// How many open setups each owner has, for the operator page (ADR-0063): counted
    /// from what is held in memory, not read from the store.
    pub(crate) fn open_by_owner(&self) -> HashMap<String, usize> {
        let mut open = HashMap::new();
        for ((owner, _), held) in &self.lock().setups {
            if held.record.status.is_open() {
                *open.entry(owner.clone()).or_default() += 1;
            }
        }
        open
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The feed of a symbol's 1m bars. A stock's is the symbol's poll of Yahoo.
    fn feed(&self, symbol: &str) -> Result<FeedKey, String> {
        match &self.hub {
            Some(hub) if fetch::market_of(symbol).is_none() => hub.feed_key(symbol, FEED_INTERVAL),
            _ => Ok((symbol.to_string(), FEED_INTERVAL.to_string())),
        }
    }

    /// Whether a setup is scored: open or with a shadow open (ADR-0066), and its owner
    /// still one the operator allows. The list is read at startup, so an owner taken off
    /// it stops here from the restart that read the new list, as its alerts do
    /// (ADR-0042). Its setups stay readable.
    fn runs(&self, record: &Record) -> bool {
        record.is_scoring() && self.access.may_create(&record.owner)
    }

    /// The feed of a setup: a stock's is its poll of Yahoo, whatever its symbol looks
    /// like (ADR-0070: ABNB ends in BNB).
    fn feed_of(&self, record: &Record) -> Result<FeedKey, String> {
        match record.market {
            Some(_) => Ok((record.symbol.clone(), FEED_INTERVAL.to_string())),
            None => self.feed(&record.symbol),
        }
    }

    /// The market of setups on `symbol`: a stock's when its symbol is read from Yahoo,
    /// or when a stock setup on it is kept here already.
    fn market_on(&self, st: &State, symbol: &str) -> Option<setups::Market> {
        fetch::market_of(symbol).or_else(|| {
            st.setups
                .values()
                .find(|h| h.record.symbol == symbol)
                .and_then(|h| h.record.market)
        })
    }

    fn wanted_feeds(&self, st: &State) -> HashSet<FeedKey> {
        st.setups
            .values()
            .filter(|h| self.runs(&h.record))
            .filter_map(|h| self.feed_of(&h.record).ok())
            .collect()
    }

    /// The feeds open setups hold, for the server's limit on watched pairs. A closed
    /// setup's open shadow holds its feed too: the server does watch it (ADR-0066).
    pub fn pairs(&self) -> HashSet<FeedKey> {
        self.wanted_feeds(&self.lock())
    }

    /// How many pairs open setups watch, and the limit on watched pairs: what the
    /// operator page shows on a server without alerts, where setups are all that watch.
    pub(crate) fn watched_pairs(&self) -> (usize, usize) {
        (self.pairs().len(), self.limits.pairs_total)
    }

    /// Join the feeds open setups need and leave the others.
    fn sync_feeds(&self, st: &mut State) {
        let Some(hub) = &self.hub else {
            return;
        };
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
            if self.market_on(st, &key.0).is_some() {
                let task = self
                    .runtime
                    .spawn(stocks::run_poll(self.me.clone(), key.0.clone()));
                st.watches.insert(key, Watch { task, _lease: None });
                continue;
            }
            // Refused now: the upkeep tries again. Bars missed meanwhile are read then.
            let (rx, lease) = match hub.watch_feed(&key) {
                Ok(joined) => joined,
                Err(e) => {
                    eprintln!("[setups] feed {} {}: {e}", key.0, key.1);
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
                    _lease: Some(lease),
                },
            );
        }
    }

    fn rejoin(&self, key: &FeedKey) {
        let mut st = self.lock();
        st.watches.remove(key);
        self.sync_feeds(&mut st);
    }

    /// Write setups with the log entries they gained, then take them into memory. On a
    /// failure memory stays as the store has it, so a restart cannot disagree.
    fn write(&self, st: &mut State, changed: Vec<(Record, Vec<Revision>)>) -> Result<(), String> {
        let rows: Vec<(StoredSetup, usize)> = changed
            .iter()
            .map(|(record, log)| {
                let revisions: Vec<(u32, String)> = log
                    .iter()
                    .map(|e| (e.seq, serde_json::to_string(e).unwrap_or_default()))
                    .collect();
                let log_bytes = revisions.iter().map(|(_, body)| body.len()).sum();
                let row = StoredSetup {
                    id: self.row_id(record),
                    owner: record.owner.clone(),
                    record: serde_json::to_string(record).unwrap_or_default(),
                    revisions,
                };
                (row, log_bytes)
            })
            .collect();
        let (rows, added): (Vec<StoredSetup>, Vec<usize>) = rows.into_iter().unzip();
        let events = self.status_events(&changed);
        self.store
            .save_setups_logging(&rows, &events)
            .map_err(|e| {
                eprintln!("[setups] cannot store a setup: {e}");
                "The setup store is unavailable".to_string()
            })?;
        for (((record, _), row), added) in changed.into_iter().zip(rows).zip(added) {
            let key = (record.owner.clone(), record.id.clone());
            let log_bytes = st.setups.get(&key).map_or(0, |h| h.log_bytes) + added;
            st.setups.insert(
                key,
                Held {
                    record,
                    record_bytes: row.record.len(),
                    log_bytes,
                },
            );
        }
        Ok(())
    }

    /// What this write says happened to a setup's status, as events of its owner's log
    /// (ADR-0061), stored with the entries that say it. None on a replay, which has no
    /// log and must answer the same on every run.
    ///
    /// A change the server made to a setup the agent took is also a wake-up, if the
    /// operator configured a receiver for that owner. A cancellation is the agent's own
    /// act, and nobody waits on a skip: those are logged and wake nobody.
    fn status_events(&self, changed: &[(Record, Vec<Revision>)]) -> Vec<NewEvent> {
        if self.replay.is_some() {
            return Vec::new();
        }
        let mut events = Vec::new();
        for (record, log) in changed {
            let wakes = self.alerts.as_ref().is_some_and(|a| a.wakes(&record.owner));
            for entry in log {
                let Some(status) = entry.changes.get("status") else {
                    continue;
                };
                let by_server =
                    entry.by == setups::By::Server && entry.kind == setups::Kind::Status;
                let cancelled = status["to"] == json!(Status::Cancelled);
                let mut body =
                    json!({ "from": status["from"], "to": status["to"], "gap": record.gap });
                if let Some(result) = entry.changes.get("result_r") {
                    body["result_r"] = result["to"].clone();
                }
                events.push(NewEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    owner: record.owner.clone(),
                    kind: "setup.status".to_string(),
                    // When it happened: the bar's time, not the time of this write. After
                    // the server was away, several are written at once.
                    at: entry.at,
                    alert_id: None,
                    setup_id: Some(record.id.clone()),
                    body: body.to_string(),
                    deliver: wakes && by_server && !cancelled && record.decision == Decision::Take,
                });
            }
        }
        events
    }

    /// The store's key of a setup. A replay's ids count per owner, so that two callers
    /// cannot change each other's.
    fn row_id(&self, record: &Record) -> String {
        match self.replay {
            Some(_) => format!("{}/{}", record.owner, record.id),
            None => record.id.clone(),
        }
    }

    fn new_id(&self, st: &mut State, owner: &str) -> String {
        if self.replay.is_none() {
            return uuid::Uuid::new_v4().to_string();
        }
        let made = st.made.entry(owner.to_string()).or_default();
        *made += 1;
        format!("setup-{made:06}")
    }

    /// The time of a call. On a replay server it is the `as_of` the call names, which
    /// moves the server's one clock, and the caller's setups, up to it.
    fn now(&self, args: &Value, owner: &str) -> Result<i64, String> {
        let as_of = args.get("as_of").filter(|v| !v.is_null());
        let Some(replay) = self.replay else {
            return match as_of {
                Some(_) => Err(crate::replay::NOT_A_REPLAY.into()),
                None => Ok(fetch::now_ms()),
            };
        };
        let now = crate::replay::as_of(as_of)?;
        replay.advance(now)?;
        self.replay_to(replay, owner, now)?;
        Ok(now)
    }

    /// Move `owner`'s open setups through the fixture bars closed by `now`, oldest
    /// setup first.
    fn replay_to(&self, replay: &Replay, owner: &str, now: i64) -> Result<(), String> {
        let mut st = self.lock();
        let mut open: Vec<Record> = st
            .setups
            .values()
            .map(|h| &h.record)
            .filter(|r| r.owner == owner && r.is_scoring())
            .cloned()
            .collect();
        open.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        let mut changed = Vec::new();
        for mut record in open {
            let before = record.clone();
            let from = record.next_bar_time();
            let read = fetch::StockMinutes {
                bars: replay.minute_bars(&record.symbol, from, now).to_vec(),
                adjustments: replay.dividends(&record.symbol, from, now),
                reached: Some(now),
            };
            let log = stocks::score(&mut record, &read, now);
            if record != before {
                changed.push((record, log));
            }
        }
        self.write(&mut st, changed)
    }

    /// The source a call at `now` reads bars from.
    fn source_at(&self, now: i64) -> DataSource {
        match self.replay {
            Some(_) => DataSource::Replay { as_of: Some(now) },
            None => self.source,
        }
    }

    /// The bars of a setup's chart, and those of them that have closed.
    fn context(&self, symbol: &str, interval: &str, now: i64) -> Result<(Vec<Bar>, usize), String> {
        let bars = fetch::fetch_ohlcv(self.source_at(now), symbol, interval, CONTEXT_BARS, None)?
            .data
            .bars;
        let secs = chartgen::data::interval_secs(interval).unwrap_or(60);
        let closed = bars
            .iter()
            .take_while(|b| {
                b.date
                    .parse::<i64>()
                    .is_ok_and(|t| (t + secs) * 1000 <= now)
            })
            .count();
        Ok((bars, closed))
    }

    /// Check `drawings` as `generate_chart` does and put them on `bars` (ADR-0054).
    fn place(requests: Vec<drawings::Request>, bars: &[Bar]) -> Result<Vec<Value>, String> {
        match drawings::resolve(requests, bars) {
            Ok(resolved) => Ok(resolved.drawings.iter().map(|d| d.to_json()).collect()),
            Err(problems) => {
                let problems: Vec<Problem> = problems.iter().map(Problem::of_drawing).collect();
                Err(setups::problems_text(&problems))
            }
        }
    }

    fn admit_change(&self, owner: &str) -> Result<(), String> {
        // A replay is not public, and a limit on the wall clock would make its answers
        // depend on how fast it is run.
        if self.replay.is_some() {
            return Ok(());
        }
        let max = self.limits.changes_per_minute;
        if self.lock().changes.admit(owner, max, Instant::now()) {
            Ok(())
        } else {
            Err(format!(
                "Limit reached: at most {max} setup changes per minute"
            ))
        }
    }

    /// Room for one more open setup of `owner` on `symbol`? `alert_pairs` are the feeds
    /// server alerts hold, read before our lock is taken.
    fn check_capacity(
        &self,
        st: &State,
        owner: &str,
        symbol: &str,
        market: Option<setups::Market>,
        alert_pairs: &HashSet<FeedKey>,
    ) -> Result<(), String> {
        let limits = &self.limits;
        let mine = || st.setups.values().filter(|h| h.record.owner == owner);
        let is_open = |h: &&Held| h.record.status.is_open();
        if mine().filter(is_open).count() >= limits.open_per_owner {
            return Err(format!(
                "Limit reached: at most {} open setups per owner",
                limits.open_per_owner
            ));
        }
        if mine().count() >= limits.setups_per_owner {
            return Err(format!(
                "Limit reached: at most {} setups per owner, closed ones included",
                limits.setups_per_owner
            ));
        }
        if st.setups.values().filter(is_open).count() >= limits.open_total {
            return Err(format!(
                "Limit reached: this server holds its maximum of {} open setups",
                limits.open_total
            ));
        }
        if st.setups.len() >= limits.setups_total {
            return Err(format!(
                "Limit reached: this server holds its maximum of {} setups",
                limits.setups_total
            ));
        }
        let symbols: HashSet<&str> = mine()
            .filter(is_open)
            .map(|h| h.record.symbol.as_str())
            .collect();
        if !symbols.contains(symbol) && symbols.len() >= limits.symbols_per_owner {
            return Err(format!(
                "Limit reached: open setups on at most {} symbols per owner",
                limits.symbols_per_owner
            ));
        }
        let Some(hub) = &self.hub else {
            return Ok(());
        };
        let feed = match market {
            Some(_) => (symbol.to_string(), FEED_INTERVAL.to_string()),
            None => self.feed(symbol)?,
        };
        let mut pairs = self.wanted_feeds(st);
        pairs.extend(alert_pairs.iter().cloned());
        if !pairs.contains(&feed) && pairs.len() >= limits.pairs_total {
            return Err(format!(
                "Limit reached: this server watches at most {} symbols for alerts and setups",
                limits.pairs_total
            ));
        }
        if !st.watches.contains_key(&feed) && market.is_none() {
            hub.feed_room(&feed).map_err(|_| {
                "Limit reached: this server is starting too many feeds; try again in a minute"
                    .to_string()
            })?;
        }
        Ok(())
    }

    /// Would `record` with `log` added fit the byte limits?
    fn check_bytes(&self, st: &State, record: &Record, log: &[Revision]) -> Result<(), String> {
        let limits = &self.limits;
        let record_bytes = json_len(record);
        if record_bytes > limits.record_bytes {
            return Err(format!(
                "Limit reached: a setup holds at most {} bytes; shorten the thesis or use fewer drawings",
                limits.record_bytes
            ));
        }
        let mut added = 0;
        for entry in log {
            let bytes = json_len(entry);
            if bytes > limits.revision_bytes {
                return Err(format!(
                    "Limit reached: one revision holds at most {} bytes",
                    limits.revision_bytes
                ));
            }
            added += bytes;
        }
        let key = (record.owner.clone(), record.id.clone());
        let held = |h: &Held| h.record_bytes + h.log_bytes;
        let before = st.setups.get(&key).map_or(0, |h| h.record_bytes);
        let grows = (record_bytes + added).saturating_sub(before);
        let of_owner: usize = st
            .setups
            .values()
            .filter(|h| h.record.owner == record.owner)
            .map(held)
            .sum();
        if of_owner + grows > limits.bytes_per_owner {
            return Err(format!(
                "Limit reached: an owner's setups and their logs hold at most {} bytes",
                limits.bytes_per_owner
            ));
        }
        let total: usize = st.setups.values().map(held).sum();
        if total + grows > limits.bytes_total {
            return Err(format!(
                "Limit reached: this server's setups hold at most {} bytes",
                limits.bytes_total
            ));
        }
        Ok(())
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
            return tool_err(id, "Setups need an authenticated OAuth client");
        };
        let result = self.now(args, owner).and_then(|now| match tool {
            "create_setup" => self.create(args, owner, now),
            "revise_setup" => self.revise(args, owner, now),
            "get_setup" => self.get(args, owner),
            "list_setups" => self.list(args, owner),
            "get_track_record" => self.track_record(args, owner, now),
            "cancel_setup" => self.cancel(args, owner, now),
            _ => Err(format!("Unknown tool: {tool}")),
        });
        match result {
            Ok(payload) => tool_ok(id, payload),
            Err(message) => tool_err(id, &message),
        }
    }

    fn create(&self, args: &Value, owner: &str, now: i64) -> Result<Value, String> {
        if !self.access.may_create(owner) {
            return Err("This server only keeps setups for sign-ins its operator allowed".into());
        }
        self.admit_change(owner)?;
        let mut problems = Vec::new();
        let symbol = args
            .get("symbol")
            .and_then(Value::as_str)
            .and_then(normalize_quote_symbol);
        // A symbol read from Yahoo is a stock's, with its costs and sessions (ADR-0070).
        let mut market = symbol
            .as_deref()
            .and_then(|s| self.market_on(&self.lock(), s));
        let draft = Draft::parse_for(args, market, now, &mut problems);
        if let (Some(given), None) = (args.get("symbol").and_then(Value::as_str), &symbol) {
            problems.push(
                Problem::new("symbol", format!("{given:?} is not a symbol")).hint(
                    "a Binance symbol, such as BTCUSDT, or a US stock or ETF, such as NVDA or SPY",
                ),
            );
        }
        if let (Some(market), Some(levels)) =
            (market, draft.as_ref().and_then(|d| d.levels.as_ref()))
        {
            problems.extend(stocks::calendar_problem(market, levels.expires_at));
        }
        let requests = self.drawing_requests(args, now, &mut problems);
        let (Some(draft), Some(symbol), true) = (draft, symbol, problems.is_empty()) else {
            return Err(setups::problems_text(&problems));
        };

        let alert_pairs = self.alert_pairs();
        let new_feed = {
            let st = self.lock();
            self.check_capacity(&st, owner, &symbol, market, &alert_pairs)?;
            let feed = match market {
                Some(_) => (symbol.clone(), FEED_INTERVAL.to_string()),
                None => self.feed(&symbol)?,
            };
            !st.watches.contains_key(&feed)
        };
        let as_stock = |symbol: &str| {
            fetch::check_us_stock(self.source, symbol).map_err(|(message, hint)| {
                setups::problems_text(&[Problem::new("symbol", message).hint(hint)])
            })
        };
        match (&self.hub, new_feed, market) {
            (Some(_), true, Some(_)) => as_stock(&symbol)?,
            (Some(hub), true, None) => {
                if let Err(e) = hub.check_symbol(&symbol, FEED_INTERVAL) {
                    // A symbol the quote suffix sends to Binance that Binance does not
                    // list may be a US stock, such as ABNB (ADR-0070).
                    as_stock(&symbol).map_err(|_| format!("Cannot watch {symbol}: {e}"))?;
                    market = Some(setups::Market::UsEquity);
                }
            }
            _ => {}
        }
        let needs_bars = requests.is_some() || draft.decision == Decision::Skip;
        let (bars, closed) = match needs_bars {
            true => self.context(&symbol, &draft.interval, now)?,
            false => (Vec::new(), 0),
        };
        let placed = match requests {
            Some(requests) => Self::place(requests, &bars)?,
            None => Vec::new(),
        };
        let (levels, rule) = match &draft.levels {
            Some(levels) => (levels.clone(), setups::Bracket::Agent),
            None => {
                let secs = chartgen::data::interval_secs(&draft.interval).unwrap_or(60);
                let bars = &bars[..closed];
                match market {
                    Some(m) => {
                        setups::stock_skip_bracket(m, draft.direction, bars, &draft.interval, now)
                    }
                    None => setups::skip_bracket(draft.direction, bars, secs, now),
                }
                .map_err(|p| setups::problems_text(&[p]))?
            }
        };
        // A skip's expiry, and a setup found to be a stock only now, are checked here.
        if let Some(problem) = market.and_then(|m| stocks::calendar_problem(m, levels.expires_at)) {
            return Err(setups::problems_text(&[problem]));
        }

        // The read of bars ran unlocked: another call may have taken the last pair. From
        // here to the write nobody may, alerts included, whose pairs are read afresh.
        let _pairs = crate::server_alerts::pair_gate();
        let alert_pairs = self.alert_pairs();
        #[cfg(test)]
        if let Some(hook) = self.before_final_check.lock().unwrap().as_ref() {
            hook();
        }
        let mut st = self.lock();
        self.check_capacity(&st, owner, &symbol, market, &alert_pairs)?;
        let id = self.new_id(&mut st, owner);
        let (mut record, created) = Record::new(id, owner, symbol, draft, levels, placed, now);
        record.bracket = rule;
        if let Some(market) = market {
            record.market = Some(market);
            record.costs = market.costs();
        }
        let log = vec![created];
        self.check_bytes(&st, &record, &log)?;
        let view = self.view(&record, &log);
        self.write(&mut st, vec![(record, log)])?;
        self.sync_feeds(&mut st);
        Ok(view)
    }

    /// The drawings of a call, shapes checked, before any bars are read.
    fn drawing_requests(
        &self,
        args: &Value,
        now: i64,
        problems: &mut Vec<Problem>,
    ) -> Option<Vec<drawings::Request>> {
        let given = args.get("drawings").filter(|v| !v.is_null())?;
        match drawings::parse(given, now) {
            Ok(requests) => Some(requests),
            Err(bad) => {
                problems.extend(bad.iter().map(Problem::of_drawing));
                None
            }
        }
    }

    fn alert_pairs(&self) -> HashSet<FeedKey> {
        self.alerts
            .as_ref()
            .map(|alerts| alerts.pairs())
            .unwrap_or_default()
    }

    /// A copy of the caller's setup. Another owner's reads as one that does not exist.
    fn find(&self, st: &State, args: &Value, owner: &str) -> Result<Record, String> {
        let id = args.get("id").and_then(Value::as_str).unwrap_or("");
        if id.is_empty() {
            return Err(field_problem("id", "id is required"));
        }
        st.setups
            .get(&(owner.to_string(), id.to_string()))
            .map(|h| h.record.clone())
            .ok_or_else(|| format!("No setup with id {id}"))
    }

    fn revise(&self, args: &Value, owner: &str, now: i64) -> Result<Value, String> {
        self.admit_change(owner)?;
        let current = self.find(&self.lock(), args, owner)?;
        let mut problems = Vec::new();
        let mut allowed = vec!["id", "reason", "thesis", "drawings", "tags", "as_of"];
        allowed.extend(LEVEL_FIELDS);
        setups::check_fields(args, &allowed, &mut problems);
        let reason = setups::reason(args, &mut problems);
        let given = |field: &str| args.get(field).is_some_and(|v| !v.is_null());
        // What the setup's state forbids is answered as that, and what was sent for it is
        // not judged: its problems would be about a change that cannot be made.
        let level_change = LEVEL_FIELDS.iter().any(|f| given(f));
        let tags_only = given("tags") && !given("thesis") && !given("drawings") && !level_change;
        let refusal = current.revise_refusal(level_change, tags_only);
        let frozen = current.takes_no_change();
        let thesis = match given("thesis") && !frozen {
            true => setups::text(args, "thesis", MAX_THESIS, &mut problems),
            false => None,
        };
        let tags = match given("tags") && (!frozen || tags_only) {
            true => setups::tags(&args["tags"], &mut problems),
            false => None,
        };
        let levels = match level_change && refusal.is_none() {
            true => setups::parse_levels(
                args,
                Some(current.latest_levels()),
                Some(current.direction),
                &current.costs,
                now,
                &mut problems,
            ),
            false => None,
        };
        if let (Some(market), Some(levels)) = (current.market, &levels) {
            problems.extend(stocks::calendar_problem(market, levels.expires_at));
        }
        let requests = match frozen {
            true => None,
            false => self.drawing_requests(args, now, &mut problems),
        };
        problems.extend(refusal);
        let (Some(reason), true) = (reason, problems.is_empty()) else {
            return Err(setups::problems_text(&problems));
        };
        let placed = match requests {
            Some(requests) => {
                let (bars, _) = self.context(&current.symbol, &current.interval, now)?;
                Some(Self::place(requests, &bars)?)
            }
            None => None,
        };

        let mut st = self.lock();
        let mut record = self.find(&st, args, owner)?;
        // Review tags on a closed setup do not count against its revisions: they come after.
        let review_tags = tags_only && !record.status.is_open();
        if record.agent_revisions >= self.limits.agent_revisions_per_setup && !review_tags {
            return Err(format!(
                "Limit reached: at most {} revisions per setup",
                self.limits.agent_revisions_per_setup
            ));
        }
        let entry = record
            .revise(thesis, placed, tags, levels, reason, now)
            .map_err(|problems| setups::problems_text(&problems))?;
        self.finish(&mut st, record, entry)
    }

    fn cancel(&self, args: &Value, owner: &str, now: i64) -> Result<Value, String> {
        self.admit_change(owner)?;
        let mut problems = Vec::new();
        setups::check_fields(args, &["id", "reason", "as_of"], &mut problems);
        let reason = setups::reason(args, &mut problems);
        let (Some(reason), true) = (reason, problems.is_empty()) else {
            return Err(setups::problems_text(&problems));
        };
        let mut st = self.lock();
        let mut record = self.find(&st, args, owner)?;
        let entry = record
            .cancel(reason, now)
            .map_err(|problems| setups::problems_text(&problems))?;
        self.finish(&mut st, record, entry)
    }

    /// Store an agent's change and answer with the setup and its whole log.
    fn finish(&self, st: &mut State, record: Record, entry: Revision) -> Result<Value, String> {
        let log = vec![entry];
        self.check_bytes(st, &record, &log)?;
        let key = (record.owner.clone(), record.id.clone());
        self.write(st, vec![(record, log)])?;
        let record = &st.setups[&key].record;
        Ok(self.view(record, &self.log_of(record)))
    }

    fn log_of(&self, record: &Record) -> Vec<Revision> {
        self.store
            .setup_revisions(&self.row_id(record))
            .unwrap_or_else(|e| {
                eprintln!("[setups] cannot read a setup's log: {e}");
                Vec::new()
            })
            .iter()
            .filter_map(|body| serde_json::from_str(body).ok())
            .collect()
    }

    /// A setup with its log, as `get_setup` answers.
    fn view(&self, record: &Record, log: &[Revision]) -> Value {
        let mut view = record.view();
        view["log"] = json!(log);
        view
    }

    fn get(&self, args: &Value, owner: &str) -> Result<Value, String> {
        let record = self.find(&self.lock(), args, owner)?;
        Ok(self.view(&record, &self.log_of(&record)))
    }

    /// The caller's setups, newest first, without their logs.
    fn list(&self, args: &Value, owner: &str) -> Result<Value, String> {
        let status = match args.get("status").filter(|v| !v.is_null()) {
            None => None,
            Some(v) => match v.as_str() {
                Some("open") => Some(None),
                Some("closed") => Some(Some(None)),
                _ => Some(Some(Some(
                    serde_json::from_value::<Status>(v.clone()).map_err(|_| {
                        field_problem("status", "status must be open, closed or a setup status")
                    })?,
                ))),
            },
        };
        let symbol = match args.get("symbol").filter(|v| !v.is_null()) {
            None => None,
            Some(v) => Some(
                v.as_str()
                    .and_then(normalize_quote_symbol)
                    .ok_or_else(|| field_problem("symbol", "symbol must be a symbol"))?,
            ),
        };
        let tags = match args.get("tags").filter(|v| !v.is_null()) {
            None => Vec::new(),
            Some(v) => {
                let mut problems = Vec::new();
                setups::tags(v, &mut problems).ok_or_else(|| setups::problems_text(&problems))?
            }
        };
        let limit = match args.get("limit").filter(|v| !v.is_null()) {
            None => 50,
            Some(v) => v
                .as_u64()
                .filter(|n| (1..=500).contains(n))
                .ok_or_else(|| field_problem("limit", "limit must be 1 to 500"))?
                as usize,
        };
        let st = self.lock();
        let mut mine: Vec<&Record> = st
            .setups
            .values()
            .map(|h| &h.record)
            .filter(|r| r.owner == owner)
            .filter(|r| symbol.as_ref().is_none_or(|s| *s == r.symbol))
            .filter(|r| tags.iter().all(|t| r.tags.contains(t)))
            .filter(|r| match status {
                None => true,
                Some(None) => r.status.is_open(),
                Some(Some(None)) => !r.status.is_open(),
                Some(Some(Some(status))) => r.status == status,
            })
            .collect();
        mine.sort_by(|a, b| (b.created_at, &b.id).cmp(&(a.created_at, &a.id)));
        let total = mine.len();
        let views: Vec<Value> = mine.into_iter().take(limit).map(Record::view).collect();
        Ok(json!({
            "enabled": true,
            "can_create": self.access.may_create(owner),
            "replay": self.replay.is_some(),
            "total": total,
            "limits": {
                "open_setups": self.limits.open_per_owner,
                "setups": self.limits.setups_per_owner,
                "symbols": self.limits.symbols_per_owner,
                "revisions_per_setup": self.limits.agent_revisions_per_setup,
                "changes_per_minute": self.limits.changes_per_minute,
                "bytes": self.limits.bytes_per_owner,
            },
            "setups": views,
        }))
    }

    /// The caller's closed setups, counted: in all, and grouped by one thing they were
    /// made with (ADR-0064). It reports counts and sums and judges nothing.
    fn track_record(&self, args: &Value, owner: &str, now: i64) -> Result<Value, String> {
        const FIELDS: [&str; 9] = [
            "symbol",
            "direction",
            "decision",
            "playbook_version",
            "from",
            "to",
            "group_by",
            "feature",
            "as_of",
        ];
        let given = |field: &str| args.get(field).filter(|v| !v.is_null());
        if let Some(unknown) = args
            .as_object()
            .and_then(|map| map.keys().find(|k| !FIELDS.contains(&k.as_str())))
        {
            return Err(read_problem(
                unknown,
                format!("{unknown} is not a field of this tool"),
            ));
        }
        let symbol = match given("symbol") {
            None => None,
            Some(v) => Some(
                v.as_str()
                    .and_then(normalize_quote_symbol)
                    .ok_or_else(|| read_problem("symbol", "symbol must be a symbol"))?,
            ),
        };
        fn one_of<T: serde::de::DeserializeOwned>(
            given: Option<&Value>,
            field: &str,
            values: &str,
        ) -> Result<Option<T>, String> {
            given
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()
                .map_err(|_| read_problem(field, format!("{field} must be {values}")))
        }
        let direction: Option<setups::Direction> =
            one_of(given("direction"), "direction", "long or short")?;
        let decision: Option<Decision> = one_of(given("decision"), "decision", "take or skip")?;
        let text = |field: &str| match given(field) {
            None => Ok(None),
            Some(v) => match v.as_str() {
                Some(s) if s.chars().count() <= setups::MAX_STATED_TEXT => Ok(Some(s.to_string())),
                _ => Err(read_problem(
                    field,
                    format!(
                        "{field} must be a string of at most {} characters",
                        setups::MAX_STATED_TEXT
                    ),
                )),
            },
        };
        let playbook_version = text("playbook_version")?;
        let feature = text("feature")?;
        let time = |field: &str| {
            given(field)
                .map(|v| drawings::parse_time(v, now).map_err(|e| read_problem(field, e)))
                .transpose()
        };
        let (from, to) = (time("from")?, time("to")?);
        let group_by =
            match (given("group_by").map(Value::as_str), feature) {
                (None, None) => None,
                (None, Some(_)) => {
                    return Err(read_problem("feature", "feature needs group_by: feature"))
                }
                (Some(Some("feature")), Some(name)) => Some(setups::GroupBy::Feature(name)),
                (Some(Some("feature")), None) => {
                    return Err(read_problem(
                        "feature",
                        "group_by: feature needs feature, the name of one",
                    ))
                }
                (Some(_), Some(_)) => {
                    return Err(read_problem("feature", "feature needs group_by: feature"))
                }
                (Some(Some("playbook_version")), None) => Some(setups::GroupBy::PlaybookVersion),
                (Some(Some("probability")), None) => Some(setups::GroupBy::Probability),
                (Some(Some("symbol")), None) => Some(setups::GroupBy::Symbol),
                (Some(Some("direction")), None) => Some(setups::GroupBy::Direction),
                (Some(_), None) => return Err(read_problem(
                    "group_by",
                    "group_by must be playbook_version, feature, probability, symbol or direction",
                )),
            };

        let st = self.lock();
        let mut mine: Vec<&Record> = st
            .setups
            .values()
            .map(|h| &h.record)
            .filter(|r| r.owner == owner)
            .filter(|r| symbol.as_ref().is_none_or(|s| *s == r.symbol))
            .filter(|r| direction.is_none_or(|d| d == r.direction))
            .filter(|r| decision.is_none_or(|d| d == r.decision))
            .filter(|r| {
                playbook_version
                    .as_ref()
                    .is_none_or(|v| r.playbook_version.as_ref() == Some(v))
            })
            .filter(|r| from.is_none_or(|t| r.created_at >= t))
            .filter(|r| to.is_none_or(|t| r.created_at < t))
            .collect();
        let open = mine.iter().filter(|r| r.status.is_open()).count();
        mine.retain(|r| !r.status.is_open());
        // Newest first, so that what is left out, if anything is, is the oldest.
        mine.sort_by(|a, b| (b.created_at, &b.id).cmp(&(a.created_at, &a.id)));
        let truncated = mine.len() > TRACK_RECORD_SETUPS;
        mine.truncate(TRACK_RECORD_SETUPS);
        let (total, groups) = setups::track_record(&mine, group_by.as_ref())
            .map_err(|e| read_problem("group_by", e))?;
        Ok(json!({
            "replay": self.replay.is_some(),
            "closed": mine.len(),
            "open": open,
            "truncated": truncated,
            "group_by": given("group_by"),
            "feature": given("feature"),
            "total": total,
            "groups": groups,
            "shadows": setups::shadow_record(&mine),
        }))
    }

    /// The closed 1m bars of `symbol` from `from` up to, not including, `to`: what the
    /// server missed while it was down or its feed was. One read, through the same
    /// gate as a chart's (ADR-0042).
    fn missed(&self, symbol: &str, from: i64, to: i64) -> Result<Vec<MinuteBar>, String> {
        let wanted = ((to - from) / MINUTE_MS).clamp(1, CATCH_UP_BARS) as usize;
        let fetched = fetch::fetch_ohlcv(self.source, symbol, FEED_INTERVAL, wanted, Some(to))?;
        Ok(fetched
            .data
            .bars
            .iter()
            .filter_map(MinuteBar::of)
            .filter(|b| b.time >= from && b.time < to)
            .collect())
    }

    /// A closed 1m bar of `key`: move every open setup on it. A setup that is behind
    /// first gets the bars it missed; where those cannot be had, its log says which
    /// stretch was not scored. Blocking.
    fn on_bar(&self, key: &FeedKey, bar: MinuteBar) {
        let on_feed = |r: &Record| {
            self.runs(r)
                && r.market.is_none()
                && self.feed(&r.symbol).is_ok_and(|feed| feed == *key)
        };
        let behind = self
            .lock()
            .setups
            .values()
            .map(|h| &h.record)
            .filter(|r| on_feed(r) && r.next_bar_time() < bar.time)
            .map(Record::next_bar_time)
            .min();
        let missed = match behind {
            Some(from) => match self.missed(&key.0, from, bar.time) {
                Ok(bars) => {
                    println!("{}", missed_note(bars.len(), &key.0));
                    bars
                }
                Err(e) => {
                    eprintln!("[setups] cannot read missed bars of {}: {e}", key.0);
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        let now = fetch::now_ms();
        let mut st = self.lock();
        let open: Vec<Record> = st
            .setups
            .values()
            .map(|h| &h.record)
            .filter(|r| on_feed(r))
            .cloned()
            .collect();
        let mut changed = Vec::new();
        for mut record in open {
            let before = record.clone();
            let mut log = Vec::new();
            for b in missed.iter().chain([&bar]) {
                let next = record.next_bar_time();
                if !record.is_scoring() || b.time < next {
                    continue;
                }
                if b.time > next {
                    log.push(record.note_gap(next, b.time, now));
                }
                log.extend(record.apply_bar(b));
            }
            if record != before {
                changed.push((record, log));
            }
        }
        if self.write(&mut st, changed).is_ok() {
            self.sync_feeds(&mut st);
        }
    }
}

/// The log line of a catch-up read that worked, so an operator can see after a restart
/// that the bars a setup missed were read, and how many.
fn missed_note(bars: usize, symbol: &str) -> String {
    format!("[setups] read {bars} missed 1m bars of {symbol}")
}

/// Could this row have been written here? A row that could not is never scored.
fn sound(record: &Record) -> bool {
    let levels = &record.levels;
    let prices = [levels.entry.low, levels.entry.high, levels.invalidation];
    !record.id.is_empty()
        && !record.owner.is_empty()
        && record.revisions > 0
        && prices
            .iter()
            .chain(&levels.targets)
            .all(|p| p.is_finite() && *p > 0.0)
        && levels.entry.low <= levels.entry.high
        && !levels.targets.is_empty()
        && (record.fill.is_some() || record.exit.is_none())
        // A closed setup has a result, but for one that expired filled and unscored.
        && match (record.status.is_open(), record.result_r) {
            (true, result) => result.is_none(),
            (false, Some(_)) => true,
            (false, None) => record.gap && record.fill.is_some(),
        }
        && (record.status != Status::Triggered || record.fill.is_some())
}

/// The length of a value's JSON.
fn json_len(value: &impl serde::Serialize) -> usize {
    serde_json::to_string(value).map_or(0, |s| s.len())
}

async fn run_watch(setups: Weak<Setups>, key: FeedKey, mut events: broadcast::Receiver<FeedEvent>) {
    loop {
        let event = events.recv().await;
        let Some(this) = setups.upgrade() else {
            return;
        };
        match event {
            Ok(FeedEvent::Bar(bar)) => {
                if let Some(bar) = MinuteBar::of(&bar) {
                    let key = key.clone();
                    let _ = tokio::task::spawn_blocking(move || this.on_bar(&key, bar)).await;
                }
            }
            // A setup moves on closed bars only, and reads the ones it missed.
            Ok(FeedEvent::Tick(_)) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => {
                eprintln!("[setups] feed {} {} ended", key.0, key.1);
                drop(this);
                tokio::time::sleep(FEED_RETRY).await;
                if let Some(this) = setups.upgrade() {
                    this.rejoin(&key);
                }
                return;
            }
        }
    }
}

/// Try again to join feeds that were refused.
async fn run_upkeep(setups: Weak<Setups>) {
    loop {
        tokio::time::sleep(UPKEEP_EVERY).await;
        let Some(this) = setups.upgrade() else {
            return;
        };
        let this2 = this.clone();
        let _ = tokio::task::spawn_blocking(move || {
            this2.expire_unscored(fetch::now_ms());
            this2.tidy_events(chrono::Utc::now().timestamp());
        })
        .await;
        this.sync_feeds(&mut this.lock());
    }
}

mod stocks;

#[cfg(test)]
mod tests;
