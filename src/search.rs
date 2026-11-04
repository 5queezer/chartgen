//! Symbol search for the `search_symbols` MCP tool: every Binance spot pair
//! (exchange info cached for 6 h, searched locally) plus Yahoo Finance's
//! search endpoint (cached per query for 60 s), ranked like TradingView:
//! exact symbol, then prefix, then contains, then name match.

use crate::fetch::{self, DataSource};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

pub const DEFAULT_LIMIT: usize = 30;
pub const MAX_LIMIT: usize = 50;
const MAX_QUERY_LEN: usize = 40;
const BINANCE_TTL_MS: i64 = 6 * 60 * 60 * 1000;
const YAHOO_TTL_MS: i64 = 60_000;
/// Distinct Yahoo queries kept; the cache is cleared when it grows past this.
const YAHOO_CACHE_MAX: usize = 500;
pub const TYPES: [&str; 7] = ["crypto", "stock", "etf", "index", "forex", "future", "fund"];

#[derive(Debug, Clone, PartialEq)]
pub struct SymbolResult {
    pub symbol: String,
    pub name: String,
    pub exchange: String,
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

/// Trim the query; 1–40 characters, no control characters.
pub fn validate_query(raw: &str) -> Result<String, String> {
    let q = raw.trim();
    if q.is_empty() || q.chars().count() > MAX_QUERY_LEN {
        return Err(format!("query must be 1-{MAX_QUERY_LEN} characters"));
    }
    if q.chars().any(char::is_control) {
        return Err("query must not contain control characters".into());
    }
    Ok(q.to_string())
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

/// TRADING spot pairs of a Binance `/api/v3/exchangeInfo` body.
fn parse_exchange_info(body: &str) -> Result<Vec<SymbolResult>, String> {
    let info: ExchangeInfo = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse Binance exchangeInfo: {e}"))?;
    Ok(info
        .symbols
        .into_iter()
        .filter(|s| s.status == "TRADING" && s.spot)
        .map(|s| SymbolResult {
            name: format!("{} / {}", s.base, s.quote),
            symbol: s.symbol,
            exchange: "Binance".into(),
            kind: "crypto",
            source: "binance",
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
                exchange: text(q, "exchDisp").unwrap_or_default(),
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

/// Sort key for `r` against the uppercased query `q`: match tier (0 exact
/// symbol, 1 prefix, 2 contains, 3 name match, 4 no match), then USDT pairs
/// before other crypto quotes, then shorter symbols.
fn rank_key(r: &SymbolResult, q: &str) -> (u8, bool, usize) {
    let base_is_q = r.kind == "crypto" && r.name.split(" / ").next() == Some(q);
    let tier = if r.symbol == q || base_is_q {
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
    let not_usdt = r.kind == "crypto" && !r.symbol.ends_with("USDT");
    (tier, not_usdt, if tier < 3 { r.symbol.len() } else { 0 })
}

/// Union of all sources ranked by `rank_key` (ties keep source order, so
/// upstream relevance survives), duplicate symbols dropped (earlier source
/// wins), type filter and limit applied. Binance pairs that do not match
/// are dropped; Yahoo already filtered its own.
fn merge(
    binance: Vec<SymbolResult>,
    yahoo: Vec<SymbolResult>,
    q: &str,
    types: &[String],
    limit: usize,
) -> Vec<SymbolResult> {
    let mut seen = HashSet::new();
    let mut all: Vec<_> = binance
        .into_iter()
        .filter(|r| rank_key(r, q).0 < 4)
        .chain(yahoo)
        .filter(|r| types.is_empty() || types.iter().any(|t| t == r.kind))
        .filter(|r| seen.insert(r.symbol.clone()))
        .collect();
    all.sort_by_cached_key(|r| rank_key(r, q));
    all.truncate(limit);
    all
}

// ---------------------------------------------------------------------------
// Caching
// ---------------------------------------------------------------------------

/// Values stamped with the ms time they were stored; `now` is passed in so
/// tests control the clock.
struct TtlCache<V> {
    ttl_ms: i64,
    max: usize,
    map: HashMap<String, (i64, V)>,
}

impl<V: Clone> TtlCache<V> {
    fn new(ttl_ms: i64, max: usize) -> Self {
        Self {
            ttl_ms,
            max,
            map: HashMap::new(),
        }
    }

    fn get(&self, key: &str, now: i64) -> Option<V> {
        let (t, v) = self.map.get(key)?;
        (now - t < self.ttl_ms).then(|| v.clone())
    }

    fn insert(&mut self, key: &str, now: i64, v: V) {
        self.map.retain(|_, (t, _)| now - *t < self.ttl_ms);
        if self.map.len() >= self.max {
            self.map.clear();
        }
        self.map.insert(key.to_string(), (now, v));
    }
}

/// `key` from `cache` when fresh, else `fetch` and store a success. The
/// lock is not held while fetching.
fn cached<V: Clone>(
    cache: &Mutex<TtlCache<V>>,
    key: &str,
    now: i64,
    fetch: impl FnOnce() -> Result<V, String>,
) -> Result<V, String> {
    if let Some(v) = cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key, now)
    {
        return Ok(v);
    }
    let v = fetch()?;
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, now, v.clone());
    Ok(v)
}

type BinanceCache = Mutex<TtlCache<Arc<Vec<SymbolResult>>>>;
type YahooCache = Mutex<TtlCache<Vec<SymbolResult>>>;

fn binance_cache() -> &'static BinanceCache {
    static CACHE: OnceLock<BinanceCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(TtlCache::new(BINANCE_TTL_MS, 1)))
}

fn yahoo_cache() -> &'static YahooCache {
    static CACHE: OnceLock<YahooCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(TtlCache::new(YAHOO_TTL_MS, YAHOO_CACHE_MAX)))
}

// ---------------------------------------------------------------------------
// Upstreams
// ---------------------------------------------------------------------------

fn fetch_binance_pairs() -> Result<Vec<SymbolResult>, String> {
    let resp = fetch::http()
        .get("https://api.binance.com/api/v3/exchangeInfo?symbolStatus=TRADING")
        .send()
        .map_err(|e| format!("Binance request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("Binance API returned status {status}"));
    }
    let body = resp
        .text()
        .map_err(|e| format!("Failed to read Binance response: {e}"))?;
    parse_exchange_info(&body)
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

fn sample_catalogue() -> Vec<SymbolResult> {
    SAMPLE_SYMBOLS
        .iter()
        .map(|&(symbol, name, exchange, kind)| SymbolResult {
            symbol: symbol.into(),
            name: name.into(),
            exchange: exchange.into(),
            kind,
            source: "sample",
        })
        .collect()
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
            results: merge(
                sample_catalogue(),
                Vec::new(),
                &query.to_uppercase(),
                types,
                limit,
            ),
            warnings: Vec::new(),
        }),
        DataSource::Live => search_live(
            &query,
            limit,
            types,
            fetch::now_ms(),
            (binance_cache(), yahoo_cache()),
            fetch_binance_pairs,
            fetch_yahoo_search,
        ),
        #[cfg(test)]
        DataSource::Unavailable => search_live(
            &query,
            limit,
            types,
            0,
            (binance_cache(), yahoo_cache()),
            || Err("upstream unavailable".into()),
            |_| Err("upstream unavailable".into()),
        ),
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

/// Binance (local after the first fetch) and Yahoo run at the same time. A
/// failing source becomes a warning and the other's results are returned;
/// only when every queried source fails is the call an error. The clock,
/// caches and upstreams are injected so tests run offline.
fn search_live<B, Y>(
    query: &str,
    limit: usize,
    types: &[String],
    now: i64,
    (bcache, ycache): (&BinanceCache, &YahooCache),
    binance: B,
    yahoo: Y,
) -> Result<Outcome, String>
where
    B: FnOnce() -> Result<Vec<SymbolResult>, String>,
    Y: Fn(&str) -> Result<Vec<SymbolResult>, String> + Sync,
{
    let wants = |t: &str| types.is_empty() || types.iter().any(|x| x == t);
    let use_binance = wants("crypto");
    let use_yahoo = TYPES.iter().any(|t| *t != "crypto" && wants(t));
    let q_upper = query.to_uppercase();

    let (from_yahoo, from_binance) = std::thread::scope(|s| {
        let y = use_yahoo.then(|| {
            s.spawn(|| {
                cached(ycache, &query.to_lowercase(), now, || yahoo(query))
                    .map_err(|e| format!("yahoo: {e}"))
            })
        });
        let b = use_binance.then(|| {
            cached(bcache, "spot", now, || binance().map(Arc::new))
                .map_err(|e| format!("binance: {e}"))
        });
        (
            y.map(|h| h.join().expect("yahoo search worker panicked")),
            b,
        )
    });

    let mut outcome = Outcome::default();
    let mut ok = 0;
    let binance_pairs: Arc<Vec<SymbolResult>> =
        take_source(from_binance, &mut ok, &mut outcome.warnings);
    let yahoo_items: Vec<SymbolResult> = take_source(from_yahoo, &mut ok, &mut outcome.warnings);
    if ok == 0 && !outcome.warnings.is_empty() {
        return Err(outcome.warnings.join("; "));
    }
    outcome.results = merge(
        binance_pairs.as_ref().clone(),
        yahoo_items,
        &q_upper,
        types,
        limit,
    );
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn caches() -> (BinanceCache, YahooCache) {
        (
            Mutex::new(TtlCache::new(BINANCE_TTL_MS, 1)),
            Mutex::new(TtlCache::new(YAHOO_TTL_MS, YAHOO_CACHE_MAX)),
        )
    }

    #[test]
    fn exchange_info_keeps_trading_spot_pairs() {
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

    #[test]
    fn ranking_orders_exact_prefix_contains_name() {
        let mk = |symbol: &str, name: &str| SymbolResult {
            symbol: symbol.into(),
            name: name.into(),
            exchange: String::new(),
            kind: "stock",
            source: "yahoo",
        };
        let items = vec![
            mk("XYZ", "Spy Holdings"),
            mk("ASPY", "A"),
            mk("SPYG", "B"),
            mk("SPY", "C"),
        ];
        assert_eq!(
            symbols(&merge(vec![], items, "SPY", &[], 10)),
            ["SPY", "SPYG", "ASPY", "XYZ"]
        );
    }

    #[test]
    fn crypto_base_match_ranks_usdt_first() {
        let out = merge(binance_pairs(), vec![], "BTC", &[], 10);
        assert_eq!(symbols(&out)[0], "BTCUSDT");
        assert!(symbols(&out).contains(&"BTCEUR"));
        // Prefix-only matches (WBTC...) rank after pairs whose base is BTC.
        assert_eq!(*symbols(&out).last().unwrap(), "WBTCBTC");
    }

    #[test]
    fn merge_dedupes_filters_types_and_limits() {
        let out = merge(binance_pairs(), yahoo_items(), "S", &[], 100);
        let mut syms = symbols(&out);
        let n = syms.len();
        syms.sort();
        syms.dedup();
        assert_eq!(syms.len(), n);

        let etf = merge(binance_pairs(), yahoo_items(), "S", &["etf".into()], 100);
        assert_eq!(symbols(&etf), ["SPY"]);
        assert_eq!(merge(binance_pairs(), yahoo_items(), "S", &[], 2).len(), 2);
    }

    #[test]
    fn query_validation() {
        assert_eq!(validate_query("  btc "), Ok("btc".into()));
        assert!(validate_query("   ").is_err());
        assert!(validate_query("").is_err());
        assert!(validate_query(&"a".repeat(41)).is_err());
        assert!(validate_query(&"a".repeat(40)).is_ok());
        assert!(validate_query("bt\u{0}c").is_err());
        assert!(validate_query("bt\nc").is_err());
    }

    #[test]
    fn ttl_cache_expires_with_injected_clock() {
        let (_, y) = caches();
        let calls = std::cell::Cell::new(0);
        let fetch = || {
            calls.set(calls.get() + 1);
            Ok(vec![])
        };
        cached(&y, "q", 1_000, fetch).unwrap();
        cached(&y, "q", 1_000 + YAHOO_TTL_MS - 1, fetch).unwrap();
        assert_eq!(calls.get(), 1);
        cached(&y, "q", 1_000 + YAHOO_TTL_MS, fetch).unwrap();
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn failed_fetches_are_not_cached() {
        let (_, y) = caches();
        assert!(cached(&y, "q", 0, || Err("boom".into())).is_err());
        assert_eq!(cached(&y, "q", 1, || Ok(vec![])).unwrap().len(), 0);
    }

    #[test]
    fn yahoo_cache_is_bounded() {
        let mut c: TtlCache<u8> = TtlCache::new(1_000, 2);
        c.insert("a", 0, 1);
        c.insert("b", 0, 1);
        c.insert("c", 0, 1);
        assert_eq!(c.map.len(), 1);
    }

    #[test]
    fn binance_is_fetched_once_per_ttl() {
        let (b, y) = caches();
        let calls = std::cell::Cell::new(0);
        for now in [0, 10, BINANCE_TTL_MS - 1] {
            search_live(
                "btc",
                10,
                &[],
                now,
                (&b, &y),
                || {
                    calls.set(calls.get() + 1);
                    Ok(binance_pairs())
                },
                |_| Ok(vec![]),
            )
            .unwrap();
        }
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn one_failing_source_yields_the_other_plus_a_warning() {
        let (b, y) = caches();
        let out = search_live(
            "btc",
            10,
            &[],
            0,
            (&b, &y),
            || Ok(binance_pairs()),
            |_| Err("429".into()),
        )
        .unwrap();
        assert_eq!(symbols(&out.results)[0], "BTCUSDT");
        assert_eq!(out.warnings, ["yahoo: 429"]);

        let (b, y) = caches();
        let out = search_live(
            "aapl",
            10,
            &[],
            0,
            (&b, &y),
            || Err("down".into()),
            |_| Ok(yahoo_items()),
        )
        .unwrap();
        assert_eq!(symbols(&out.results)[0], "AAPL");
        assert_eq!(out.warnings, ["binance: down"]);
    }

    #[test]
    fn all_sources_failing_is_an_error() {
        let (b, y) = caches();
        let err = search_live(
            "x",
            10,
            &[],
            0,
            (&b, &y),
            || Err("a".into()),
            |_| Err("b".into()),
        )
        .unwrap_err();
        assert!(
            err.contains("binance: a") && err.contains("yahoo: b"),
            "{err}"
        );
    }

    #[test]
    fn type_filter_skips_irrelevant_sources() {
        let (b, y) = caches();
        let out = search_live(
            "btc",
            10,
            &["crypto".into()],
            0,
            (&b, &y),
            || Ok(binance_pairs()),
            |_| panic!("yahoo must not be queried"),
        )
        .unwrap();
        assert!(!out.results.is_empty());
        let (b, y) = caches();
        let out = search_live(
            "spy",
            10,
            &["etf".into()],
            0,
            (&b, &y),
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
