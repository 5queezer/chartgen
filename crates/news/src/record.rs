//! Recording the news (ADR-0088).
//!
//! A thread reads every enabled source about every five minutes, asking with its `ETag` and
//! `Last-Modified` so an unchanged feed is not sent again, and records each headline once,
//! by its link: the source (id, name, kind, language), title, link, published time, when
//! it was first read, the categories, the author and the coins matched then. A source that
//! fails is tried again after 5, 10, 20 minutes, up to 6 hours, or after what its
//! `Retry-After` asks. Each source's last read is kept in the store too, and `get_news`
//! answers from the record while it is current. The sources are read again each round, so
//! a change to them takes effect within five minutes.

use super::{coins_of, compose, parse, request, Failed, Got, Item, Source};
use super::{Recorded, Store};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// How often every feed is read.
const ROUND_S: i64 = 300;
/// The wait after each request. Feeds are on different hosts but two, so no host is
/// asked more than twice in a round, and those two at least this far apart.
const PACE: Duration = Duration::from_secs(2);
/// The longest a failing feed waits for its next try.
const MAX_BACKOFF_S: i64 = 6 * 3600;
/// The longest a feed's `Retry-After` is followed.
const MAX_RETRY_AFTER_S: i64 = 24 * 3600;
/// The record answers `get_news` while a feed was read within this.
const CURRENT_S: i64 = 30 * 60;
const DEFAULT_DAYS: i64 = 365;
const DAY_S: i64 = 86_400;

/// `CHARTGEN_NEWS_RECORD_DAYS`: how long a headline is kept after it was published,
/// default 365; 0 records nothing.
fn days() -> Result<i64, String> {
    match std::env::var("CHARTGEN_NEWS_RECORD_DAYS") {
        Err(_) => Ok(DEFAULT_DAYS),
        Ok(v) => {
            v.trim().parse().ok().filter(|d| *d >= 0).ok_or_else(|| {
                format!("CHARTGEN_NEWS_RECORD_DAYS must be a number of days, not {v}")
            })
        }
    }
}

/// Start recording into `store`, unless turned off.
pub fn start(store: &'static dyn Store) {
    let days = match days() {
        Ok(0) => return,
        Ok(days) => days,
        Err(e) => {
            eprintln!("[news-record] {e}; the news is not recorded");
            return;
        }
    };
    let spawned = std::thread::Builder::new()
        .name("news-record".into())
        .spawn(move || {
            let (mut new, mut rounds, mut first) = (0, 0, true);
            loop {
                let began = Instant::now();
                let enabled: Vec<Source> = match super::sources() {
                    Ok(all) => all.iter().filter(|s| s.enabled).cloned().collect(),
                    Err(e) => {
                        if first {
                            eprintln!("[news-record] {e}; nothing is recorded until it is fixed");
                        }
                        Vec::new()
                    }
                };
                new += round(
                    store,
                    &enabled,
                    days,
                    &mut || chrono::Utc::now().timestamp(),
                    &mut request,
                    &mut || std::thread::sleep(PACE),
                );
                rounds += 1;
                // After the first round, then once an hour: how much came in.
                if first || rounds == 12 {
                    eprintln!(
                        "[news-record] {new} new headlines from {} sources in the last {rounds} rounds",
                        enabled.len()
                    );
                    (new, rounds, first) = (0, 0, false);
                }
                let round = Duration::from_secs(ROUND_S as u64);
                std::thread::sleep(round.saturating_sub(began.elapsed()));
            }
        });
    if let Err(e) = spawned {
        eprintln!("[news-record] cannot start: {e}");
    }
}

type Fetch<'a> = dyn FnMut(&str, Option<&str>, Option<&str>) -> Result<Got, Failed> + 'a;

