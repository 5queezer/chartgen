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
    days_from(std::env::var("CHARTGEN_NEWS_RECORD_DAYS").ok().as_deref())
}

/// The most days a headline can be kept: a century, far inside what the cutoff's arithmetic
/// holds, so a cutoff can never land in the future and drop what is current.
const MAX_DAYS: i64 = 36_500;

fn days_from(value: Option<&str>) -> Result<i64, String> {
    match value {
        None => Ok(DEFAULT_DAYS),
        Some(v) => v
            .trim()
            .parse()
            .ok()
            .filter(|d| (0..=MAX_DAYS).contains(d))
            .ok_or_else(|| {
                format!("CHARTGEN_NEWS_RECORD_DAYS must be a number of days from 0 to {MAX_DAYS}, not {v}")
            }),
    }
}

/// The oldest a headline may be published to be kept, `days` before `now`.
fn cutoff(now: i64, days: i64) -> i64 {
    now.saturating_sub(days.clamp(0, MAX_DAYS) * DAY_S)
}

/// Whether the news is recorded (`CHARTGEN_NEWS_RECORD_DAYS` is not 0, nor bad). A record
/// left from before it was turned off is not answered from.
pub fn enabled() -> bool {
    days().is_ok_and(|d| d > 0)
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
        let now = clock();
        if state.next_attempt > now {
            continue;
        }
        // Kept longer now than when the validators were saved: what the feed left out
        // then is read again, whole.
        if days > state.kept_days {
            (state.etag, state.last_modified) = (None, None);
        }
        let got = fetch(
            &feed.url,
            state.etag.as_deref(),
            state.last_modified.as_deref(),
        );
        pace();
        let now = clock();
        // What is new is recorded before the source counts as read: its validators move
        // on, and it is current, only once the headlines are in the store. A store that
        // fails is a failure to try again, never a "not modified" later.
        let read = got.and_then(|got| match got {
            Got::NotModified => Ok(None),
            Got::Feed {
                xml,
                etag,
                last_modified,
            } => {
                let items = parse(&xml, &feed.url).map_err(Failed::from)?;
                let rows = rows(feed, &items, cutoff(now, days), now);
                let saved = store
                    .save_items(&rows)
                    .map_err(|e| Failed::from(format!("cannot record: {e}")))?;
                new += saved;
                Ok(Some((etag, last_modified)))
            }
        });
        state.id.clone_from(&feed.id);
        state.url.clone_from(&feed.url);
        state.name.clone_from(&feed.name);
        match read {
            Ok(read) => {
                // Only from a feed that was read and recorded: a page that is not one must
                // not be asked for with its validators and answer "not modified".
                if let Some((etag, last_modified)) = read {
                    (state.etag, state.last_modified) = (etag, last_modified);
                    state.kept_days = days;
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
                // The backoff less half a round, as the source is read at another point of a
                // later round when the sources before it took longer or shorter; what the
                // source asked for (`Retry-After`), never early.
                let backoff = (ROUND_S << (state.failures - 1).min(10)).min(MAX_BACKOFF_S);
                let asked = retry_after.unwrap_or(0).clamp(0, MAX_RETRY_AFTER_S);
                state.next_attempt = (now + backoff - ROUND_S / 2).max(now + asked);
                state.failing_since.get_or_insert(now);
                state.last_error = Some(error);
            }
        }
        if let Err(e) = store.save_feed(&state) {
            eprintln!("[news-record] cannot save {}'s state: {e}", feed.name);
        }
    }
    let now = clock();
    if let Err(e) = store.forget_items(cutoff(now, days)) {
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
            carriers: Vec::new(),
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
    // A source's state counts for the URL it is configured with now: one given a new URL
    // is not read until that one is.
    let state_of = |f: &Source| states.iter().find(|s| s.id == f.id && s.url == f.url);
    if !feeds.iter().any(|f| {
        state_of(f)
            .and_then(|s| s.last_success)
            .is_some_and(|t| now - t <= CURRENT_S)
    }) {
        return None;
    }
    // Only the asked-for sources' headlines are read, so a call for one source does not
    // hold the store for all of them.
    let ids: Vec<String> = feeds.iter().map(|f| f.id.clone()).collect();
    let rows = store
        .items(now - hours as i64 * 3600, now + super::SKEW_S, Some(&ids))
        .ok()?;
    // A headline once for each asked-for source that carried it, with that source's own
    // categories, in the order they first did; `compose` matches each and keeps a link
    // once, so a source that files it under the coin is not hidden by one that does not.
    let items: Vec<(&Source, Item)> = rows
        .iter()
        .flat_map(|r| {
            r.carriers.iter().filter_map(move |c| {
                let source = feeds
                    .iter()
                    .find(|s| s.id == c.source && s.language == c.language)?;
                let item = Item {
                    title: r.title.clone(),
                    url: r.url.clone(),
                    published: r.published,
                    from_updated: r.from_updated,
                    categories: c.categories.clone(),
                    author: r.author.clone().unwrap_or_default(),
                };
                Some((*source, item))
            })
        })
        .collect();
    let found = items.iter().map(|(o, i)| (*o, i)).collect();
    let (mut read_ok, mut failed) = (Vec::new(), Vec::new());
    for feed in feeds {
        match state_of(feed) {
            Some(s) if s.failures == 0 && s.last_success.is_some() => {
                read_ok.push(feed.name.as_str())
            }
            Some(s) if s.failures > 0 => {
                let mut f = json!({
                    "source": feed.name,
                    "error": s.last_error.as_deref().unwrap_or("failed"),
                });
                if let Some(t) = s.failing_since {
                    f["since"] = json!(super::iso(t));
                }
                failed.push(f);
            }
            _ => failed.push(json!({ "source": feed.name, "error": "not read yet" })),
        }
    }
    read_ok.dedup();
    Some(compose(symbol, hours, limit, now, found, read_ok, failed))
}

#[cfg(test)]
mod tests;
