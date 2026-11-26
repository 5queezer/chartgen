use super::super::tests::{fixture, NOW};
use super::*;
use crate::FeedState;
use std::sync::{Arc, Mutex};

/// A store in memory, as chartgen's SQLite store keeps the record.
#[derive(Default)]
struct Memory {
    items: Mutex<Vec<Recorded>>,
    feeds: Mutex<Vec<FeedState>>,
    /// Every save fails, as a full disk would.
    broken: Mutex<bool>,
}

impl Store for Memory {
    fn save_items(&self, items: &[Recorded]) -> Result<usize, String> {
        if *self.broken.lock().unwrap() {
            return Err("disk full".into());
        }
        let mut held = self.items.lock().unwrap();
        let mut new = 0;
        for i in items {
            match held.iter_mut().find(|h| h.url == i.url) {
                Some(h) if !h.carried_by.contains(&i.source) => h.carried_by.push(i.source.clone()),
                Some(_) => {}
                None => {
                    held.push(Recorded {
                        carried_by: vec![i.source.clone()],
                        ..i.clone()
                    });
                    new += 1;
                }
            }
        }
        Ok(new)
    }

    fn items(&self, from: i64, to: i64) -> Result<Vec<Recorded>, String> {
        let mut found: Vec<Recorded> = (self.items.lock().unwrap().iter())
            .filter(|i| (from..=to).contains(&i.published))
            .cloned()
            .collect();
        found.sort_by_key(|i| std::cmp::Reverse(i.published));
        Ok(found)
    }

    fn forget_items(&self, before: i64) -> Result<usize, String> {
        let mut held = self.items.lock().unwrap();
        let n = held.len();
        held.retain(|i| i.published >= before);
        Ok(n - held.len())
    }

    fn feeds(&self) -> Result<Vec<FeedState>, String> {
        Ok(self.feeds.lock().unwrap().clone())
    }

    fn save_feed(&self, feed: &FeedState) -> Result<(), String> {
        let mut held = self.feeds.lock().unwrap();
        held.retain(|f| f.url != feed.url);
        held.push(feed.clone());
        Ok(())
    }
}

fn feed(name: &str, url: &str) -> Source {
    super::super::tests::source(name, url)
}

/// Two feeds: an RSS feed with an `ETag`, and an Atom feed with a `Last-Modified`.
fn two() -> Vec<Source> {
    vec![
        feed("CoinDesk", "https://www.coindesk.com/rss"),
        feed("Atom News", "https://news.example.com/atom"),
    ]
}

type Asked = Vec<(String, Option<String>, Option<String>)>;

/// The outlets: each answers its fixture, or `304` when asked with what it last sent;
/// `down` ones fail. Records what was asked.
fn outlets<'a>(
    asked: &'a mut Asked,
    down: &'a [&'a str],
) -> impl FnMut(&str, Option<&str>, Option<&str>) -> Result<Got, Failed> + 'a {
    move |url, etag, modified| {
        asked.push((url.into(), etag.map(Into::into), modified.map(Into::into)));
        if down.iter().any(|d| url.contains(d)) {
            return Err(Failed {
                error: "HTTP 503".into(),
                retry_after: None,
            });
        }
        let (xml, tag, at) = match url {
            u if u.contains("coindesk") => (fixture("coindesk"), Some("\"v1\""), None),
            u if u.contains("example.com/atom") => {
                (fixture("atom"), None, Some("Wed, 25 Nov 2026 07:21:37 GMT"))
            }
            _ => (
                "<html><body>Just a moment</body></html>".into(),
                Some("\"x\""),
                None,
            ),
        };
        if (tag.is_some() && etag == tag) || (at.is_some() && modified == at) {
            return Ok(Got::NotModified);
        }
        Ok(Got::Feed {
            xml,
            etag: tag.map(Into::into),
            last_modified: at.map(Into::into),
        })
    }
}

fn refs(sources: &[Source]) -> Vec<&Source> {
    sources.iter().collect()
}

fn at(t: i64) -> impl FnMut() -> i64 {
    move || t
}

