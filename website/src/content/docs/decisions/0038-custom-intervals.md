---
title: "0038 — Custom intervals and session-aware future bars"
description: "A timeframe is a number and a unit, parsed once on the server. Bars are resampled from the coarsest native interval, on a stable grid, with limits that bound what one request can cost. Projected bars stay in the exchange's session and skip its known holidays."
sidebar:
  order: 38
---

**Status:** Accepted
**Date:** 2026-11-06
**Extends:** [ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/) (paging and tails), [ADR-0021](/chartgen/decisions/0021-realtime-chart-push/) (chart push), [ADR-0032](/chartgen/decisions/0032-indicator-aware-warmup/) (warmup)
**Amends:** [ADR-0033](/chartgen/decisions/0033-indicator-projection-past-last-bar/) (future bar times)

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

Separately, ADR-0033 deferred sessions and holidays until bars carried their
session. Its future bars skip weekends on daily stock charts and nothing else, so
an intraday stock chart put cloud bars in closed hours.

## Decision Drivers

- One grammar. The same interval must mean the same thing in the MCP tool, a
  share link, a stored layout and the toolbar.
- A request must not be able to cost an unbounded number of upstream calls.
- A reload gives the same bars, and a live bar is the bar a reload shows.
- Native intervals stay native. A resampled `8h` should not replace Binance's.
- Reuse what exists: session splitting, the phantom-print merge, the grids.
- Do not invent market data. A calendar we cannot source is one we do not have.

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

### Exchange sessions for future bars

1. **Infer hours from the loaded bars' first and last times.** No new data, but a
   tail of a `1m` chart can hold less than one session, and the result would differ
   between a full load and a tail.
2. **A table of exchanges and their hours.** Data to maintain and get wrong, for
   something the source already reports.
3. **Yahoo's metadata.** Every chart response names the exchange's time zone and
   its regular session.

### Holidays

1. **Compute them by rule** (third Monday of January, and so on). Looks complete
   and is unbounded, but exchange rules have exceptions the rules do not capture.
2. **A paid or external calendar service.** A new runtime dependency for labels.
3. **A small table copied from the exchange's published calendar**, that ends
   when the calendar ends.
4. **No holidays.** Simple, and wrong a few days a year.

## Decision Outcome

Chosen: the server's parser with a shared fixture, both limits, resampling on each
recompute, Yahoo's metadata for sessions, and a small NYSE table for holidays.

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

- **Upper bounds per unit, after normalization: under 1440 minutes, then `168h`,
  `365D`, `52W`, `12M`.** These are bounds, not lengths that all parse: `1439m` and
  `167h` fail the next rule, `1440m` normalizes to `1d`, and `168h` to `1wk`. Each
  stops short of what the next unit says better, or at a year. Lengths past these
  are not bars anyone reads, and days, weeks and months are bounded by the source's
  history: Binance's oldest daily klines are a few pages away.
- **At most 60 native bars per bar (`MAX_BASE_PER_BAR`), for minutes and hours, on
  Binance's ladder.** Days, weeks and months have no such rule (`365D` takes 365
  `1d` klines), because their history is what bounds them.
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

### Tail polls

A tail poll cannot drop its warmup: the server computes indicators over it, and
many indicators declare no `warmup_bars` and rely on the client's 300. A `59m` tail
at warmup 300 therefore needs about 18,000 `1m` klines, which is 18 requests, every
2 s for a client over its push limit. But it only needs to *read* the ones it does
not have, because a closed kline never changes. `fetch::newest_klines` holds the
native klines of a resampled chart for 2 minutes since last used, per symbol and
kline. A repeat reads one page of 16 and lays it over them, so a poll costs one
request after the first. A gap reads everything again, as does a first read, or a
read of more than is held. Only the live end is cached; `end_time` pages read
Binance as before.

### A budget in front of Binance

The limits above bound one request. Nothing bounded a sequence of them: parallel
history pages at 20 requests each, nine symbols thrashing a cache of eight (about
8000 weight a minute), or subscribe churn. A 429 was only an error string, so the
next call simply tried again, and Binance answers that with an escalating 418 ban
of the IP, here a home one.

`binance_gate::Gate` stands in front of every REST request, native intervals, seeds,
quotes, the symbol list and the feed's backfill included:

- After a 429 or 418, nothing is sent until `Retry-After` has passed (60 s if
  it names none), and a later, shorter answer does not shorten it.
- Nothing starts once `x-mbx-used-weight-1m` reaches 80% of the limit, until the next
  minute. The limits are Binance's published ones: 6000 for `api.binance.com` and,
  on a gate of its own because its counter is separate, 2400 for `fapi.binance.com`.
- A refused call fails with an error starting `Binance rate limit:` and stores
  nothing, so no cache holds a partial read, and a symbol is not marked unknown.

At most 4 paged walks run at once (`Walks`). Concurrent first reads of one chart share
a walk. Reads of the newest bars of any interval share `newest_klines`, so a push
seed is one request after the first as well.

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
`--trade` still needs a stream of its own, and refuses other intervals at startup. It
keys its state by the canonical form and subscribes under Binance's name for it, so
`-i 1W` follows `1w` and `get_indicators` finds it by any spelling.

### Future bars

