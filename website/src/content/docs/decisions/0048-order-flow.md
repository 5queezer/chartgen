---
title: "0048 — Order flow from lower-timeframe bars"
description: "Volume footprints, TPO charts and the volume profile family are computed on the server from finer klines through the Binance gate, with taker buy volume where Binance has it, and say how each chart was made."
sidebar:
  order: 48
---

**Status:** Accepted
**Date:** 2026-11-06
**Amends:** [ADR-0049](/chartgen/decisions/0049-one-kline-read-per-symbol/) (its windows hold taker buy volume, and read `1s` klines)

## Context and Problem Statement

TradingView's Premium plan has a Volume Footprint chart type, a TPO chart type and a
volume profile family, all built from data finer than the bars on the chart. chartgen
spread each bar's volume over its range, which the parity guide listed as a gap. Doing
this needs a new data source and a new wire shape, from a server on a home IP with a
Binance budget it must not exhaust, on a machine with 4 GB of memory.

## Considered Options

**Data**

- **Trades (`aggTrades`) by default.** The only true per-price, per-side data. A busy
  symbol has thousands of trades a minute, and each request returns at most 1000, so one
  1h footprint can take dozens of requests: not boundable for a screen of bars.
- **Finer klines, side from close versus open.** TradingView's documented aggregated
  method. Each intrabar goes entirely to one side.
- **Finer klines with Binance's taker buy volume (chosen, with the previous as the
  fallback).** Field 9 of a kline is the real aggressive buy volume of that kline, so
  totals and delta per intrabar are measured rather than inferred.

**Where to compute**

- **In the browser, from raw intrabars.** Settings would apply without a request, but
  it sends thousands of bars per chart and puts indicator maths in the client, against
  [ADR-0003](/chartgen/decisions/0003-web-frontend-stack/).
- **On the server (chosen).** Two tools, testable on fixtures, usable by any MCP client.

## Decision Outcome

### Data and honesty about it

`get_footprint` picks the finest interval below the chart's that keeps the request under
5000 intrabars: 1s or 1m on Binance when it can, coarser for long or daily charts.

| What | Binance | Yahoo |
|---|---|---|
| Buy and sell per bar | **Measured**, summed from taker buy volume of each intrabar | **Estimated**, each intrabar all to one side |
| Delta per bar, min and max delta | Measured per intrabar | Estimated |
| Volume per row | **Estimated**: each intrabar spread over its high-low range | The same |
| Finest intrabar | 1s | 1m for 7 days, 5m to 30m for 59 days, 1h for 729 days |

A row's volume is an estimate on both sources: a kline does not say at which price
anything traded. The finer the intrabar, the better; on a 1m chart with 1s klines it is
close, on a daily chart with 30m klines it is rough. A kline has one split, so all the
rows it touches share it.

The response says which interval (`intrabar_interval`, null when nothing finer exists and
the bar is its own only intrabar) and which side method (`side`), and the chart's status
shows both. The forming bar repaints as data arrives, and any bar can change if the source
revises it, as TradingView also says of its footprint.

A poll sends back the interval of its load, as a poll's few bars are younger and could read
finer ones than the load's oldest could: a stock's load of 60 hourly bars reaches 12 days back
and reads 5m, and its poll's cells would otherwise change to 1m. Only bars that cannot afford
that interval, after a long gap, read a coarser one, and the status says so.

Without trades there is no tick-based mode, so nothing is exact per price. It remains the
way to add one, behind its own limits.

### Cost, worst case

Klines cost a Binance weight of 2 per request of up to 1000, and every request passes the
gate ([ADR-0038](/chartgen/decisions/0038-custom-intervals/)), which refuses from 80% of
the 6000 per minute. A refusal is an error for that one call.

| Action | Requests | Weight |
|---|---|---|
| `get_footprint`, cold: the chart's bars, and up to 6 pages of intrabars | 7 | 14 |
| `get_footprint`, the same window again | 1 | 2 |
| A footprint page of older bars, each as the chart scrolls to it | up to 7 | up to 14 |
| Live poll per chart, every 10 s: the chart's bars and its intrabars, each at most one request a second per symbol and kline, however many charts share them | 2 | 4 |
| A poll after a gap: up to 200 bars, then a load instead | up to 7 | up to 14 |
| `get_tpo`, cold: up to 3000 blocks | 3 | 6 |
| Live TPO poll per chart, every 30 s: two periods | 1 | 2 |
| A chart load with `volume_profile`, `fixed_range_vp` or `anchored_vp`: extra | up to 6 | up to 12 |
| A history page or a tail of such a chart: extra | 0 | 0 |

