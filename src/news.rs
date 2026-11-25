//! `get_news` (ADR-0086): recent headlines about one crypto symbol, from the public RSS feeds
//! of a few crypto news outlets, so an agent client does not spend a web search on them.
//!
//! No key and no article text: a headline's title, outlet, time and link. Each feed is read
//! at most once in ten minutes and held; a feed that cannot be read is named in the answer
//! and never fails the call. A headline is about the symbol when its title or one of the
//! feed's categories for it names the coin's ticker or name, as whole words: a heuristic,
//! which can miss a story or take one that only mentions the coin.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::{json, Value};

use crate::mcp::{tool_err, tool_ok};

/// The feeds read when `CHARTGEN_NEWS_FEEDS` is unset: large crypto outlets whose feeds need
/// no key and carry a day or more of headlines.
const DEFAULT_FEEDS: [(&str, &str); 5] = [
    (
        "CoinDesk",
        "https://www.coindesk.com/arc/outboundfeeds/rss/",
    ),
    ("Cointelegraph", "https://cointelegraph.com/rss"),
    ("Decrypt", "https://decrypt.co/feed"),
    ("The Block", "https://www.theblock.co/rss.xml"),
    ("Bitcoin Magazine", "https://bitcoinmagazine.com/.rss/full/"),
];
/// How long a feed that was read is served before it is read again.
const FRESH: Duration = Duration::from_secs(600);
/// How long a feed that could not be read waits before it is tried again.
const RETRY: Duration = Duration::from_secs(60);
const MAX_HOURS: u64 = 168;
const MAX_LIMIT: u64 = 50;
/// The quote assets a symbol's coin is read off: BTCUSDT is BTC.
const QUOTES: [&str; 6] = ["FDUSD", "USDT", "USDC", "BUSD", "-USD", "USD"];
/// Names a headline may call a coin by, besides its ticker. Matched in any case.
const NAMES: &[(&str, &[&str])] = &[
    ("BTC", &["Bitcoin"]),
    ("ETH", &["Ethereum", "Ether"]),
    ("SOL", &["Solana"]),
    ("XRP", &["Ripple"]),
    ("BNB", &["BNB Chain"]),
    ("DOGE", &["Dogecoin"]),
    ("ADA", &["Cardano"]),
    ("AVAX", &["Avalanche"]),
    ("LINK", &["Chainlink"]),
    ("DOT", &["Polkadot"]),
    ("TRX", &["Tron"]),
    ("TON", &["Toncoin"]),
    ("LTC", &["Litecoin"]),
    ("BCH", &["Bitcoin Cash"]),
    ("SHIB", &["Shiba Inu"]),
    ("SUI", &["Sui"]),
    ("APT", &["Aptos"]),
    ("ARB", &["Arbitrum"]),
    ("OP", &["Optimism"]),
    ("POL", &["Polygon"]),
    ("ATOM", &["Cosmos"]),
    ("UNI", &["Uniswap"]),
    ("AAVE", &["Aave"]),
    ("XLM", &["Stellar"]),
    ("HBAR", &["Hedera"]),
    ("XMR", &["Monero"]),
    ("ETC", &["Ethereum Classic"]),
    ("FIL", &["Filecoin"]),
    ("HYPE", &["Hyperliquid"]),
    ("ENA", &["Ethena"]),
    ("TAO", &["Bittensor"]),
    ("WLD", &["Worldcoin"]),
    ("INJ", &["Injective"]),
    ("NEAR", &["NEAR Protocol"]),
];

#[derive(Clone, Debug, PartialEq)]
struct Feed {
    name: String,
    url: String,
}

/// One headline of a feed.
#[derive(Clone, Debug, Default, PartialEq)]
struct Item {
    title: String,
    url: String,
    /// Unix seconds.
    published: i64,
    categories: Vec<String>,
}

type Fetched = Result<Arc<Vec<Item>>, String>;
/// Per feed URL: when it was last read, and what that gave. Its own lock, held during a
/// read, so callers at once share one request.
type Slot = Arc<Mutex<Option<(Instant, Fetched)>>>;

