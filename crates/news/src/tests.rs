use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static DOWNLOADS: AtomicUsize = AtomicUsize::new(0);

pub(super) fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}.xml", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).unwrap()
}

/// A source of a test.
pub(super) fn source(name: &str, url: &str) -> Source {
    Source {
        id: name.to_lowercase().replace(' ', "-"),
        name: name.into(),
        url: url.into(),
        ..Source::default()
    }
}

/// The sources a test reads, which `download` serves: the five of ADR-0086, the default
/// set, and a sixth, which is not, and a seventh about Solana only.
pub(super) fn sources() -> Vec<Source> {
    let mut sources: Vec<Source> = [
        (
            "CoinDesk",
            "https://www.coindesk.com/arc/outboundfeeds/rss/",
        ),
        ("Cointelegraph", "https://cointelegraph.com/rss"),
        ("Decrypt", "https://decrypt.co/feed"),
        ("The Block", "https://www.theblock.co/rss.xml"),
        ("Bitcoin Magazine", "https://bitcoinmagazine.com/.rss/full/"),
        ("Wide", "https://wide.example/feed"),
        ("r/solana", "https://www.reddit.com/r/solana/.rss"),
    ]
    .iter()
    .map(|(name, url)| Source {
        default: true,
        ..source(name, url)
    })
    .collect();
    sources[5].default = false;
    sources[6] = Source {
        id: "r-solana".into(),
        kind: Kind::Reddit,
        coins: vec!["SOL".into()],
        default: false,
        ..sources[6].clone()
    };
    sources
}

/// The test's feeds, served from fixtures: no test reaches an outlet. Decrypt is down.
pub(super) fn download(url: &str) -> Result<String, String> {
    DOWNLOADS.fetch_add(1, Ordering::SeqCst);
    let name = match url {
        u if u.contains("coindesk") => "coindesk",
        u if u.contains("cointelegraph") => "cointelegraph",
        u if u.contains("bitcoinmagazine") => "bitcoinmagazine",
        u if u.contains("theblock") => "atom",
        u if u.contains("wide.example") => "cointelegraph",
        u if u.contains("reddit") => "reddit",
        _ => return Err("HTTP 503".into()),
    };
    Ok(fixture(name))
}

fn parsed(name: &str) -> Vec<Item> {
    parse(&fixture(name), "https://news.example.com/feed").unwrap()
}

fn titles(v: &Value) -> Vec<&str> {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap())
        .collect()
}

/// 2026-11-25T08:00:00Z.
pub(super) const NOW: i64 = 1_795_593_600;

#[test]
fn an_rss_item_is_its_own_title_link_time_and_categories() {
    let items = parsed("coindesk");
    // The item without a date and the one whose link is not http(s) are left out.
    assert_eq!(items.len(), 4);
    assert_eq!(
        items[0],
        Item {
            title: "Solana developers propose a fix for slow block production".into(),
            url: "https://www.coindesk.com/tech/2026/11/25/solana-fix-example".into(),
            published: time_of("Wed, 25 Nov 2026 06:33:10 +0000").unwrap(),
            from_updated: false,
            categories: vec!["Tech".into(), "News".into()],
            author: "Example Author".into(),
        }
    );
}

#[test]
fn entities_are_decoded_and_tracking_parameters_dropped() {
    let items = parsed("cointelegraph");
    assert_eq!(
        items[0].title,
        "Bitcoin\u{2019}s funding turns negative & shorts pile in"
    );
    assert_eq!(
        items[0].url,
        "https://cointelegraph.com/news/bitcoin-funding-negative-example"
    );
    assert_eq!(
        items[1].url,
        "https://cointelegraph.com/news/lender-pauses-example?id=7"
    );
}

#[test]
fn an_atom_entry_takes_its_alternate_link_and_published_time() {
    let items = parsed("atom");
    assert_eq!(items.len(), 3);
    assert_eq!(
        items[0].title,
        "Solana\u{2019}s validators vote on fee changes"
    );
    assert_eq!(items[0].url, "https://news.example.com/solana-validators");
    assert_eq!(items[0].published, time_of("2026-11-25T06:00:00Z").unwrap());
    assert_eq!(items[0].categories, vec!["Markets".to_string()]);
    // No published: its updated time, and that is said.
    assert_eq!(items[1].published, time_of("2026-11-25T02:00:00Z").unwrap());
    assert!(items[1].from_updated && !items[0].from_updated);
    // An author's name, not its email.
    assert_eq!(items[0].author, "Example Author");
    assert_eq!(items[1].categories, vec!["Solana".to_string()]);
    assert_eq!(items[2].url, "https://news.example.com/stories/relative");
}

