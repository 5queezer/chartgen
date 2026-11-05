//! User-authored indicators in Starlark: proposal 0001, ADR-0024 (runtime)
//! and ADR-0025 (per-user registry + MCP surface).
//!
//! A script defines `PARAMS`, `META` and `compute(ohlcv, params, chart)`.
//! Scripts are stored per user (`persistence`), compiled once per source
//! (LRU cache below) and run through `ScriptIndicator`, which implements the
//! `Indicator` trait like any built-in.

pub mod alloc;
pub mod chart_builder;
pub mod persistence;
pub mod runtime;
pub mod series;
pub mod ta;
#[cfg(test)]
mod tests;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::data::OhlcvData;
pub use crate::indicator::IndicatorError as ScriptError;
use crate::indicator::{Indicator, PanelResult};
pub use runtime::{Compiled, ScriptInfo};

/// Scripting is opt-in: `CHARTGEN_SCRIPTING=1` lists and serves the
/// scripting MCP tools and resolves user scripts in `generate_chart`.
pub fn enabled() -> bool {
    std::env::var("CHARTGEN_SCRIPTING").is_ok_and(|v| v == "1")
}

/// Most scripts one user may store.
pub const MAX_SCRIPTS_PER_USER: usize = 50;

/// `^[a-z][a-z0-9_]{0,47}$`: the same shape as built-in names.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && name.len() <= 48
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

pub fn source_hash(source: &str) -> String {
    hex::encode(Sha256::digest(source.as_bytes()))
}

/// The full register-time pipeline: parse → static check → presence check
/// (all in `runtime::compile`) → dry run on synthetic bars within 200 ms.
///
/// Param names that collide with other indicator-entry keys (`id`, `hidden`,
/// `style`; the wire strips or consumes them, so they can never be set) are
/// rejected here only: stored scripts that already use them still compile.
pub fn validate(name: &str, source: &str) -> Result<Compiled, ScriptError> {
    let compiled = runtime::compile(name, source)?;
    for p in &compiled.info.params {
        let n = p["name"].as_str().unwrap_or_default();
        if matches!(n, "id" | "hidden" | "style") {
            return Err(ScriptError::new(
                "presence",
                None,
                format!("PARAMS: a param cannot be called \"{n}\" (reserved)"),
            ));
        }
    }
    let sample = crate::data::sample_data(runtime::DRY_RUN_BARS);
    runtime::run(
        &compiled,
        &sample,
        &json!({}),
        runtime::DRY_RUN_TIMEOUT,
        "dry_run",
    )?;
    Ok(compiled)
}

// ---- Concurrency: at most 4 evaluations at once, one per user ----

/// Script evaluations (compile, dry run or compute) running at once.
const MAX_CONCURRENT: usize = 4;
/// Scripted indicators one chart request may compute.
pub const MAX_SCRIPTS_PER_CHART: usize = 3;
/// Wall-clock budget shared by every script of one chart request.
pub const CHART_BUDGET: Duration = runtime::COMPUTE_TIMEOUT;
/// How long registering waits for a free slot.
const REGISTER_WAIT: Duration = Duration::from_secs(1);

#[derive(Default)]
struct Slots {
    running: usize,
    users: HashSet<String>,
}

fn slots() -> &'static (Mutex<Slots>, Condvar) {
    static S: OnceLock<(Mutex<Slots>, Condvar)> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// A held evaluation slot; dropping it frees the slot.
struct Permit(String);

impl Drop for Permit {
    fn drop(&mut self) {
        let (m, cv) = slots();
        let mut s = m.lock().unwrap_or_else(|e| e.into_inner());
        s.running -= 1;
        s.users.remove(&self.0);
        cv.notify_all();
    }
}

/// Wait until fewer than `MAX_CONCURRENT` evaluations run and none of
/// `user`'s, or fail at `deadline`.
fn acquire(user: &str, deadline: Instant, step: &'static str) -> Result<Permit, ScriptError> {
    let (m, cv) = slots();
    let mut s = m.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if s.running < MAX_CONCURRENT && !s.users.contains(user) {
            s.running += 1;
            s.users.insert(user.to_owned());
            return Ok(Permit(user.to_owned()));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ScriptError::new(
                step,
                None,
                "server busy: too many scripts running (one at a time per user); try again",
            ));
        }
        s = cv
            .wait_timeout(s, left)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
}

/// `validate` while holding one of `user`'s evaluation slots.
pub fn validate_as(user: &str, name: &str, source: &str) -> Result<Compiled, ScriptError> {
    let _permit = acquire(user, Instant::now() + REGISTER_WAIT, "dry_run")?;
    validate(name, source)
}

thread_local! {
    /// Deadline and scripts started of the chart request on this thread.
    static CHART: Cell<Option<(Instant, usize)>> = const { Cell::new(None) };
}

/// Run `f` (one chart request's indicator computations) with one shared
/// script budget: `CHART_BUDGET` of wall clock and `MAX_SCRIPTS_PER_CHART`
/// scripts, over all its scripted indicators together.
pub fn with_chart_budget<R>(f: impl FnOnce() -> R) -> R {
    CHART.with(|c| c.set(Some((Instant::now() + CHART_BUDGET, 0))));
    let out = f();
    CHART.with(|c| c.set(None));
    out
}

