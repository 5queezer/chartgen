//! Symbol search for the `search_symbols` MCP tool: every Binance spot pair
//! (exchange info cached for 6 h, searched locally) plus Yahoo Finance's
//! search endpoint (cached per query for 60 s), ranked like TradingView:
//! exact symbol or root, then prefix, then contains, then name match.
//!
//! Upstream protection: concurrent identical lookups share one request
//! (singleflight), failures are cached briefly, Binance data is served stale
//! while it refreshes, Yahoo is limited by a token bucket, and each source
//! has its own deadline so a slow one cannot hold back the other.

use crate::fetch::{self, DataSource};
use serde::Deserialize;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const DEFAULT_LIMIT: usize = 30;
pub const MAX_LIMIT: usize = 50;
const MAX_QUERY_LEN: usize = 40;
const BINANCE_TTL_MS: i64 = 6 * 60 * 60 * 1000;
const YAHOO_TTL_MS: i64 = 60_000;
/// How long a failed lookup is remembered instead of retried.
const ERROR_TTL_MS: i64 = 60_000;
/// Distinct Yahoo queries kept; the oldest entry goes first when full.
const YAHOO_CACHE_MAX: usize = 500;
/// How long one source may take before the call answers without it.
const SOURCE_DEADLINE: Duration = Duration::from_secs(4);
/// Yahoo search requests per second (and burst size) across all clients.
const YAHOO_RATE_PER_SEC: f64 = 2.0;
const YAHOO_BURST: f64 = 2.0;
pub const TYPES: [&str; 7] = ["crypto", "stock", "etf", "index", "forex", "future", "fund"];
/// Quote currencies that make a crypto pair a fiat pair (EUR/USDT is a
/// currency, not a coin).
const FIAT: [&str; 22] = [
    "USD", "EUR", "GBP", "TRY", "BRL", "ARS", "JPY", "AUD", "CAD", "CHF", "RUB", "UAH", "PLN",
    "RON", "CZK", "MXN", "ZAR", "NGN", "COP", "IDR", "KRW", "SEK",
];

#[derive(Debug, Clone, PartialEq)]
pub struct SymbolResult {
    pub symbol: String,
    pub name: String,
    pub exchange: Cow<'static, str>,
    pub kind: &'static str,
    pub source: &'static str,
}

impl SymbolResult {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "symbol": self.symbol,
            "name": self.name,
            "exchange": self.exchange,
            "type": self.kind,
            "source": self.source,
        })
    }
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub results: Vec<SymbolResult>,
    pub warnings: Vec<String>,
}

/// Trim the query and collapse inner whitespace; 1–40 characters, no
/// control characters.
pub fn validate_query(raw: &str) -> Result<String, String> {
    let q = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if q.is_empty() || q.chars().count() > MAX_QUERY_LEN {
        return Err(format!("query must be 1-{MAX_QUERY_LEN} characters"));
    }
    if q.chars().any(char::is_control) {
        return Err("query must not contain control characters".into());
    }
    Ok(q)
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ExchangeInfo {
    symbols: Vec<ExchangeSymbol>,
}

#[derive(Deserialize)]
struct ExchangeSymbol {
    symbol: String,
    status: String,
    #[serde(rename = "baseAsset")]
    base: String,
    #[serde(rename = "quoteAsset")]
    quote: String,
    #[serde(rename = "isSpotTradingAllowed", default)]
    spot: bool,
}

/// TRADING spot pairs of a Binance `/api/v3/exchangeInfo` body whose symbol
/// fits the allowed charset.
fn parse_exchange_info(body: &str) -> Result<Vec<SymbolResult>, String> {
    let info: ExchangeInfo = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse Binance exchangeInfo: {e}"))?;
    Ok(info
        .symbols
        .into_iter()
        .filter(|s| s.status == "TRADING" && s.spot)
        .filter_map(|s| {
            Some(SymbolResult {
                name: format!("{} / {}", s.base, s.quote),
                symbol: crate::mcp::normalize_quote_symbol(&s.symbol)?,
                exchange: Cow::Borrowed("Binance"),
                kind: "crypto",
                source: "binance",
            })
        })
        .collect())
}

fn yahoo_kind(quote_type: &str) -> Option<&'static str> {
    match quote_type {
        "EQUITY" => Some("stock"),
        "ETF" => Some("etf"),
        "INDEX" => Some("index"),
        "CURRENCY" => Some("forex"),
        "FUTURE" => Some("future"),
        "MUTUALFUND" => Some("fund"),
        // CRYPTOCURRENCY ("BTC-USD") is Binance's job; options etc. have no chart.
        _ => None,
    }
}

