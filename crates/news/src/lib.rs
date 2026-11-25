//! The news of chartgen's `get_news` (ADR-0086, ADR-0088): recent headlines about one crypto
//! symbol from the public RSS and Atom feeds of crypto outlets and financial outlets' markets
//! desks, and a recorder that keeps every headline those feeds carry.
//!
//! The interface is narrow, so this can move out of chartgen into a service of its own:
//! [`record::start`] records into a [`Store`] the host provides, [`read`] answers one
//! symbol's headlines (from the record while it is current, else from the feeds now), and
//! [`Store::feeds`] is each feed's status.
//!
//! No key and no article text: a headline's title, outlet, time and link. A feed that
//! cannot be read is named in the answer and never fails it. A headline is about the
//! symbol when its title or one of the feed's categories for it names the coin's ticker or
//! name, as whole words: a heuristic, which can miss a story or take one that only
//! mentions the coin.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::{json, Value};

mod sources;
pub use sources::{select, sources, Kind, Pick, Source};

/// How long a feed that was read is served before it is read again.
const FRESH: Duration = Duration::from_secs(600);
/// How long a feed that could not be read waits before it is tried again.
const RETRY: Duration = Duration::from_secs(60);
/// How far ahead of now a headline's time may be, for an outlet whose clock runs ahead.
const SKEW_S: i64 = 300;
/// The most hours `read` reaches back.
pub const MAX_HOURS: u64 = 168;
/// The most headlines `read` answers.
pub const MAX_LIMIT: u64 = 50;
/// The quote assets a symbol's coin is read off: BTCUSDT is BTC.
const QUOTES: [&str; 6] = ["FDUSD", "USDT", "USDC", "BUSD", "-USD", "USD"];
/// Names a headline may call a coin by, besides its ticker. Matched in any case, except
/// those in `ORDINARY`.
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

/// Names that are also ordinary words ("crypto optimism", "a ripple effect"): matched only
/// as written, capitalised.
const ORDINARY: [&str; 7] = [
    "Ripple",
    "Avalanche",
    "Optimism",
    "Polygon",
    "Cosmos",
    "Stellar",
    "Sui",
];

/// One headline of a feed.
#[derive(Clone, Debug, Default, PartialEq)]
struct Item {
    title: String,
    url: String,
    /// Unix seconds.
    published: i64,
    /// `published` is an Atom entry's `updated`, as it has no `published`.
    from_updated: bool,
    categories: Vec<String>,
    /// Who the feed says wrote it (`dc:creator`, `author`, an Atom author's `name`), at most
    /// 100 characters; empty when it does not say. A press-release wire shows here.
    author: String,
}

type Fetched = Result<Arc<Vec<Item>>, String>;
/// Per feed URL: when it was last read, and what that gave. Its own lock, held during a
/// read, so callers at once share one request.
type Slot = Arc<Mutex<Option<(Instant, Fetched)>>>;

static SLOTS: LazyLock<Mutex<HashMap<String, Slot>>> = LazyLock::new(Mutex::default);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A feed's headlines, as held if fresh. Blocking.
fn read_feed(feed: &Source) -> Fetched {
    let slot = lock(&SLOTS).entry(feed.url.clone()).or_default().clone();
    let mut held = lock(&slot);
    if let Some((at, read)) = &*held {
        if at.elapsed() < if read.is_ok() { FRESH } else { RETRY } {
            return read.clone();
        }
    }
    let read = download(&feed.url).and_then(|xml| parse(&xml, &feed.url).map(Arc::new));
    *held = Some((Instant::now(), read.clone()));
    read
}

#[cfg(test)]
fn download(url: &str) -> Result<String, String> {
    tests::download(url)
}

#[cfg(not(test))]
fn download(url: &str) -> Result<String, String> {
    match request(url, None, None) {
        Ok(Got::Feed { xml, .. }) => Ok(xml),
        Ok(Got::NotModified) => Err("HTTP 304".into()),
        Err(e) => Err(e.error),
    }
}

/// What a request for a feed gave.
#[derive(Debug, PartialEq)]
enum Got {
    /// The feed, and what to ask with next time, so an unchanged feed is not sent again.
    Feed {
        xml: String,
        etag: Option<String>,
        last_modified: Option<String>,
    },
    /// `304 Not Modified`: unchanged since the `ETag` or `Last-Modified` asked with.
    NotModified,
}

/// Why a feed could not be read, and how many seconds it asked to be left alone
/// (`Retry-After`), if it did.
#[derive(Debug, PartialEq)]
struct Failed {
    error: String,
    retry_after: Option<i64>,
}

