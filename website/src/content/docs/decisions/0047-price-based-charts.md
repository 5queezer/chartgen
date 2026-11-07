---
title: "0047 — Price-based charts: Renko, Kagi, Point & Figure, Line Break, Range"
description: "These bars are built on the server from the chart's own timeframe, opened at a stable time, on a box fixed when the chart loads. Times are not evenly spaced, so replay and compare are refused rather than half-working. Also adds seconds intervals and HLC area."
sidebar:
  order: 47
---

**Status:** Accepted
**Date:** 2026-11-06
**Extends:** [ADR-0003](/chartgen/decisions/0003-web-frontend-stack/) (server-side series styles), [ADR-0030](/chartgen/decisions/0030-custom-chart-types/) (client-drawn types), [ADR-0029](/chartgen/decisions/0029-shareable-chart-links/) (share links)
**Amends:** [ADR-0038](/chartgen/decisions/0038-custom-intervals/) (the grammar now has seconds)

## Context and Problem Statement

TradingView's Premium plan has Renko, Kagi, Point & Figure, Line Break and custom Range
bars, on intraday data as well as daily. Indicators run on those bars. `chart_style`
already builds Heikin Ashi on the server, so the same place suits these. But Heikin
Ashi keeps one output bar per input bar, and everything else in the app quietly
assumes that: bars have evenly spaced open times, a tail poll overlaps by two bars, a
reload shows the same bars, replay steps a clock from one bar's close to the next.

Price-based bars break each of these. Several bricks can come from one source bar and
none from the next hundred. What exists depends on where the build started and on
the box. Their box can depend on the latest price. How should they work, and what
should be refused rather than left half working?

## Decision Drivers

- Indicators run on the bars the chart draws, as on TradingView.
- A reload and a live tail show the same bars, or merges fail and charts keep reloading.
- A request's cost through the Binance gate stays bounded, and so does memory: the
  container is capped at 256 MiB on a 4 GB host.
- Say what only approximates tick data, as TradingView's documentation does.
- The rest of the app needs as few changes as possible; what cannot work is refused.

## Considered Options

### Where the bars are built

1. **In the browser, from candles.** Indicators would still run on candles, and every MCP
   client and the PNG would have to build their own.
2. **On the server, from the bars of the chart's timeframe (chosen).** `timeframe` is the
   source. `src/price_bars.rs` runs before indicators, like Heikin Ashi.
3. **From `aggTrades`.** True to the tape, and far too expensive per chart.

### How a bar is timed

1. **The source bar that confirmed it.** Natural, but the forming bar's time then moves
   with each source bar, so a tail does not line up with what the client holds.
2. **A running sequence number.** No relation to time: drawings, the axis and a page's
   `end_time` would all mean nothing.
3. **When it opened (chosen):** the first source bar after the last confirmed bar, as for
   a candle. The open bar keeps that time when it is confirmed, so a tail replaces it in
   place. A second bar from one source bar takes the next free second.

### Keeping a reload and a tail on the same bars

The result depends on where the build starts.

1. **Build per chunk of time, each a pure function of itself and the chunk before.**
   Exactly stable forever, at the cost of a seam per chunk, a clamp on times at each
   seam, and a second kind of open bar.
2. **Build from wherever the read begins.** The start moves with every source bar, so
   the oldest bars keep changing.
3. **One build from a start held on a grid of time (chosen).** Reads in one chunk start
   at the same bar, so they are identical. When the chunk rolls over the start moves
   forward, and the two builds usually agree at the live end: the grid types once price
   leaves the range between their two states, and Kagi, Line Break and Range after a
   reversal. Usually is not always, so a series names its build (below).

### The box

1. **Resolve it on each call.** Each poll could get another box.
2. **A stable reference**, such as the price at the chunk's edge. Reloads would be exactly
   stable, but "1% of the last price" could be a month old.
3. **Resolve at load, round, echo, and send it back (chosen).** It is what TradingView
   describes: a value at load, rounded so it stays stable. The client sends the echoed
   size with every later poll, page and subscribe.

## Decision Outcome

### The types

All on closes of the source bars, except Range, which walks each source bar's assumed path.

- **Renko:** a grid of multiples of the box. A brick continues a box past the last brick's
  end, and reverses a box past its start, so two from its end.
- **Kagi:** a line runs while the close goes its way, and turns at its extreme once the
  close comes back by the reversal amount. One bar per line. Thick and thin follow the
  last shoulder and waist, and a line takes one weight along its whole length.
- **Point & Figure:** a grid of multiples of the box. One bar per column, from lowest to
  highest level. Up takes the highest level the close has passed; a reversal of N needs N
  boxes, and the new column starts one box from the old one's end.
- **Line Break:** a new line when the close passes the highest or lowest of the last N
  lines. Up lines start at the last line's top, and down lines at its bottom.
- **Range:** a bar closes when its high and low are the range apart, at the side reached,
  and the next opens there. A gap makes as many bars as it spans.

`box` is the box, the Kagi reversal or the Range size: `atr` (length, default 14),
`fixed`, or `percent` (default 1% of the last price). ATR is Wilder's, over closed source
bars. ATR and percent round to two significant digits, so a small move keeps the box.
A later load can still land on another box when the value crosses a rounding edge.