So a grid of 4 footprints costs at most 56 weight to load and 96 per minute while live, of
the 4800 the gate allows. Those are worst cases: reads go through the windows of
[ADR-0049](/chartgen/decisions/0049-one-kline-read-per-symbol/), so what any chart on that symbol
and kline already read costs nothing more, and 100 charts on one pair cost what about 10 do.

An interval Binance does not have is built from bars it does, and that counts as for any
chart ([ADR-0038](/chartgen/decisions/0038-custom-intervals/)): a cold 59m footprint from the
web chart reads 5 pages of 1m bars for its chart bars, on top of its intrabars, and an MCP caller
asking for 200 bars and an ATR of 500 reads up to the 20 pages of that cap. Later reads without
`end_time` refresh those from the kline cache.

Only a request with `profile_intrabars` reads finer bars for a profile, and only the web
chart's full load sends it. A page or a tail keeps its load's histogram, so reading them there
would pay for what is thrown away. A bar count cannot tell them apart: a catch-up tail is up to
700 bars, a load 500.

Yahoo has no gate. It takes 2 requests per footprint load or poll, the chart's bars and the
intrabars, up to 7 cold; closed, a poll's intrabars are cached and it takes 1. `get_tpo` takes 1.

### Memory

Binance intrabars live in the windows and pages of
[ADR-0049](/chartgen/decisions/0049-one-kline-read-per-symbol/), inside their bound in klines, and
are shared with charts. A kline there is now 104 bytes rather than 96, for its taker buy volume: 72
for a `Kline`, 32 for its date. That is about 2.6 MB more at the bound of 320,000, about 33 MB of live
klines in all, not resident memory. Taker volume is not a field of `Bar`, as every bar of every chart
would then carry it through every request, and about 70 places build one. The reads no longer need a
cache of their own there. Yahoo's has 8 symbol and interval pairs of at most 6000 intrabars, about
3 MB, evicted by least recent use. A footprint has at most 200 bars and about 200 rows per bar, and a row size
that would give more is widened and reported as such. The bound takes the width of the intrabars
as well as of the bars, as rows are built from them and they are read later: a bar that read
flat would otherwise leave any row size standing. `build_footprint` caps its rows whatever it is
given, and a row size cannot go under a tick or over the highest price.

TPO requests read at most 3000 blocks, and report `periods_cut` when that leaves out older
periods. A block too short for even one period in that (a month of 5m) is widened to the next
that fits, and reported in `block` and `block_widened`, or the request is refused beyond what 4h
blocks can cover: the alternative was a profile with only the end of its period, with the wrong
open, letters and initial balance. A browser chart keeps 600 items.

### Row size

Auto is 0.2 times the ATR, rounded to 1, 2, 2.5 or 5 times a power of ten so that row
prices read well and stay put. TradingView says 0.2 times a normalized ATR and does not
publish the normalization, so this is not claimed to give the same rows. The first load
fixes it, and pages and polls send it back, so rows do not move under a chart. Each bar and
profile carries the row it was built with and is drawn with that: one reply may have had to
widen its own, which the status then says, and the others stay as they were.

A TPO's auto row is a 24th of the median period range. This is a departure: see below.

### Reading them

Binance goes through the windows and pages of
[ADR-0049](/chartgen/decisions/0049-one-kline-read-per-symbol/): one read per symbol and kline
whoever asks, a failed read loses nothing held, and walks of more than a page take a permit and a
deadline. Its `1s` handling did not run until this decision, as `Interval` does not read that
kline, which is no chart interval: a refresh read 16 klines rather than the 130 it states, and a
close did not end the window. A footprint on a 1m chart reads `1s`, so both now hold.

A window does not fit Yahoo, which has no gate, closes for the night and limits fine intervals by
age, and reads through the closed hours would each cost a request. Its reads keep a cache here:
one read at a time per symbol and interval, on a copy, so that a failure leaves what was held. A
caller waiting on a read that fails fails with it, rather than each trying in turn against a
source that is down. No page begins past 20 s.

A chart's load gives up on a profile's finer bars after 8 s, even in the middle of a request,
and spreads its own bars, and the legend then names no finer ones. The read goes on by
itself, bounded by its own deadlines, so what it finds serves the next load. `get_footprint` has
no such fallback, as a footprint without rows is not one: it fails, the status says so, and
the feed retries.

### Imbalance and value area