impl From<String> for Failed {
    fn from(error: String) -> Self {
        Self {
            error,
            retry_after: None,
        }
    }
}

/// Read a feed; conditionally, when `etag` or `last_modified` is given. Blocking.
fn request(url: &str, etag: Option<&str>, last_modified: Option<&str>) -> Result<Got, Failed> {
    use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, RETRY_AFTER};
    use std::io::Read;
    /// One feed's request, connection to last byte.
    const TIMEOUT: Duration = Duration::from_secs(8);
    /// The most of a feed that is read; the largest default is under 2 MB.
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
    let mut req = client.get(url);
    if let Some(etag) = etag {
        req = req.header(IF_NONE_MATCH, etag);
    }
    if let Some(at) = last_modified {
        req = req.header(IF_MODIFIED_SINCE, at);
    }
    let response = req.send().map_err(|e| {
        Failed::from(match e.is_timeout() {
            true => "timed out".to_string(),
            false => "unreachable".to_string(),
        })
    })?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(Got::NotModified);
    }
    let header = |name: reqwest::header::HeaderName| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    if !status.is_success() {
        return Err(Failed {
            error: format!("HTTP {}", status.as_u16()),
            retry_after: header(RETRY_AFTER).and_then(|v| v.trim().parse().ok()),
        });
    }
    let (etag, last_modified) = (header(ETAG), header(LAST_MODIFIED));
    let mut body = Vec::new();
    response
        .take(MAX_BYTES)
        .read_to_end(&mut body)
        .map_err(|_| Failed::from("read failed".to_string()))?;
    Ok(Got::Feed {
        xml: String::from_utf8_lossy(&body).into_owned(),
        etag,
        last_modified,
    })
}

/// Which child of an item a text belongs to.
#[derive(Clone, Copy, PartialEq)]
enum Field {
    Title,
    Link,
    Date,
    /// An Atom entry's `updated`, read while it has no `published`.
    Updated,
    Category,
    Author,
}

/// The items of an RSS 2.0 or Atom feed that have a title, an http(s) link and a time they
/// were published (an Atom entry without `published`: when it was updated). Only an item's
/// own children are read, so `media:title` and the like are not. A relative link is read
/// against the feed's URL (`xml:base` is not read). Atom's elements are read unprefixed, as
/// feeds write them: an Atom feed whose elements carry a prefix (`atom:entry`) has no items.
fn parse(xml: &str, feed_url: &str) -> Result<Vec<Item>, String> {
    let base = url::Url::parse(feed_url).map_err(|e| format!("bad feed URL: {e}"))?;
    let mut reader = Reader::from_str(xml);
    reader.config_mut().allow_dangling_amp = true;
    let mut items = Vec::new();
    // The item being read, and how deep in it the reader is: 1 inside one of its children.
    let mut item: Option<Item> = None;
    let mut depth = 0;
    let mut field: Option<Field> = None;
    // Inside an Atom author's child other than its `name` (`email`, `uri`).
    let mut skip = false;
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
                            "updated" if it.published == 0 => Some(Field::Updated),
                            "category" => Some(Field::Category),
                            "author" | "dc:creator" => Some(Field::Author),
                            _ => None,
                        };
                        atom_attributes(it, &e, &base);
                    } else if depth == 2 && field == Some(Field::Author) {
                        skip = name.as_ref() != "name";
                    }
                } else if matches!(name.as_ref(), "item" | "entry") {
                    item = Some(Item::default());
                    depth = 0;
                }
            }
            Event::Empty(e) => {
                if let Some(it) = item.as_mut() {
                    if depth == 0 {
                        atom_attributes(it, &e, &base);
                    }
                }
            }
            Event::Text(e) if field.is_some() && !skip => {
                text.push_str(&e.html_content());
            }
            Event::CData(e) if field.is_some() && !skip => {
                text.push_str(&e.html_content());
            }
            Event::GeneralRef(e) if field.is_some() && !skip => {
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
                        Some(Field::Link) if it.url.is_empty() => {
                            it.url = clean_url(&value, &base).unwrap_or_default()
                        }
                        Some(f @ (Field::Date | Field::Updated)) => {
                            if let Some(t) = time_of(&value) {
                                it.published = t;
                                it.from_updated = f == Field::Updated;
                            }
                        }
                        Some(Field::Category) if !value.is_empty() => it.categories.push(value),
                        Some(Field::Author) if it.author.is_empty() => {
                            it.author = value.chars().take(100).collect()
                        }
                        _ => {}
                    }
                }
                if depth == 2 {
                    skip = false;
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
fn atom_attributes(item: &mut Item, e: &quick_xml::events::BytesStart, base: &url::Url) {
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
                item.url = clean_url(&href, base).unwrap_or_default();
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

/// A link, read against `base` if relative, without its `utm_` tracking parameters. None
/// unless it is http(s), and for an empty link, which would read as the feed's own URL.
fn clean_url(link: &str, base: &url::Url) -> Option<String> {
    if link.trim().is_empty() {
        return None;
    }
    let mut url = base
        .join(link)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))?;
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
    Some(url.to_string())
}

