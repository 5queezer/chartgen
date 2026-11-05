---
title: "0030 — Chart types klinecharts does not draw"
description: "Baseline, Columns, High-low, Step line and Line with markers are painted by one klinecharts indicator on the candle pane, over a transparent built-in series that keeps the autoscale, crosshair, last price line and data handling."
sidebar:
  order: 30
---

**Status:** Accepted
**Date:** 2026-11-05

## Context and Problem Statement

TradingView offers Baseline, Columns, High-low, Step line and Line with
markers next to candles, bars, line and area. klinecharts 9.8 has six
built-in candle types (`candle_solid`, `candle_stroke`, `candle_up_stroke`,
`candle_down_stroke`, `ohlc`, `area`) and none of these five. The web chart
needs them without losing what the candle pane already does for the built-in
types: price-axis autoscale, the percent and log axes, the crosshair, the last
price line and label, live updates, history paging, bar replay and compare
lines. How should the new types be drawn?

## Decision Drivers

- Every existing feature of the candle pane keeps working for the new types.
- No server or wire change; the types only change how the same bars look
  ([ADR-0003](./0003-web-frontend-stack/)).
- Colors follow the theme and the chart settings
  ([ADR-0019](./0019-chart-settings/)).
- The chart type stays one persisted string, so layouts and share links keep
  working.

## Considered Options

**One indicator on the candle pane over a hidden built-in series (chosen).**
klinecharts keeps drawing one of its own types with fully transparent
colors: `area` for the close-based types (its autoscale spans the closes,
as TradingView's does for them) and `candle_solid` for High-low (it spans
high and low). A registered indicator, `chartgen_series`, with no figures
paints the visible series in its `draw` callback from the bars, mapping
through klinecharts' own `xAxis` and `yAxis`, so pan, zoom, the percent and
log axes and live data need nothing extra. Its negative `zLevel` makes
klinecharts paint it with `destination-over`: beneath the last price line,
the other indicators and the compare lines, where the candles would be.

**Indicator figures (built-in `line`/`bar`/`circle`, or custom ones from
`registerFigure`).** Figure values are per-bar numbers that also drive the
autoscale. Columns from the pane bottom, a step between bars or a fill to a
moving base level are pixel geometry, not values, so they would need custom
figures fed with that geometry anyway, while the figure values skew the
axis.

**Fork or patch klinecharts' candle widget.** The full set of types in one
place, but a patched dependency to carry through every upgrade.

**Render a separate canvas over the chart.** Its own coordinate mapping,
event handling and z-order against klinecharts' canvases, duplicating what
the axes already provide.

## Decision Outcome

Chosen: one indicator over a hidden built-in series. In
`web/src/lib/chartTypes.ts`:

- **Line with markers**: the close line, 2px, plus a dot per bar whose
  radius shrinks with the bar spacing.
- **Step line**: the closes joined horizontally to the next bar, then
  vertically.
- **Baseline**: the close line and its fill to a base level, clipped into
  an up half above and a down half below, with a gradient from 28% to 5%
  opacity towards the base. The base level is the middle of the visible
  close range (TradingView's default of 50%), recomputed every frame, and is
  shown as a dotted line.
- **Columns**: a bar from the pane bottom to the close, in the up or down
  color at 50% opacity by the close against the previous close.
- **High-low**: a body-width bar from low to high in the line color.

The hiding style is applied after the chart-settings styles, so no setting
brings the built-in series back; picking a built-in type again re-applies
its own style. The indicator is created when a custom type is chosen and
removed when a built-in one is. Its `extendData` carries the type and the
resolved up, down and line colors, so a theme or settings change is one
`overrideIndicator`.

## Consequences

**Positive.** Autoscale, percent and log axes, crosshair, the OHLC legend,
the last price line, live ticks, paging, replay and compare work unchanged,
because klinecharts still holds and scales the series. No server change. A
new client-drawn type is one more case in `drawSeries`.

**Negative.** The series is drawn every frame in JavaScript, not by
klinecharts' own views (cheap: only the visible bars). It relies on
klinecharts 9.8 behaviour: indicators with a negative `zLevel` are painted
with `destination-over`, the `area` type autoscales on its value key only,
and the candle pane uses the lowest indicator precision for the price axis
(the indicator declares precision 10 so it never lowers it). The package is
pinned to `~9.8.12`, and the chart-types E2E spec counts each type's pixels,
so an upgrade that changes this fails the suite. The Baseline base level is
not configurable yet.

## More Information

- Related: [ADR-0006](./0006-tradingview-visual-parity/),
  [ADR-0010](./0010-overlay-geometry-rendering/),
  [ADR-0019](./0019-chart-settings/),
  [ADR-0022](./0022-compare-symbols/).
