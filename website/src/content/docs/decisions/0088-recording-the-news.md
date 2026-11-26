---
title: "0088 — Recording the news"
description: "A server with a store reads 34 public crypto and finance feeds every 5 minutes, conditionally, and keeps every headline once, by its link, with its source, kind and language, when it was first read, its categories, author and the coins matched then, for a year by default. The sources are a JSON file read again when it changes. get_news answers from that record while it is current, from four of ADR-0086's five outlets by default, and names about 130 coins. The news lives in a crate of its own behind a Cargo feature."
sidebar:
  order: 88
---

**Status:** Accepted
**Date:** 2026-11-25
**Amends:** [ADR-0086](/chartgen/decisions/0086-news-per-symbol-from-public-feeds/) (five feeds,
read on demand and held in memory only)

## Context and Problem Statement

`get_news` ([ADR-0086](/chartgen/decisions/0086-news-per-symbol-from-public-feeds/)) reads five
outlets' feeds when asked and keeps only each feed's last read in memory. That answers "what are
the last headlines about this coin". It cannot answer what comes next: which stories about a
coin are getting attention, and how unusual that is. That needs

- **many outlets**, since "how many outlets carry this story" has no resolution with five;
- **when each outlet picked a story up**, which only a reader that polls on a schedule sees, as a
  feed nobody asked about for six hours has dropped headlines by then;
- **what a normal day looks like** for a coin, which needs weeks of history.

None of it exists after a restart, and none of it can be had later for a time nobody recorded.
Grouping headlines into stories and scoring them come later (ADR-0089, ADR-0090); they need the
record to start now.

Should chartgen record the news, where, and from which feeds?

## Decision Drivers

- What is not recorded now is lost for good.
- The server already has one store, SQLite at `CHARTGEN_DB`, migrated on open, and a pattern for
  recording into it ([ADR-0082](/chartgen/decisions/0082-recording-open-interest-history/)).
- Outlets publish their feeds for readers: polling has to be polite (conditional requests, a
  pause between requests, a failing feed left alone longer each time).
- The store must stay small on a small host, and recording must not slow a tool call.
- `get_news` reaches an agent client the moment it deploys: its answer keeps its shape and its
  defaults.
- The news may later want to run apart from chartgen (recording through chartgen's redeploys,
  or a store that outgrows the host).
- Sources change (an outlet moves its feed, a subreddit is worth watching for one coin): that
  must not need a release.

## Considered Options

### What is kept

1. **On demand only** (ADR-0086 as it is): nothing kept.
2. **Record in the SQLite store** (chosen): a table of headlines and one of the feeds' state,
   filled by a background thread.
3. **Record to files** beside the store (one append-only file a day).

### Where the sources are configured

- i. **In code**, as ADR-0086's list: a change is a release.
- ii. **An environment variable**, as ADR-0086's `CHARTGEN_NEWS_FEEDS`: a change is a restart,
  and a list of 34 sources with their attributes does not fit one line.
- iii. **A JSON file the server reads, again when it changes** (chosen).
- iv. **A table in the store with an admin page or tool** to change it: no file to deploy, but
  a page or tool to build and secure, for a list that changes a few times a year.

### Where the code lives

- a. **A module of chartgen**, as ADR-0086's `src/news.rs`.
- b. **A crate of its own in this repository, behind a Cargo feature, in the chartgen process**
  (chosen).
- c. **A service of its own**, with its own MCP endpoint.

## Pros and Cons of the Options

### 1. On demand only

- Good: nothing to build, store or back up.
- Bad: no history beyond what the feeds still carry (hours to three days), no first-seen times,
  nothing to group or score against.

### 2. Record in the SQLite store

- Good: one store, migrated and backed up as everything else is; the record is queried by time,
  which is what grouping and scoring need.
- Good: each feed's state (its `ETag`, its failures) survives a restart, so a restart neither
  asks every feed for its whole body again nor forgets a feed it should leave alone.
- Bad: a background thread to run, and a store that grows unless a retention is set.

### 3. Files beside the store

- Good: cheap to append, easy to copy elsewhere.
- Bad: a second thing to back up and migrate, its own reader, and "each headline once" needs an
  index of its own anyway.