// ---- Compile cache: (user, name, source_hash) → frozen module, LRU ----

const CACHE_SIZE: usize = 1024;
/// Memory the cached modules may hold in total.
const CACHE_BYTES: usize = 64 << 20;
type CacheKey = (String, String, String);

#[derive(Default)]
struct Cache {
    map: HashMap<CacheKey, (Arc<Compiled>, u64)>,
    clock: u64,
    bytes: usize,
}

fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(Mutex::default)
}

/// Compile `source` or reuse the cached module for the same key.
pub fn compiled(user: &str, name: &str, source: &str) -> Result<Arc<Compiled>, ScriptError> {
    let key = (user.to_owned(), name.to_owned(), source_hash(source));
    {
        let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
        c.clock += 1;
        let now = c.clock;
        if let Some(entry) = c.map.get_mut(&key) {
            entry.1 = now;
            return Ok(entry.0.clone());
        }
    }
    let fresh = {
        let _permit = acquire(user, Instant::now() + REGISTER_WAIT, "static")?;
        Arc::new(runtime::compile(name, source)?)
    };
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    while !c.map.is_empty() && (c.map.len() >= CACHE_SIZE || c.bytes + fresh.bytes > CACHE_BYTES) {
        let oldest = c
            .map
            .iter()
            .min_by_key(|(_, v)| v.1)
            .map(|(k, _)| k.clone());
        if let Some((old, _)) = oldest.and_then(|k| c.map.remove(&k)) {
            c.bytes -= old.bytes;
        }
    }
    let now = c.clock;
    c.bytes += fresh.bytes;
    if let Some((old, _)) = c.map.insert(key, (fresh.clone(), now)) {
        c.bytes -= old.bytes;
    }
    Ok(fresh)
}

// ---- Indicator adapter ----

/// A user script behind the `Indicator` trait. A script that no longer
/// compiles still resolves, so the chart reports its error instead of
/// "unknown indicator".
pub struct ScriptIndicator {
    user: String,
    name: String,
    compiled: Result<Arc<Compiled>, ScriptError>,
    params: Value,
}

impl ScriptIndicator {
    pub fn new(user: &str, name: &str, source: &str) -> Self {
        Self {
            user: user.to_owned(),
            name: name.to_owned(),
            compiled: compiled(user, name, source),
            params: json!({}),
        }
    }
}

impl Indicator for ScriptIndicator {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        match &self.compiled {
            Ok(c) if !c.info.description.is_empty() => &c.info.description,
            Ok(c) => &c.info.title,
            Err(_) => "",
        }
    }

    fn params(&self) -> Value {
        match &self.compiled {
            Ok(c) => json!(c.info.params),
            Err(_) => json!([]),
        }
    }

    fn configure(&mut self, params: &Value) {
        self.params = params.clone();
    }

    fn compute(&self, data: &OhlcvData) -> PanelResult {
        self.try_compute(data).unwrap_or_else(|_| PanelResult {
            label: self.name.clone(),
            ..Default::default()
        })
    }

    fn try_compute(&self, data: &OhlcvData) -> Result<PanelResult, ScriptError> {
        let compiled = self.compiled.as_ref().map_err(Clone::clone)?;
        let fail = |msg: String| Err(ScriptError::new("runtime", None, msg));
        // Outside a chart request (e.g. `compute` called directly) the
        // script gets its own budget.
        let (deadline, started) = CHART
            .with(Cell::get)
            .unwrap_or((Instant::now() + runtime::COMPUTE_TIMEOUT, 0));
        if started >= MAX_SCRIPTS_PER_CHART {
            return fail(format!(
                "at most {MAX_SCRIPTS_PER_CHART} scripted indicators per chart"
            ));
        }
        CHART.with(|c| c.set(c.get().map(|(d, n)| (d, n + 1))));
        let _permit = acquire(&self.user, deadline, "runtime")?;
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return fail("time limit exceeded (the chart's script budget is used up)".into());
        }
        runtime::run(compiled, data, &self.params, left, "runtime")
    }
}

/// `user`'s script `name` as an indicator, if they stored one.
pub fn lookup(user: &str, name: &str) -> Option<Box<dyn Indicator>> {
    let row = persistence::global()?.get(user, name).ok()??;
    Some(Box::new(ScriptIndicator::new(user, name, &row.source)))
}

// ---- Reference implementations of forkable built-ins ----

/// Built-ins with a `.star` reference implementation, parity-tested against
/// the Rust indicator (`fork_indicator` returns it).
pub const REFIMPLS: &[(&str, &str)] = &[("wavetrend", include_str!("refimpl/wavetrend.star"))];

pub fn refimpl(builtin: &str) -> Option<&'static str> {
    REFIMPLS
        .iter()
        .find(|(n, _)| *n == builtin)
        .map(|(_, src)| *src)
}
