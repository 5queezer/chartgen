---
title: "0086 — News per symbol from public RSS feeds"
description: "get_news answers one symbol's recent crypto headlines (time, outlet, title, link) from the keyless RSS feeds of five crypto outlets, each read at most once in ten minutes. A headline is matched to the symbol by its coin's ticker and name as whole words, a heuristic. CryptoPanic's API was the better fit but needs a paid plan."
sidebar:
  order: 86
---

**Status:** Accepted; superseded in part by
[ADR-0088](/chartgen/decisions/0088-recording-the-news/) (the feed list, nothing kept, the table
of names)
**Date:** 2026-11-25

## Context and Problem Statement

An agent client that makes paper trade decisions looks up a symbol's news before each one. It
does that with a web search ("BTC news" and the date): about five a day, each a call to a search
service, an answer of whole pages, and the client reading the dates out of them. chartgen has no
news.

What the client needs is small: the last day's headlines about one coin, each with when it was
published, by whom, and a link. Not the articles.

Where do the headlines come from, and how does chartgen tell which are about the symbol?

## Considered Options

- a. **CryptoPanic's API.** A news aggregator with the right shape: a filter per coin, the
  outlet, a vote-based sentiment. Its free developer plan ended in April 2026; the API now needs
  a paid plan ($50 a week or $199 a month) and a key. A consumer subscription to its site does
  not include API access. Its public RSS feed (`/news/rss/`) answers `410 Gone`.
- b. **The public RSS feeds of a few crypto outlets, read by chartgen (chosen).** No key, no
  cost. Each outlet's feed carries its last 10 to 40 headlines, a day or more. chartgen has to
  tell for itself which headlines are about the symbol.
- c. **Nothing in chartgen: the client keeps searching the web.** No new dependency. The client
  keeps paying a search per decision and reading dates out of web pages.

## Decision Outcome

Chosen: **b**, in `src/news.rs`, as the tool `get_news { symbol, hours = 24, limit = 10 }`:

```json
{ "symbol": "BTCUSDT", "terms": ["BTC", "Bitcoin"], "hours": 24,
  "items": [ { "time": "2026-11-25T06:54:59Z", "source": "Cointelegraph",
               "title": "…", "url": "https://cointelegraph.com/news/…" } ],
  "sources": ["CoinDesk", "Cointelegraph", "The Block", "Bitcoin Magazine"],
  "sources_failed": [ { "source": "Decrypt", "error": "timed out" } ] }
```

- **The feeds**, checked on 2026-11-25, all keyless: CoinDesk, Cointelegraph, Decrypt, The Block
  and Bitcoin Magazine. Large outlets, feeds that answered with a day or more of dated headlines,
  and sites whose `robots.txt` does not refuse the feed. Left out: Blockworks' feed (its newest
  entry was ten months old), U.Today (its `robots.txt` content signal says `ai-input=no`), and
  CryptoPanic (above). `CHARTGEN_NEWS_FEEDS` replaces the list: `Name=https://…` entries separated
  by commas, RSS 2.0 or Atom.
- **Politely:** a feed's last read is held in memory and read again only when a call comes more
  than ten minutes after it (refreshed on demand, not evicted); a feed that failed is tried
  again after a minute. One request is 8 s at most and 4 MB at most. All feeds are read at once,
  so a call waits for the slowest. The client says what it is (`chartgen/<version>` and the
  project's URL). Callers at once share one read of a feed.
- **A feed that fails is named** under `sources_failed` with why (`HTTP 503`, `timed out`, `not
  a feed`, `no headlines in it`), and the other feeds answer. The call fails only on bad
  arguments, or when no feed is configured.
- **Matching is a heuristic.** The coin is read off the symbol (BTCUSDT, BTC-USD and
  1000PEPEUSDT are BTC, BTC and PEPE). A headline is about it when its title or one of the
  outlet's categories for it holds the ticker, in capitals, or one of the coin's names in a table
  of about 30 large coins (Bitcoin; Ethereum and Ether; Solana; …), in any case, as a whole word:
  `Etherscan` is not Ether and `link` is not LINK. A name that is also an ordinary word (Ripple,
  Optimism, Cosmos, …) only capitalised. A two-letter ticker that has a name (OP) is found by its
  name only; a ticker of under 3 letters without one (AI, S) only as a cashtag (`$AI`) in the
  title. As a word it matched everyday headlines ("AI agents", "U.S."), and as a category it
  matched outlets' `AI` section for stories about artificial intelligence. The answer's `terms` lists the words used. Descriptions are not
  matched: some feeds put the whole article there, and an article mentions many coins.
- **Small answers:** time (UTC), outlet, title and link, `utm_` parameters dropped. No article
  text. A link that two feeds carry is shown once. A headline dated more than 5 minutes after
  now (a clock running ahead) is left out, and so is an item with an empty link.
- **Links** are read against the feed's URL when relative (not against `xml:base`), and only
  `http` and `https` links are kept, so a relative link cannot be mistaken for another
  outlet's.
- **Times:** an RSS item's `pubDate`; an Atom entry's `published`, or `updated` where it has
  none, so an edited old entry can count as new.
- **Titles are third-party text**, and the tool's description tells the client to treat them as
  data, never as instructions.
- **XML:** `quick-xml` reads the feeds. Nothing in chartgen parsed XML; it is a small crate with
  one dependency (`memchr`) and handles CDATA and entities, which a hand-written scanner would
  get wrong on the first feed that differs. Elements are matched by their written name, so an
  Atom feed that prefixes its elements (`atom:entry`) is not read; none of the default feeds
  does, and resolving namespaces is left until a configured feed needs it.

### Consequences

- An agent client gets a symbol's headlines in one call of about 1 to 3 kB instead of a web
  search.
- **History is what the feeds still carry**: one to three days for the default feeds, less for a
  busy one. `hours` goes to 168, but older news needs recording, which this does not do.
- **Matching misses and over-takes.** A story about a coin that names neither ticker nor name in
  its title is missed; one that names Bitcoin in passing is taken. A coin outside the table is
  found by its ticker only. The client should read titles as leads, not as a filtered feed.
- **Feeds change.** An outlet can move, block or end its feed. That shows as `sources_failed`,
  not as a failed call, and `CHARTGEN_NEWS_FEEDS` replaces a feed without a release.
- **Terms of use.** The feeds are published for readers and aggregators. chartgen shows the
  headline, the outlet and a link to the article, and keeps only each feed's last read in
  memory, as a feed reader does. An operator who republishes the answers is bound by each
  outlet's terms.
- A replay server refuses the tool, as it does `get_market_context`: its fixtures hold no news.
- **After a rollback**, the tool is gone from `tools/list`; nothing is stored, so nothing is left
  behind.
