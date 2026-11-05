---
title: "0022 — Compare symbols"
description: "Compare lines are other symbols' closes as-of joined onto the main bars and drawn as one klinecharts indicator on the candle pane, rebased so the percent axis normalises them correctly; the list is persisted and part of a layout."
sidebar:
  order: 22
---

**Status:** Accepted
**Date:** 2026-11-05
**Amends:** [ADR-0015](./0015-named-layouts/) (a layout's state gains `compare`; layouts saved before it load with none)

## Context and Problem Statement

TradingView's "Compare symbol" draws other symbols as lines over the main
chart and switches the price axis to percent, so series of different price
scale can be compared from a common start. The web chart needs the same:
several compare symbols, live, removable from the legend, persisted. Two
questions have more than one answer: how the lines are normalised on the
klinecharts percent axis, and where the state lives.

## Decision Drivers

- No indicator maths in the browser ([ADR-0003](./0003-web-frontend-stack/)):
  alignment and display normalisation only.
- The lines must read the same percent values as the axis shows for the
  main series.
- Compare data must follow the main series: same loaded range, live tail,
  paging, timeframe changes.
- The persisted shape grows, so it must be validated and capped.
- No Rust or wire changes.

## Considered Options

**One klinecharts indicator on the candle pane, values rebased to the main
price space (chosen).** klinecharts' percent axis maps every value drawn on
a pane, indicators included, through `(v - c0) / c0 * 100`, where `c0` is the
*main* series' close at the first visible bar (`getVisibleFirstData`, the bar
at `visibleRange.from`). Raw closes of another symbol would be divided by
the wrong base. The indicator's `calc` therefore scales each compare line by
`mainClose[f] / compareClose[a]`, where `f` is the first visible bar and `a`
the first bar from `f` where the line has a value. The line then starts at
0% at `a` (at `f` for a symbol that has data from the start) and the axis
reads the compare symbol's own percent change from there, which is also what
the legend reports; the lines share the main series' y-axis and autoscale.
The first visible bar changes on scroll and zoom, so the Chart re-anchors the
indicator (an `extendData` override) once per animation frame, recalculating
only a window of the visible bars plus a margin. Outside percent mode the
lines are anchored at the first loaded bar and nothing runs per frame.

**A second chart pane or overlay canvas with its own scale.** Independent
normalisation, but it must replicate the axis, crosshair, autoscale and
pane layout that the candle-pane indicator gets for free.

**Normalise on the server** (`format=series` returning percent). A new wire
field and Rust change for what is a display transform; the server cannot
know the first visible bar.

## Decision Outcome

Chosen: the first option.

- **Data.** For each compare symbol the web app calls
  `generate_chart(format=series, indicators: [], chart_style: candles)`
  with `bars` equal to the main series' loaded bars (at most 5000), tails
  it every poll period of that symbol's own market (fast for 24/7 symbols,
  slow for session markets), merging by timestamp,
  and pages older history (`end_time` = its oldest bar, 1000 bars) until it
  reaches the main series' first bar. A timeframe change drops and refetches
  everything; a gap after sleep restarts that symbol. Only the main series'
  bars sized for the current timeframe count, so a kept previous chart never
  sizes the loads.
- **Alignment.** As-of join on the main bar's end: each main bar takes the
  last compare close whose bar opens before the main bar ends
  (`t < t_main + step`); bars before the compare symbol's first bar are empty.
  Joining on the open would lag one session on 1D and 1W, where Binance bars
  open at 00:00 UTC and Yahoo's at the exchange open.
- **Scale.** The first compare symbol switches the axis to percent and
  remembers the mode it replaced; removing the last one restores it, unless the
  user has picked another mode in the meantime. A layout switch drops the
  remembered mode. The
  `%`, `log` and normal buttons still work in between (the lines are in the
  main price space, so they are meaningful in every mode).
- **Persistence.** `chartgen.layout.compare` (array of symbols, normalised,
  de-duplicated, at most 5), `chartgen.layout.compareSlots` (the color slot
  of each) and `chartgen.layout.compareRestore` (the scale mode to return
  to). All are validated on load; invalid slots are rebuilt in order. A layout snapshot gains a
  `compare` array, validated the same way; layouts without it have none.
  Applying a layout sets its scale mode and its compare list separately.
- Each compare symbol keeps its palette slot until removed, so removing one
  does not recolor the others. Making a compare symbol the main symbol
  removes it from the list.

## Consequences

**Positive.** Reuses the axis, crosshair and autoscale of the candle pane;
percent values on the axis and in the legend agree. No server change.

**Negative.** Up to five extra `generate_chart` requests per poll while
compare symbols are active. Normalisation depends on klinecharts using
`getVisibleFirstData` for the percent axis. That is verified for klinecharts
9.8.12, which `package.json` pins (`~9.8.12`), and `klinechartsPercent.test.ts`
checks it against the installed source, so an upgrade that changes it fails
the unit tests.

## More Information

- Related: [ADR-0003](./0003-web-frontend-stack/),
  [ADR-0009](./0009-live-tail-polling-and-history-paging/),
  [ADR-0015](./0015-named-layouts/).