fn recorded(store: &dyn Store) -> Vec<Recorded> {
    store.items(0, i64::MAX).unwrap()
}

fn state(store: &dyn Store, url: &str) -> FeedState {
    store
        .feeds()
        .unwrap()
        .into_iter()
        .find(|s| s.url == url)
        .unwrap()
}

#[test]
fn a_round_records_each_headline_once_with_when_it_was_first_read() {
    let store = Memory::default();
    let feeds = two();
    let mut asked = Vec::new();
    let mut paced = 0;
    let new = round(
        &store,
        &feeds,
        365,
        &mut at(NOW),
        &mut outlets(&mut asked, &[]),
        &mut || paced += 1,
    );
    // CoinDesk's four with a date and an http(s) link, and the Atom feed's three.
    assert_eq!((new, paced), (7, 2));
    let rows = recorded(&store);
    assert_eq!(rows.len(), 7);
    let btc = rows
        .iter()
        .find(|r| r.title.starts_with("BTC slips"))
        .unwrap();
    assert_eq!(
        *btc,
        Recorded {
            url: "https://www.coindesk.com/markets/2026/11/25/btc-slips-example".into(),
            source: "coindesk".into(),
            carried_by: vec!["coindesk".into()],
            outlet: "CoinDesk".into(),
            kind: "rss".into(),
            language: "en".into(),
            title: "BTC slips below $90K as ETF outflows continue".into(),
            published: super::super::time_of("Wed, 25 Nov 2026 05:15:08 +0000").unwrap(),
            from_updated: false,
            first_seen: NOW,
            categories: vec!["Markets".into()],
            coins: vec!["BTC".into()],
            author: None,
        }
    );
    // Filed under a coin, and dated by its update as it has no published time.
    let filed = rows
        .iter()
        .find(|r| r.title.contains("filed under"))
        .unwrap();
    assert_eq!(
        (filed.coins.clone(), filed.from_updated),
        (vec!["SOL".into()], true)
    );

    // Five minutes on: asked with what each sent, nothing new, the first read kept.
    asked.clear();
    let new = round(
        &store,
        &feeds,
        365,
        &mut at(NOW + 300),
        &mut outlets(&mut asked, &[]),
        &mut || {},
    );
    assert_eq!(new, 0);
    assert_eq!(
        asked,
        [
            (
                "https://www.coindesk.com/rss".into(),
                Some("\"v1\"".into()),
                None
            ),
            (
                "https://news.example.com/atom".into(),
                None,
                Some("Wed, 25 Nov 2026 07:21:37 GMT".into())
            ),
        ]
    );
    assert!(recorded(&store).iter().all(|r| r.first_seen == NOW));
    assert_eq!(state(&store, &feeds[0].url).last_success, Some(NOW + 300));
}

#[test]
fn a_failing_feed_waits_longer_after_each_failure_and_says_since_when() {
    let store = Memory::default();
    let feeds = two();
    let url = feeds[0].url.clone();
    let mut asked = Vec::new();
    let mut tries = Vec::new();
    // Every five minutes for two hours.
    for i in 0..24 {
        let t = NOW + i * 300;
        let before = asked.len();
        round(
            &store,
            &feeds,
            365,
            &mut at(t),
            &mut outlets(&mut asked, &["coindesk"]),
            &mut || {},
        );
        if asked[before..].iter().any(|a| a.0 == url) {
            tries.push((t - NOW) / 60);
        }
    }
    // After 5, 10, 20, 40 minutes, then 80; the other feed every round.
    assert_eq!(tries, [0, 5, 15, 35, 75]);
    assert_eq!(asked.iter().filter(|a| a.0 != url).count(), 24);
    let s = state(&store, &url);
    assert_eq!(
        (s.failures, s.failing_since, s.last_error.as_deref()),
        (5, Some(NOW), Some("HTTP 503"))
    );
    // Read again: all is forgotten.
    let mut asked = Vec::new();
    round(
        &store,
        &feeds,
        365,
        &mut at(NOW + 155 * 60),
        &mut outlets(&mut asked, &[]),
        &mut || {},
    );
    let s = state(&store, &url);
    assert_eq!(
        (s.failures, s.failing_since, s.last_error, s.next_attempt),
        (0, None, None, 0)
    );
}