static SLOTS: LazyLock<Mutex<HashMap<String, Slot>>> = LazyLock::new(Mutex::default);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The feeds to read: `CHARTGEN_NEWS_FEEDS` if set, else the defaults.
fn feeds() -> &'static Result<Vec<Feed>, String> {
    static FEEDS: OnceLock<Result<Vec<Feed>, String>> = OnceLock::new();
    FEEDS.get_or_init(|| match std::env::var("CHARTGEN_NEWS_FEEDS") {
        Ok(list) if !cfg!(test) => feeds_from(&list),
        _ => Ok(DEFAULT_FEEDS
            .iter()
            .map(|(name, url)| Feed {
                name: name.to_string(),
                url: url.to_string(),
            })
            .collect()),
    })
}

/// `CHARTGEN_NEWS_FEEDS`: entries separated by commas, each `Name=https://…` or a bare URL,
/// which is then named by its host. Empty, no feed is read.
fn feeds_from(list: &str) -> Result<Vec<Feed>, String> {
    let mut feeds = Vec::new();
    for entry in list.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let (name, url) = match entry.split_once('=') {
            Some((name, url)) if !name.contains("://") => (Some(name.trim()), url.trim()),
            _ => (None, entry),
        };
        let parsed = url::Url::parse(url)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https"))
            .ok_or_else(|| format!("CHARTGEN_NEWS_FEEDS: {entry:?} is not an http(s) URL"))?;
        let name = name
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .or_else(|| parsed.host_str().map(str::to_string))
            .unwrap_or_else(|| url.to_string());
        feeds.push(Feed {
            name,
            url: url.to_string(),
        });
    }
    Ok(feeds)
}

/// A feed's headlines, as held if fresh. Blocking.
fn read(feed: &Feed) -> Fetched {
    let slot = lock(&SLOTS).entry(feed.url.clone()).or_default().clone();
    let mut held = lock(&slot);
    if let Some((at, read)) = &*held {
        if at.elapsed() < if read.is_ok() { FRESH } else { RETRY } {
            return read.clone();
        }
    }
    let read = download(&feed.url).and_then(|xml| parse(&xml).map(Arc::new));
    *held = Some((Instant::now(), read.clone()));
    read
}

#[cfg(test)]
fn download(url: &str) -> Result<String, String> {
    tests::download(url)
}

#[cfg(not(test))]
fn download(url: &str) -> Result<String, String> {
    use std::io::Read;
    /// One feed's request, connection to last byte.
    const TIMEOUT: Duration = Duration::from_secs(8);
    /// The most of a feed that is read; the largest default is under 100 kB.
    const MAX_BYTES: u64 = 4 << 20;
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!(
                "chartgen/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/5queezer/chartgen)"
            ))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new())
    });
    let response = client.get(url).send().map_err(|e| {
        if e.is_timeout() {
            "timed out".to_string()
        } else {
            "unreachable".to_string()
        }
    })?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let mut body = Vec::new();
    response
        .take(MAX_BYTES)
        .read_to_end(&mut body)
        .map_err(|_| "read failed".to_string())?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Which child of an item a text belongs to.
#[derive(Clone, Copy, PartialEq)]
enum Field {
    Title,
    Link,
    Date,
    Category,
}

