---
title: "0010 — Overlay geometry rendering via indicator draw callbacks"
description: "Render fills, hbars and divlines in the web frontend through klinecharts indicator draw callbacks fed by a pure, unit-tested geometry module."
sidebar:
  order: 10
---

**Status:** Accepted
**Date:** 2026-11-04

## Context and Problem Statement

[ADR-0007](/chartgen/decisions/0007-complete-series-serialization/) put
`fills`, `hbars` and `divlines` on the `format=series` wire. The web
frontend ignored them and logged a `console.debug` placeholder instead.
The result was that Bollinger/Keltner bands had no fill, Ichimoku had no
cloud, and volume-profile indicators were skipped completely, because they
emit no lines or histogram. Cipher B divergences were also missing. All
of these are part of the TradingView look the frontend aims for
([ADR-0006](/chartgen/decisions/0006-tradingview-visual-parity/)).

klinecharts 9 has no figure type for a band between two series, for
bars anchored to the pane edge, or for a segment between two arbitrary
bars. We had to choose where this geometry is drawn and how it stays in
sync with pan, zoom and y-axis rescale.

## Decision Drivers

- Geometry must move with the chart: pan, zoom, pane resize and
  y-axis rescale, with no separate synchronization code.
- Keep ADR-0003's rule that all indicator math stays on the server.
  The browser only maps payload values to pixels.
- The geometry maths (gaps, crossings, clipping, edge anchoring) must
  be testable without a browser.
- `Chart.tsx` changes stay small, because other work touches the same file.

## Considered Options

**DOM / separate canvas overlay on top of the chart.** Full control,
but it has to subscribe to every klinecharts scroll, zoom and resize
event and repeat the axis transforms. It also falls behind by a frame
during interaction. Rejected.

**klinecharts overlays (`registerOverlay` + `createOverlay`).** Overlays
are meant for user drawings anchored to points. One overlay per
fill polygon, profile row or divergence segment would mean hundreds of
objects to create and remove on every payload. Overlays can also be
selected and dragged, which these shapes must not be. Rejected.

**Custom figures (`registerFigure`) emitted per bar from `calc`.**
Figures are drawn bar by bar, so a continuous polygon or a pane-anchored
bar does not fit that model. Rejected.

**`draw` callback on the generic chartgen indicator template.** The
template's `draw` callback runs on every frame with the live `xAxis`,
`yAxis`, `visibleRange` and pane `bounding`, and reads the indicator's
geometry from a store that `Chart.tsx` keeps. Chosen.

## Decision Outcome

Chosen option: **indicator `draw` callbacks plus a pure geometry
module.**

- `web/src/components/geometry.ts` has no DOM or klinecharts
  dependency. It turns payload channels and a `{ x(index), y(value) }`
  mapper into pixel primitives:
  - fill polygons, split at `null` gaps and at crossings, with the
    crossing point interpolated. Each polygon is clipped to the visible
    range plus one bar.
  - hbar rects. `width` and `offset` are fractions of the pane width,
    measured from the edge that `left` selects.
  - divline segments, culled when they fall outside the visible range.

  The visible range is klinecharts' own `VisibleRange`, which is already
  clamped to the data. Vitest covers the module (`npm test`).
- **Data space.** `seriesToKLines` drops bars with `t: null`, so a
  payload index is not always a klinecharts data index.
  `payloadToDataIndex` builds the remap, and `toDataSpace` compacts the
  fill arrays and remaps divline endpoints once per payload. After that,
  everything is drawn in data indices.
- **Geometry store, not `extendData`.** The data effect writes each
  indicator's data-space geometry into a module-level
  `Map<regName, SeriesIndicator>` before `applyNewData`. The draw
  callback reads it from there. `applyNewData` already recalculates and
  repaints, so a new payload costs nothing extra. Passing the geometry
  through `overrideIndicator({ extendData })` would re-run calc and layout
  for every indicator on every payload, and live tail polling sends one
  every few seconds.
- **Paint order and z-order.** `ensureRegistered` attaches
  `draw: drawGeometry(name)`. The callback paints fills first, with
  `destination-over`, so they land behind the candles already on the pane
  canvas, as TradingView bands do. The grid is painted afterwards, on top.
  With `destination-over` each paint lands beneath the previous one, so
  fills are iterated in reverse to keep the server's paint order. The
  callback then paints hbars and divlines and returns `false`, so
  klinecharts still draws the line and bar figures.

  **Dependency:** this relies on nothing opaque being painted on the pane
  canvas before the indicator's `draw`. klinecharts 9 paints no canvas
  background, and the chart background comes from CSS. If a future change
  (a klinecharts upgrade or a background style) paints one, every fill
  would silently disappear beneath it.
  `web/tests/e2e/overlay-geometry.spec.ts` guards this. It loads
  Bollinger Bands and asserts that band-fill pixels are present on the
  canvas and differ from the page background. With an opaque layer
  injected, the test fails.
- **Y-axis.** klinecharts auto-scales over figure values only. Two
  typeless figures (`range_lo`, `range_hi`) draw nothing, but carry
  `valueExtents` into the scale: the min and max of every fill edge per
  bar, plus the payload's `y_range` for oscillator panes. That keeps
  Kalman OB/OS zones and RSI 0–100 on-pane and gives geometry-only panes
  a real axis. Like klinecharts' `minValue`/`maxValue`, `y_range` widens
  the range and does not clamp it. We did not use those two fields:
  klinecharts ignores them when no figure has a value, and its
  `overrideIndicator` path writes `maxValue` through `setMinValue`.
  Overlays ignore `y_range`, so the price axis is never pinned.
- An indicator that emits only geometry (e.g. `volume_profile`) is no
  longer skipped by the "no lines, no histogram" check.
- **Colors.** The wire drops server-side colors (ADR-0007), so the
  geometry module's theme helpers mirror the Rust palettes per indicator.
  The Ichimoku cloud is green or red by the sign of Senkou A − Senkou B.
  Profile rows are tiered POC (widest row), value area (between the
  VAH and VAL hlines) and the rest. Divergences are classified bull or
  bear from their slope and pane: in an oscillator a bearish divergence
  joins lower highs, and in the price pane it joins higher highs. If the
  wire later carries colors, only these helpers change.

## Consequences

**Positive.** Every indicator that emits fills, hbars or divlines now
renders, and the geometry follows every chart interaction with no extra
code. The edge cases (gaps, crossings, single points, clipping, scaling)
are unit-tested.

**Negative.** Geometry is recomputed on every frame (O(visible bars)
per fill). This is negligible at chartgen's bar counts, but it scales
with zoom-out. The client-side palette can drift from the Rust colors.
The VPVR `split_up_down` mode cannot be colored up/down, because an
up-only row and a down-only row look identical on the wire.

**Neutral.** The geometry store is module-level, like klinecharts' own
indicator registry. That is fine while the app mounts a single `Chart`,
but a second chart instance would need the store keyed per chart.

## More Information

- [ADR-0006 — TradingView visual parity](/chartgen/decisions/0006-tradingview-visual-parity/)
- [ADR-0007 — Complete series serialization](/chartgen/decisions/0007-complete-series-serialization/)
- klinecharts 9 `IndicatorTemplate.draw` / `IndicatorDrawParams`
  (`web/node_modules/klinecharts/dist/index.d.ts`)