#[test]
fn a_feed_that_asks_to_wait_is_left_alone_that_long() {
    let store = Memory::default();
    let feeds = vec![feed("Slow", "https://slow.example/rss")];
    let ask = |t: i64, asked: &mut usize| {
        round(
            &store,
            &feeds,
            365,
            &mut at(t),
            &mut |_: &str, _: Option<&str>, _: Option<&str>| {
                *asked += 1;
                Err(Failed {
                    error: "HTTP 429".into(),
                    retry_after: Some(3600),
                })
            },
            &mut || {},
        );
    };
    let mut asked = 0;
    for i in 0..12 {
        ask(NOW + i * 300, &mut asked);
    }
    assert_eq!(asked, 1);
    ask(NOW + 3600, &mut asked);
    assert_eq!(asked, 2);
}

#[test]
fn a_page_that_is_not_the_feed_fails_and_its_validators_are_not_kept() {
    let store = Memory::default();
    let feeds = vec![feed("Challenged", "https://challenged.example/rss")];
    let mut asked = Vec::new();
    for t in [NOW, NOW + 300] {
        round(
            &store,
            &feeds,
            365,
            &mut at(t),
            &mut outlets(&mut asked, &[]),
            &mut || {},
        );
    }
    // Asked again without its ETag, so it cannot answer "not modified".
    assert_eq!(asked[1].1, None);
    let s = state(&store, &feeds[0].url);
    assert_eq!((s.failures, s.etag), (2, None));
    assert!(s.last_error.unwrap().starts_with("no headlines"));
}

#[test]
fn headlines_older_than_the_retention_are_dropped_and_not_recorded_again() {
    let store = Memory::default();
    let feeds = two();
    let mut asked = Vec::new();
    // Two days' retention a week on: only what was published since.
    let later = NOW + 7 * DAY_S;
    let old = Recorded {
        url: "https://old.example/a".into(),
        source: "old".into(),
        carried_by: vec![],
        outlet: "Old".into(),
        kind: "rss".into(),
        language: "en".into(),
        title: "Bitcoin a year ago".into(),
        published: later - 3 * DAY_S,
        from_updated: false,
        first_seen: later - 3 * DAY_S,
        categories: vec![],
        coins: vec!["BTC".into()],
        author: None,
    };
    store.save_items(&[old]).unwrap();
    let new = round(
        &store,
        &feeds,
        2,
        &mut at(later),
        &mut outlets(&mut asked, &[]),
        &mut || {},
    );
    assert_eq!(new, 0);
    assert!(recorded(&store).is_empty());
}

#[test]
fn get_news_answers_from_the_record_as_from_the_feeds_while_it_is_current() {
    let store = Memory::default();
    let feeds = two();
    let mut asked = Vec::new();
    round(
        &store,
        &feeds,
        365,
        &mut at(NOW),
        &mut outlets(&mut asked, &[]),
        &mut || {},
    );
    let parsed = |name: &str, url: &str| Ok(Arc::new(parse(&fixture(name), url).unwrap()));
    let live = super::super::answer(
        "SOLUSDT",
        24,
        10,
        NOW,
        &[
            (&feeds[0], parsed("coindesk", &feeds[0].url)),
            (&feeds[1], parsed("atom", &feeds[1].url)),
        ],
    );
    let from_record = answer(&store, &refs(&feeds), "SOLUSDT", 24, 10, NOW).unwrap();
    assert_eq!(from_record, live);
    assert_eq!(live["items"].as_array().unwrap().len(), 3);

    // A feed failing since: named with the time, its recorded headlines still in.
    let mut asked = Vec::new();
    round(
        &store,
        &feeds,
        365,
        &mut at(NOW + 300),
        &mut outlets(&mut asked, &["coindesk"]),
        &mut || {},
    );
    let v = answer(&store, &refs(&feeds), "SOLUSDT", 24, 10, NOW + 300).unwrap();
    assert_eq!(v["items"], live["items"]);
    assert_eq!(v["sources"], json!(["Atom News"]));
    assert_eq!(
        v["sources_failed"],
        json!([{ "source": "CoinDesk", "error": "HTTP 503", "since": "2026-11-25T08:05:00Z" }])
    );
    // Pinned for the web app's types, as `news_response.json` is; written again with
    // `CHARTGEN_WRITE_FIXTURES=1 cargo test -p chartgen-news get_news_answers_from_the_record`.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/news_recorded_response.json");
    let text = serde_json::to_string_pretty(&v).unwrap() + "\n";
    if std::env::var("CHARTGEN_WRITE_FIXTURES").is_ok() {
        std::fs::write(&path, &text).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), text);
    // Half an hour after the last read the record is not current: the feeds are read.
    assert!(answer(
        &store,
        &refs(&feeds),
        "SOLUSDT",
        24,
        10,
        NOW + 300 + 31 * 60
    )
    .is_none());
    // A store that never recorded: the feeds are read.
    assert!(answer(&Memory::default(), &refs(&feeds), "SOLUSDT", 24, 10, NOW).is_none());
}

