---
title: "0026 — Line and histogram colors on the wire"
description: "Serialize the server renderer's line colors and widths, per-bar histogram colors (as a palette plus indices) and histogram labels in format=series, and draw them in the web chart."
sidebar:
  order: 26
---

**Status:** Accepted
**Date:** 2026-11-05
**Amends:** [ADR-0012](/chartgen/decisions/0012-geometry-colors-on-the-wire/)

## Context and Problem Statement

[ADR-0012](/chartgen/decisions/0012-geometry-colors-on-the-wire/) put the
renderer's colors on the `format=series` wire for fills, hbars, divlines
and hlines. It left `lines` and `histogram` without color, so the web chart
colored them from a rotating palette keyed by indicator name. That palette
cannot express what the PNG draws. Volume bars are green or red by candle
direction, MACD's histogram is green or red by sign, and the Awesome
Oscillator's by slope. In the web chart all three were one flat palette
color, and EMA lines did not match the PNG either. Histograms also had no
label, so the legend and Style tab showed `hist0`.

## Decision Drivers

- The web chart should look like the PNG. The server's colors are the
  source of truth (as in ADR-0012).
- Histogram colors are per bar. They must stay aligned with `values`
  through warmup trimming, push tails, tail merges, history pages and
  replay slices.
- Keep the payload small. A 500-bar Volume pane has two distinct colors.
- Additive change: older servers and older web builds keep working.

## Considered Options

**Per-bar CSS strings (`colors: ["rgba(…)", …]`).** This is the simplest
option. It repeats a ~20-byte string per bar, about 10 KB per 500-bar
histogram.

**Palette plus indices (chosen).** `palette` lists each distinct color
once and `color_idx[i]` indexes it. Merging two responses remaps indices,
because each response builds its own palette.

**Client rules per indicator (up/down by candle, sign, slope).** This
mirrors Rust logic by hand, the same drift ADR-0012 removed.

## Decision Outcome

Chosen option: palette plus indices.

- Each item of `lines` gets `color` (CSS `"rgba(r,g,b,a)"` from the same
  `css_rgba` helper as ADR-0012) and `width` (the PNG stroke width in px).
- Each item of `histogram` gets:
  - `label`. `Bars` carry no label of their own, so the panel label is
    used (`"Volume"`, `"AO"`). When a line already has that name (MACD's
    `"MACD"` line) the label is `"Histogram"`. With several histograms the
    labels are numbered (`"KVF 1"`, `"KVF 2"`).
  - `palette`, the distinct colors in order of first appearance.
  - `color_idx`, one entry per value, aligned with `values`: an index into
    `palette`, or `null` where the value is null (warmup) or the bar has no
    color.
- Each item of `signals` gets the dot's `color`.

Warmup trimming already cuts `Bars.colors` with `y`, and push tails use
the same serializer (`series_tail` → `series_json`), so pushed and polled
tails stay identical.

On the web the resolution order is **user override > server color >
palette**. The palette remains only as a fallback for older servers. For
histograms the user's Style-tab color (ADR-0017) replaces every bar's
color. Otherwise each bar takes its server color. The klinecharts `bar`
figure's `styles(data, indicator, defaultStyles)` callback reads that color
from the bar's `KLineData`, where it is stashed next to the value. Lines
take the server width unless the user set one. The legend and the Data
Window show a histogram's value in the color of the bar they read.
`mergeOlder`, `mergeTail` and replay slicing carry `color_idx` like
`values`, remapping incoming indices into the current palette and adding
any color it lacks.

### Consequences

- Good: Volume, MACD, AO and every other histogram match the PNG, and line
  colors and widths do too. Legend and Style tab labels are real names.
- Good: a 500-bar histogram costs about 500 small integers plus a
  two-entry palette.
- Bad: merges must remap palette indices. This is a small helper
  (`colorIdxInto`) shared by paging, tails and replay.
- Bad: one more piece of wire contract. The Zod schema and
  `reference/mcp.md` must follow any change.
- Neutral: stored histogram overrides keep their index keys (`hist0`),
  so saved styles still apply.

## Links

- Amends [ADR-0012](/chartgen/decisions/0012-geometry-colors-on-the-wire/)
  (which left lines and histograms uncolored)
- Plot overrides: [ADR-0017](/chartgen/decisions/0017-indicator-plot-styles/)
- Paging and tails: [ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/),
  push: [ADR-0021](/chartgen/decisions/0021-realtime-chart-push/)
