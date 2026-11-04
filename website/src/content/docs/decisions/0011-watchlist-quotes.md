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
rolling 24 h ticker. Yahoo uses the v8 chart `meta` per symbol, at most 8
requests at a time. The sample source compares the forming daily bar with
the previous one.**

- Each quote carries `source` (`binance`, `yahoo`, `sample`) so clients
  and docs can say which reference `prev_close` is. For Binance it is the
  price at the start of the rolling 24 h window.
- Binance rejects a whole batch if one symbol is unknown (`-1121 Invalid
  symbol`). On a 4xx the batch is retried per symbol, so only the bad
  symbol gets `{symbol, error}`. A transport error is not retried.
- Input is trimmed, uppercased, deduplicated and capped at 50 distinct
  symbols, the same rules as the web store.

## Consequences

**Positive.** One MCP call and one Binance request per poll, plus one
request per Yahoo symbol. Stock, ETF, index and FX numbers match
TradingView. A failing symbol stays isolated. E2E runs offline with
values that move over time.

**Negative.**

- Crypto change differs from TradingView's: rolling 24 h, not since
  00:00 UTC. The two agree only right after the daily candle opens.
  Switching to daily klines with a cached previous close (the third
  option above) is the upgrade path if parity matters more than the
  extra state.
- A list of mostly stocks costs one Yahoo request per symbol every 5 s
  per open tab. There is no server-side cache yet, so many tabs multiply
  the load.

**Neutral.** The frontend stops polling while the tab is hidden or the
panel is collapsed.

## More Information

- [`reference/mcp.md`](/chartgen/reference/mcp/#get_quotes): `get_quotes` contract
- [Web guide](/chartgen/guides/web/#watchlist): watchlist panel
- [ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/): polling and the sample source