/// Charting-capable instruments of a Yahoo `/v1/finance/search` body, in
/// Yahoo's order. Symbols are uppercased; those outside the allowed charset
/// are dropped.
fn parse_yahoo_search(body: &Value) -> Vec<SymbolResult> {
    let Some(quotes) = body.get("quotes").and_then(Value::as_array) else {
        return Vec::new();
    };
    let text = |q: &Value, k: &str| q.get(k).and_then(Value::as_str).map(str::to_string);
    quotes
        .iter()
        .filter_map(|q| {
            let kind = yahoo_kind(q.get("quoteType")?.as_str()?)?;
            let symbol = crate::mcp::normalize_quote_symbol(q.get("symbol")?.as_str()?)?;
            Some(SymbolResult {
                name: text(q, "longname")
                    .or_else(|| text(q, "shortname"))
                    .unwrap_or_default(),
                exchange: Cow::Owned(text(q, "exchDisp").unwrap_or_default()),
                symbol,
                kind,
                source: "yahoo",
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Ranking
// ---------------------------------------------------------------------------

/// What a query is compared with besides the full symbol: the base of a
/// crypto pair, the base currency of `XXXYYY=X`, the root of `ROOT=F` and
/// the name of `^NAME`.
fn root(r: &SymbolResult) -> &str {
    let s = r.symbol.as_str();
    match r.kind {
        "crypto" => r.name.split(" / ").next().unwrap_or(s),
        "forex" => s.strip_suffix("=X").map_or(s, |b| b.get(..3).unwrap_or(b)),
        "future" => s.strip_suffix("=F").unwrap_or(s),
        "index" => s.strip_prefix('^').unwrap_or(s),
        _ => s,
    }
}

/// Order of kinds among exact matches: coins, futures, forex, indices,
/// fiat "crypto" pairs, then listed securities.
fn kind_rank(r: &SymbolResult) -> u8 {
    match r.kind {
        "crypto" if FIAT.contains(&root(r)) => 4,
        "crypto" => 0,
        "future" => 1,
        "forex" => 2,
        "index" => 3,
        _ => 5,
    }
}

/// Crypto pairs of one base that rank among the other exact matches; the
/// rest of that base's pairs come after them (`BTC=F` and the BTC ETF
/// must not be buried under dozens of `BTC*` pairs).
const MAX_PAIRS_PER_BASE: usize = 4;

/// Kind rank of the pairs past the cap: behind every other exact match.
const OVER_CAP: u8 = 6;

/// Position of a quote among the preferred ones, `quotes.len()` for any other.
fn quote_rank(symbol: &str, quotes: &[&str]) -> u8 {
    quotes
        .iter()
        .position(|q| symbol.ends_with(q))
        .unwrap_or(quotes.len()) as u8
}

type RankKey = (u8, u8, u8, usize);

/// Sort key for `r` against the uppercased query `q`: match tier (0 exact
/// symbol or root, 1 prefix, 2 contains, 3 name match, 4 no match), kind
/// among exact matches, then USDT/USDC/FDUSD/USD quotes before other
/// quotes, then shorter symbols.
fn rank_key(r: &SymbolResult, q: &str) -> RankKey {
    let tier = if r.symbol == q || root(r) == q {
        0
    } else if r.symbol.starts_with(q) {
        1
    } else if r.symbol.contains(q) {
        2
    } else if r.name.to_uppercase().contains(q) {
        3
    } else {
        4
    };
    let quote = match r.kind {
        "crypto" => quote_rank(&r.symbol, &["USDT", "USDC", "FDUSD", "USD"]),
        "forex" => quote_rank(&r.symbol, &["USD=X"]),
        _ => 0,
    };
    (
        tier,
        if tier == 0 { kind_rank(r) } else { 0 },
        quote,
        if tier < 3 { r.symbol.len() } else { 0 },
    )
}

/// Demote the exact-match coin pairs past the cap per base. `ranked` must
/// be sorted by key; the caller sorts again afterwards.
fn cap_pairs_per_base(ranked: &mut [(RankKey, &SymbolResult)]) {
    let mut per_base: HashMap<&str, usize> = HashMap::new();
    for (k, r) in ranked.iter_mut() {
        if k.0 == 0 && k.1 == 0 {
            let n = per_base.entry(root(r)).or_default();
            *n += 1;
            if *n > MAX_PAIRS_PER_BASE {
                k.1 = OVER_CAP;
            }
        }
    }
}

/// Union of all sources ranked by `rank_key` (ties keep source order, so
/// upstream relevance survives), duplicate symbols dropped (earlier source
/// wins), type filter and limit applied. Binance pairs that do not match
/// are dropped; Yahoo already filtered its own. Only the returned results
/// are cloned.
fn merge(
    binance: &[SymbolResult],
    yahoo: &[SymbolResult],
    q: &str,
    types: &[String],
    limit: usize,
) -> Vec<SymbolResult> {
    let mut seen = HashSet::new();
    let mut ranked: Vec<_> = binance
        .iter()
        .chain(yahoo)
        .filter(|r| types.is_empty() || types.iter().any(|t| t == r.kind))
        .map(|r| (rank_key(r, q), r))
        .filter(|&(k, r)| (r.source != "binance" || k.0 < 4) && seen.insert(r.symbol.as_str()))
        .collect();
    ranked.sort_by_key(|a| a.0);
    cap_pairs_per_base(&mut ranked);
    ranked.sort_by_key(|a| a.0);
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, r)| r.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Caching, singleflight, rate limiting
// ---------------------------------------------------------------------------

/// A source failure; `cache` says whether it is remembered for
/// `ERROR_TTL_MS` (upstream errors) or retried at once (our own limits).
#[derive(Debug)]
struct SrcErr {
    msg: String,
    cache: bool,
}

impl SrcErr {
    fn upstream(msg: String) -> Self {
        Self { msg, cache: true }
    }
    fn transient(msg: &str) -> Self {
        Self {
            msg: msg.into(),
            cache: false,
        }
    }
}

/// A call the budget refused says nothing about Binance's symbols, so it is not
/// remembered: the search works again as soon as the gate reopens.
fn binance_err(msg: String) -> SrcErr {
    if msg.starts_with("Binance rate limit") {
        SrcErr::transient(&msg)
    } else {
        SrcErr::upstream(msg)
    }
}

struct Entry<V> {
    stored_at: i64,
    fresh_until: i64,
    val: Result<V, String>,
}

/// One source's results or remembered failures by key. `now` is passed in
/// everywhere so tests control the clock.
struct Source<V> {
    ok_ttl: i64,
    max: usize,
    map: Mutex<HashMap<String, Entry<V>>>,
    /// Per-key locks: the holder is the one request fetching that key.
    flights: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    refreshing: AtomicBool,
}

impl<V: Clone + Send + 'static> Source<V> {
    fn new(ok_ttl: i64, max: usize) -> Arc<Self> {
        Arc::new(Self {
            ok_ttl,
            max,
            map: Mutex::default(),
            flights: Mutex::default(),
            refreshing: AtomicBool::new(false),
        })
    }

    fn peek(&self, key: &str) -> Option<(i64, Result<V, String>)> {
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let e = map.get(key)?;
        Some((e.fresh_until, e.val.clone()))
    }

    fn store(&self, key: &str, now: i64, val: Result<V, String>, keep_stale: bool) {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let fresh_until = now
            + match val {
                Ok(_) => self.ok_ttl,
                Err(_) => ERROR_TTL_MS,
            };
        if keep_stale && val.is_err() {
            if let Some(old) = map.get_mut(key).filter(|e| e.val.is_ok()) {
                // A failed refresh keeps the stale data and retries later.
                old.fresh_until = fresh_until;
                return;
            }
        }
        if !map.contains_key(key) && map.len() >= self.max {
            if let Some(oldest) = map
                .iter()
                .min_by_key(|(_, e)| e.stored_at)
                .map(|(k, _)| k.clone())
            {
                map.remove(&oldest);
            }
        }
        map.insert(
            key.to_string(),
            Entry {
                stored_at: now,
                fresh_until,
                val,
            },
        );
    }

    /// `key`'s value: fresh from the cache, else fetched. Concurrent callers
    /// for one key wait for a single fetch. With `serve_stale`, expired
    /// successful data is returned at once and refreshed in the background.
    fn lookup<F>(
        self: &Arc<Self>,
        key: &str,
        now: i64,
        serve_stale: bool,
        fetch: F,
    ) -> Result<V, String>
    where
        F: FnOnce() -> Result<V, SrcErr> + Send + 'static,
    {
        match self.peek(key) {
            Some((until, val)) if now < until => return val,
            Some((_, Ok(stale))) if serve_stale => {
                if !self.refreshing.swap(true, Ordering::AcqRel) {
                    let (this, key) = (Arc::clone(self), key.to_string());
                    std::thread::spawn(move || {
                        let _ = this.fetch_once(&key, now, true, fetch);
                        this.refreshing.store(false, Ordering::Release);
                    });
                }
                return Ok(stale);
            }
            _ => {}
        }
        self.fetch_once(key, now, serve_stale, fetch)
    }

    fn fetch_once<F>(&self, key: &str, now: i64, keep_stale: bool, fetch: F) -> Result<V, String>
    where
        F: FnOnce() -> Result<V, SrcErr>,
    {
        let flight = {
            let mut flights = self.flights.lock().unwrap_or_else(|e| e.into_inner());
            Arc::clone(flights.entry(key.to_string()).or_default())
        };
        let result = {
            let _turn = flight.lock().unwrap_or_else(|e| e.into_inner());
            // Whoever held the turn before us may have filled the cache.
            match self.peek(key) {
                Some((until, val)) if now < until => val,
                _ => match fetch() {
                    Ok(v) => {
                        self.store(key, now, Ok(v.clone()), keep_stale);
                        Ok(v)
                    }
                    Err(e) => {
                        if e.cache {
                            self.store(key, now, Err(e.msg.clone()), keep_stale);
                        }
                        Err(e.msg)
                    }
                },
            }
        };
        let mut flights = self.flights.lock().unwrap_or_else(|e| e.into_inner());
        // Only the map and this call still hold it: nobody is waiting.
        if Arc::strong_count(&flight) == 2 {
            flights.remove(key);
        }
        result
    }
}

/// Refilling allowance for upstream requests; `now` in ms.
struct Bucket {
    tokens: f64,
    last: i64,
}

impl Bucket {
    fn new() -> Self {
        Self {
            tokens: YAHOO_BURST,
            last: 0,
        }
    }

    fn take(&mut self, now: i64) -> bool {
        let elapsed = (now - self.last).max(0) as f64 / 1000.0;
        self.tokens = (self.tokens + elapsed * YAHOO_RATE_PER_SEC).min(YAHOO_BURST);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

type Pairs = Arc<Vec<SymbolResult>>;

struct Engine {
    binance: Arc<Source<Pairs>>,
    yahoo: Arc<Source<Pairs>>,
    yahoo_bucket: Mutex<Bucket>,
}

impl Engine {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            binance: Source::new(BINANCE_TTL_MS, 1),
            yahoo: Source::new(YAHOO_TTL_MS, YAHOO_CACHE_MAX),
            yahoo_bucket: Mutex::new(Bucket::new()),
        })
    }
}

fn engine() -> &'static Arc<Engine> {
    static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();
    ENGINE.get_or_init(Engine::new)
}

// ---------------------------------------------------------------------------
// Upstreams
// ---------------------------------------------------------------------------

fn fetch_binance_pairs() -> Result<Vec<SymbolResult>, String> {
    let resp =
        fetch::binance().get("https://api.binance.com/api/v3/exchangeInfo?symbolStatus=TRADING")?;
    if !(200..300).contains(&resp.status) {
        return Err(format!("Binance API returned status {}", resp.status));
    }
    parse_exchange_info(&resp.body)
}

fn yahoo_search_url(query: &str) -> String {
    format!(
        "https://query1.finance.yahoo.com/v1/finance/search?q={}&quotesCount=20&newsCount=0",
        fetch::percent_encode(query)
    )
}

fn fetch_yahoo_search(query: &str) -> Result<Vec<SymbolResult>, String> {
    let resp = fetch::http()
        .get(yahoo_search_url(query))
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .map_err(|e| format!("Yahoo request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("Yahoo API returned status {status}"));
    }
    let body: Value = resp
        .json()
        .map_err(|e| format!("Failed to parse Yahoo response: {e}"))?;
    Ok(parse_yahoo_search(&body))
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// Deterministic offline catalogue for the sample data source.
const SAMPLE_SYMBOLS: [(&str, &str, &str, &str); 30] = [
    ("BTCUSDT", "BTC / USDT", "Binance", "crypto"),
    ("ETHUSDT", "ETH / USDT", "Binance", "crypto"),
    ("SOLUSDT", "SOL / USDT", "Binance", "crypto"),
    ("BNBUSDT", "BNB / USDT", "Binance", "crypto"),
    ("XRPUSDT", "XRP / USDT", "Binance", "crypto"),
    ("DOGEUSDT", "DOGE / USDT", "Binance", "crypto"),
    ("ETHBTC", "ETH / BTC", "Binance", "crypto"),
    ("BTCEUR", "BTC / EUR", "Binance", "crypto"),
    ("AAPL", "Apple Inc.", "NASDAQ", "stock"),
    ("MSFT", "Microsoft Corporation", "NASDAQ", "stock"),
    ("NVDA", "NVIDIA Corporation", "NASDAQ", "stock"),
    ("TSLA", "Tesla, Inc.", "NASDAQ", "stock"),
    ("AMZN", "Amazon.com, Inc.", "NASDAQ", "stock"),
    ("GOOGL", "Alphabet Inc.", "NASDAQ", "stock"),
    ("META", "Meta Platforms, Inc.", "NASDAQ", "stock"),
    ("JPM", "JPMorgan Chase & Co.", "NYSE", "stock"),
    ("SPY", "SPDR S&P 500 ETF Trust", "NYSEArca", "etf"),
    ("QQQ", "Invesco QQQ Trust", "NASDAQ", "etf"),
    ("VTI", "Vanguard Total Stock Market ETF", "NYSEArca", "etf"),
    ("VFIAX", "Vanguard 500 Index Fund Admiral", "NASDAQ", "fund"),
    ("^GSPC", "S&P 500", "S&P Indices", "index"),
    ("^IXIC", "NASDAQ Composite", "NASDAQ", "index"),
    ("^DJI", "Dow Jones Industrial Average", "DJI", "index"),
    ("EURUSD=X", "EUR/USD", "CCY", "forex"),
    ("GBPUSD=X", "GBP/USD", "CCY", "forex"),
    ("USDJPY=X", "USD/JPY", "CCY", "forex"),
    ("ES=F", "E-Mini S&P 500", "CME", "future"),
    ("NQ=F", "E-Mini NASDAQ 100", "CME", "future"),
    ("GC=F", "Gold", "COMEX", "future"),
    ("CL=F", "Crude Oil", "NYMEX", "future"),
];

fn sample_catalogue() -> &'static [SymbolResult] {
    static CATALOGUE: OnceLock<Vec<SymbolResult>> = OnceLock::new();
    CATALOGUE.get_or_init(|| {
        SAMPLE_SYMBOLS
            .iter()
            .map(|&(symbol, name, exchange, kind)| SymbolResult {
                symbol: symbol.into(),
                name: name.into(),
                exchange: Cow::Borrowed(exchange),
                kind,
                source: "sample",
            })
            .collect()
    })
}

/// Run `search_symbols`. `types` empty means every type.
pub fn search_symbols(
    source: DataSource,
    raw_query: &str,
    limit: usize,
    types: &[String],
) -> Result<Outcome, String> {
    let query = validate_query(raw_query)?;
    if let Some(bad) = types.iter().find(|t| !TYPES.contains(&t.as_str())) {
        return Err(format!("unknown type {bad:?}; expected one of {TYPES:?}"));
    }
    let limit = limit.clamp(1, MAX_LIMIT);
    match source {
        DataSource::Sample => Ok(Outcome {
            results: merge(sample_catalogue(), &[], &query.to_uppercase(), types, limit),
            warnings: Vec::new(),
        }),
        DataSource::Live => {
            let eng = Arc::clone(engine());
            let yahoo_eng = Arc::clone(&eng);
            search_live(
                &eng,
                &query,
                limit,
                types,
                fetch::now_ms(),
                SOURCE_DEADLINE,
                || fetch_binance_pairs().map_err(binance_err),
                move |q| {
                    let allowed = yahoo_eng
                        .yahoo_bucket
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .take(fetch::now_ms());
                    if !allowed {
                        return Err(SrcErr::transient("rate limited, try again shortly"));
                    }
                    fetch_yahoo_search(&q).map_err(SrcErr::upstream)
                },
            )
        }
        #[cfg(test)]
        DataSource::Unavailable => search_live(
            &Engine::new(),
            &query,
            limit,
            types,
            0,
            SOURCE_DEADLINE,
            || Err(SrcErr::upstream("upstream unavailable".into())),
            |_| Err(SrcErr::upstream("upstream unavailable".into())),
        ),
    }
}

/// Run `f` on its own thread; the answer arrives on the returned channel.
fn spawn_source<V: Send + 'static>(
    f: impl FnOnce() -> Result<V, String> + Send + 'static,
) -> mpsc::Receiver<Result<V, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx
}

/// The source's answer, `Err` with a message when it failed or missed `end`.
/// A late answer still fills the cache for the next call.
fn await_source<V>(
    name: &str,
    rx: mpsc::Receiver<Result<V, String>>,
    end: Instant,
) -> Result<V, String> {
    match rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
        Ok(r) => r.map_err(|e| format!("{name}: {e}")),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!("{name}: timed out")),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(format!("{name}: lookup failed")),
    }
}

