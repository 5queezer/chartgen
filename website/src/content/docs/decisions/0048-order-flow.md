---
title: "0048 — Order flow from lower-timeframe bars"
description: "Volume footprints, TPO charts and the volume profile family are computed on the server from finer klines through the Binance gate, with taker buy volume where Binance has it, and say how each chart was made."
sidebar:
  order: 48
---

**Status:** Accepted
**Date:** 2026-11-06

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
| Live poll per chart, every 10 s; intrabars at most once per 5 s per symbol and interval | 2 | 4 |
| `get_tpo`, cold: up to 3000 blocks | 3 | 6 |
| Live TPO poll per chart, every 30 s | 1 | 2 |
| A chart load with a volume profile: extra | 6 | 12 |

So a grid of 4 footprints costs at most 56 weight to load and 96 per minute while live, of
the 4800 the gate allows. Yahoo takes one request per load or poll. Pages only load for
bars drawn as cells, a page at a time.

### Memory

The server holds 8 symbol and interval pairs of at most 6000 intrabars, about 3 MB, evicted
by least recent use. A footprint has at most 200 bars and 200 rows per bar, and a row size
that would give more is widened and reported as such. TPO requests read at most 3000 blocks,
and report `periods_cut` when that shortens them. A browser chart keeps 600 items.

### Row size

Auto is 0.2 times the ATR, rounded to 1, 2, 2.5 or 5 times a power of ten so that row
prices read well and stay put. TradingView says 0.2 times a normalized ATR and does not
publish the normalization, so this is not claimed to give the same rows. The first load
fixes it, and pages and polls send it back, so rows do not move under a chart. A TPO's auto
row is a 24th of the median period range; TradingView does not publish that one either.

### Imbalance and value area

A buy compares with the sell one level below, a sell with the buy one level above, as
TradingView documents. A level with nothing to compare with is **no** imbalance. TradingView
does not say; at this data's resolution an empty edge row usually says nothing about the
market. Stacked imbalances are per bar on the wire; the chart projects them until a later
bar touches them, from the bars it has. The value area grows from the POC by the busier
neighbour, like the volume profile's, one row at a time.

### Rendering

Both chart types are painters on the hidden series of
[ADR-0030](/chartgen/decisions/0030-chart-types-beyond-klinecharts/). klinecharts' widest
bars are 50 px, so choosing a footprint zooms there and leaving gives the zoom back. Under
22 px a bar, or under 2 px a row, cells give way to candles and the status says to zoom in.
Numbers show from 34 px and only when they fit. Volume candles scale their width by volume
within the bar's slot, as the time axis stays even; there is no help page for this type, so
this is from general knowledge.

### Volume profiles

`volume_profile`, the new `fixed_range_vp` and `anchored_vp`, and `session_vp` read finer
bars on a chart load, and fall back to the chart's bars when none can be had. The series
says which in `profile_interval`, and the legend shows it. A tail never pays for it, as a
profile on live bars keeps its load ([ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/)).
`session_vp` now has one line each for POC, VAH and VAL, broken between sessions: with a line
per session, a tail had fewer lines than its load and failed to fit on every update.

### Consequences

- Good: real delta on Binance at a cost that is bounded and small.
- Good: every chart says how it was made.
- Bad: rows are estimates. A footprint must not be read as exact per price.
- Bad: settings the server computes with cost a reload, though from cached intrabars.
- Bad: live cells lag the candle by up to 10 s, while a pushed tail moves the bar at once.
- Bad: a TPO is off in bar replay, as a profile holds its whole period.

Not in this version: tick-based mode, open interest, footprint alerts, right click to split
or merge TPO profiles, pre and post market sessions (the Yahoo source reads the regular
session), and drawing fixed range profiles over their range rather than at the chart's edge.
