---
title: "0056 — A png or summary reads its indicators' warmup"
description: "generate_chart in png, summary and both reads the history its indicators need before the bars it shows, at least the web chart's 300 bars, and trims it, so their values match the series and the web chart."
sidebar:
  order: 56
---

**Status:** Accepted
**Date:** 2026-11-09
**Amends:** [ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/) (png, summary and both no longer compute over exactly `bars`) and [ADR-0032](/chartgen/decisions/0032-indicator-aware-warmup/) (the indicators' need now applies to them too, without a `warmup` argument)

## Context and Problem Statement

ADR-0009 gave `warmup` to `format=series` alone: png, summary and both computed their
indicators over exactly `bars` bars. An agent asking for the default 200 bars therefore got
values the web chart never shows (#125, #131):

- EMA 200 and SMA 200 had one value. The image drew no line, but the legend and the summary
  printed that value as if it were the average.
- kalman_volume needs about 285 bars before its first value, so its panel was empty and its
  summary values were `null`, while `format=series` with any warmup had them (ADR-0032 raises
  the warmup to what an indicator declares).
- CCI, RSI and the rest started 14 to 20 bars into the image, and every EMA-seeded value
  differed from the web chart's, which always loads with `warmup: 300`.

The same call should give the same numbers in every format and in the web app.

## Decision Drivers

- One value per bar: png, summary, series and the web chart agree.
- The bars shown stay exactly `bars`, so drawings, signals and indices mean what they did.
- Bounded cost: no more upstream reads than a series call may already make.

## Considered Options

1. **Read the indicators' need before the bars shown, at least the web chart's 300, capped at
   1000, then trim (png, summary and both).**
2. Read only what the indicators declare (`warmup_bars`), without the 300 floor.
3. Keep exactly `bars` and mark unconverged values (`—` in the legend, `null` in the summary).
4. Honour the `warmup` argument in png and summary as series does.

## Decision Outcome

Chosen option: **1**. A png, summary or both call with indicators reads
`max(300, the largest warmup_bars_at of its indicators)` older bars, at most `MAX_WARMUP`
(1000), computes over all of them and trims every result to the `bars` it shows, the same way a
series is trimmed (`trim_panel_result`). Patterns, levels and events are re-based onto the
bars shown, and an index before the first of them is negative, as in a series. Volume and time
profiles that aggregate over their window (`windowed_profile`) are recomputed over the bars
shown, so warmup bars do not leak into them. A call without indicators reads nothing more.
Price-based styles read their fixed source window, as before (ADR-0047). The `warmup` argument
still applies to series alone.

Option 2 leaves indicators that declare no need (CCI, Williams %R, Bollinger Bands) starting
mid-image and slightly different from the web chart. Option 3 hides the problem instead of
fixing it, though the image keeps it as a safety net: a line with a single value shows `—` in
its legend, for a symbol with too little history. Option 4 puts the burden on every agent, and
the defaults stay wrong.

### Consequences

- Good: an image's legend, a summary's `last_value` and the web chart show the same number.
  kalman_volume, EMA 200 and SMA 200 have values on 200 bars.
- Good: the image draws every indicator from its first bar.
- Bad: a call reads up to 1000 more bars. 200 bars with `ema_stack` read 1,091 (two Binance
  requests instead of one), and `chart_patterns` or `auto_fib` read 600 more. Series calls
  could already ask for this much, and `call_cost` (ADR-0052) counts the bars shown, not those
  read, for every format alike. The extra compute is transient: only the trimmed results stay.
- Bad: summary values on the demo series (no ticker) change. It is generated from bar 0, so a
  summary of 30 bars now shows bars 300 to 329 of it. The summary golden was regenerated.
- Neutral: cumulative indicators (OBV, A/D, CVD) start from a different bar, as in the web
  chart. Their level was never meaningful, only its changes.
