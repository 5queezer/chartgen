---
title: "0032 — Indicator-aware warmup"
description: "The server raises a non-zero series warmup to what the requested indicators need to converge, so EMA 200 joins history pages and live tails without a step."
sidebar:
  order: 32
---

**Status:** Accepted
**Date:** 2026-11-05
**Amends:** [ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/) (the meaning of `warmup` and its "cumulative-indicator seam" consequence; paging and tail polling stay as decided there)

## Context and Problem Statement

ADR-0009 pages history with `end_time` plus `warmup` and accepts that
recursive indicators "can step slightly at a seam". The web app sends
`warmup: 300` with every load, page, tail poll and push subscription.

QA found the step is not slight. On 1m BTCUSDT the `ema_stack` EMA 200
jumped by about $250 at $80k (0.3%) at every history seam. The deterministic
sample data shows 1.9%. Pine's `ta.ema` is seeded with the SMA of its first
200 bars, so with 300 bars of warmup the first returned bar is only 100
recursions past the seed. The seed still weighs (1 − 2/201)^100 ≈ 37%. The
newer side of a seam, which is the left edge of the response loaded before,
is the one that is off. Keeping already-loaded values when a page is merged
therefore does not help. A tail poll computed over 302 bars has the same
error against the history it is merged into.

An SMA-seeded EMA of length N needs N bars for the seed and about
ln(1000) / −ln(1 − α) more until the seed weighs under 0.1%. For EMA 200
that is 200 + 691 = 891 bars. RSI, ATR, ADX, MACD and Supertrend at their
default lengths converge within 300.

## Decision Drivers

- Values at a seam or on a live tail should match one long request, as on
  TradingView.
- Charts whose indicators converge within 300 bars should not pay for
  longer fetches on every poll and push.
- Indicator math, and what it needs, stays in the Rust engine (ADR-0003).

## Considered Options

1. **A larger fixed client warmup (1000).** Fixes EMA 200 everywhere, but
   every tail poll, push window and page then costs 1000 more bars, also for
   an RSI-only chart. Two Binance requests per poll.
2. **Keep already-loaded values when merging a page.** Does not help: the
   badly warmed side of the seam is the newer response's left edge. Fixing
   it would need pages overlapping it by about 700 bars, and tails would
   still be off.
3. **The client picks a warmup per indicator.** The web app would need each
   indicator's lookback and smoothing, duplicating engine knowledge.
4. **The server raises `warmup` to what the indicators declare.** Each
   indicator reports the bars it needs to converge; the server uses at
   least that.

## Decision Outcome

Chosen: **option 4.**

- `Indicator::warmup_bars()` returns the bars of history an indicator needs
  before its values converge (an SMA-seeded EMA's seed weighs under 0.1%).
  The default is 0. `ema_stack` declares 891 (EMA 200). `ma` and
  `ma_ribbon` declare it for their type and lengths: EMA-type kinds as
  above, DEMA and TEMA two and three times that, RMA/SMMA with α = 1/N,
  window kinds their length (`pine::ma_warmup`).
- `generate_chart(format=series)` and `subscribe_chart` raise a non-zero
  `warmup` to the largest `warmup_bars()` of the requested indicators,
  still capped at 1000. `warmup: 0` (or none) stays 0, so a caller asking
  for the bare window still gets it.
- The payload is unchanged: warmup bars are trimmed as before. `has_more`
  compares against the raised `bars + warmup`.
- The web app keeps sending `warmup: 300` and does not change its merge.

## Consequences

**Positive.** EMA 200 joins pages and tails within 0.01% of one long
request (measured on the sample data: the extra step at a seam went from
1.9% to 0.005%). The test `ema_stack_pages_join_without_a_step` holds it
under 0.05%. Charts without long EMAs fetch exactly what they did before.

**Negative.**

- A chart with `ema_stack` fetches up to 891 + `bars` bars per request: a
  tail poll is two Binance requests instead of one, and its push group
  recomputes 893 bars, so it pushes at most every 500 ms instead of 250 ms
  (ADR-0021's rule for windows above 300).
- The 1000 cap still limits user-configured long EMAs: an `ma` of type EMA
  converges to 0.1% only up to length 224 (RMA: 126). Longer ones step
  less than before but still visibly.
- Only built-in indicators declare a need. User scripts
  ([ADR-0025](/chartgen/decisions/0025-user-indicator-registry/)) report 0
  and get the client's warmup.
- Running sums (OBV, CVD, A/D line) and VWAP sessions are unaffected:
  they still differ by a constant or reset at anchors, as ADR-0009 says.

## More Information

- [ADR-0009 — Live tail polling and history paging](/chartgen/decisions/0009-live-tail-polling-and-history-paging/)
- [ADR-0018 — Pine semantics for indicators](/chartgen/decisions/0018-pine-semantics-for-indicators/): SMA-seeded `ta.ema` / `ta.rma`
- [ADR-0021 — Real-time chart push](/chartgen/decisions/0021-realtime-chart-push/)