/// A queried source's items; a failure becomes a warning and no items. `None`
/// (source not queried) yields no items and no warning.
fn take_source<T: Default>(
    r: Option<Result<T, String>>,
    ok: &mut usize,
    warnings: &mut Vec<String>,
) -> T {
    match r {
        Some(Ok(v)) => {
            *ok += 1;
            v
        }
        Some(Err(w)) => {
            warnings.push(w);
            T::default()
        }
        None => T::default(),
    }
}

/// Binance (local after the first fetch) and Yahoo run at the same time,
/// each on its own thread with its own deadline. A failing or slow source
/// becomes a warning and the other's results are returned; only when every
/// queried source fails is the call an error. The clock and upstreams are
/// injected so tests run offline.
#[allow(clippy::too_many_arguments)]
fn search_live<B, Y>(
    engine: &Engine,
    query: &str,
    limit: usize,
    types: &[String],
    now: i64,
    deadline: Duration,
    binance: B,
    yahoo: Y,
) -> Result<Outcome, String>
where
    B: FnOnce() -> Result<Vec<SymbolResult>, SrcErr> + Send + 'static,
    Y: FnOnce(String) -> Result<Vec<SymbolResult>, SrcErr> + Send + 'static,
{
    let wants = |t: &str| types.is_empty() || types.iter().any(|x| x == t);
    let end = Instant::now() + deadline;

    let from_binance = wants("crypto").then(|| {
        let source = Arc::clone(&engine.binance);
        spawn_source(move || source.lookup("spot", now, true, move || binance().map(Arc::new)))
    });
    let from_yahoo = TYPES.iter().any(|t| *t != "crypto" && wants(t)).then(|| {
        let source = Arc::clone(&engine.yahoo);
        let key = query.to_lowercase();
        let q = query.to_string();
        spawn_source(move || source.lookup(&key, now, false, move || yahoo(q).map(Arc::new)))
    });

    let mut outcome = Outcome::default();
    let mut ok = 0;
    let binance_pairs: Pairs = take_source(
        from_binance.map(|rx| await_source("binance", rx, end)),
        &mut ok,
        &mut outcome.warnings,
    );
    let yahoo_items: Pairs = take_source(
        from_yahoo.map(|rx| await_source("yahoo", rx, end)),
        &mut ok,
        &mut outcome.warnings,
    );
    if ok == 0 && !outcome.warnings.is_empty() {
        return Err(outcome.warnings.join("; "));
    }
    outcome.results = merge(
        &binance_pairs,
        &yahoo_items,
        &query.to_uppercase(),
        types,
        limit,
    );
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn binance_pairs() -> Vec<SymbolResult> {
        parse_exchange_info(&fixture("binance_exchange_info.json")).unwrap()
    }

    fn yahoo_items() -> Vec<SymbolResult> {
        parse_yahoo_search(&serde_json::from_str(&fixture("yahoo_search_mixed.json")).unwrap())
    }

    fn symbols(r: &[SymbolResult]) -> Vec<&str> {
        r.iter().map(|r| r.symbol.as_str()).collect()
    }

    fn mk(symbol: &str, name: &str, kind: &'static str) -> SymbolResult {
        SymbolResult {
            symbol: symbol.into(),
            name: name.into(),
            exchange: Cow::Borrowed(""),
            kind,
            source: if kind == "crypto" { "binance" } else { "yahoo" },
        }
    }

    fn pair(base: &str, quote: &str) -> SymbolResult {
        mk(
            &format!("{base}{quote}"),
            &format!("{base} / {quote}"),
            "crypto",
        )
    }

    type Fetched = Result<Vec<SymbolResult>, SrcErr>;

    fn run<B, Y>(
        e: &Engine,
        q: &str,
        types: &[String],
        now: i64,
        b: B,
        y: Y,
    ) -> Result<Outcome, String>
    where
        B: FnOnce() -> Fetched + Send + 'static,
        Y: FnOnce(String) -> Fetched + Send + 'static,
    {
        search_live(e, q, 10, types, now, Duration::from_secs(5), b, y)
    }

    #[test]
    fn exchange_info_keeps_trading_spot_pairs_with_valid_symbols() {
        let pairs = binance_pairs();
        assert!(pairs
            .iter()
            .all(|p| p.kind == "crypto" && p.source == "binance"));
        assert!(
            !symbols(&pairs).contains(&"NEOBTC"),
            "non-TRADING pair kept"
        );
        let btc = pairs.iter().find(|p| p.symbol == "BTCUSDT").unwrap();
        assert_eq!(btc.name, "BTC / USDT");
        assert_eq!(btc.exchange, "Binance");
        assert!(parse_exchange_info("not json").is_err());

        let odd = r#"{"symbols":[
            {"symbol":"A/B","status":"TRADING","baseAsset":"A","quoteAsset":"B","isSpotTradingAllowed":true},
            {"symbol":"ABC","status":"TRADING","baseAsset":"AB","quoteAsset":"C","isSpotTradingAllowed":true}]}"#;
        assert_eq!(symbols(&parse_exchange_info(odd).unwrap()), ["ABC"]);
    }

    #[test]
    fn yahoo_parse_maps_types_and_drops_unusable_entries() {
        let items = yahoo_items();
        assert_eq!(
            symbols(&items),
            ["AAPL", "SPY", "EURUSD=X", "ES=F", "^GSPC", "VFIAX"]
        );
        let kinds: Vec<_> = items.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, ["stock", "etf", "forex", "future", "index", "fund"]);
        assert_eq!(items[0].name, "Apple Inc.");
        assert_eq!(items[0].exchange, "NASDAQ");
        // No longname: falls back to the short name.
        assert_eq!(items[2].name, "EUR/USD");
        assert!(parse_yahoo_search(&serde_json::json!({})).is_empty());
    }

    fn ranked(items: Vec<SymbolResult>, q: &str) -> Vec<String> {
        merge(&items, &[], q, &[], 50)
            .into_iter()
            .map(|r| r.symbol)
            .collect()
    }

    #[test]
    fn ranking_orders_exact_prefix_contains_name() {
        let items = vec![
            mk("XYZ", "Spy Holdings", "stock"),
            mk("ASPY", "A", "stock"),
            mk("SPYG", "B", "stock"),
            mk("SPY", "C", "stock"),
        ];
        assert_eq!(ranked(items, "SPY"), ["SPY", "SPYG", "ASPY", "XYZ"]);
    }

    #[test]
    fn btc_ranks_coin_then_future_then_etf() {
        let r = ranked(
            vec![
                mk("BTC", "Grayscale Bitcoin Mini Trust", "etf"),
                mk("BTC=F", "Bitcoin Futures", "future"),
                pair("BTC", "EUR"),
                pair("WBTC", "BTC"),
                pair("BTC", "USDT"),
                mk("BTCS", "Bitcoin Depot", "stock"),
            ],
            "BTC",
        );
        assert_eq!(r, ["BTCUSDT", "BTCEUR", "BTC=F", "BTC", "BTCS", "WBTCBTC"]);
    }

    #[test]
    fn btc_caps_pairs_per_base_before_futures_and_etf() {
        let mut items: Vec<_> = ["EUR", "TRY", "USDC", "USDT", "FDUSD", "BRL", "JPY", "USD"]
            .iter()
            .map(|q| pair("BTC", q))
            .collect();
        items.push(mk("BTC=F", "Bitcoin Futures", "future"));
        items.push(mk("BTC", "Grayscale Bitcoin Mini Trust", "etf"));
        items.push(mk("BTCS", "Bitcoin Depot", "stock"));
        let out = merge(&items, &[], "BTC", &[], 8);
        assert_eq!(
            symbols(&out),
            ["BTCUSDT", "BTCUSDC", "BTCFDUSD", "BTCUSD", "BTC=F", "BTC", "BTCEUR", "BTCTRY"]
        );
    }

    #[test]
    fn eur_ranks_forex_before_fiat_pairs() {
        let r = ranked(
            vec![
                pair("EUR", "USDT"),
                mk("EURCHF=X", "EUR/CHF", "forex"),
                mk("EUR=X", "USD/EUR", "forex"),
                mk("EURUSD=X", "EUR/USD", "forex"),
                pair("EURI", "USDT"),
            ],
            "EUR",
        );
        assert_eq!(r, ["EURUSD=X", "EUR=X", "EURCHF=X", "EURUSDT", "EURIUSDT"]);
    }

    #[test]
    fn es_ranks_future_first() {
        let r = ranked(
            vec![
                mk("ESR=F", "Euro short-term rate", "future"),
                mk("ES", "Eversource Energy", "stock"),
                mk("ES=F", "E-Mini S&P 500", "future"),
            ],
            "ES",
        );
        assert_eq!(r, ["ES=F", "ES", "ESR=F"]);
    }

    #[test]
    fn listed_symbols_rank_first_for_their_own_ticker() {
        let r = ranked(
            vec![
                pair("SPYB", "USDT"),
                mk("SPYM", "x", "etf"),
                mk("SPY", "SPDR S&P 500", "etf"),
            ],
            "SPY",
        );
        assert_eq!(r, ["SPY", "SPYM", "SPYBUSDT"]);

        let r = ranked(
            vec![
                mk("AAPL.TO", "Apple", "stock"),
                mk("AAPL", "Apple Inc.", "stock"),
                mk("AAPLX", "x", "fund"),
            ],
            "AAPL",
        );
        assert_eq!(r, ["AAPL", "AAPLX", "AAPL.TO"]);
    }

    #[test]
    fn mixed_fixtures_rank_across_sources() {
        let out = merge(&binance_pairs(), &yahoo_items(), "BTC", &[], 10);
        assert_eq!(symbols(&out)[0], "BTCUSDT");
        assert_eq!(symbols(&out)[..2], ["BTCUSDT", "BTCEUR"]);
        let out = merge(&binance_pairs(), &yahoo_items(), "EUR", &[], 10);
        assert_eq!(symbols(&out)[0], "EURUSD=X");
        let out = merge(&binance_pairs(), &yahoo_items(), "ES", &[], 10);
        assert_eq!(symbols(&out)[0], "ES=F");
        let out = merge(&binance_pairs(), &yahoo_items(), "SPY", &[], 10);
        assert_eq!(symbols(&out)[0], "SPY");
        let out = merge(&binance_pairs(), &yahoo_items(), "AAPL", &[], 10);
        assert_eq!(symbols(&out)[0], "AAPL");
    }

    #[test]
    fn merge_dedupes_filters_types_and_limits() {
        let (b, y) = (binance_pairs(), yahoo_items());
        let out = merge(&b, &y, "S", &[], 100);
        let mut syms = symbols(&out);
        let n = syms.len();
        syms.sort();
        syms.dedup();
        assert_eq!(syms.len(), n);

        let etf = merge(&b, &y, "S", &["etf".into()], 100);
        assert_eq!(symbols(&etf), ["SPY"]);
        assert_eq!(merge(&b, &y, "S", &[], 2).len(), 2);
    }

    #[test]
    fn query_validation() {
        assert_eq!(validate_query("  btc "), Ok("btc".into()));
        assert_eq!(validate_query(" a \t  b "), Ok("a b".into()));
        assert!(validate_query("   ").is_err());
        assert!(validate_query("").is_err());
        assert!(validate_query(&"a".repeat(41)).is_err());
        assert!(validate_query(&"a".repeat(40)).is_ok());
        assert!(validate_query("bt\u{0}c").is_err());
        assert!(validate_query("bt\u{7}c").is_err());
    }

    fn ok_fetch(n: &'static AtomicUsize) -> impl FnOnce() -> Result<Vec<usize>, SrcErr> + Send {
        move || {
            n.fetch_add(1, Ordering::SeqCst);
            Ok(vec![1])
        }
    }

    #[test]
    fn cache_expires_with_injected_clock() {
        static N: AtomicUsize = AtomicUsize::new(0);
        let s: Arc<Source<Vec<usize>>> = Source::new(YAHOO_TTL_MS, 10);
        s.lookup("q", 1_000, false, ok_fetch(&N)).unwrap();
        s.lookup("q", 1_000 + YAHOO_TTL_MS - 1, false, ok_fetch(&N))
            .unwrap();
        assert_eq!(N.load(Ordering::SeqCst), 1);
        s.lookup("q", 1_000 + YAHOO_TTL_MS, false, ok_fetch(&N))
            .unwrap();
        assert_eq!(N.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn failures_are_cached_briefly_unless_transient() {
        static N: AtomicUsize = AtomicUsize::new(0);
        let s: Arc<Source<Vec<usize>>> = Source::new(YAHOO_TTL_MS, 10);
        let fail = || {
            N.fetch_add(1, Ordering::SeqCst);
            Err(SrcErr::upstream("boom".into()))
        };
        assert_eq!(s.lookup("q", 0, false, fail).unwrap_err(), "boom");
        assert_eq!(
            s.lookup("q", ERROR_TTL_MS - 1, false, fail).unwrap_err(),
            "boom"
        );
        assert_eq!(N.load(Ordering::SeqCst), 1);
        assert!(s.lookup("q", ERROR_TTL_MS, false, ok_fetch(&N)).is_ok());
        assert_eq!(N.load(Ordering::SeqCst), 2);

        let t = || {
            N.fetch_add(1, Ordering::SeqCst);
            Err(SrcErr::transient("slow down"))
        };
        assert!(s.lookup("t", 0, false, t).is_err());
        assert!(s.lookup("t", 1, false, ok_fetch(&N)).is_ok());
        assert_eq!(N.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn eviction_drops_the_oldest_entry_only() {
        let s: Arc<Source<u8>> = Source::new(1_000_000, 2);
        s.store("a", 1, Ok(1), false);
        s.store("b", 2, Ok(1), false);
        s.store("c", 3, Ok(1), false);
        assert!(s.peek("a").is_none());
        assert!(s.peek("b").is_some() && s.peek("c").is_some());
    }

    #[test]
    fn concurrent_cold_lookups_share_one_fetch() {
        static N: AtomicUsize = AtomicUsize::new(0);
        let s: Arc<Source<Vec<usize>>> = Source::new(YAHOO_TTL_MS, 10);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let s = Arc::clone(&s);
                std::thread::spawn(move || {
                    s.lookup("q", 5, false, || {
                        N.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(100));
                        Ok(vec![1])
                    })
                })
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap().unwrap(), vec![1]);
        }
        assert_eq!(N.load(Ordering::SeqCst), 1);
        assert!(s.flights.lock().unwrap().is_empty(), "flight slot leaked");
    }

    #[test]
    fn stale_data_is_served_while_it_refreshes() {
        let s: Arc<Source<u8>> = Source::new(100, 1);
        s.lookup("k", 0, true, || Ok(1)).unwrap();
        let (release, gate) = mpsc::channel::<()>();
        let stale = s.lookup("k", 500, true, move || {
            gate.recv().unwrap();
            Ok(2)
        });
        assert_eq!(stale, Ok(1), "stale value is returned at once");
        release.send(()).unwrap();
        for _ in 0..200 {
            if s.peek("k").is_some_and(|(_, v)| v == Ok(2)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("background refresh never landed");
    }

    #[test]
    fn a_refused_binance_call_is_not_remembered() {
        let refused = binance_err("Binance rate limit: waiting 30s after HTTP 429".into());
        assert!(!refused.cache);
        assert!(refused.msg.contains("30s"));
        let failed = binance_err("Binance API returned status 500".into());
        assert!(failed.cache);
    }

    #[test]
    fn failed_refresh_keeps_stale_data_and_backs_off() {
        let s: Arc<Source<u8>> = Source::new(100, 1);
        s.store("k", 0, Ok(1), true);
        s.store("k", 500, Err("down".into()), true);
        let (until, v) = s.peek("k").unwrap();
        assert_eq!(v, Ok(1));
        assert_eq!(until, 500 + ERROR_TTL_MS);
    }

    #[test]
    fn token_bucket_limits_and_refills() {
        let mut b = Bucket::new();
        assert!(b.take(10_000) && b.take(10_000));
        assert!(!b.take(10_000));
        assert!(!b.take(10_400));
        assert!(b.take(10_500));
        // Never accumulates beyond the burst.
        assert!(b.take(1_000_000) && b.take(1_000_000) && !b.take(1_000_000));
    }

    #[test]
    fn binance_is_fetched_once_per_ttl_across_queries() {
        let e = Engine::new();
        static N: AtomicUsize = AtomicUsize::new(0);
        for (q, now) in [("btc", 0), ("eth", 10), ("sol", BINANCE_TTL_MS - 1)] {
            run(
                &e,
                q,
                &["crypto".into()],
                now,
                || {
                    N.fetch_add(1, Ordering::SeqCst);
                    Ok(binance_pairs())
                },
                |_| panic!("yahoo not queried"),
            )
            .unwrap();
        }
        assert_eq!(N.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn one_failing_source_yields_the_other_plus_a_warning() {
        let e = Engine::new();
        let out = run(
            &e,
            "btc",
            &[],
            0,
            || Ok(binance_pairs()),
            |_| Err(SrcErr::upstream("429".into())),
        )
        .unwrap();
        assert_eq!(symbols(&out.results)[0], "BTCUSDT");
        assert_eq!(out.warnings, ["yahoo: 429"]);

        let e = Engine::new();
        let out = run(
            &e,
            "aapl",
            &[],
            0,
            || Err(SrcErr::upstream("down".into())),
            |_| Ok(yahoo_items()),
        )
        .unwrap();
        assert_eq!(symbols(&out.results)[0], "AAPL");
        assert_eq!(out.warnings, ["binance: down"]);
    }

    #[test]
    fn rate_limited_yahoo_degrades_to_binance_and_is_not_remembered() {
        let e = Engine::new();
        let out = run(
            &e,
            "btc",
            &[],
            0,
            || Ok(binance_pairs()),
            |_| Err(SrcErr::transient("rate limited")),
        )
        .unwrap();
        assert!(!out.results.is_empty());
        assert_eq!(out.warnings, ["yahoo: rate limited"]);
        let out = run(
            &e,
            "btc",
            &[],
            1,
            || Ok(binance_pairs()),
            |_| Ok(yahoo_items()),
        )
        .unwrap();
        assert!(out.warnings.is_empty());
    }

    #[test]
    fn all_sources_failing_is_an_error() {
        let e = Engine::new();
        let err = run(
            &e,
            "x",
            &[],
            0,
            || Err(SrcErr::upstream("a".into())),
            |_| Err(SrcErr::upstream("b".into())),
        )
        .unwrap_err();
        assert!(
            err.contains("binance: a") && err.contains("yahoo: b"),
            "{err}"
        );
    }

    #[test]
    fn slow_source_misses_its_deadline_without_holding_the_other() {
        let e = Engine::new();
        let (release, gate) = mpsc::channel::<()>();
        let started = Instant::now();
        let out = search_live(
            &e,
            "aapl",
            10,
            &[],
            0,
            Duration::from_millis(150),
            move || {
                let _ = gate.recv();
                Ok(binance_pairs())
            },
            |_| Ok(yahoo_items()),
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(symbols(&out.results)[0], "AAPL");
        assert_eq!(out.warnings, ["binance: timed out"]);
        release.send(()).unwrap();
    }

    #[test]
    fn type_filter_skips_irrelevant_sources() {
        let e = Engine::new();
        let out = run(
            &e,
            "btc",
            &["crypto".into()],
            0,
            || Ok(binance_pairs()),
            |_| panic!("yahoo must not be queried"),
        )
        .unwrap();
        assert!(!out.results.is_empty());
        let e = Engine::new();
        let out = run(
            &e,
            "spy",
            &["etf".into()],
            0,
            || panic!("binance must not be queried"),
            |_| Ok(yahoo_items()),
        )
        .unwrap();
        assert_eq!(symbols(&out.results), ["SPY"]);
    }

    #[test]
    fn sample_source_is_offline_and_deterministic() {
        let a = search_symbols(DataSource::Sample, "btc", 30, &[]).unwrap();
        assert_eq!(symbols(&a.results)[0], "BTCUSDT");
        assert!(a.warnings.is_empty());
        let eth = search_symbols(DataSource::Sample, "eth", 30, &["crypto".into()]).unwrap();
        assert_eq!(symbols(&eth.results)[0], "ETHUSDT");
        assert!(search_symbols(DataSource::Sample, "x", 30, &["bogus".into()]).is_err());
        assert!(search_symbols(DataSource::Sample, "", 30, &[]).is_err());
    }

    #[test]
    fn yahoo_url_encodes_the_query() {
        assert!(yahoo_search_url("es=f & x").contains("q=es%3Df%20%26%20x&"));
    }
}