#[test]
fn a_page_that_is_no_feed_is_an_error() {
    let feed = "https://news.example.com/feed";
    assert!(parse("<html><body><p>Just a moment...</body></html>", feed).is_err());
    assert!(parse("<html><body><p>Just a moment...</p></body></html>", feed).is_err());
}

#[test]
fn a_symbol_is_found_by_its_coins_ticker_and_names() {
    let terms = |s: &str| terms_of(s);
    assert_eq!(terms("BTCUSDT"), ["BTC", "Bitcoin"]);
    assert_eq!(terms("ETHUSDC"), ["ETH", "Ethereum", "Ether"]);
    assert_eq!(terms("SOLUSDT"), ["SOL", "Solana"]);
    assert_eq!(terms("BTC-USD"), ["BTC", "Bitcoin"]);
    assert_eq!(terms("1000PEPEUSDT"), ["PEPE", "Pepe"]);
    // A two-letter ticker is found by its name, and as a cashtag.
    assert_eq!(terms("OPUSDT"), ["OP", "Optimism"]);
    assert!(is_about("OPUSDT", "$OP unlock this week"));
    assert!(!is_about("OPUSDT", "OP ed: markets"));
    assert_eq!(terms("WIFUSDT"), ["WIF", "dogwifhat"]);
    assert_eq!(terms("ZZZUSDT"), ["ZZZ"]);
}

fn headline(title: &str, categories: &[&str]) -> Item {
    Item {
        title: title.into(),
        categories: categories.iter().map(|c| c.to_string()).collect(),
        ..Item::default()
    }
}

fn is_about(symbol: &str, title: &str) -> bool {
    about(&headline(title, &[]), &terms_of(symbol))
}

#[test]
fn a_term_is_a_whole_word_and_a_ticker_in_capitals() {
    assert!(is_about("BTCUSDT", "Bitcoin\u{2019}s funding"));
    assert!(is_about("BTCUSDT", "BITCOIN holders"));
    assert!(is_about("ETHUSDT", "Ethereum-based lender"));
    assert!(is_about("BTCUSDT", "Buy $BTC now"));
    assert!(is_about("ETHUSDT", "ETH/BTC ratio"));
    assert!(!is_about("ETHUSDT", "Etherscan adds a label"));
    assert!(!is_about("SOLUSDT", "SOLID is not it"));
    assert!(!is_about("LINKUSDT", "a new link to funds"));
    assert!(!is_about("BTCUSDT", "Btc in lower case"));
    assert!(is_about("LINKUSDT", "Chainlink oracles"));
}

#[test]
fn a_short_ticker_without_a_name_is_only_a_cashtag() {
    assert_eq!(terms_of("AIUSDT"), ["AI"]);
    assert!(!is_about("AIUSDT", "AI agents fail 4 in 10 tasks"));
    assert!(!is_about("AUSDT", "A year after buying bitcoin"));
    assert!(!is_about("SUSDT", "U.S. lawmakers weigh a stablecoin bill"));
    assert!(!about(
        &headline("U.S. policy", &["U.S."]),
        &terms_of("SUSDT")
    ));
    assert!(is_about("AIUSDT", "$AI jumps 20%"));
    assert!(is_about("SUSDT", "$S rallies"));
    // Outlets file stories about artificial intelligence under "AI".
    assert!(!about(
        &headline("Miner pivots to compute", &["AI"]),
        &terms_of("AIUSDT")
    ));
}

#[test]
fn a_name_that_is_an_ordinary_word_is_matched_capitalised() {
    assert!(!is_about("OPUSDT", "Crypto optimism returns to markets"));
    assert!(is_about("OPUSDT", "Optimism upgrade goes live"));
    assert!(!is_about("XRPUSDT", "A ripple effect across exchanges"));
    assert!(is_about("XRPUSDT", "Ripple wins a licence"));
    // Not an ordinary word: any case.
    assert!(is_about("SOLUSDT", "solana validators vote"));
}

#[test]
fn a_headline_dated_after_now_is_left_out_beyond_a_skew() {
    let feed = source("Outlet", "https://outlet.example/rss");
    let at = |published: i64, url: &str| Item {
        title: "Bitcoin headline".into(),
        url: url.into(),
        published,
        ..Item::default()
    };
    let items = vec![
        at(NOW - 60, "https://outlet.example/now"),
        at(NOW + 3600, "https://outlet.example/future"),
    ];
    let v = answer("BTCUSDT", 24, 1, NOW, &[(&feed, Ok(Arc::new(items)))]);
    assert_eq!(v["items"][0]["url"], "https://outlet.example/now");
    assert!(v.get("more").is_none());
    // Within five minutes ahead: an outlet's clock running ahead, kept.
    let items = vec![at(NOW + 120, "https://outlet.example/ahead")];
    let v = answer("BTCUSDT", 24, 1, NOW, &[(&feed, Ok(Arc::new(items)))]);
    assert_eq!(v["items"][0]["url"], "https://outlet.example/ahead");
}

