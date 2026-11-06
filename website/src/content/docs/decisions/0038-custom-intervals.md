---
title: "0038 — Custom intervals"
description: "A timeframe is a number and a unit, parsed once on the server. Bars are resampled from the coarsest native interval, on a stable grid, with limits that bound what one request can cost."
sidebar:
  order: 38
---

**Status:** Accepted
**Date:** 2026-11-06
**Extends:** [ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/) (paging and tails), [ADR-0021](/chartgen/decisions/0021-realtime-chart-push/) (chart push), [ADR-0032](/chartgen/decisions/0032-indicator-aware-warmup/) (warmup)

## Context and Problem Statement

The app knew a fixed list of intervals. `data::interval_secs` listed 21 names
and the web app 14. Its **Custom interval...** popup only mapped typed text onto
those 14, so `7m`, `45m`, `8h`, `2D` and `2W` were refused. TradingView lets a user
type a number and a unit.

Accepting more names has a cost that a fixed list hid. Neither source has most
of these intervals, so the server must fetch finer bars and resample them. The
finer the source, the more bars one chart needs: a free-form `1439m` from `1m`
klines is 1439 bars of input per bar of output. On Binance that means requests,
and tail polls repeat them every 2 s. Bars must also land on stable boundaries,
or a reload would show different bars and a live tick would fold into the wrong
one.

## Decision Drivers

- One grammar. The same interval must mean the same thing in the MCP tool, a
  share link, a stored layout and the toolbar.
- A request must not be able to cost an unbounded number of upstream calls.
- A reload gives the same bars, and a live bar is the bar a reload shows.
- Native intervals stay native. A resampled `8h` should not replace Binance's.
- Reuse what exists: session splitting, the phantom-print merge, the grids.

## Considered Options

### The grammar's home

1. **The web app canonicalizes, the server keeps its list.** MCP clients would
   not get custom intervals.
2. **A `parse_interval` MCP tool the popup calls.** One implementation, but the
   popup answers as you type, and a round trip per keystroke buys little.
3. **The server's parser is the source of truth; the web app mirrors it, and a
   shared fixture runs against both.** Two implementations that cannot drift.

### Bounding the cost

1. **A lower limit on each unit only** (say `240m`). Simple, but `239m` still takes
   239 `1m` klines per bar.
2. **Only multiples of a native interval.** Rejects `7m`, which is the point.
3. **A limit on native bars per bar, and one on native bars per fetch.** Bounds
   the cost directly, whatever the interval.

### Pushing a custom interval

1. **Fold each feed bar into the forming bar.** Cheap per tick, but a kline
   update replaces a running candle, so the fold must subtract what the last
   update added. The seed's partial candle has no such record, so volume would
   drift from what a reload shows.
2. **Keep native bars and resample on each recompute.** The window is larger,
   and the result is the fetch path's result by construction.

## Decision Outcome

Chosen: the server's parser with a shared fixture, both limits, and resampling
on each recompute.

### Grammar

`src/interval.rs` parses `<number><unit>`: `m` minutes, `M` or `mo` months, and
`h`, `d`, `w`, `wk` in any case. There are no seconds: neither source has them
on every symbol. The result is canonical, so the same bars have one name: `120m`
is `2h`, `24h` is `1d`, `7d` is `1wk`, `2W` is `2wk`. The server echoes that form,
and everything keyed by an interval (the web cache and merge, alerts, layouts,
push groups) compares it. `data::interval_secs` is now this parser.

`tests/fixtures/interval_grammar.json` lists valid and invalid inputs. Both
`src/interval.rs` and `web/src/lib/timeframe.test.ts` run it. The popup adds two
shortcuts of its own: a bare number means minutes, and a lone `D`, `W` or `M`
means one of that unit.

### Limits, and why

- **Longest per unit: `1439m`, `168h`, `365D`, `52W`, `12M`.** Each stops short of
  what the next unit says better, or at a year. Lengths past these are not bars
  anyone reads, and days, weeks and months are bounded by the source's history:
  Binance's oldest daily klines are a few pages away.