### Approximation

These bars are built from candles and not from ticks, as TradingView's are for its
non-standard types. Within one source bar the order of moves is unknown. Only closes
count for four types, so a wick that reversed and came back makes no brick. Range assumes
the wick away from the close came first. A finer `timeframe` is closer to the tape, and
costs less history.

### Live

Only closed source bars confirm a bar. What is left is the **open bar**, from the last
confirmed level to the last price. A spike in a forming source bar stretches the open bar
and confirms nothing, so a bar never appears and then disappears. Kagi and Point & Figure's
open bar is their current line or column. It turns only when a source bar closes.

**The newest bar of a live build always closes on the last source close**, inside its high
and low, whether or not a source bar is forming: when a kline has just closed, and all the
time a stock's market is shut. So a confirmed brick's close can read past its level until
the next tail replaces it. History pages keep their levels. Alerts, indicators and the
price line all read this close.

A poll asks for 64 bars (`PRICE_TAIL_BARS`), because one source bar can confirm several. A
push sends 64 first, then the bars since the last tail plus one of overlap. A tail past our
newest bar leaves a gap and reloads, without a time-based catch-up: time says nothing
about how many bars a gap spans. `subscribe_chart` refuses an ATR or percentage box without
the size a load echoed: one resolved there could differ from the chart it serves, and a group
outlives its first subscriber's load. Past 64 new bars, or on joining a running group, a tail
can start past the client's newest bar, which then reloads.

### A new build

A series echoes `anchor`, the time of the first source bar its build used. That is the bar
actually read, not the grid's edge, so a read that starts later for any reason counts.

- **Same anchor:** a tail must line up with our bars, and one that ends before our newest
  bar is an outdated response, ignored as `stale`.
- **A later anchor:** a new build. Its bars need not line up with ours and it may end before
  our newest, so from its first bar on it replaces ours, whatever their times, and the
  chart keeps updating. No reload.
- **An earlier anchor:** an outdated response, `stale`.

Paged history survives a new build: bars older than the tail stay, from the old build, until
a reload. So a chart that has been open across a rollover can differ from a fresh load in
its older bars, not at its live end. A reload for any other reason drops paged history, as
on a candle chart.

### Behaviour on these bars

| Feature | On a price-based chart |
|---|---|
| Time axis | Bars are evenly spaced on screen and the labels are real times, so the time scale is not linear, as on TradingView. |
| Legend, Data Window | OHLC of the bar and when it opened, with `Renko 3.3` beside the interval. The phone's short legend shows only the OHLC. |
| Countdown | Hidden: these bars close on price. |
| Indicators | Run on these bars, including on tails and pages. No `future` block: there is no next bar's time to project to. |
| Volume profiles | `profile_intrabars` ([ADR-0048](/chartgen/decisions/0048-order-flow/)) plans its finer bars on the time these bars cover, as their count says nothing of it. A read planned on the count would stop at its page limit, with the profile holding only the chart's newest part. Too long a time for finer bars leaves the profile computed from these bars, with no `profile_interval`. |
| Drawings | Stored by time and price as on any chart, on the bars' own times. Back on candles, a point lands on the bar at that time. A new box, or a chunk rollover, can leave a point between bars, which then lands on the nearest. |
| Paging | Works. A page reads a window of source bars before `end_time` and returns bars before it. Its build starts elsewhere, so the join can differ slightly from an unbroken build. A reload shows the same join. |
| Share links | `type=renko`, plus `tp=box:fixed;size:50` with what differs from the defaults. A price-based link without `tp` means the defaults. An older app ignores both. |
| Layouts | `chartParams` beside `chartType`, left out when all default, so older layouts load unchanged and stay clean. Per chart of a grid. |
| Refused boxes | A saved or linked fixed or percentage box that is refused goes to ATR, with a message. Where ATR cannot make a box, as on a market with no movement, the chart goes to candles, with a message. So a fixed box taken to a much dearer symbol is lost, and a type picked on a flat market does not stick. |
| Price alerts | Work: they watch the last price, which the newest bar always closes on. |
| Indicator and sloped-line alerts | Work, but tied to the type as well as the interval (`1h renko`), since their values and bar indices mean something else on these bars. An alert made on candles waits for a candle chart, and says so. |
| Server alerts | Run on the interval's candles (ADR-0042), never on bricks. |
| PNG | Candles of the same bars, with the type and box in the title: `BTCUSDT · 1H · P&F 3.4 x 3`. Kagi and Point & Figure are not drawn as lines or marks. |
| **Bar replay** | **Refused.** The clock steps by bar closes, and a brick's open time says nothing about when it was confirmed, so a frame would show bars before their time, on a box from the future. |
| **Compare** | **Refused.** Compare joins on a time grid and sizes its loads by bar count. The list is kept for when the type changes back. Until then it acts as if empty: no percent axis, no Object Tree entry, no pause of the ratio lock. |

### Cost and memory