#[test]
fn an_item_with_an_empty_link_is_left_out() {
    let xml = r#"<rss><channel>
        <item><title>Bitcoin one</title><link></link><pubDate>Wed, 25 Nov 2026 06:00:00 +0000</pubDate></item>
        <item><title>Bitcoin two</title><link> </link><pubDate>Wed, 25 Nov 2026 05:00:00 +0000</pubDate></item>
        <item><title>Bitcoin three</title><link>https://news.example.com/three</link><pubDate>Wed, 25 Nov 2026 04:00:00 +0000</pubDate></item>
    </channel></rss>"#;
    let items = parse(xml, "https://news.example.com/feed").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].url, "https://news.example.com/three");
    // An Atom link with an empty href, the same.
    let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry><title>Bitcoin</title>
        <link href=""/><published>2026-11-25T06:00:00Z</published></entry></feed>"#;
    assert!(parse(atom, "https://news.example.com/feed").is_err());
}

#[test]
fn the_answer_is_the_newest_headlines_about_the_symbol_in_the_window() {
    let feeds: Vec<Source> = ["CoinDesk", "Cointelegraph", "Bitcoin Magazine", "Decrypt"]
        .iter()
        .map(|n| source(n, &format!("https://{}.example/rss", n.replace(' ', ""))))
        .collect();
    let reads: Vec<(&Source, Fetched)> = vec![
        (&feeds[0], Ok(Arc::new(parsed("coindesk")))),
        (&feeds[1], Ok(Arc::new(parsed("cointelegraph")))),
        (&feeds[2], Ok(Arc::new(parsed("bitcoinmagazine")))),
        (&feeds[3], Err("HTTP 503".into())),
    ];
    let v = answer("BTCUSDT", 24, 10, NOW, &reads);
    assert_eq!(
        titles(&v),
        [
            "Bitcoin\u{2019}s funding turns negative & shorts pile in",
            // Its link is in Bitcoin Magazine's feed too: shown once, from the first feed.
            "BTC slips below $90K as ETF outflows continue",
            // By its category.
            "Long-Term Holders Return to Accumulation",
        ]
    );
    assert_eq!(v["items"][1]["source"], "CoinDesk");
    assert_eq!(v["items"][0]["time"], "2026-11-25T06:54:59Z");
    assert_eq!(
        v["items"][0]["url"],
        "https://cointelegraph.com/news/bitcoin-funding-negative-example"
    );
    assert_eq!(v["terms"], json!(["BTC", "Bitcoin"]));
    assert_eq!(
        v["sources"],
        json!(["CoinDesk", "Cointelegraph", "Bitcoin Magazine"])
    );
    assert_eq!(
        v["sources_failed"],
        json!([{ "source": "Decrypt", "error": "HTTP 503" }])
    );
    assert!(v.get("more").is_none());
    // Over three days, the miners' story is in too; two of four are shown.
    let v = answer("BTCUSDT", 72, 2, NOW, &reads);
    assert_eq!(titles(&v).len(), 2);
    assert_eq!(v["more"], 2);
    // Pinned in a file the web app's types are tested against
    // (`web/src/types/responses.test.ts`). To write it again after a deliberate change:
    // `CHARTGEN_WRITE_FIXTURES=1 cargo test news::`.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/news_response.json");
    let text = serde_json::to_string_pretty(&v).unwrap() + "\n";
    if std::env::var("CHARTGEN_WRITE_FIXTURES").is_ok() {
        std::fs::write(&path, &text).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), text);
    let v = answer("ETHUSDT", 24, 10, NOW, &reads);
    assert_eq!(
        titles(&v),
        [
            "Ethereum-based lender pauses withdrawals after exploit",
            "Ether options traders turn bearish into year-end",
        ]
    );
}

#[test]
fn feeds_are_read_once_in_ten_minutes_and_a_failed_one_is_named() {
    let call = || read(None, "SOLUSDT", 168, 10, &Pick::Default, "en").unwrap();
    let v = call();
    let read = DOWNLOADS.load(Ordering::SeqCst);
    assert_eq!(v["symbol"], "SOLUSDT");
    assert_eq!(
        v["sources"],
        json!(["CoinDesk", "Cointelegraph", "The Block", "Bitcoin Magazine"])
    );
    assert_eq!(
        v["sources_failed"],
        json!([{ "source": "Decrypt", "error": "HTTP 503" }])
    );
    call();
    // Held: the second call read nothing, the failed feed included (within its minute).
    assert_eq!(DOWNLOADS.load(Ordering::SeqCst), read);
}

