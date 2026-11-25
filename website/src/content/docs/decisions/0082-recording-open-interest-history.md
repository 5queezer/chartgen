---
title: "0082 — Recording open interest beyond the 30 days Binance keeps"
description: "Proposed. Binance keeps open interest history for 30 days; what is not recorded now cannot be had later. The options for chartgen to record it itself: not at all, a series per symbol and period in the SQLite store, or files beside it, and what each costs. get_indicators' change_bars answers the change within the 30 days without any of them."
sidebar:
  order: 82
---

**Status:** Proposed
**Date:** 2026-11-24

## Context and Problem Statement

The `oi` indicator reads Binance's `openInterestHist` (`src/indicators/external.rs`). Binance
keeps that history for 30 days and refuses a request reaching further (code -1130); the
long/short ratios and the taker buy/sell ratio (`/futures/data/*`) are kept as long. chartgen
holds what it reads only in memory (`SeriesCache`), per symbol and period, and loses it on a
restart.

An agent client deciding on a short wants to know whether open interest is rising or falling.
Within the 30 days that is answerable now: `get_indicators` with `change_bars` answers each
line's change over N bars, and `generate_chart` with `format=series` answers each bar's value.
Beyond 30 days nothing is answerable, and it never will be for a time nobody recorded: a
question about open interest over the last quarter, or a backtest of a rule that reads open
interest (a strategy tester over the replay fixtures), needs a history that only exists if
chartgen writes it down while Binance still has it.

Should chartgen record open interest itself, and if so, where and how much?

## Decision Drivers

- What is not recorded within 30 days is lost for good; a later decision cannot recover it.
- The server already has one store, SQLite at `CHARTGEN_DB`, migrated on open; a second
  store is a second thing to back up and migrate.
- Binance's budget for `/futures/data/*` is 1000 requests per 5 minutes per IP, which chartgen
  shares with every chart that reads `oi` (`futures_data_budget`, refusing at 800).
- The store must stay small on a small host, and recording must not slow a tool call.
- A recorded series must answer the same values the live read does for the overlap, or the
  same question gets two answers.

## Considered Options

1. **Do not record.** Answer within the 30 days only (as now, with `change_bars`).
2. **Record a series per symbol and period in the SQLite store.** A table
   `futures_series(kind, symbol, period, ts, value, PRIMARY KEY (kind, symbol, period, ts))`
   filled by a background task for a configured list of symbols, and read by `oi` for bars
   older than Binance keeps.
3. **Record to files beside the store** (one append-only file per symbol and period, CSV or
   Parquet), read the same way.
4. **Record what chartgen happens to read** (write `SeriesCache` through to the store) instead
   of polling a list.

## Cost of recording (options 2 to 4)

One row is a time and a value; in SQLite with its key about 40 to 60 bytes.

| Period recorded | Rows per symbol and year | Size per symbol and year | Requests to keep it current |
|---|---|---|---|
| 5m (the finest Binance has) | 105,120 | about 5 MB | 1 per symbol every 41 h (500 rows a request), plus a catch-up after a pause |
| 1h | 8,760 | about 0.5 MB | 1 per symbol every 20 days |
| 4h | 2,190 | about 0.1 MB | 1 per symbol well within 30 days: a page holds 83 days, but rows older than the retention are already gone |

Whatever the page size, a series must be read again before its oldest unread row is 30 days
old, with a margin; a pause longer than that leaves a gap that cannot be filled.

Open interest and the three long/short ratios are snapshots: a coarser period's row is the 5m
row with the same timestamp (checked against the API for BTCUSDT on 1h: 29 of 29 hours equal,
for `sumOpenInterest`, `sumOpenInterestValue` and each `longShortRatio`). For them one 5m
series answers every interval from 5m up. The taker buy/sell ratio is not: a period's row
aggregates the volume within it (its 1h `buyVol` and `sellVol` differ from the sum of the
twelve 5m rows by up to 1.7 %, and its ratio from any single 5m row), so it would have to be
recorded at each period it is read at. Twenty symbols of open interest at 5m are about 100 MB a
year and about a dozen requests a day, far inside Binance's budget; each long/short ratio costs
the same.

## Pros and Cons of the Options

### 1. Do not record

- Good: nothing to build, store, back up or keep in budget.
- Bad: no answer beyond 30 days, ever, for the time already passed; a backtest of an open
  interest rule is limited to the last 30 days.

### 2. A series in the SQLite store

- Good: one store, migrated as everything else is; a query by symbol and time
  range is the table's key.
- Good: the overlap with the live read can be checked row by row.
- Bad: a background task per server to run, with its own failure modes (a gap while the
  server was down longer than 30 days cannot be filled).
- Bad: the store grows without bound unless a retention is set.

### 3. Files beside the store

- Good: cheap to append and to copy into replay fixtures.
- Bad: a second thing to back up and migrate, and its own reader.

### 4. Record what chartgen reads

- Good: no list of symbols to configure; no requests beyond today's.
- Bad: the record has holes wherever nobody looked at a symbol for 30 days, so it cannot be
  relied on for exactly the questions it is for.

## Decision Outcome

None yet. If recording is wanted, option 2 at 5m for a configured list of symbols, with a
retention in days, is the smallest that answers open interest (and the long/short ratios) at
every interval and keeps one store; the taker ratio would need a row per period. The choice
of symbols and retention is the owner's. Until then, `change_bars` answers the change within
the 30 days.

## More Information

- `get_indicators` `change_bars`: [MCP reference](/chartgen/reference/mcp/#change-over-n-bars).
- Binance retention and budget: `RETENTION_MS` in `src/indicators/external.rs`,
  `futures_data_budget` in `src/binance_gate.rs`.
