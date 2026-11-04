---
title: "0011 — Watchlist quotes: batched get_quotes, rolling 24 h change for Binance"
description: "A get_quotes MCP tool serves the web watchlist; Binance change is the rolling 24 h ticker, Yahoo change is against the previous session close."
sidebar:
  order: 11
---

**Status:** Accepted
**Date:** 2026-11-04

## Context and Problem Statement

The web frontend gets a TradingView-style watchlist: up to 50 symbols,
each with last price, change and change %, refreshed every few seconds.
Fetching a full `generate_chart` series per row would be far too heavy,
so the server needs a quote tool. Two questions need answers:

1. What does "change" mean? TradingView measures it against the previous
   daily close (for crypto, the close of the previous UTC daily candle).
   Binance's ticker API reports change over a rolling 24 h window instead.
2. How many upstream requests may one watchlist poll cost? A poll runs
   every 5 s per open tab.

## Decision Drivers

- One MCP call per poll, whatever the list length.
- Few upstream requests per poll, to stay clear of Binance request
  weights and Yahoo throttling.
- Match TradingView's numbers where that is cheap.
- One bad symbol must not blank the whole watchlist.
- Deterministic offline data for E2E (`CHARTGEN_DATA_SOURCE=sample`,
  [ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/)).

## Considered Options

### Binance change reference

**Rolling 24 h ticker, batched.** One `GET /api/v3/ticker/24hr?symbols=[…]`
returns last price, the price 24 h ago (`openPrice`), the high, the low
and the volume for every Binance symbol in the list. One request per
poll. The change matches the Binance UI, but not TradingView's
daily-candle change.

**Daily kline per symbol** (`/api/v3/klines?interval=1d&limit=2`). The
change is against the current UTC day's open, which is what TradingView
shows for crypto. It costs one request per symbol per poll: 30 crypto
rows at 5 s is 6 requests/s from one tab.

**Daily klines plus a cache.** Fetch the previous daily close once per UTC
day per symbol and combine it with the batched ticker's last price. This
matches TradingView in two requests per poll, but adds server-side cache
state, day-rollover handling and a cold-start burst.

### Yahoo

**v8 chart per symbol** (`range=1d&interval=1d`). The `meta` block has
`regularMarketPrice`, `chartPreviousClose` (the previous session's close,
which is TradingView's reference) and the day's high, low and volume.
Works without authentication. One request per symbol.

**v7 `quote` batch.** Returns everything for many symbols at once, but
now answers `Unauthorized` without a cookie and crumb. Rejected.

**v8 `spark` batch.** Works without authentication and batches, but has
no day high, low or volume. Its `fulldayPrice` mixes in extended-hours
trading. Kept as a fallback if per-symbol requests get throttled.

## Decision Outcome

Chosen: **a `get_quotes(symbols[])` tool. Binance uses the batched
rolling 24 h ticker. Yahoo uses the v8 chart `meta` per symbol, on a work
queue of at most 8 workers, running at the same time as the Binance
request. A 3 s per-symbol cache, shared by all clients, sits in front of
both. The sample source compares the forming daily bar with the previous
one.**

- Each quote carries `source` (`binance`, `yahoo`, `sample`) so clients
  and docs can say which reference `prev_close` is. For Binance it is the
  price at the start of the rolling 24 h window.
- Binance rejects a whole batch if one symbol is unknown or malformed
  (HTTP 400 with `-1121 Invalid symbol` or `-1100 Illegal characters`).
  Only on those errors is the batch retried per symbol, so only the bad
  symbol gets `{symbol, error}`. The bad symbol is remembered for 5
  minutes and kept out of later batches. A rate limit (429/418) or
  transport error is never fanned out into per-symbol requests, because
  that would multiply the load just when Binance asks for less.
- Input is trimmed, uppercased and deduplicated. An array of more than
  50 entries is rejected before any entry is read. Symbols use the web
  store's charset without `/`, and each symbol goes into upstream URLs
  percent-encoded as one path segment, so input cannot change the
  request path.

## Consequences

**Positive.** One MCP call per poll, and at most one Binance request and
one request per Yahoo symbol per 3 s, however many tabs are polling.
Stock, ETF, index and FX numbers match TradingView. A failing symbol
stays isolated. E2E runs offline with values that move over time.

**Negative.**

- Crypto change differs from TradingView's: rolling 24 h, not since
  00:00 UTC. The two agree only right after the daily candle opens.
  Switching to daily klines with a cached previous close (the third
  option above) is the upgrade path if parity matters more than the
  extra state.
- A list of mostly stocks still costs one Yahoo request per symbol per
  poll. The shared cache stops many tabs from multiplying that, but not
  many different lists.
- Quotes can be up to 3 s older than upstream, and a symbol Binance lists
  newly can stay "invalid" for up to 5 minutes.

**Neutral.** The frontend stops polling while the tab is hidden or the
panel is collapsed.

## More Information

- [`reference/mcp.md`](/chartgen/reference/mcp/#get_quotes): `get_quotes` contract
- [Web guide](/chartgen/guides/web/#watchlist): watchlist panel
- [ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/): polling and the sample source