/// The items of an RSS 2.0 or Atom feed that have a title, a link and a time they were
/// published. Only an item's own children are read, so `media:title` and the like are not.
fn parse(xml: &str) -> Result<Vec<Item>, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().allow_dangling_amp = true;
    let mut items = Vec::new();
    // The item being read, and how deep in it the reader is: 1 inside one of its children.
    let mut item: Option<Item> = None;
    let mut depth = 0;
    let mut field: Option<Field> = None;
    let mut text = String::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("not a feed: {e}"))?;
        match event {
            Event::Start(e) => {
                let name = e.name();
                if let Some(it) = item.as_mut() {
                    depth += 1;
                    if depth == 1 {
                        text.clear();
                        field = match name.as_ref() {
                            "title" => Some(Field::Title),
                            "link" => Some(Field::Link),
                            "pubDate" | "published" | "dc:date" => Some(Field::Date),
                            "updated" if it.published == 0 => Some(Field::Date),
                            "category" => Some(Field::Category),
                            _ => None,
                        };
                        atom_attributes(it, &e);
                    }
                } else if matches!(name.as_ref(), "item" | "entry") {
                    item = Some(Item::default());
                    depth = 0;
                }
            }
            Event::Empty(e) => {
                if let Some(it) = item.as_mut() {
                    if depth == 0 {
                        atom_attributes(it, &e);
                    }
                }
            }
            Event::Text(e) if field.is_some() => {
                text.push_str(&e.html_content());
            }
            Event::CData(e) if field.is_some() => {
                text.push_str(&e.html_content());
            }
            Event::GeneralRef(e) if field.is_some() => {
                if let Ok(Some(c)) = e.resolve_char_ref() {
                    text.push(c);
                } else if let Some(s) =
                    quick_xml::escape::resolve_predefined_entity(&e.html_content())
                {
                    text.push_str(s);
                }
            }
            Event::End(_) => {
                let Some(it) = item.as_mut() else {
                    continue;
                };
                if depth == 0 {
                    if let Some(it) = item.take() {
                        if !it.title.is_empty() && !it.url.is_empty() && it.published != 0 {
                            items.push(it);
                        }
                    }
                    continue;
                }
                if depth == 1 {
                    let value = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    match field.take() {
                        Some(Field::Title) => it.title = value,
                        Some(Field::Link) if it.url.is_empty() && !value.is_empty() => {
                            it.url = clean_url(&value)
                        }
                        Some(Field::Date) => {
                            if let Some(t) = time_of(&value) {
                                it.published = t;
                            }
                        }
                        Some(Field::Category) if !value.is_empty() => it.categories.push(value),
                        _ => {}
                    }
                }
                depth -= 1;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    // A feed always carries headlines; none is a page that is not the feed, such as a
    // challenge page.
    if items.is_empty() {
        return Err("no headlines in it".into());
    }
    Ok(items)
}

/// An Atom entry's link (`<link href>`, the alternate one) and category (`<category term>`).
fn atom_attributes(item: &mut Item, e: &quick_xml::events::BytesStart) {
    let attr = |key: &str| {
        e.try_get_attribute(key)
            .ok()
            .flatten()
            .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok())
            .map(|v| v.trim().to_string())
    };
    match e.name().as_ref() {
        "link" => {
            let alternate = attr("rel").is_none_or(|r| r == "alternate");
            if let Some(href) = attr("href").filter(|_| alternate && item.url.is_empty()) {
                item.url = clean_url(&href);
            }
        }
        "category" => {
            if let Some(term) = attr("term").filter(|t| !t.is_empty()) {
                item.categories.push(term);
            }
        }
        _ => {}
    }
}

/// RFC 2822 (RSS) or RFC 3339 (Atom), as unix seconds.
fn time_of(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc2822(text)
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(text))
        .ok()
        .map(|t| t.timestamp())
}

/// A link without its `utm_` tracking parameters.
fn clean_url(link: &str) -> String {
    let Ok(mut url) = url::Url::parse(link) else {
        return link.to_string();
    };
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| !k.starts_with("utm_"))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept.is_empty() {
        url.set_query(None);
    } else {
        url.query_pairs_mut().clear().extend_pairs(kept);
    }
    url.to_string()
}

/// The words a headline about `symbol` is found by: its coin's ticker and the names in
/// `NAMES`. A ticker of under 3 letters that has a name (OP) is left to its name.
fn terms_of(symbol: &str) -> Vec<String> {
    let mut coin = symbol;
    for quote in QUOTES {
        if let Some(base) = coin.strip_suffix(quote).filter(|b| !b.is_empty()) {
            coin = base;
            break;
        }
    }
    // Binance's 1000PEPEUSDT and 1MBABYDOGEUSDT trade PEPE and BABYDOGE.
    for scale in ["1000000", "1000", "1M"] {
        if let Some(base) = coin.strip_prefix(scale).filter(|b| b.len() >= 2) {
            coin = base;
            break;
        }
    }
    let names = NAMES
        .iter()
        .find(|(ticker, _)| *ticker == coin)
        .map_or(&[][..], |(_, names)| *names);
    let mut terms = Vec::new();
    if coin.len() >= 3 || names.is_empty() {
        terms.push(coin.to_string());
    }
    terms.extend(names.iter().map(|n| n.to_string()));
    terms
}