`OhlcvData` gains `session: Option<Session>`, parsed from Yahoo's `meta`: time zone
from `exchangeTimezoneName`, open from `currentTradingPeriod.regular`, and close
from the latest of that and `tradingPeriods`, because today may be an early close.
`chrono-tz` turns those into each future day's UTC time, so DST changes need no
special case. A response with a regular session of 23 hours or more (futures), a
24/7 instrument, or a zone it does not know has no session and steps on as before.
That includes Binance and the sample source.

`data::future_bar_times` then works per interval:

- **Intraday with a session.** Step from the last bar until the next would reach
  the close, then restart at the next open day's open. That is the grid resampled
  bars use, and Yahoo's own: its `1h` bars open at 09:30, 10:30 and so on.
- **Days.** Next slot of the day grid, on its first open day.
- **Weeks and months.** As before. They do not move for a closed day.

Closed weekdays come from the bars. Saturday and Sunday each close when no bar falls
on them, and a Sunday with neither a Friday nor a Saturday is a Friday and Saturday
weekend, which finds a Sunday to Thursday market without a list. Nothing else closes,
so a Thursday holiday inside a one-week window does not close Thursdays. A market with
a session is known to have closed days. Without one, crypto included, it takes bars
that could show them: a span of 7 days of `1d` bars, or 5 cycles of 7 longer ones,
so a few long bars do not read as a weekend.

The block is computed from every loaded bar, warmup included. It used the visible
bars before, so a two-bar tail of a daily crypto chart saw no weekend bar and
skipped weekends the full load did not.

**Holidays.** `US_HOLIDAYS` and `US_EARLY_CLOSES` hold the New York Stock Exchange's
published 2026 to 2028 calendar, read from
[its site](https://www.nyse.com/markets/hours-calendars) on 2026-11-06. They apply to
equities, ETFs, indices and mutual funds in New York's time zone, which covers
NYSE and Nasdaq listings. Their end is a known limit: past 2028 only weekends are
skipped, until someone adds the next years. No other exchange has a table: its
future bars keep correct hours and weekdays and step through its holidays.

The existing cap of 500 projected bars and the calendar months are unchanged.

## Consequences

**Positive.** Any sensible interval works for both sources, live, with paging,
replay, alerts, layouts and share links, and none of them needed interval logic
of their own beyond the canonical string. MCP clients get it too.

**Negative.**

- Two parsers. The fixture catches a grammar that drifts, but not a new shortcut.
- `61m` is refused, which may surprise someone used to typing any number. The
  message says why.
- A process-wide cache of native klines. It is at most 8 entries of 20,000 bars,
  about 13 MB, held only while such charts are being read. An idle entry's memory
  is freed by the next read of any custom chart, not on its own.
- Scrolling back on a custom chart is not cached: a `59m` history page still costs
  up to 20 requests, though only when a person pages, and the budget applies.
- One client can spend the budget for all: the gate is process-wide, not per token,
  so past the threshold everyone's Binance charts fail for the rest of the minute.
  A chart already loaded keeps what it has, and the error names the wait.
- A push group at the limit holds up to 20,000 bars and clones them for each
  recompute, at most 4 times a second. At most 64 groups exist.
- Depth of history follows the base. A stock's `7m` ends after about 30 days,
  the limit of Yahoo's `1m`.
- A stock's `2D` follows the calendar, not trading days, because that grid is what
  stays stable across requests. A Friday can be a bar of its own.
- Epoch-counted lengths do not restart daily. TradingView's bar opens may differ
  on 24/7 symbols for lengths that do not divide a day.
- A new dependency, `chrono-tz`, with its time zone tables, for DST. Without it a
  projection across a change would be an hour off.
- Holidays are known for one market until 2028, and only as scheduled. An
  unscheduled closure, another exchange's holiday, or a US date past 2028 shows a
  future bar on a closed day. These times only label future bars, in the legend,
  the Data Window and the `future.t` an MCP client reads, so the harm is small.
- Yahoo's session has no lunch break, so future bars run through it. Its Tokyo,
  Shanghai and Hong Kong `5m` rows continue through the break with null prices
  (checked 2026-11-06), so charts have no bars there while future bars do. Because
  those rows mark the session as unbroken, the afternoon stays on the morning's
  grid: a Shanghai `45m` bar can be stamped 12:30, inside lunch. Yahoo's own `1h`
  rows are stamped the same way. Modelling lunch stays out of scope.
- US indices quoted from Chicago get no holidays. `^VIX` and `^TNX` report their
  own exchanges in `America/Chicago`, with their own hours, and the NYSE's table
  does not say those follow it, so their future bars run through US holidays.
- The 20,000-kline cap also shortens warmup, and a poll or push never pages to make
  up for it. A `59m` chart holds about 338 bars at most, so with `warmup: 1000` a long
  indicator (`ema` of 500) stays unconverged or null on the forming bar. The response
  does not say so.
- A market that changes its hours is only reflected when Yahoo reports them.
- Extended hours are not covered, and neither are futures' daily and weekend breaks.

## More Information

- [ADR-0009 — Live tail polling and history paging](/chartgen/decisions/0009-live-tail-polling-and-history-paging/)
- [ADR-0021 — Real-time chart push](/chartgen/decisions/0021-realtime-chart-push/)
- [ADR-0029 — Shareable chart links](/chartgen/decisions/0029-shareable-chart-links/)
- [ADR-0032 — Indicator-aware warmup](/chartgen/decisions/0032-indicator-aware-warmup/)
