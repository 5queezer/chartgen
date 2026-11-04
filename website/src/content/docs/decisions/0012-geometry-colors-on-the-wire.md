---
title: "0012 — Geometry colors on the wire"
description: "Serialize the server renderer's colors for fills, hbars, divlines and hlines in format=series, and draw hlines in the web chart."
sidebar:
  order: 12
---

**Status:** Accepted
**Date:** 2026-11-04
**Supersedes:** the "client themes per indicator" part of
[ADR-0007](/chartgen/decisions/0007-complete-series-serialization/)

## Context and Problem Statement

[ADR-0007](/chartgen/decisions/0007-complete-series-serialization/) put
`fills`, `hbars` and `divlines` on the `format=series` wire but dropped
their colors, on the grounds that "the client themes per indicator".
[ADR-0010](/chartgen/decisions/0010-overlay-geometry-rendering/) then
implemented that theming in `web/src/components/geometry.ts`.

In practice the client-side themes drift from the server. The PNG
renderer (`src/renderer.rs`) uses the colors each indicator sets, and
the web heuristics guess them back from the indicator name and fill
index: Cipher B colors are wrong when toggles such as `wt_show=false`
change which fills are emitted, volume-profile tiers are a POC/value-area
guess, divergence colors come from slope, and Ichimoku gets a
TradingView-style green/red cloud while the PNG used a single blue. Separately, `hlines`
(RSI 70/30, profile POC/VAH/VAL) are on the wire but the web chart never
drew them.

## Decision Drivers

- The web chart should look like the PNG; the server's colors are the
  source of truth.
- Additive wire change: existing fields stay, older servers must still
  work with a newer web build.
- One helper on the Rust side, no per-indicator color tables on the web.

## Considered Options

**Keep client theming and fix the heuristics.** Needs every Rust palette
change mirrored by hand and cannot know which optional fills were emitted.

**Serialize the color of each fill, hbar, divline and hline (chosen).**

## Decision Outcome

Chosen option: serialize colors. Every item of `fills`, `hbars`,
`divlines` and `hlines` in `format=series` gets a `color` field, a CSS
string `"rgba(r,g,b,a)"` produced by a single helper from the
`RGBAColor` the indicator set. `divlines` already carry `dashed`.
A fill may also carry an optional `color_below`, used where `y1 < y2`
(`color` applies where `y1 >= y2`). It is omitted when absent, so
existing fills behave as before. Ichimoku uses it for the TradingView
cloud, green `rgba(76,175,80,0.2)` above and red `rgba(239,83,80,0.2)`
below. The PNG renderer picks the color per bar and the web chart reuses
its existing split-at-crossings polygons. `lines` and `histogram` are unchanged and still carry no color; the web
keeps its palette rotation for them.

On the web, the wire color wins. `geometry.ts` validates it (an
unparsable string would make a canvas silently keep its previous style)
and falls back to the existing theme heuristics when it is absent, so a
newer web build still works against an older server.

`hlines` are now drawn by the indicator `draw` callback as full-width 1px
solid lines in the wire color, clipped to the pane, as the PNG renderer
does. The server does not mark any hline as dashed, so none is.

Y-axis rule: hline levels feed the same invisible `range_lo`/`range_hi`
figures as fill edges in oscillator panes. Overlay indicators on the
price pane do not include their hlines, because a far-away level such as
a distant volume-profile node would squash the candles; those lines are
simply not drawn when they fall outside the visible range.

### Consequences

- Good: web overlay geometry matches the PNG, including toggled-off
  Cipher B fills; the Cipher B index and Kalman heuristics remain only as
  a fallback.
- Good: RSI 70/30 and similar reference lines appear in the web chart.
- Bad: payload grows by one short string per fill, hbar, divline and
  hline (volume profiles emit a few hundred hbars).
- Bad: web and server share one more piece of the wire contract; the
  Zod schema and `docs/reference/mcp.md` must follow any change.

## Links

- Supersedes the color decision in
  [ADR-0007](/chartgen/decisions/0007-complete-series-serialization/)
- Drawing approach:
  [ADR-0010](/chartgen/decisions/0010-overlay-geometry-rendering/)