/// Whether `text` holds `word` as a whole word: no letter or digit on either side. A
/// ticker (a word in capitals) in capitals only, so LINK is not "link"; a name in any case.
fn has_word(text: &str, word: &str) -> bool {
    let (text, word) = if word.chars().any(char::is_lowercase) {
        (text.to_lowercase(), word.to_lowercase())
    } else {
        (text.to_string(), word.to_string())
    };
    text.match_indices(&word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

fn about(item: &Item, terms: &[String]) -> bool {
    std::iter::once(&item.title)
        .chain(&item.categories)
        .any(|text| terms.iter().any(|t| has_word(text, t)))
}

/// The answer from what each feed gave: the newest `limit` headlines about `symbol`
/// published in the `hours` before `now` (unix seconds).
fn answer(symbol: &str, hours: u64, limit: usize, now: i64, reads: &[(&Feed, Fetched)]) -> Value {
    let terms = terms_of(symbol);
    let since = now - hours as i64 * 3600;
    let mut found: Vec<(&Feed, &Item)> = Vec::new();
    let mut failed = Vec::new();
    let mut read_ok = Vec::new();
    for (feed, read) in reads {
        match read {
            Ok(items) => {
                read_ok.push(feed.name.as_str());
                found.extend(
                    items
                        .iter()
                        .filter(|i| i.published >= since && about(i, &terms))
                        .map(|i| (*feed, i)),
                );
            }
            Err(e) => failed.push(json!({ "source": feed.name, "error": e })),
        }
    }
    found.sort_by_key(|(_, i)| std::cmp::Reverse(i.published));
    let mut seen = std::collections::HashSet::new();
    found.retain(|(_, i)| seen.insert(i.url.as_str()));
    let more = found.len().saturating_sub(limit);
    let items: Vec<Value> = found
        .iter()
        .take(limit)
        .map(|(feed, i)| {
            json!({
                "time": chartgen::drawings::iso(i.published * 1000),
                "source": feed.name,
                "title": i.title,
                "url": i.url,
            })
        })
        .collect();
    let mut v = json!({
        "symbol": symbol,
        "terms": terms,
        "hours": hours,
        "items": items,
        "sources": read_ok,
        "sources_failed": failed,
    });
    if more > 0 {
        v["more"] = json!(more);
    }
    v
}

fn integer(args: &Value, key: &str, default: u64, max: u64) -> Result<u64, String> {
    match args.get(key).filter(|v| !v.is_null()) {
        None => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=max).contains(n))
            .ok_or_else(|| format!("{key} must be an integer from 1 to {max}")),
    }
}

/// `get_news`: one symbol's recent headlines. A feed that cannot be read is named under
/// `sources_failed` and never fails the call.
pub fn call_tool(id: Option<Value>, args: &Value) -> Value {
    if crate::replay::global().is_some() {
        return tool_err(
            id,
            "This is a replay server: get_news is not available, because it reads what the fixtures do not hold",
        );
    }
    let Some(text) = args.get("symbol").and_then(Value::as_str) else {
        return tool_err(id, "symbol is required, e.g. \"BTCUSDT\"");
    };
    let Some(symbol) = crate::mcp::normalize_quote_symbol(text) else {
        return tool_err(id, &format!("{text:?} is not a symbol"));
    };
    let (hours, limit) = match (
        integer(args, "hours", 24, MAX_HOURS),
        integer(args, "limit", 10, MAX_LIMIT),
    ) {
        (Ok(h), Ok(l)) => (h, l as usize),
        (Err(e), _) | (_, Err(e)) => return tool_err(id, &e),
    };
    let feeds = match feeds() {
        Ok(feeds) if !feeds.is_empty() => feeds,
        Ok(_) => return tool_err(id, "No news feeds: CHARTGEN_NEWS_FEEDS is empty"),
        Err(e) => return tool_err(id, e),
    };
    // At once, so a call waits for the slowest feed rather than for all of them in turn.
    let reads: Vec<(&Feed, Fetched)> = std::thread::scope(|s| {
        let handles: Vec<_> = feeds.iter().map(|f| s.spawn(move || read(f))).collect();
        feeds
            .iter()
            .zip(handles)
            .map(|(f, h)| (f, h.join().unwrap_or_else(|_| Err("failed".into()))))
            .collect()
    });
    let now = chrono::Utc::now().timestamp();
    tool_ok(id, answer(&symbol, hours, limit, now, &reads))
}

#[cfg(test)]
mod tests;