#[test]
fn an_answer_is_from_the_default_set_unless_sources_are_named() {
    let names = |v: &Value| v["sources"].clone();
    let v = read(None, "SOLUSDT", 168, 10, &Pick::All, "en").unwrap();
    assert_eq!(
        names(&v),
        json!([
            "CoinDesk",
            "Cointelegraph",
            "The Block",
            "Bitcoin Magazine",
            "Wide",
            "r/solana"
        ])
    );
    let ids = |ids: &[&str]| Pick::Ids(ids.iter().map(|i| i.to_string()).collect());
    let v = read(None, "SOLUSDT", 168, 10, &ids(&["wide"]), "en").unwrap();
    assert_eq!(names(&v), json!(["Wide"]));
    assert!(read(None, "SOLUSDT", 168, 10, &ids(&["nope"]), "en")
        .unwrap_err()
        .contains("not a source"));
    let v = read(None, "SOLUSDT", 168, 10, &Pick::All, "de").unwrap();
    assert_eq!((names(&v), v["items"].clone()), (json!([]), json!([])));
}

#[test]
fn a_source_about_certain_coins_answers_for_those_only() {
    let sources = sources();
    let solana = &sources[6];
    let url = &solana.url;
    let posts = Ok(Arc::new(parse(&fixture("reddit"), url).unwrap()));
    let reads = [(solana, posts)];
    // Its posts are Solana's, whether they name it or not.
    let v = answer("SOLUSDT", 24, 10, NOW, &reads);
    assert_eq!(
        titles(&v),
        [
            "Validators are upgrading tonight",
            "Bridging Bitcoin over, any tips?",
            "Is staking worth it?"
        ]
    );
    // Not Bitcoin's, though one names it.
    assert!(titles(&answer("BTCUSDT", 24, 10, NOW, &reads)).is_empty());
    // A subreddit's author is its user's name, not the link to them.
    let Ok(posts) = &reads[0].1 else { panic!() };
    assert_eq!(posts[0].author, "/u/example_one");
}

#[test]
fn a_coin_without_a_name_is_said_to_be_found_by_its_ticker_alone() {
    let feed = source("Outlet", "https://outlet.example/rss");
    let read = |symbol: &str| answer(symbol, 24, 10, NOW, &[(&feed, Ok(Arc::new(Vec::new())))]);
    assert!(read("BTCUSDT").get("note").is_none());
    assert_eq!(read("TIAUSDT")["terms"], json!(["TIA", "Celestia"]));
    assert!(read("TIAUSDT").get("note").is_none());
    let v = read("ZZZUSDT");
    assert_eq!(v["terms"], json!(["ZZZ"]));
    assert!(v["note"]
        .as_str()
        .unwrap()
        .contains("No name is known for ZZZ"));
}

#[test]
fn the_coins_an_agent_is_woken_for_have_names() {
    assert!(is_about("ONEUSDT", "Harmony bridge reopens"));
    assert!(!is_about("ONEUSDT", "Markets in harmony after the Fed"));
    assert!(is_about("FETUSDT", "Fetch.ai token jumps on listing"));
    assert!(is_about("SAGAUSDT", "Saga launches a gaming chain"));
    assert!(is_about("TIAUSDT", "celestia rollups grow"));
    // A two-letter ticker is found by its name, and as a cashtag.
    assert_eq!(terms_of("ZKUSDT"), ["ZK", "ZKsync"]);
}

#[test]
fn a_headlines_coins_are_those_of_the_table_it_names_and_its_cashtags() {
    let coins = |title: &str, categories: &[&str]| coins_of(&headline(title, categories));
    assert_eq!(
        coins("Bitcoin and Ether slide as $HYPE jumps", &[]),
        ["BTC", "ETH", "HYPE"]
    );
    assert_eq!(coins("Solana validators vote", &["Ripple"]), ["SOL", "XRP"]);
    // Once, whether by name or cashtag; a price or a cashtag in lower case is none.
    assert_eq!(coins("$BTC: Bitcoin to $100K?", &[]), ["BTC"]);
    assert!(coins("Fees fall to $5 as $pepe holders wait", &[]).is_empty());
    assert!(coins("Crypto optimism returns", &[]).is_empty());
    assert_eq!(coins("A$AI is not a cashtag; $AI is", &[]), ["AI"]);
}
