---
title: "0009 — Live tail polling and history paging"
description: "Keep the web chart live by polling generate_chart for the last bars, and load older history in pages with end_time + warmup."
sidebar:
  order: 9
---

**Status:** Accepted
**Date:** 2026-11-04

## Context and Problem Statement

The web frontend loads one `generate_chart(format=series)` payload and
then sits still. Two TradingView behaviours are missing:

1. **Live candles.** The newest, still-forming candle should move while
   the page is open, and a new candle should appear when the interval
   rolls over. Indicator values on those candles must be right too.
2. **Scroll-back history.** Scrolling left should load older bars on
   demand instead of fixing the chart at the first `bars` returned.

Indicators are computed on the server and the browser does no indicator
math ([ADR-0003](/chartgen/decisions/0003-web-frontend-stack/),
[ADR-0007](/chartgen/decisions/0007-complete-series-serialization/)).
That rules out fetching raw candles and computing values in the client.
An indicator value at bar *i* depends on the bars before *i*, so a short
request ("the last 2 bars") or an older page ("the 500 bars before X")
needs extra earlier bars to compute correct values for the bars it
returns.

## Decision Drivers

- Keep indicator math in one place: the Rust indicator engine.
- Reuse the existing MCP tool and its Zod-validated response shape
  instead of adding a new transport.
- Make E2E tests possible without network access.
- Keep requests bounded. Binance serves at most 1000 klines per call;
  Yahoo keeps only a limited intraday history.

## Considered Options

### Live updates

**Poll `generate_chart` for the tail.** The client calls
`generate_chart(format=series, bars=2, warmup=W)` every few seconds and
merges the two bars (the last closed bar and the forming bar) into its
arrays by `t`. Stateless, and it uses the same tool, the same auth and
the same response schema. The cost is one small request per poll and a
latency equal to the poll interval.

**SSE push from `src/feed.rs`.** The server already runs a Binance
WebSocket feed for the trading engine and could push bar updates over
`GET /mcp`. Latency is lower, but it needs per-subscriber
symbol/interval state, recomputation of indicators per push, and
reconnect/backfill logic. The feed covers Binance only. Deferred, not
rejected.

**Browser connects straight to the Binance WebSocket.** Lowest latency,
but the browser would get raw candles with no indicator values. It would
have to compute indicators itself (ruled out by ADR-0003/0007) or ask the
server again on every tick. It is also Binance-only. Rejected.

### Older history

**Return the full history every time.** The client asks for a larger
`bars` count and replaces its arrays. Simple, but each scroll re-fetches
and re-serializes everything already loaded, the cost grows without
limit, and Binance's 1000-bar cap still applies. Rejected.

**Page with `end_time` plus `warmup`.** The client asks for the `bars`
bars that open strictly before `end_time`, which is the oldest bar it has
loaded. `warmup` extra bars are fetched before them so the indicators are
warmed up when the returned window starts. The server computes over
`bars + warmup` and returns only the last `bars`. Every request has a
bounded cost, and pages join exactly at `end_time`. Chosen.

## Decision Outcome

Chosen: **tail polling via `generate_chart` + `end_time`/`warmup`
paging. Push is deferred.**

- `end_time` (ms since epoch) is an exclusive upper bound on bar open
  time. With it set, a failed upstream fetch is a tool error. The silent
  sample-data fallback would put synthetic bars into a real history.
- `warmup` (series only, at most 2000) adds bars that are used for
  computation and then trimmed. Every index-based channel is trimmed and
  re-indexed: `lines`, `histogram`, `fills`, `signals`, `divlines`.
  Divlines that fall entirely inside the warmup are dropped. Divlines that
  cross into the returned window are clipped at its first bar.
  Volume-profile `hbars` and their `hlines` are recomputed over the
  returned window, so warmup bars don't enter the profile.
- `has_more` in the series payload is `true` when the source returned
  the full `bars + warmup` count. It tells the client to keep paging.
- Binance pagination goes backwards with `endTime` past the 1000-bar cap.
  Yahoo uses `period1`/`period2` windows sized to the request. Its 1h→4h
  aggregation uses the epoch 4h grid, so bucket boundaries match between
  pages.
- `CHARTGEN_DATA_SOURCE=sample` switches to a deterministic offline
  source. Each bar's price is a pure function of the symbol and the bar's
  open time, so pages that overlap agree exactly. The forming bar moves
  with wall-clock time.

## Consequences

**Positive.** Live candles and scroll-back history need no new transport,
no client-side indicator code, and no server-side session state. E2E
tests can run offline against stable data.

**Negative.**

- *Cumulative-indicator seam.* Indicators with unbounded memory (EMA,
  Wilder-smoothed RSI/ATR, OBV, CVD, A/D line, VWAP sessions) never fully
  converge with a finite `warmup`. Recursive ones converge with
  geometrically shrinking error. Running sums (OBV, CVD, A/D) are offset
  by a constant that depends on where the computation started. At a page
  seam the values from the two requests can differ slightly, or by a
  constant for cumulative sums. The same applies to a tail poll compared
  with the initially loaded history. Finite-window indicators (SMA,
  Bollinger, Donchian) match exactly once `warmup` is at least their
  lookback. A larger `warmup` reduces the difference but cannot remove
  it.
- Polling adds a request every interval per open chart, and latency is
  bounded by the poll period.
- Yahoo intraday history is short (1m ≈ 30 days, sub-hour ≈ 60 days,
  1h ≈ 730 days). Paging ends there and `has_more` becomes `false`.

**Neutral.** `png`, `summary` and `both` ignore `warmup` and render
exactly `bars`. `end_time` applies to every format.

## More Information

- [ADR-0003 — Web frontend stack](/chartgen/decisions/0003-web-frontend-stack/)
- [ADR-0007 — Complete series serialization](/chartgen/decisions/0007-complete-series-serialization/)
- [`reference/mcp.md`](/chartgen/reference/mcp/): `end_time`, `warmup`, `has_more`
- [Deployment guide](/chartgen/guides/deploy/): `CHARTGEN_DATA_SOURCE`