### a. A module of chartgen

- Good: nothing new to set up.
- Bad: nothing stops the rest of chartgen from reaching into the news, or the news into chartgen,
  so moving it out later means untangling it first.

### b. A crate in this repository, behind a feature

- Good: a hard boundary with a narrow interface: `record::start(store)`, `read(store, symbol,
  hours, limit)`, and the feeds' state through the `Store` trait. Nothing else in chartgen uses
  the news but `get_news` (and later an alert condition).
- Good: it runs in the chartgen process and uses its store, so no second endpoint, sign-in,
  deployment or watchdog.
- Good: it can become a service in a day, by giving it its own store behind the same trait.
- Bad: the repository becomes a Cargo workspace, and the build copies one more directory.

### c. A service of its own

- Good: records through chartgen's redeploys; can scale apart.
- Bad: today another MCP endpoint, its own auth, deployment and watchdog, for a load that is a
  few requests a minute.

## Decision Outcome

Chosen: **2, iii and b**. The news is the crate `chartgen-news` in `crates/news`, which chartgen uses
behind the feature `news` (on by default). Recording needs the store as well, so the feature
`scripting` too, like every default build. The repository is a Cargo workspace whose default
members are chartgen and the crate, so `cargo test` and `cargo clippy` at its root cover both, and
the chartgen binary is built as before.

