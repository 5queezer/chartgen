---
title: "0017 — Indicator plot styles"
description: "Persist client-only per-plot color, width, line style and visibility on the stored indicator entry and apply them through klinecharts figure styles."
sidebar:
  order: 17
---

**Status:** Accepted
**Date:** 2026-11-05
**Amends:** [ADR-0013](./0013-indicator-visibility-and-instances/) (stored shape only; 0013 stays in force and is not superseded)

## Context and Problem Statement

The settings dialog has an Inputs tab only. TradingView's Style tab lets the
user change color, line width, line style and visibility of each plotted line
or histogram. The server sends no line colors for lines and histograms, so
the web client assigns them from a palette rotation, and the legend must show
the same color as the chart. Where do the overrides live, how are plots
identified, and how are they applied without refetching?

## Decision Drivers

- Style is presentation only: it must not reach the server, change the query
  key, or cause a refetch.
- Existing persisted arrays must keep loading unchanged, and corrupt
  or hand-edited storage must not break rendering.
- Plot labels come from the server and can change with parameters or repeat.
- No Rust or wire-format changes.

## Considered Options

**Optional `style` on the stored entry (chosen).**
`{ name, ...params, hidden?, style?: { plots: { [plotKey]: { color?, width?, lineStyle?, visible? } } } }`.
Like `hidden`, it is dropped by `toWire`, so the wire entry and the query key
are unchanged. Entries without `style` load as before.

**Separate storage key for styles.** Keeps entries pure, but the style must be
kept in sync when an indicator is removed or re-added and needs a second
load/persist path.

**Style by plot index only.** Stable in the stored data, but an index follows
whichever plot lands there: with `cipher_b`'s `wt_show` off, RSI moves to
`line0` and would inherit WT1's hidden state or color.

## Decision Outcome

Chosen: the optional `style` field on the stored entry.

- **Plot key.** A line is keyed by its server label, `l:<label>`, when the
  label is unique among the indicator's lines. This survives plots appearing
  or disappearing before it. Lines with a missing or repeated label, and all
  histograms (which have no label), fall back to the index key `line<i>` /
  `hist<i>`. An index-keyed line entry also stores the `label` it was saved
  for and is ignored once the line at that index has another label.
  Legacy `line<i>` entries without a label (from earlier builds of this
  branch) still apply to the line at that index and are re-keyed when the
  Style tab is opened and saved.
- **Validation on load** (`sanitizeStyle`): colors must be `#rgb`, `#rrggbb`,
  `#rrggbbaa` or `rgba(r, g, b, a)`; width an integer 1-4; line style one of
  `solid`, `dashed`, `dotted`; visibility a boolean; plot keys must match
  `line<n>`, `hist<n>` or `l:<label>`; labels are at most 64 characters; `rgba()`
channels are 0-255. Invalid fields are dropped, and a style with nothing
  valid left is removed.
- **Application:** the style is passed to klinecharts as the indicator's
  `extendData`. The figure `styles` callback reads it for color, `size`,
  `style` and `dashedValue` (dotted is a short dash pattern), and `calc`
  returns null for hidden plots, so nothing is drawn and the y-axis ignores
  them. Applying a change is one `overrideIndicator`, with no new data and no
  request. Histograms only take a color and visibility.
- **Legend:** the pane legend uses the same resolved colors and omits hidden
  plots; the indicator's controls stay.
- **Settings dialog:** Kobalte Tabs "Inputs" and "Style". Defaults resets the
  tab being shown; OK applies both in one store write. A style-only OK leaves
  the params entry untouched, so it never changes the wire entry. For a hidden
  indicator the Style tab only says to show it first, since it has no plots in
  the payload.
- **Fills** (and bands, hbars) are not plots: they stay visible when the edge
  lines they connect are hidden. This matches TradingView, where fills are
  separate Style items; they are not styleable here yet.

## Consequences

**Positive.** Backward-compatible persistence; style edits are instant and
free of server traffic; storage is validated, so a bad value cannot reach the
canvas.

**Negative.** Overrides of a plot that disappears (for example after a
parameter change that removes a line) stay stored but are unused. The
default colors still come from the client palette rotation; if the server
later sends plot colors they become the defaults and overrides still win.
Geometry colors are not styleable here.

## More Information

- Related: [ADR-0006](./0006-tradingview-visual-parity/),
  [ADR-0012](./0012-geometry-colors-on-the-wire/),
  [ADR-0013](./0013-indicator-visibility-and-instances/).
