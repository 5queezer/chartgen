---
title: "0049 — One Binance kline read per symbol and interval"
description: "Every read of the newest klines shares one window per symbol and kline, with a one-second freshness window and one caller refreshing. Identical history pages share a read. The cache is bounded by klines, not only entries."
sidebar:
  order: 49
---

**Status:** Accepted
**Date:** 2026-11-06
**Amends:** [ADR-0038](/chartgen/decisions/0038-custom-intervals/) (the kline cache now holds every newest read, repeats may send nothing, and `end_time` pages are shared)

## Context and Problem Statement

Binance was loaded once per user. Only reads of more than 1000 klines went through the
cache of ADR-0038, and even those sent a page on every repeat. A normal chart load (500
bars and 300 of warmup), every tail poll and every history page each sent a request of
their own, so 100 users on BTCUSDT 1h sent 100 requests, from one home IP. How can load
follow the number of symbols rather than the number of users, without a chart ever
showing something a direct read would not?

## Decision Drivers

- A closed kline never changes. The forming one does, and how old it may get must be bounded.
- `has_more`, resampling, history that runs out and a short new listing must behave as
  with a direct read.
- A push seed subscribes first and reads second, so that no event in between is lost.
- Callers wait on tokio's blocking pool, so a burst must not hold it up.
- The container is capped at 256 MiB and idles at 40 MiB.

## Considered Options

For what callers get while one of them refreshes:

1. **All wait for the refresh.** Simple and always current, but every expiry blocks a
   thread per caller for a round trip, on the hottest charts.
2. **All get the old klines until it lands.** Blocks nobody, but when Binance hangs the
   klines grow old with no limit, and a closed kline can be served short of its last trades.
3. **Served while young, then wait.** Option 2 up to an age limit and never across a
   kline's close, then option 1.

## Decision Outcome

Option 3. Each symbol and kline has one window, which every read without an `end_time`
is served from:

- **Sizes.** Only klines the window lacks are read, in whole pages because a page costs
  weight 2 at any size, so reads of mixed sizes add up to one walk. A first read stores
  its first page as soon as it lands and goes on from there, so a small read need not wait
  for a long cold walk, and a failed walk keeps that page.
- **Freshness.** Within 1 s of the read behind the newest klines, a repeat sends nothing.
  That is half the web app's 2 s poll, so a polled candle still moves on every poll.
- **Refresh.** After that, one caller reads enough klines to overlap for the window's whole
  lifetime (at least 16, and 130 on `1s` klines) and lays them over the window. Others get
  the window if it is under 3 s old, and otherwise wait for the refresh and take its result
  however long it took. Were it too old to take, each would refresh in turn until all timed
  out.
- **A kline's close ends the window at once**, by this host's clock, so a poll sees the new
  kline at once. That does not say the closed kline is final. Its last trades can reach REST
  late, and this clock can be behind Binance's, so a poll can get a closed kline short of its
  last trades for as long as that read is served, 1 s, or 3 s during a refresh. A later poll
  lays over it. On `1s` klines every
  second is a close, so serving during a refresh does not apply: every caller after a close
  waits for the round trip.
- **A push seed takes only a read sent after it asked**, refreshes included, so it gets what
  a direct read gets for any lag and any clock. It has subscribed by then, so it hears every
  later event, but nothing would repair a kline whose closing event it missed. Seeds asking
  together still share a read.
- **Failure.** A failed or refused read stores nothing, and fails those who waited for it,
  rather than each trying again in turn. A panic ends the read the same way.
- **History pages.** Identical concurrent `end_time` pages share one read, and a page is
  kept for 60 s once its newest kline closed at least 5 s ago. Nothing lays over a kept page,
  so a page reaching the forming kline, or closed only just now, is not kept.

Age is counted from when a read was *sent*, so the bound holds however slow the answer
is. An answer's forming kline was read at most 1 s ago, or 3 s while a refresh is under
way. A caller that waits for a refresh gets one read at most 10 s ago, the HTTP timeout.
The first read of a window is as old as a direct read of that size.

The cache is bounded by klines as well as entries: 256 windows and 256,000 klines, 128 pages
and 64,000 klines, with the least recently used going first. At about 96 bytes a kline that
is about 31 MB of live klines. That is not resident memory: a review probe, a debug build
on x86_64, measured about 2.3 times that above its start, from transient JSON and malloc
arenas. A window unused for 120 s goes at the next read of any Binance chart; with no traffic
it stays, within the bound.

### Consequences

- **Positive.** N users on one chart cost one first read and at most about one request a
  second, however large N is. Tail polls cost at most `min(30N, 61M)` requests a minute for
  N users on M symbol and kline pairs, with the two exceptions below.
- **Positive.** Callers of a hot chart do not block during a refresh, so an expiry costs
  one thread rather than one per caller.
- **Negative.** A polled candle can be up to 3 s old where it used to be about one round
  trip. Push is unaffected once it runs.
- **Negative.** A push seed costs about one request, as it did before this decision. Only
  seeds asking at the same moment share one.
- **Negative.** The host's clock now decides when a kline closed. If it runs ahead of
  Binance's, a read just after the boundary can be kept for up to 1 s without the new kline.
  If it runs behind, a poll can be served past the true close until it gets there. Polls
  recover on their next read, and seeds are not affected.
- **Negative.** More active windows than fit evict each other, and each poll then walks again:
  more than 12 windows at 20,000 klines, or about 42 at 6000. `Walks` and the gate bound the
  cost, and it is still better than the 8 entries before.
- **Negative.** While Binance answers 5xx, the bound on polls does not hold. A failed read
  stores nothing, so the next caller tries again: one read at a time per key, up to one
  request per round trip, limited by the callers and so never more than before this
  decision. 429, 418 and 403 stop at the gate.
- **Negative.** 40 charts polled at once, each by at least two clients, reach the gate's
  threshold of about 2400 requests a minute. `KLINE_FRESH` is the setting to raise then.
- **Neutral.** The copy each reader takes is not part of the bound, the same as a direct
  read's. Windows are shared, but a request still holds its own bars while it computes.