**The store stays chartgen's.** The crate does not open a database. It declares a `Store` trait
(save headlines, read them by time, drop old ones, read and save a feed's state), and chartgen
implements it on its SQLite store, where the tables are one more migration in the one list of
migrations. Sharing that list from the crate would tie both to one `PRAGMA user_version`; a
service of its own would implement the same trait on a store of its own.

### The sources are configuration

`CHARTGEN_NEWS_SOURCES` names a JSON file, a list of sources:

```json
[
  { "id": "coindesk", "name": "CoinDesk", "url": "https://www.coindesk.com/arc/outboundfeeds/rss/",
    "kind": "rss", "language": "en", "enabled": true, "syndicates": false, "default": true },
  { "id": "r-solana", "name": "r/solana", "url": "https://…", "kind": "reddit", "coins": ["SOL"] }
]
```

- `id`: stable, recorded with every headline; lower case letters, digits and `-`, `all`
  reserved. `name`: what an answer calls it. `url`: the feed.
- `kind`: `rss` (default), `atom`, `reddit` (a subreddit's feed) or `youtube` (a channel's
  feed). Each is read as RSS or Atom; the kind is recorded so a later score can weigh a post or
  a video apart from an article (ADR-0090 decides whether they count).
- `language`: default `en`. `enabled`: default true; a source switched off is neither read
  nor answered from, and its rows stay. `syndicates`: it mostly republishes others.
- `coins`: the only coins it is about (a coin's subreddit). Its headlines are about those
  coins whatever their titles say, and about no other.
- `default`: in `get_news`'s answer when the call names no sources. When no enabled source is
  marked, all are.

The file is read when the news is first asked for or recorded, and again whenever its
modification time changes: the recorder looks each round, so a change takes effect within 5
minutes, without a restart. A file that does not parse (bad JSON, an unknown field, an id twice,
a URL that is not `http(s)`) keeps the sources read before, and the log says why once; a file
that never parsed makes `get_news` an error naming it. Without the variable the server uses
`CHARTGEN_NEWS_FEEDS` (ADR-0086's form, each entry an English RSS source, all in the default
set), and without that the list built in (`crates/news/sources.json`), the 34 below (29
enabled, five opt-in) with
CoinDesk, Cointelegraph, Decrypt and The Block as the default set.

### What is recorded

Three tables in the store:

```sql
CREATE TABLE news_items (
    url          TEXT PRIMARY KEY,  -- the link, its utm_ parameters dropped
    source       TEXT NOT NULL,     -- the source's id
    outlet       TEXT NOT NULL,     -- the source's name
    kind         TEXT NOT NULL,     -- rss, atom, reddit, youtube
    language     TEXT NOT NULL,     -- the source's language
    title        TEXT NOT NULL,     -- as the feed wrote it, whitespace collapsed
    published    INTEGER NOT NULL,  -- unix seconds
    from_updated INTEGER NOT NULL,  -- 1: published is an Atom entry's `updated`
    first_seen   INTEGER NOT NULL,  -- when the recorder first read it
    categories   TEXT NOT NULL,     -- the outlet's categories for it, JSON
    coins        TEXT NOT NULL,     -- the coins matched when it was read, JSON
    author       TEXT               -- dc:creator, author, an Atom author's name
) WITHOUT ROWID;
CREATE INDEX news_items_published ON news_items (published);
-- Every source that carried a link, and when it first did.
CREATE TABLE news_item_sources (url TEXT NOT NULL, source TEXT NOT NULL,
    first_seen INTEGER NOT NULL, PRIMARY KEY (url, source)) WITHOUT ROWID;
CREATE TABLE news_feeds (id TEXT PRIMARY KEY, url TEXT NOT NULL, name TEXT NOT NULL, etag TEXT,
    last_modified TEXT, last_success INTEGER, last_error TEXT, failing_since INTEGER,
    failures INTEGER NOT NULL DEFAULT 0, next_attempt INTEGER NOT NULL DEFAULT 0);
```

- **Each headline once**, by its link: a link read again (a later round, an edited title)
  leaves the row as it was first read. Every recorded headline has a link, since an item without
  one is not read (ADR-0086), so the outlet and title are never needed as the key. A link that a
  second source carries too (two feeds of one outlet, an aggregator) keeps its one row and adds
  that source to `news_item_sources`, with when it first carried it, so a call that names only
  the second source still finds it, and grouping (ADR-0089) can count who carried a story and
  when.
- **Recorded before it counts as read:** a source's `ETag`, `Last-Modified` and last success
  move on only once its new headlines are in the store. A store that fails is a failure of that
  source, tried again with the backoff, and the next try reads the feed whole, rather than being
  told "not modified" about headlines that were never kept.
- **The coins** are ADR-0086's heuristic applied when the headline is read: each coin of its
  table of about 30 that the title or a category names, then each cashtag in the title
  (`$PEPE`); for a source about certain coins, those. They are kept so a later change to the table does not rewrite what was matched
  then; the raw title and categories are kept too, so a reader can match again.
- **The author** is kept because press-release copies are what the record most needs to tell
  apart, and they show there (`Chainwire`, `Advertorial`) more often than in a category; see the
  feed list below.
- **Not kept:** descriptions and article text, as in ADR-0086.

### What is kept, for whom, and the publishers' terms

ADR-0086 argued that chartgen used the feeds as a feed reader does: it showed the headline, the
outlet and a link, and kept each feed's last read in memory for ten minutes. Recording keeps a
year, so that argument no longer covers it, and it is restated here:

- **What is kept** is what a feed publishes for readers to see in a list: the headline, its
  link, its time, the outlet, the outlet's categories and author line. No description, no
  article text, no images.
- **For whom:** the server's own operator and the operator's own agent client, to decide what
  to read and whether a coin is in the news. `get_news` answers with headlines and links, as
  before; chartgen neither publishes the record nor serves it to others, and an operator who
  republishes it is bound by each outlet's terms, as ADR-0086 said.
- **The publishers whose feed terms allow personal, non-commercial use only**, and are big
  enough to enforce it, are **opt-in**: Dow Jones (The Wall Street Journal, MarketWatch),
  Bloomberg, the Financial Times and CNBC ship with `"enabled": false` in the built-in list, so
  nothing is read or kept from them until an operator who has checked their terms for their own
  use switches them on in the sources file. The crypto outlets' feeds carry no such terms that
  this vetting found; it did not read every outlet's terms of use, and an operator who adds or
  enables a source owns that check.

### How

A thread, started with the server when it has a store (not on a replay server, as with ADR-0082),
reads every enabled source once a round, a round every 5 minutes, one feed after another with 2 s between
requests, so a round takes about a minute and a half and no host is asked twice within 2 s (two
feeds share a host). Each request:

- is **conditional**: it sends the feed's last `ETag` (`If-None-Match`) and `Last-Modified`
  (`If-Modified-Since`), and a `304 Not Modified` is a read with nothing new. They are kept only
  from a response that was a feed, so a challenge page cannot make the next request answer "not
  modified".
- has the limits of ADR-0086: 8 s, 4 MB, and the user agent `chartgen/<version>` with the
  project's URL.
- records the items published within the retention, each new one with the time of the read as its
  first seen.

**A feed that fails** (an HTTP error, a time-out, a page that is not a feed, a store that cannot
record) is tried again after 5 minutes, then 10, 20, 40, and so on up to 6 hours between tries,
or after its `Retry-After` if that is longer (up to a day), never before it. Its state says why it last failed and since when; the log says when
a feed starts failing and when it is read again, and once an hour how many headlines came in.

**Retention:** `CHARTGEN_NEWS_RECORD_DAYS`, default 365; a headline published longer ago is
dropped, and one that old is not recorded. `0` records nothing. A year lets a coin's normal be
taken over many weeks and leaves the record small (below).

**`get_news` reads the record** when the store has a source read within the last 30 minutes,
and the feeds as before otherwise (no store, recording off, or a server stopped for a while).
The answer from the record is what the feeds would give for the same headlines: the same
matching, on the stored title and categories, the same order, the same fields. Its defaults do
not change: **without `sources` it answers from the default set, ADR-0086's outlets less
Bitcoin Magazine** (CoinDesk, Cointelegraph, Decrypt, The Block), though all enabled sources
are recorded.
Bitcoin Magazine's site refuses some servers: it answered a server running chartgen `403` at
every call while answering this vetting's client, so it is recorded where it answers and left out
of the default set, where it would be a failure in every answer. The wide list is for grouping and scoring (ADR-0089, ADR-0090); as a list of
headlines it is noisier, and at `limit` 10 it covered about the last hour where ADR-0086's five
covered about four (measured below). Added, all optional:

- `sources`: source ids to answer from, or `["all"]` for every enabled source. An id that is
  not a source, or is switched off, is an error that lists the ids.
- `language`: only sources of this language, default `en`.
- `hours` up to 168 now reaches as far back as the record does, not only as far as the feeds
  still carry.
- An entry of `sources_failed` from the record has `since`, when its failures began, so a
  source failing for days is told from one that failed a minute ago. Its headlines recorded
  before still count.

### The feeds

Each candidate was read once on 2026-11-25, with one request for the feed and one for its site's
`robots.txt`, with chartgen's user agent. Kept: it answered without a key, its items are dated in
a form RSS or Atom allows, its newest item is recent, and `robots.txt` neither disallows the feed
for every agent nor carries `ai-input=no`. A `robots.txt` that answered 403 is no refusal (RFC
9309 reads an unavailable `robots.txt` as no rule). "Items a day" is from that one read: the items
of the last 24 hours, or for a feed that holds less than a day, its items spread over a day.
"Press-release copies" counts the items of that read whose author is a press-release wire or
"Advertorial", or whose category is "Press Release" or "Sponsored".

| Outlet | Feed | Items a day | Newest item | `robots.txt` | Press-release copies |
|---|---|---|---|---|---|
| CoinDesk | `https://www.coindesk.com/arc/outboundfeeds/rss/` | 17 | 2 h | allows | 0 of 25 |
| Cointelegraph | `https://cointelegraph.com/rss` | 23 | 1 h | allows | 0 of 30 |
| Decrypt | `https://decrypt.co/feed` | 15 | 30 min | allows | 0 of 38 |
| The Block | `https://www.theblock.co/rss.xml` | 22 | 18 min | allows | 0 of 20 |
| Bitcoin Magazine | `https://bitcoinmagazine.com/feed` | 48 | 6 min | allows | 0 of 10 |
| CryptoSlate | `https://cryptoslate.com/feed/` | 25 | 1 h | allows | 0 of 10 |
| CryptoPotato | `https://cryptopotato.com/feed/` | 15 | 1 h | allows | 1 of 36 |
| NewsBTC | `https://www.newsbtc.com/feed/` | 3 | 7 h | allows | 3 of 10 |
| Cryptonews | `https://cryptonews.com/news/feed/` | 6 | 12 min | allows | 0 of 20 |
| Crypto Briefing | `https://cryptobriefing.com/feed/` | 259 | 12 min | allows | 0 of 30 |
| BeInCrypto | `https://beincrypto.com/feed/` | 38 | 2 h | allows (403) | 4 of 12 |
| The Daily Hodl | `https://dailyhodl.com/feed/` | 3 | 6 h | allows | 2 of 10 |
| CoinGape | `https://coingape.com/feed/` | 53 | 2 h | allows | 0 of 20 |
| Bitcoin.com News | `https://news.bitcoin.com/feed/` | 32 | 24 min | allows | 2 of 10 |
| Coinpedia | `https://coinpedia.org/feed/` | 71 | 2 h | allows | 0 of 10 |
| CoinJournal | `https://coinjournal.net/feed/` | 1 | 12 h | allows | 0 of 9 |
| Unchained | `https://unchainedcrypto.com/feed/` | 42 | 4 h | allows | 0 of 10 |
| Protos | `https://protos.com/feed/` | 5 | 1 h | allows | 0 of 10 |
| crypto.news | `https://crypto.news/feed/` | 47 | 24 min | allows | 2 of 50 |
| Blockonomi | `https://blockonomi.com/feed/` | 136 | 2 h | allows | 0 of 10 |
| ZyCrypto | `https://zycrypto.com/feed/` | 2 | 12 min | allows | 0 of 14 |
| Coincu | `https://coincu.com/feed/` | 8 | 5 h | allows | 2 of 10 |
| Crypto Daily | `https://cryptodaily.co.uk/feed` | 23 | 1 h | allows | 22 of 200 |
| Finbold | `https://finbold.com/feed/` | 55 | 4 h | allows | 1 of 11 |
| CoinCentral | `https://coincentral.com/feed/` | 76 | 6 h | allows (403) | 0 of 10 |
| Cryptopolitan | `https://www.cryptopolitan.com/feed/` | 32 | 24 min | allows | 0 of 22 |
| Coinspeaker | `https://www.coinspeaker.com/feed/` | 3 | 4 h | allows | 0 of 10 |
| FXStreet | `https://www.fxstreet.com/rss/crypto` | 11 | 2 h | allows | 0 of 30 |
| TheStreet | `https://www.thestreet.com/crypto/.rss/full/` | 13 | 30 min | allows | 0 of 50 |
| CNBC (opt-in) | `https://www.cnbc.com/id/10000664/device/rss/rss.html` | 5 | 3 h | allows | 0 of 30 |
| MarketWatch (opt-in) | `https://feeds.content.dowjones.io/public/rss/mw_topstories` | 97 | 12 min | allows (403) | 0 of 10 |
| The Wall Street Journal (opt-in) | `https://feeds.content.dowjones.io/public/rss/RSSMarketsMain` | 28 | 24 min | allows (403) | 0 of 61 |
| Bloomberg (opt-in) | `https://feeds.bloomberg.com/crypto/news.rss` | 2 | 15 h | allows (none) | 0 of 3 |
| Financial Times (opt-in) | `https://www.ft.com/cryptocurrencies?format=rss` | under 1 | 5 days | allows | 0 of 25 |

Reddit and YouTube were vetted the same way, with two or three feeds each, and **neither is in
the list**: both sites' `robots.txt` refuse them.

| Kind | Feeds read | Answer | `robots.txt` |
|---|---|---|---|
| reddit | `/r/CryptoCurrency/.rss`, `/r/Bitcoin/.rss`, `/r/solana/.rss` | 404, 429, 429 | `User-agent: *` `Disallow: /`, with a pointer to Reddit's Public Content Policy |
| youtube | three channels' `/feeds/videos.xml?channel_id=…` | 200, 15 dated entries each (one to two videos a day) | `User-agent: *` `Disallow: /feeds/videos.xml` |

The kinds stay in the source model, so an operator with Reddit's permission (its API terms) or
a source that serves such feeds elsewhere can add one in the file.

The five marked opt-in ship switched off (above): their terms are for personal,
non-commercial use.

Left out:

| Outlet | Feed | Because |
|---|---|---|
| Investing.com | `https://www.investing.com/rss/news_301.rss` | its dates have no time zone (`2026-11-25 16:34:12`), so its times are guesses; it was also the one outlet mostly republishing (6 of 10 items from a press-release wire) |
| Blockworks | `https://blockworks.co/feed` | newest item 322 days old |
| Bitcoinist | `https://bitcoinist.com/feed/` | newest item 28 hours old: the feed lags the site |
| DL News | `https://www.dlnews.com/arc/outboundfeeds/rss/` | newest item 27 hours old; the feed is a selection reaching back months |
| The Defiant | `https://thedefiant.io/api/feed` | `robots.txt` disallows `/api/*`, where the feed is |
| AMBCrypto | `https://ambcrypto.com/feed/` | `robots.txt` disallows `/feed/` |
| U.Today | `https://u.today/rss` | `robots.txt` content signal `ai-input=no` |
| CCN | `https://www.ccn.com/feed/` | HTTP 403 |
| Glassnode Insights | `https://insights.glassnode.com/rss/` | HTTP 403 |
| GlobeNewswire (blockchain) | `https://www.globenewswire.com/RssFeed/subjectcode/37-Blockchain/…` | HTTP 403 |
| Yahoo Finance | `https://finance.yahoo.com/news/rssindex` | HTTP 404 |
| The Crypto Basic | `https://thecryptobasic.com/feed/` | answers a web page, not a feed |
| Watcher.Guru | `https://watcher.guru/news/feed` | answers a web page, not a feed |
| Benzinga | `https://www.benzinga.com/feed` | mostly single stocks, about 120 items a day, 238 kB a read without an `ETag` |
| Bloomberg (markets) | `https://feeds.bloomberg.com/markets/news.rss` | one feed per outlet: its crypto feed is kept |

**No outlet kept mostly republishes others**, so every `syndicates` of the list is false.
Press-release copies are spread instead: the same release appears under several outlets with the same title
(in that read, two releases each under two outlets), and they are up to a third of some outlets' items.
They are one voice, not several, and the record keeps what tells them apart (the author, the
categories, the identical title), so grouping (ADR-0089) can count them once, the flag or not. The default set is ADR-0086's outlets less Bitcoin Magazine (above). Bitcoin Magazine's old feed
URL (`/.rss/full/`) now redirects to `http://bitcoinmagazine.com/feed`; the list has the `https`
one.

### The coins' names

`get_news TIAUSDT` answered `terms: ["TIA"]` and no headlines: ADR-0086's table named about 30
large coins, so a headline that calls the coin Celestia was not found, and the coins an agent client is woken for
are often others (TIA, ONE, SAGA, FET). The table now names about 130: the large coins and the
USDT perpetuals most traded on Binance, each with the names outlets use (Celestia; Harmony;
Saga; Fetch.ai and Artificial Superintelligence Alliance). It is kept by hand, not derived: a
coin registry such as CoinGecko's list maps one ticker to many coins (dozens share `ONE`), so a
name taken from it would match the wrong coin's news, and its names are often not what headlines
say. A name that is also a word (Harmony, Saga, Render, Jupiter, Maker, Compound, Flow) is
matched only as written, as ADR-0086 did for Optimism. That was not enough: many outlets write
titles in Title Case, and over 168 hours of the record every title that matched MKR or FLOW by
name alone was about something else ("Market Maker Rules", "Yacht Maker", "Cash Flow", "SHIB Flow
Onto Exchanges"). So in a Title Case title (at least half the words after the first, short ones
and the name aside, capitalised; or none left to tell by) such a name counts only together with
the ticker or its cashtag; alone it counts in a sentence-case title ("Maker votes to raise the
stability fee") or as a category.

For a coin the table does not name, the answer has `note`: "No name is known for … headlines
are found by the ticker alone", so an empty list is not read as "no news". A ticker of under 3
letters with a name (OP, ZK, S) is now also found as a cashtag (`$OP`), as one without a name
already was; `terms` then lists the ticker too. The coins recorded with a headline are matched
with this table.

### Cost

Measured with SQLite 3.50 on a branch build reading the 34 sources from a desktop, on
2026-11-25 and 26:

- **Headlines a day: about 870 from the 34, about 820 from the 29 enabled by default.** The
  record was stopped for 24 hours (the machine slept) and the first round after it recorded 406
  new headlines, 403 of them published in those 24 hours. That is a lower bound: 24 of the 34
  feeds hold less than a day, so the headlines they dropped are missing; spread over the hours
  each of those feeds still held, the 34 come to about 870 a day, of which the five opt-in
  publishers about 56.
  The busiest: Crypto Briefing (about 185), crypto.news (65), Coinpedia (63), Blockonomi (56),
  Bitcoin.com News (43). Over 31 minutes of an evening (20:38 to 21:09 UTC) 9 came in; the rate
  follows the American working day.
- **Bytes:** a row is about 250 bytes of data; with its key, the index on `published` and the
  free space of SQLite's pages, 1282 rows took 618 kB, about 480 bytes a row (a source that
  carries a link another already did adds a row of about 60 bytes to `news_item_sources`). With
  the 29 enabled sources **about 390 kB a day, 140 MB a year** at the default retention of 365
  days; 90 days would be about 35 MB.
- **Requests:** one per enabled source a round, 29 every 5 minutes, about 8,400 a day, 288 to
  each host a day. All but CoinDesk send an `ETag` or `Last-Modified`, so an unchanged feed
  answers `304` without a body; CoinDesk is read whole each round (30 kB). The largest feed
  (Crypto Daily, 1.4 MB, 200 items) is read whole only when it changed. The bandwidth was not
  measured. A feed's `Cache-Control: max-age` or RSS `<ttl>` is not read: the conditional
  request already costs an unchanged feed no body, and reading them is left until a feed asks
  for more than 5 minutes.
- **A restart** lost nothing and recorded nothing twice: 872 rows before it, 872 after the first
  round after it, which asked each source with what it had sent before.
- **Failures seen:** CryptoSlate and CryptoPotato answered `429` once each in the first 15
  minutes and were read again 5 minutes later; none since. Investing.com could not be read (its
  dates) and is left out.
- **`get_news BTCUSDT`** on 2026-11-25 at 20:40 UTC, on the record and on the feeds read now:
  the same answer, 2,707 bytes (10 headlines of the last 3.7 hours, `more` 21) from ADR-0086's
  five; from all 34, 2,982 bytes (10 headlines of the last hour, `more` 93). On 2026-11-26 at
  21:10 UTC from the default set of four, 2,649 bytes (10 headlines of the last 9 hours, `more`
  9); from all 34, 2,815 bytes (the last 1.8 hours, `more` 99).

### Consequences

- From the first start of this build the store holds every headline the feeds carry: on its
  first round what they still hold (hours to days back), from then on each new one within about 5
  minutes of its publication, with when it was first read.
- `get_news` without `sources` answers from ADR-0086's outlets less Bitcoin Magazine, which
  failed there on the server anyway, now from the record; `sources: ["all"]` answers from all
  enabled ones (29 built in).
- A server stopped for a while records nothing meanwhile; what the feeds still carry at the next
  start is recorded then, with that start as its first seen, so a stop of more than a few hours
  loses headlines of the busiest feeds and the first-seen times of the rest.
- A replay server neither records nor answers `get_news`, as before.
- **Off:** `CHARTGEN_NEWS_RECORD_DAYS=0` (the record is then not answered from either; its
  rows stay), a build without the store, or one without the feature `news`, which has no
  `get_news` at all. A source is switched off in the file.
- **After a rollback**, a build from before this ignores the three tables and reads the feeds on
  demand, and nothing is recorded while it runs. Rolling forward again needs no step; the rows
  stay until the retention drops them, and dropping the three tables removes the record.

## More Information

- Running it (settings, cost, the log): [deploy guide](/chartgen/guides/deploy/#the-news-recorded).
- `get_news`: [MCP reference](/chartgen/reference/mcp/#get_news).