/// One round: each source that is due read and its new headlines recorded, then headlines
/// older than `days` dropped. The number of new headlines.
fn round(
    store: &dyn Store,
    feeds: &[Source],
    days: i64,
    clock: &mut dyn FnMut() -> i64,
    fetch: &mut Fetch,
    pace: &mut dyn FnMut(),
) -> usize {
    let known = store.feeds().unwrap_or_else(|e| {
        eprintln!("[news-record] cannot read the feeds' state: {e}");
        Vec::new()
    });
    let mut new = 0;
    for feed in feeds {
        let mut state = known
            .iter()
            .find(|s| s.id == feed.id)
            .cloned()
            .unwrap_or_default();
        // A source given another URL starts afresh: what it knew was the old URL's.
        if state.url != feed.url {
            state = Default::default();
        }
        // A feed is read at another point of the next round, when the feeds before it
        // took longer or shorter: due within half a round.
        let now = clock();
        if state.next_attempt > now + ROUND_S / 2 {
            continue;
        }
        let got = fetch(
            &feed.url,
            state.etag.as_deref(),
            state.last_modified.as_deref(),
        );
        pace();
        let now = clock();
        let read = got.and_then(|got| match got {
            Got::NotModified => Ok(None),
            Got::Feed {
                xml,
                etag,
                last_modified,
            } => parse(&xml, &feed.url)
                .map(|items| Some((items, etag, last_modified)))
                .map_err(Failed::from),
        });
        state.id.clone_from(&feed.id);
        state.url.clone_from(&feed.url);
        state.name.clone_from(&feed.name);
        match read {
            Ok(read) => {
                if let Some((items, etag, last_modified)) = read {
                    let rows = rows(feed, &items, now - days * DAY_S, now);
                    match store.save_items(&rows) {
                        Ok(n) => new += n,
                        Err(e) => eprintln!("[news-record] cannot record {}: {e}", feed.name),
                    }
                    // Only from a feed that was read: a page that is not one must not
                    // be asked for with its validators and answer "not modified".
                    (state.etag, state.last_modified) = (etag, last_modified);
                }
                if state.failures > 0 {
                    eprintln!(
                        "[news-record] {} is read again, after {} failures",
                        feed.name, state.failures
                    );
                }
                state.last_success = Some(now);
                (state.last_error, state.failing_since) = (None, None);
                (state.failures, state.next_attempt) = (0, 0);
            }
            Err(Failed { error, retry_after }) => {
                state.failures += 1;
                if state.failures == 1 {
                    eprintln!("[news-record] {} failed: {error}", feed.name);
                }
                let backoff = (ROUND_S << (state.failures - 1).min(10)).min(MAX_BACKOFF_S);
                let asked = retry_after.unwrap_or(0).clamp(0, MAX_RETRY_AFTER_S);
                state.next_attempt = now + backoff.max(asked);
                state.failing_since.get_or_insert(now);
                state.last_error = Some(error);
            }
        }
        if let Err(e) = store.save_feed(&state) {
            eprintln!("[news-record] cannot save {}'s state: {e}", feed.name);
        }
    }
    let now = clock();
    if let Err(e) = store.forget_items(now - days * DAY_S) {
        eprintln!("[news-record] cannot drop old headlines: {e}");
    }
    new
}

/// The rows to record of a source's items published since `oldest`, first seen `now`. The
/// coins of a source about certain coins only are those.
fn rows(source: &Source, items: &[Item], oldest: i64, now: i64) -> Vec<Recorded> {
    items
        .iter()
        .filter(|i| i.published >= oldest)
        .map(|i| Recorded {
            url: i.url.clone(),
            source: source.id.clone(),
            outlet: source.name.clone(),
            kind: source.kind.as_str().into(),
            language: source.language.clone(),
            title: i.title.clone(),
            published: i.published,
            from_updated: i.from_updated,
            first_seen: now,
            categories: i.categories.clone(),
            coins: match source.coins.is_empty() {
                true => coins_of(i),
                false => source.coins.clone(),
            },
            author: Some(i.author.clone()).filter(|a| !a.is_empty()),
        })
        .collect()
}

/// `get_news`'s answer from the record of `feeds`, as it would be from the feeds, while a
/// source was read within the last 30 minutes; else None, and the feeds are read. A source
/// whose last read failed is in `sources_failed` with `since`, its first failure in a row,
/// and its headlines recorded before still count.
pub(super) fn answer(
    store: &dyn Store,
    feeds: &[&Source],
    symbol: &str,
    hours: u64,
    limit: usize,
    now: i64,
) -> Option<Value> {
    let states = store.feeds().ok()?;
    if !states
        .iter()
        .any(|s| s.last_success.is_some_and(|t| now - t <= CURRENT_S))
    {
        return None;
    }
    let rows = store
        .items(now - hours as i64 * 3600, now + super::SKEW_S)
        .ok()?;
    let items: Vec<(&Source, Item)> = rows
        .iter()
        .filter_map(|r| {
            let source = feeds
                .iter()
                .find(|s| s.id == r.source && s.language == r.language)?;
            let item = Item {
                title: r.title.clone(),
                url: r.url.clone(),
                published: r.published,
                from_updated: r.from_updated,
                categories: r.categories.clone(),
                author: r.author.clone().unwrap_or_default(),
            };
            Some((*source, item))
        })
        .collect();
    let found = items.iter().map(|(o, i)| (*o, i)).collect();
    let (mut read_ok, mut failed) = (Vec::new(), Vec::new());
    for feed in feeds {
        match states.iter().find(|s| s.id == feed.id) {
            Some(s) if s.failures == 0 => read_ok.push(feed.name.as_str()),
            Some(s) => {
                let mut f = json!({
                    "source": feed.name,
                    "error": s.last_error.as_deref().unwrap_or("failed"),
                });
                if let Some(t) = s.failing_since {
                    f["since"] = json!(super::iso(t));
                }
                failed.push(f);
            }
            None => failed.push(json!({ "source": feed.name, "error": "not read yet" })),
        }
    }
    read_ok.dedup();
    Some(compose(symbol, hours, limit, now, found, read_ok, failed))
}

#[cfg(test)]
mod tests;
