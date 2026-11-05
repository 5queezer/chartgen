---
title: "0033 — Indicator projection past the last bar"
description: "Series indicators carry an optional `future` block with the values they plot after the last bar, so the web chart draws the Ichimoku cloud ahead of price as TradingView does."
sidebar:
  order: 33
---

**Status:** Accepted
**Date:** 2026-11-05
**Extends:** [ADR-0007](/chartgen/decisions/0007-complete-series-serialization/) (the series payload), [ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/) (tail merging)

## Context and Problem Statement

TradingView plots Ichimoku's leading spans with `offset = displacement - 1`,
so the cloud runs 25 bars (at the default 26) past the last bar. Every
other chart reads it there: where the cloud will be is the point of the
indicator. chartgen computes the spans the same way, but the series payload
carries one value per bar, aligned to the bars. The values that land after
the last bar have no slot and were dropped, so the web cloud stopped at the
last candle.

The payload needs room for values past the last bar. The values in the
existing arrays must keep their meaning: clients index them by bar, and
the merge code (ADR-0009) checks that every array has one value per bar.

## Decision Drivers

- Old clients must keep working: they index every per-bar array by bar and
  ignore fields they do not know.
- Live tails and pushes (ADR-0021) must move the projection with the
  forming bar. Its spans change the newest projected value.
- The indicator math stays in the Rust engine (ADR-0003). The web app only
  draws.

## Considered Options

1. **Longer arrays.** Lines and fills of a projecting indicator get
   `bars + n` values, and a `projection_bars: n` says how many lie past the
   last bar. One array per plot, but every existing client misreads it (the
   merge rejects the shape, and a last-value reader takes a projected value
   for the last bar's).
2. **A separate `future` block per indicator.** The per-bar arrays stay as
   they are. Next to them, `future` lists the projected values of each line
   and fill that continues, for the bars after the last one.
3. **Placeholder bars.** The server appends `n` empty bars with future
   timestamps. That breaks the OHLC contract (`null` closes), the last
   price, alerts and the live merge, which matches bars by time.

## Decision Outcome

Chosen: **option 2.**

```json
"ichimoku": {
  "lines": [ ... ], "fills": [ ... ],
  "future": {
    "bars": 25,
    "t": [1700006400, 1700020800, ...],
    "lines": [{ "index": 2, "values": [...] }, { "index": 3, "values": [...] }],
    "fills": [{ "index": 0, "y1": [...], "y2": [...] }]
  }
}
```

- `bars` is the number of projected bars, at most 500 (Pine's limit on a
  plot `offset`). `lines[k].index` and
  `fills[k].index` name the plot they continue. Each array holds `bars`
  values: value `j` belongs to the `j`-th bar after the last one. NaN is
  `null`, as everywhere in the payload.
- `t` holds those bars' open times (`data::future_bar_times`). They step
  on from the last bar by the interval; monthly and quarterly bars step by
  calendar months. Daily and longer bars skip Saturday and Sunday when no
  loaded bar falls on a weekend (a stock, not crypto). `t` is absent when
  the last bar has no timestamp.
- Engine: `Indicator::projection(&data) -> Option<Projection>`, `None` by
  default. Ichimoku returns the Senkou A and B spans of its last
  `displacement - 1` bars, which are lines 2 and 3 and fill 0. Its new
  `warmup_bars()` (the longest window plus `displacement - 1`, 77 at the
  defaults) makes a tail's spans, and so its projection, match one long
  request (ADR-0032).
- Every `format=series` response carries the block: full loads, tails,
  pushes and history pages. A page's projection starts after that page's
  last bar. The web merge takes a tail's `future` (its last bar is ours)
  and ignores a page's.
- Web: the chart sets klinecharts' right offset to `bars + 3` bars while
  something projects, as TradingView widens its right margin (80 px
  otherwise), but to at most half the pane. klinecharts stores that offset
  in pixels and returns to it on `scrollToRealTime`, so the chart refits it
  at the current bar width on every full load, End, Reset and date-range
  fit. A wheel or key zoom keeps the margin in bars and needs no refit. The indicator's `draw` callback paints the projected cloud
  and span lines from the last bar on, at `xAxis.convertToPixel(index)`
  for indices past the data. Over that area the crosshair uses
  klinecharts' unclamped `realDataIndex`. The legend and Data Window then
  show the projected spans at the future bar's time, with no OHLC. The
  last bar's y-axis range includes the projected cloud. Bar replay shows
  the projection seen from the cursor: the plotted spans after it, then
  the loaded projection. The leading spans are causal, so this is exact.

## Consequences

**Positive.** The web chart draws the cloud ahead of price like
TradingView's, and it updates with every tick. MCP consumers get the
projected spans with timestamps. Old clients parse the payload unchanged.

**Negative.**

- A second channel to keep in step. Code that slices or merges per-bar
  arrays (merge, replay) must also handle `future`.
- Exchange sessions and holidays are not modelled: intraday stock bars
  step through closed hours and daily bars through holidays, which
  TradingView skips. That is deferred until bars carry their session.
- At a wide zoom the half-pane cap leaves the end of a long projection off
  screen; scrolling left shows it.
- klinecharts draws no x-axis ticks or crosshair time label past the last
  bar. The future time shows only in the legend and the Data Window.
- The PNG renderer still ends at the last bar. Only the web chart draws
  the projection.
- Each response with Ichimoku carries 2 × 2 × 25 more values (lines and
  fill). The fill repeats the line values, as the per-bar fill arrays do.

## More Information

- [ADR-0007 — Complete series serialization](/chartgen/decisions/0007-complete-series-serialization/)
- [ADR-0009 — Live tail polling and history paging](/chartgen/decisions/0009-live-tail-polling-and-history-paging/)
- [ADR-0010 — Overlay geometry rendering](/chartgen/decisions/0010-overlay-geometry-rendering/)
- [ADR-0021 — Real-time chart push](/chartgen/decisions/0021-realtime-chart-push/)
- [ADR-0032 — Indicator-aware warmup](/chartgen/decisions/0032-indicator-aware-warmup/)