/// The words a headline about `symbol` is found by: its coin's ticker and the names in
/// `NAMES`. A ticker of under 3 letters that has a name (OP) is left to its name; one without
/// (AI) is kept, and `about` matches it only as `$AI`.
fn terms_of(symbol: &str) -> Vec<String> {
    let coin = coin_of(symbol);
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

/// A symbol's coin: BTCUSDT, BTC-USD and 1000PEPEUSDT are BTC, BTC and PEPE.
fn coin_of(symbol: &str) -> &str {
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
    coin
}

/// Whether `text` holds `word` as a whole word: no letter or digit on either side. `exact`:
/// in the same case, else in any case.
fn has_word(text: &str, word: &str, exact: bool) -> bool {
    let (text, word) = if exact {
        (text.to_string(), word.to_string())
    } else {
        (text.to_lowercase(), word.to_lowercase())
    };
    text.match_indices(&word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// Whether a headline is about the coin: its title or a category names a term. A ticker (a
/// term in capitals) in capitals only, so LINK is not "link"; one of under 3 letters (AI, S)
/// only as `$AI` in the title, since "AI" and "U.S." are everyday words, and outlets file
/// stories about artificial intelligence under an "AI" category. A name in any case, except an ordinary word, which only as written.
fn about(item: &Item, terms: &[String]) -> bool {
    terms.iter().any(|t| {
        let ticker = !t.chars().any(char::is_lowercase);
        if ticker && t.len() < 3 {
            return has_word(&item.title, &format!("${t}"), true);
        }
        let exact = ticker || ORDINARY.contains(&t.as_str());
        std::iter::once(&item.title)
            .chain(&item.categories)
            .any(|text| has_word(text, t, exact))
    })
}

/// The coins a headline is about, as their tickers: those of `NAMES` that `about` finds in
/// it, then each cashtag in its title (`$PEPE`), once. Recorded with the headline
/// (ADR-0088), so a later change to the table does not rewrite what was matched then.
fn coins_of(item: &Item) -> Vec<String> {
    let mut coins: Vec<String> = NAMES
        .iter()
        .filter(|(ticker, _)| about(item, &terms_of(ticker)))
        .map(|(ticker, _)| ticker.to_string())
        .collect();
    let title = &item.title;
    for (at, _) in title.match_indices('$') {
        if title[..at]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
        {
            continue;
        }
        let tag: String = title[at + 1..]
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        let cashtag = tag.len() <= 10
            && tag.starts_with(|c: char| c.is_ascii_uppercase())
            && !tag.chars().any(|c| c.is_ascii_lowercase());
        if cashtag && !coins.contains(&tag) {
            coins.push(tag);
        }
    }
    coins
}

/// The answer from what each feed gave: the newest `limit` headlines about `symbol`
/// published in the `hours` before `now` (unix seconds).
fn answer(symbol: &str, hours: u64, limit: usize, now: i64, reads: &[(&Source, Fetched)]) -> Value {
    let mut found: Vec<(&Source, &Item)> = Vec::new();
    let mut failed = Vec::new();
    let mut read_ok = Vec::new();
    for (feed, read) in reads {
        match read {
            Ok(items) => {
                read_ok.push(feed.name.as_str());
                found.extend(items.iter().map(|i| (*feed, i)));
            }
            Err(e) => failed.push(json!({ "source": feed.name, "error": e })),
        }
    }
    compose(symbol, hours, limit, now, found, read_ok, failed)
}

/// The answer from headlines, each with its source, and the sources read and not read. A
/// headline of a source about certain coins only is about the symbol when its coin is one
/// of them; any other when `about` says so.
fn compose(
    symbol: &str,
    hours: u64,
    limit: usize,
    now: i64,
    mut found: Vec<(&Source, &Item)>,
    read_ok: Vec<&str>,
    failed: Vec<Value>,
) -> Value {
    let terms = terms_of(symbol);
    let coin = coin_of(symbol);
    let since = now - hours as i64 * 3600;
    found.retain(|(source, i)| {
        let about = match source.coins.is_empty() {
            true => about(i, &terms),
            false => source.coins.iter().any(|c| c == coin),
        };
        (since..=now + SKEW_S).contains(&i.published) && about
    });
    found.sort_by_key(|(_, i)| std::cmp::Reverse(i.published));
    let mut seen = std::collections::HashSet::new();
    found.retain(|(_, i)| seen.insert(i.url.as_str()));
    let more = found.len().saturating_sub(limit);
    let items: Vec<Value> = found
        .iter()
        .take(limit)
        .map(|(source, i)| {
            json!({
                "time": iso(i.published),
                "source": source.name,
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

/// `2026-11-25T08:00:00Z`, from unix seconds.
fn iso(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| t.to_string())
}

/// One symbol's headlines published in the last `hours` (1 to `MAX_HOURS`), the newest
/// `limit` (1 to `MAX_LIMIT`), from the enabled sources of `language` that `pick` names:
/// from `store`'s record while it is current, else from the sources, read now or held from
/// a read within ten minutes. `symbol` as chartgen normalises it (`BTCUSDT`). An error
/// without sources, or for a source `pick` names that is not one or is switched off.
/// Blocking.
pub fn read(
    store: Option<&dyn Store>,
    symbol: &str,
    hours: u64,
    limit: usize,
    pick: &Pick,
    language: &str,
) -> Result<Value, String> {
    let all = sources()?;
    if all.is_empty() {
        return Err("No news sources: the list is empty".into());
    }
    let picked = select(&all, pick, language)?;
    let now = chrono::Utc::now().timestamp();
    if let Some(v) =
        store.and_then(|store| record::answer(store, &picked, symbol, hours, limit, now))
    {
        return Ok(v);
    }
    // At once, so a call waits for the slowest feed rather than for all of them in turn.
    let reads: Vec<(&Source, Fetched)> = std::thread::scope(|s| {
        let handles: Vec<_> = picked
            .iter()
            .map(|f| s.spawn(move || read_feed(f)))
            .collect();
        picked
            .iter()
            .zip(handles)
            .map(|(f, h)| (*f, h.join().unwrap_or_else(|_| Err("failed".into()))))
            .collect()
    });
    Ok(answer(symbol, hours, limit, now, &reads))
}

/// A headline as recorded (ADR-0088). Times in unix seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Recorded {
    /// The link, as the feed gave it less its `utm_` parameters: what makes a headline one.
    pub url: String,
    /// The source's id.
    pub source: String,
    /// The source's name.
    pub outlet: String,
    /// The source's kind (`rss`, `atom`, `reddit`, `youtube`).
    pub kind: String,
    /// The source's language.
    pub language: String,
    /// The title as the feed wrote it, its whitespace collapsed.
    pub title: String,
    pub published: i64,
    /// `published` is an Atom entry's `updated`, as it had no `published`.
    pub from_updated: bool,
    /// When the recorder first read it.
    pub first_seen: i64,
    /// The outlet's categories for it.
    pub categories: Vec<String>,
    /// The coins matched when it was recorded, as tickers.
    pub coins: Vec<String>,
    /// Who the feed says wrote it; None when it does not say.
    pub author: Option<String>,
}

/// A feed's last read, by the recorder (ADR-0088): its status. Times in unix seconds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeedState {
    /// The source's id.
    pub id: String,
    /// The URL it was read from: its `etag` and `last_modified` are this URL's.
    pub url: String,
    pub name: String,
    /// What to ask with next time, so an unchanged feed is not sent again.
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub last_success: Option<i64>,
    /// Why the last read failed, while it is failing.
    pub last_error: Option<String>,
    /// The first failure of the reads failing in a row.
    pub failing_since: Option<i64>,
    /// Reads failed in a row.
    pub failures: i64,
    /// Not read before this, after a failure.
    pub next_attempt: i64,
}

/// Where the record is kept: the host's store (in chartgen, its SQLite file).
pub trait Store: Sync {
    /// Record headlines; one whose link is recorded already is left as it was. How many
    /// were new.
    fn save_items(&self, items: &[Recorded]) -> Result<usize, String>;
    /// The headlines published from `from` to `to`, both inclusive, the newest first.
    fn items(&self, from: i64, to: i64) -> Result<Vec<Recorded>, String>;
    /// Delete the headlines published before `before`; how many went.
    fn forget_items(&self, before: i64) -> Result<usize, String>;
    /// Every source's last read.
    fn feeds(&self) -> Result<Vec<FeedState>, String>;
    fn save_feed(&self, feed: &FeedState) -> Result<(), String>;
}

pub mod record;
#[cfg(test)]
mod tests;