A buy compares with the sell one level below, a sell with the buy one level above, as
TradingView documents. Stacked imbalances are per bar on the wire; the chart projects them until
a later bar touches them, from the bars it has. A level counts towards a stack only where at
least 2 klines with volume traded, on it and on the level it is compared with. One kline has one
split, spread evenly, so past the ratio all its rows would pass together and a fast 1m wick would
project a zone from arithmetic. Saying so on the status was the other option, but the status
cannot sit on the zone, and the zone is what gets read. Imbalance markers on cells stay as they are.

The value area grows from the POC by the busier neighbour, one row at a time; on a tie the row
closest to the POC, and then the higher one, as TradingView documents for TPO and for its volume
profiles. It adds whole rows until it reaches the target. The same tie rule holds in
`volume_profile` and `compute_profile`.

A TPO block prints at every row it touches, its exact high included; volume on such a row is
zero, as a touch has no share of the range.

### Departures from TradingView

- **An empty level is no imbalance.** TradingView's test is `max >= pct/100 * min`, which read
  literally makes an empty side an imbalance, though its guide does not address that case. At this
  data's resolution an empty edge row usually says nothing about the market.
- **A stack needs 2 klines per level.** It has no such rule, and reads trades in its tick mode.
- **TPO auto row.** Its TPO indicator page publishes a rule: the latest 300 bars, their range in
  ticks divided by 80 rows, rounded to an increment. Ours is a 24th of the median period range, so
  that rows fit one profile rather than 300 bars. That page describes the indicator; the chart
  type's rule may differ.
- **Single prints.** It says non-extreme levels with one block. Ours leaves out a whole run that
  reaches an extreme, as a tail, so a tail's inner rows do not count.
- **A value area may pass its target.** Its volume profile page stops before a row would pass it;
  its TPO page adds until it reaches it, which is what ours does for both.
- **Blocks can be widened**, and rows in a reply can be, within the limits above.

### Rendering

Both chart types are painters on the hidden series of
[ADR-0030](/chartgen/decisions/0030-chart-types-beyond-klinecharts/). klinecharts' widest
bars are 50 px, so choosing a footprint zooms there and leaving gives the zoom back. Under
22 px a bar, or under 2 px a row, cells give way to candles and the status says to zoom in.
Numbers show from 34 px and only when they fit. A TPO draws a candle for any bar no profile
covers, as the series' own candles are hidden: older than the source has blocks, or a gap.
Volume candles scale their width by volume within the bar's slot, as the time axis stays even;
there is no help page for this type, so this is from general knowledge.

A poll asks for what it missed since its last good read, so a hidden tab, sleep or errors leave
no hole. Past 200 bars, or 10 periods of a TPO, the chart loads again. The time since its last
read decides this and not its newest item, which stays put while a market is closed.

### Volume profiles

`volume_profile`, and the new `fixed_range_vp` and `anchored_vp`, read finer bars on a chart
load, and fall back to the chart's bars when none can be had. The series says which in
`profile_interval`, and the legend shows it. A tail or a page never pays for it, as these keep
the histogram of their load ([ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/)).

`session_vp` does not read them. Unlike those, its lines are per-bar values that every tail
overwrites from the chart's bars, so a load from finer bars would step to another level at the
first tick under a legend still naming them. It now has one line each for POC, VAH and VAL: with
a line per session, a tail had fewer lines than its load and failed to fit on every update. It
holds a value on every bar of a session, and `level_lines` tells both painters to draw nothing
across a change of value, so sessions do not join. `hvn_lvn`, `naked_poc`, `tpo`, `cvd` and PNG
and summary output also spread each chart bar's volume.

### Consequences

- Good: real delta on Binance at a cost that is bounded and small.
- Good: every chart says how it was made.
- Bad: rows are estimates. A footprint must not be read as exact per price.
- Bad: settings the server computes with cost a reload, though from cached intrabars.
- Bad: `session_vp` keeps spreading each bar's volume, as what could improve it cannot reach its tails.
- Bad: a slow Binance leaves a thread per profile load finishing its read after the load gave up, bounded by that read's deadlines.
- Bad: a footprint's forming intrabar is refreshed up to once a second per symbol and kline, not once per 5 s, as ADR-0049's freshness applies. That only costs more on a pair polled by over 2 charts.
- Bad: live cells lag the candle by up to 10 s, while a pushed tail moves the bar at once.
- Bad: a TPO is off in bar replay, as a profile holds its whole period.

Not in this version: tick-based mode, open interest, footprint alerts, right click to split
or merge TPO profiles, pre and post market sessions (the Yahoo source reads the regular
session), and drawing fixed range profiles over their range rather than at the chart's edge.