#[test]
fn a_store_that_fails_leaves_the_source_to_be_read_again_in_full() {
    let store = Memory::default();
    let feeds = two();
    *store.broken.lock().unwrap() = true;
    let mut asked = Vec::new();
    let new = round(
        &store,
        &feeds,
        365,
        &mut at(NOW),
        &mut outlets(&mut asked, &[]),
        &mut || {},
    );
    assert_eq!(new, 0);
    let s = state(&store, &feeds[0].url);
    // Neither current nor holding what the feed sent, so it is not told "not modified".
    assert_eq!((s.etag, s.last_success), (None, None));
    assert!(s.last_error.unwrap().contains("disk full"));
    // The store is back: the next try reads the feed whole and records it.
    *store.broken.lock().unwrap() = false;
    let mut asked = Vec::new();
    let new = round(
        &store,
        &feeds,
        365,
        &mut at(NOW + 300),
        &mut outlets(&mut asked, &[]),
        &mut || {},
    );
    assert_eq!((new, asked[0].1.clone()), (7, None));
}

#[test]
fn a_retry_after_that_is_no_multiple_of_a_round_is_kept_to_the_second() {
    let store = Memory::default();
    let feeds = vec![feed("Slow", "https://slow.example/rss")];
    let mut asked = 0;
    for t in [NOW, NOW + 300, NOW + 600] {
        round(
            &store,
            &feeds,
            365,
            &mut at(t),
            &mut |_: &str, _: Option<&str>, _: Option<&str>| {
                asked += 1;
                Err(Failed {
                    error: "HTTP 429".into(),
                    retry_after: Some(400),
                })
            },
            &mut || {},
        );
    }
    // Asked at 0, not at 300 (100 s early), again at 600.
    assert_eq!(asked, 2);
}

#[test]
fn a_headline_two_sources_carry_is_one_row_and_answers_for_either() {
    let store = Memory::default();
    let a = feed("Desk A", "https://a.example/rss");
    let b = feed("Desk B", "https://b.example/rss");
    let item = Item {
        title: "Solana validators vote".into(),
        url: "https://news.example.com/solana".into(),
        published: NOW - 600,
        ..Item::default()
    };
    store
        .save_items(&rows(&a, std::slice::from_ref(&item), 0, NOW - 300))
        .unwrap();
    assert_eq!(store.save_items(&rows(&b, &[item], 0, NOW)).unwrap(), 0);
    store
        .save_feed(&FeedState {
            id: b.id.clone(),
            url: b.url.clone(),
            last_success: Some(NOW),
            ..FeedState::default()
        })
        .unwrap();
    // Asked for B alone, the headline is B's.
    let v = answer(&store, &[&b], "SOLUSDT", 24, 10, NOW).unwrap();
    assert_eq!(v["items"][0]["source"], "Desk B");
    // Asked for both, it is shown once, as the first that read it.
    let v = answer(&store, &[&a, &b], "SOLUSDT", 24, 10, NOW).unwrap();
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
    assert_eq!(v["items"][0]["source"], "Desk A");
}
