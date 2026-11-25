use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static DOWNLOADS: AtomicUsize = AtomicUsize::new(0);

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/news/{name}.xml",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).unwrap()
}

/// The default feeds, served from fixtures: no test reaches an outlet. Decrypt is down.
pub(super) fn download(url: &str) -> Result<String, String> {
    DOWNLOADS.fetch_add(1, Ordering::SeqCst);
    let name = match url {
        u if u.contains("coindesk") => "coindesk",
        u if u.contains("cointelegraph") => "cointelegraph",
        u if u.contains("bitcoinmagazine") => "bitcoinmagazine",
        u if u.contains("theblock") => "atom",
        _ => return Err("HTTP 503".into()),
    };
    Ok(fixture(name))
}

fn parsed(name: &str) -> Vec<Item> {
    parse(&fixture(name)).unwrap()
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
const NOW: i64 = 1_795_593_600;

#[test]
fn an_rss_item_is_its_own_title_link_time_and_categories() {
    let items = parsed("coindesk");
    // The item without a date is left out.
    assert_eq!(items.len(), 4);
    assert_eq!(
        items[0],
        Item {
            title: "Solana developers propose a fix for slow block production".into(),
            url: "https://www.coindesk.com/tech/2026/11/25/solana-fix-example".into(),
            published: time_of("Wed, 25 Nov 2026 06:33:10 +0000").unwrap(),
            categories: vec!["Tech".into(), "News".into()],
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
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0].title,
        "Solana\u{2019}s validators vote on fee changes"
    );
    assert_eq!(items[0].url, "https://news.example.com/solana-validators");
    assert_eq!(items[0].published, time_of("2026-11-25T06:00:00Z").unwrap());
    assert_eq!(items[0].categories, vec!["Markets".to_string()]);
    // No published: its updated time.
    assert_eq!(items[1].published, time_of("2026-11-25T02:00:00Z").unwrap());
    assert_eq!(items[1].categories, vec!["Solana".to_string()]);
}

#[test]
fn a_page_that_is_no_feed_is_an_error() {
    assert!(parse("<html><body><p>Just a moment...</body></html>").is_err());
    assert!(parse("<html><body><p>Just a moment...</p></body></html>").is_err());
}

#[test]
fn a_symbol_is_found_by_its_coins_ticker_and_names() {
    let terms = |s: &str| terms_of(s);
    assert_eq!(terms("BTCUSDT"), ["BTC", "Bitcoin"]);
    assert_eq!(terms("ETHUSDC"), ["ETH", "Ethereum", "Ether"]);
    assert_eq!(terms("SOLUSDT"), ["SOL", "Solana"]);
    assert_eq!(terms("BTC-USD"), ["BTC", "Bitcoin"]);
    assert_eq!(terms("1000PEPEUSDT"), ["PEPE"]);
    // A two-letter ticker with a name is found by its name only.
    assert_eq!(terms("OPUSDT"), ["Optimism"]);
    assert_eq!(terms("WIFUSDT"), ["WIF"]);
}

#[test]
fn a_term_is_a_whole_word_and_a_ticker_in_capitals() {
    assert!(has_word("Bitcoin\u{2019}s funding", "Bitcoin"));
    assert!(has_word("BITCOIN holders", "Bitcoin"));
    assert!(has_word("Ethereum-based lender", "Ethereum"));
    assert!(has_word("Buy $BTC now", "BTC"));
    assert!(has_word("ETH/BTC ratio", "ETH"));
    assert!(!has_word("Etherscan adds a label", "Ether"));
    assert!(!has_word("SOLID is not it", "SOL"));
    assert!(!has_word("a new link to funds", "LINK"));
    assert!(!has_word("Btc in lower case", "BTC"));
    assert!(has_word("Chainlink oracles", "Chainlink"));
}

#[test]
fn the_answer_is_the_newest_headlines_about_the_symbol_in_the_window() {
    let feeds: Vec<Feed> = ["CoinDesk", "Cointelegraph", "Bitcoin Magazine", "Decrypt"]
        .iter()
        .map(|n| Feed {
            name: n.to_string(),
            url: format!("https://{n}.example/rss"),
        })
        .collect();
    let reads: Vec<(&Feed, Fetched)> = vec![
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
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/news_response.json");
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
    let call = || {
        let r = call_tool(
            Some(json!(1)),
            &json!({ "symbol": "solusdt", "hours": 168 }),
        );
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        serde_json::from_str::<Value>(text).unwrap()
    };
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
fn bad_arguments_are_refused() {
    let err = |args: Value| {
        let r = call_tool(None, &args);
        assert_eq!(r["result"]["isError"], true, "{args}");
        r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(err(json!({})).contains("symbol is required"));
    assert!(err(json!({ "symbol": "BTC USDT" })).contains("is not a symbol"));
    assert!(err(json!({ "symbol": "BTCUSDT", "hours": 0 })).contains("hours"));
    assert!(err(json!({ "symbol": "BTCUSDT", "hours": 169 })).contains("hours"));
    assert!(err(json!({ "symbol": "BTCUSDT", "limit": 51 })).contains("limit"));
}

#[test]
fn the_feeds_are_named_urls_separated_by_commas() {
    let feeds =
        feeds_from(" Outlet A=https://a.example/rss , https://b.example/feed?x=1,").unwrap();
    assert_eq!(
        feeds,
        [
            Feed {
                name: "Outlet A".into(),
                url: "https://a.example/rss".into()
            },
            Feed {
                name: "b.example".into(),
                url: "https://b.example/feed?x=1".into()
            },
        ]
    );
    assert_eq!(feeds_from("").unwrap(), []);
    assert!(feeds_from("Old=ftp://a.example/rss").is_err());
}
