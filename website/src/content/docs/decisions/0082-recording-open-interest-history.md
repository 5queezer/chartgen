---
title: "0082 — Recording open interest beyond the 30 days Binance keeps"
description: "Binance keeps open interest history for 30 days, and what is not recorded then is lost. A server with a store records the 5m open interest of the symbols its setups and server alerts watch, and of a configured list, for a year by default, in a table of the SQLite store, and the oi indicator reads it for bars older than Binance keeps."
sidebar:
  order: 82
---

**Status:** Accepted
**Date:** 2026-11-25 (proposed 2026-11-24)
**Amends:** [ADR-0085](/chartgen/decisions/0085-open-interests-change-on-server-alerts/) (an
open interest alert's `change_bars` reaches past the 30 days where the server recorded them)

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

One row is a time and a value; in SQLite with its key about 40 bytes (measured, see [Cost](#cost)).

| Period recorded | Rows per symbol and year | Size per symbol and year | Requests to keep it current |
|---|---|---|---|
| 5m (the finest Binance has) | 105,120 | about 4.2 MB | 1 per symbol every 41 h (500 rows a request), plus a catch-up after a pause |
| 1h | 8,760 | about 0.35 MB | 1 per symbol every 20 days |
| 4h | 2,190 | about 0.09 MB | 1 per symbol well within 30 days: a page holds 83 days, but rows older than the retention are already gone |

Whatever the page size, a series must be read again before its oldest unread row is 30 days
old, with a margin; a pause longer than that leaves a gap that cannot be filled.

Open interest and the three long/short ratios are snapshots: a coarser period's row is the 5m
row with the same timestamp (checked against the API for BTCUSDT on 1h: 29 of 29 hours equal,
for `sumOpenInterest`, `sumOpenInterestValue` and each `longShortRatio`). For them one 5m
series answers every interval from 5m up. The taker buy/sell ratio is not: a period's row
aggregates the volume within it (its 1h `buyVol` and `sellVol` differ from the sum of the
twelve 5m rows by up to 1.7 %, and its ratio from any single 5m row), so it would have to be
recorded at each period it is read at. Twenty symbols of open interest at 5m are about 84 MB a
year and, read every 6 hours as chosen below, 80 requests a day, far inside Binance's budget;
each long/short ratio costs the same.

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

Chosen: **option 2**, open interest at 5m, in `src/indicators/external/record.rs`, the store
(`src/scripting/persistence.rs`) and `hist_values` in `src/indicators/external.rs`.

**What is recorded.** `futures_series` as above, `WITHOUT ROWID` since the key is the row:
`kind` `oi`, `period` `5m`, `value` the `sumOpenInterestValue` the `oi` panel shows. One 5m series
answers every interval, since a coarser period's row is the 5m row of the same time (checked
again for BTCUSDT: 11 of 11 4h rows and 2 of 2 1d rows equal, on the epoch grid of their
period). The long/short ratios could be recorded the same way under another `kind`, and the
taker ratio per period; neither is asked for yet.

**Which symbols, and how long.** The Binance symbols of every setup and server alert in the
store, whoever owns them, and those of `CHARTGEN_OI_RECORD_SYMBOLS`, for
`CHARTGEN_OI_RECORD_DAYS` (default 365; `0` records nothing). The watched symbols are the ones
an agent asks about later, and need no list kept by hand; the list adds a symbol before anything
watches it, since a history begins only when its recording does. A symbol stays recorded while
a setup or alert names it, and its rows stay for the retention after that. One year is what a
question about a quarter, or a backtest over the replay fixtures, needs, at about 4.2 MB a symbol.

**How.** A thread, started with the server when it has a store (not on a replay server), runs
every 6 hours. For each symbol it reads from after its newest recorded row (the first time, as
far back as Binance keeps) up to 10 minutes ago, 500 rows a request with `startTime` and
`endTime` both set (with `startTime` alone Binance answers the newest 500), and writes each
page as it comes. The last 10 minutes wait for the next round, so a row is never recorded before
it is final. It waits 2 s after each request, through the same gate and budget as the charts.
Then it drops rows older than the retention, for every symbol in the table, watched or not,
a symbol at a time along the key: one delete over
the whole table held the store's lock for about 350 ms on a year of 10 symbols, one per symbol
0.3 ms.

**How it is read.** Within the 30 days the `oi` indicator reads Binance as before, and answers
the same values. A window reaching further back takes the recorded rows before Binance's first
row, on its period's grid, when its read from Binance comes back whole; they are kept with that
series in the cache (`SeriesCache`), so the store is asked once per such read, not at every
recompute of a chart. Such a read is complete from the window's start or Binance's 30 days,
whichever is older, so a window reaching further back reads again and gets its recorded rows
too. A window older than Binance keeps altogether is read from the store alone, and says
`last 30 days only` as before when nothing was recorded for it. `get_indicators`' `change_bars`
and an open interest alert's `change_bars`
([ADR-0085](/chartgen/decisions/0085-open-interests-change-on-server-alerts/)) read through it:
an alert reaching past 30 days is accepted when the rows it needs were recorded.

### Cost

Measured with SQLite 3.50: a year of 10 symbols is 1.05 million rows and 41.5 MB, 40 bytes a
row, so per symbol 288 rows and about 11 kB a day, about 4.2 MB a year. Requests per symbol: 18
at its first round, then 4 a day, against the 1000 per 5 minutes Binance allows these endpoints
(chartgen refuses at 800); with 2 s between requests a first round of many symbols takes at most
150 in any 5 minutes. Reading 1000 bars of 4h (166 days) from a year of rows took 6 ms, 1000 of
1h 0.6 ms.

### Consequences

- Questions about open interest older than 30 days are answerable for the watched symbols, from
  30 days before this build first ran: on its first day the history is Binance's 30 days, after
  N days it is N + 30 days long, up to the retention.
- The round at a start fills the time the server was stopped, as long as Binance still keeps
  it. Up to the last 6 hours 10 minutes may not have been read at a stop, so a stop of up to
  about 29 days loses nothing; a longer one leaves a gap that cannot be filled, and those bars
  have no value.
- The store grows by about 4.2 MB a symbol and year, and is backed up with everything else in it.
- A build from before this ignores the table and records nothing; rolling forward again needs no
  step.

## More Information

- `get_indicators` `change_bars`: [MCP reference](/chartgen/reference/mcp/#change-over-n-bars).
- Running it (settings, cost, rollback): [deploy guide](/chartgen/guides/deploy/#open-interest-beyond-30-days).
- Binance retention and budget: `RETENTION_MS` in `src/indicators/external.rs`,
  `futures_data_budget` in `src/binance_gate.rs`.