- **At most 60 native bars per bar (`MAX_BASE_PER_BAR`), on Binance's ladder.**
  Yahoo returns one window per request whatever the interval, so its finer bars
  cost history rather than requests; Binance is what needs bounding. The rule
  is the same for both sources, so a share link cannot work for one symbol and
  fail for another. At 60, every length up to an hour qualifies, and above that
  the rounder lengths people use (`65m`, `90m`, `10h`).
- **At most 20 Binance requests per fetch (`MAX_BASE_BARS`, 20,000 native bars).**
  The existing 20 s budget for a paginated fetch stays. Past the limit a call
  returns the newest bars that fit with `has_more: true`, and the client pages
  on. The web app's first load of 500 bars plus 300 warmup fits up to about 25
  native bars per bar. Above that the first page is shorter and its oldest indicator
  values are unconverged until the next page arrives.

### Native interval and grid

The base is the coarsest native interval that divides the requested one, so the
fewest bars are fetched: `45m` from `15m`, `10h` from `2h`. Binance's `6d` comes
from `3d`, whose grid it lies on. On Yahoo whole hours always come from `1h` for
its 730 days of history, rather than `3h` from `90m` with 60 days.

`Interval::grid_start` is the grid:

- Minutes and hours count from the unix epoch on 24/7 symbols. That agrees with
  Binance's klines wherever it has them. It does not restart each day, so `7h`
  bars do not open at the same hour every day.
- Exchange-traded symbols keep session anchoring: buckets start at the session's
  open and the last may be short. The anchor is now the session's first row from
  Yahoo, priced or not. Yahoo sends null prices for some minutes, and anchoring
  on the first priced bar shifted the whole session's grid. That was already
  wrong for `2h` and `4h`.
- Days count from epoch day 1 (Binance's `3d`), weeks from Monday 1970-01-05
  (Binance's `1w`), months from January 1970, so `3M` is a calendar quarter.

Yahoo stamps weekly and monthly bars at local midnight in the offset of that
date. With today's offset across a DST change, that reads as 23:00 the day before,
so `1wk` showed Sundays and `1mo` the last of the previous month. Stamps now round
to the nearest local midnight, which `2W` and `2M` need to bucket correctly.

### Push

`feed_interval` returns the base stream. A group whose feed is finer than its
chart seeds native bars, upserts feed events as before, and resamples on each
recompute with `fetch::resample`. Groups on the same base share one WebSocket.
`--trade` still needs a stream of its own, and refuses other intervals at startup.

## Consequences

**Positive.** Any sensible interval works for both sources, live, with paging,
replay, alerts, layouts and share links, and none of them needed interval logic
of their own beyond the canonical string. MCP clients get it too.

**Negative.**

- Two parsers. The fixture catches a grammar that drifts, but not a new shortcut.
- `61m` is refused, which may surprise someone used to typing any number. The
  message says why.
- Polling is the expensive path. A `59m` tail with warmup 300 reads about 18
  requests per poll. Binance charts normally get pushes and only seed once, but
  a client over its push limit polls every 2 s. That is the worst case the limits
  allow, and it only arises on the longest prime-like lengths.
- A push group at the limit holds up to 20,000 bars and clones them for each
  recompute, at most 4 times a second. At most 64 groups exist.
- Depth of history follows the base. A stock's `7m` ends after about 30 days,
  the limit of Yahoo's `1m`.
- A stock's `2D` follows the calendar, not trading days, because that grid is what
  stays stable across requests. A Friday can be a bar of its own.
- Epoch-counted lengths do not restart daily. TradingView's bar opens may differ
  on 24/7 symbols for lengths that do not divide a day.

## More Information

- [ADR-0009 — Live tail polling and history paging](/chartgen/decisions/0009-live-tail-polling-and-history-paging/)
- [ADR-0021 — Real-time chart push](/chartgen/decisions/0021-realtime-chart-push/)
- [ADR-0029 — Shareable chart links](/chartgen/decisions/0029-shareable-chart-links/)
- [ADR-0032 — Indicator-aware warmup](/chartgen/decisions/0032-indicator-aware-warmup/)