`source_bars` is what a build reads: 5 chunks of 1000 source bars, fewer when an interval
is resampled, so a read stays within `MAX_BASE_BARS` (20,000 native bars, ADR-0038).

- **Binance, first load:** at most 5 requests for a native interval, 20 for a resampled
  one, through the gate and `Walks`. It goes through the window of that symbol and kline
  ([ADR-0049](/chartgen/decisions/0049-one-kline-read-per-symbol/)), so only what the window
  lacks is read, and a chart on a kline already held costs less.
- **Binance, a history page:** 5 or 20 requests again, as a page does not read the window.
  Identical pages share one read, and one is kept for 60 s once its newest kline closed at
  least 5 s ago, but the page cache holds 64,000 klines: 12 pages of 5000, or 3 of 20,000.
- **Binance, polls:** nothing within 1 s of the read behind the window, and otherwise one
  request for all the charts on that symbol and kline, however long the gap, so at most
  about one a second per window. A refresh overlaps the window's whole 120 s lifetime: 16
  klines at least, 130 on `1s`. On `1s` every second is a close, so a poll waits for its
  round trip instead of being served during a refresh.
- **Binance, push seeds:** about one request when the window is held, since a seed takes only
  a read sent after it asked, and the first-load cost when it is not.
- **Evicted or idle windows:** a window unused for 120 s goes at the next read, which then costs
  a first load. The cache holds 256,000 klines, so about 51 active windows of 5000, or 12 of
  20,000. More than that evict each other, and each poll then walks again, 5 or 20 requests,
  bounded only by `Walks` and the gate.
- **Yahoo:** one request per call, not gated and not cached. It is far more than a candle
  poll asks for, every 15 s per chart, all from one server IP: 60 days against 10 at `5m`,
  730 against 75 at `1h`, about 10,000 against 600 at `1d`. This stays open: it would take
  a held window like Binance's.
- **Memory:** at most 20,000 bars per build (`MAX_PRICE_BARS`). A box that would make
  more is refused with a message, before anything is allocated for it, and so is any box
  under a millionth of the last price. A push group holds at most 20,000 native bars, about
  1.9 MB, on top of the cache's 31 MB of live klines. Those are estimates, not a measurement
  of this type: 64 resampled groups come to about 123 MB, and ADR-0049 measured about 2.3
  times its live klines above the start, so the worst case nears the container's 256 MiB
  from an idle 40 MiB. Typical charts are far below it. A poll recomputes every
  indicator over all the built bars, where a candle chart's covers about 300, so a chart with
  thousands of bricks costs more CPU each poll. Pushes go at half the rate.

### Other types and intervals

**HLC area** joins the client-drawn types of ADR-0030. **Seconds:** `1s` is Binance's own
spot kline, and `5s`, `15s`, `30s` and any other length under a minute are resampled from
it. Quiet seconds are rows with zero volume, not missing rows, checked on an illiquid
symbol on 2026-11-07, now and 30 days back. Futures-only symbols have none. Yahoo has no seconds data, so a stock refuses them
with a message saying so, and its menu does not offer them. Tick intervals are not
offered.

## Consequences

**Positive.** One builder for every path: load, page, tail, push, PNG. On Binance, reloads
inside a chunk are identical by construction, and the box is what can still change them.
Everything keyed by time and bar order works without knowing about these types.

**Negative.**

- At a chunk rollover the new build's older bars can differ, and on a wide box the live end
  can too, for as long as the market makes no reversal that brings them together. A chart
  takes the new build from the tail on (see above), so a bar can move or vanish at that
  moment, every 16.7 h on `1m`, 66 min on `5s` and 16.7 min on `1s`.
- **Yahoo is less stable than Binance.** A build's chunk there is a sixth of what its read
  reaches, so the start lies inside what Yahoo returns: every 28 h or so on `1m`, 10 days on
  `5m`, and 121 days on `1h`, the rollovers are that much closer. Inside a chunk, bars are the
  same only if Yahoo returns the same rows: it can revise them, return null prices, and
  shorten a window near its age limit, and none of that is under our control. The series'
  anchor catches a read that starts elsewhere, and the chart takes it without a reload.
  None of this has been run against Yahoo.
- A box that crosses a rounding edge between loads changes every bar, and drawings
  then land on the nearest ones.
- A window can hold fewer bars than a chart wants when price barely moved against the box,
  so `has_more` stays true and the client pages. One source window with no bar at all ends
  the history early.
- A wide box on a fine timeframe gives few bars per request, as history per source bar is
  short. Binance `1s` holds about 83 minutes per load.

## More Information

- [ADR-0009 — Live tail polling and history paging](/chartgen/decisions/0009-live-tail-polling-and-history-paging/)
- [ADR-0021 — Real-time chart push](/chartgen/decisions/0021-realtime-chart-push/)
- [ADR-0049 — One Binance kline read per symbol and interval](/chartgen/decisions/0049-one-kline-read-per-symbol/)
- [ADR-0023 — Bar replay](/chartgen/decisions/0023-bar-replay/)
- [ADR-0034 — Drawing and indicator alerts](/chartgen/decisions/0034-drawing-and-indicator-alerts/)
