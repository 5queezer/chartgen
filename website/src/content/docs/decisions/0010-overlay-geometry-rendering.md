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

**`draw` callback on the generic chartgen indicator template.** Each
indicator instance carries its payload entry as `extendData`. The
template's `draw` callback runs on every frame with the live `xAxis`,
`yAxis`, `visibleRange` and pane `bounding`. Chosen.

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

  Vitest covers the module (`npm test`).
- `ensureRegistered` attaches `draw: drawGeometry(name)`. The callback
  paints fills first, with `destination-over` so they sit behind grid and
  candles like TradingView bands. It then paints hbars and divlines, and
  returns `false` so klinecharts still draws the line and bar figures.
- `createIndicator` / `overrideIndicator` pass the payload entry as
  `extendData`, so each new payload replaces the geometry.
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

**Neutral.** The geometry assumes payload bar index == klinecharts data
index. That holds whenever bars carry timestamps, which every live
source does. `seriesToKLines` only drops bars with `t: null` (synthetic
sample data).

## More Information

- [ADR-0006 — TradingView visual parity](/chartgen/decisions/0006-tradingview-visual-parity/)
- [ADR-0007 — Complete series serialization](/chartgen/decisions/0007-complete-series-serialization/)
- klinecharts 9 `IndicatorTemplate.draw` / `IndicatorDrawParams`
  (`web/node_modules/klinecharts/dist/index.d.ts`)
