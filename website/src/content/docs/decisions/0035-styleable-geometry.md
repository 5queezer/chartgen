---
title: "0035 — Styleable indicator geometry colors"
description: "The Style tab edits the colors and visibility of fills, levels, histogram palettes, profile bars and divergence lines. Overrides are keyed by server color (palettes) or by position with a server-color guard (fills, levels)."
sidebar:
  order: 35
---

**Status:** Accepted
**Date:** 2026-11-05
**Extends:** [ADR-0017](/chartgen/decisions/0017-indicator-plot-styles/) (the per-instance style record)

## Context and Problem Statement

ADR-0017 stores per-plot overrides (color, width, line style, visibility)
for lines and histograms, keyed by plot label. Everything else an
indicator draws had no override: fills (Bollinger, Keltner, the Ichimoku
cloud), horizontal levels (RSI 70/30), volume-profile bars, divergence
lines, and the per-bar colors of a histogram (Volume up/down). TradingView's
Style tab edits all of them. These colors come from the server (ADR-0012,
ADR-0026) and have no stable identity on the wire: a fill has no name, a
level is a bare `y`, and a histogram palette lists its colors in order of
first appearance, so the same color can sit at index 0 in one window and
index 1 in the next.

ADR-0017 already rejected keys that are only a position, because the entry
at a position can change under the override. How do geometry overrides stay
attached to what the user edited?

## Decision Drivers

- An override must never recolor a different thing than the one the user
  edited (the ADR-0017 concern).
- Live ticks, pages, replay and theme switches must keep overrides.
- Stored styles from before this change load unchanged.
- No wire change: the server keeps sending colors as today.

## Considered Options

1. **Position only** (`fills[i]`, `palette[i]`). Simple, but a palette
   reorder swaps Volume's up and down colors, and a changed fill list
   recolors the wrong fill.
2. **Position plus a server-side identity** (a fill / level name or id on
   the wire). Robust, but needs a wire change and server work for every
   indicator.
3. **Palette keyed by server color; fills and levels keyed by position with
   a server-color guard.**

## Decision Outcome

Chosen option: **3**. The override record `style` gains an optional
`geometry` object, and a histogram's plot entry gains `palette`:

- `plots.hist<i>.palette` is `{ "<server css color>": "<user color>" }`.
  A bar takes the user color registered for its server color, whatever the
  palette order. Only colors the user touched are stored. A plain `color`
  (legacy, recolors every bar) still wins; editing a palette color clears
  it. More than 16 entries are rejected on load, and a histogram with more
  than 16 palette colors shows one swatch.
- `geometry.fills[i]` is `{ color?, colorBelow?, visible?, ref? }` and
  `geometry.hlines[i]` is `{ color?, lineStyle?, visible?, ref?, y? }`.
  `ref` is the server color of the fill / level and `y` the level's value
  when the override was saved. The override applies only while the payload's
  entry at index `i` still has that color (and, outside volume profiles,
  whose levels move with every payload, that `y`). Otherwise it is ignored,
  and dropped when the settings dialog opens.
- `geometry.hbars` is `{ color?, vaColor?, pocColor?, visible? }` (bars
  outside the value area, inside it, and the widest bar) and
  `geometry.divlines` is `{ bullColor?, bearColor?, visible? }`. Each has at
  most one instance per indicator, so they need no key.

Overrides are applied at draw time over the server and theme colors, so live
ticks and theme switches keep them, and colors the user did not touch keep
following the server. Fills the user hid do not feed the axis range.
Layouts store and compare the whole style, as for plot styles (ADR-0017).
Share links still carry no styles.

### Consequences

- Good: palette overrides survive reordering, symbol changes and replay.
- Good: a stale fill or level override disappears instead of recoloring
  another one. Old styles load unchanged: the new fields are optional.
- Bad: a level override on a chart whose server color or `y` changes (an
  RSI level moved by a parameter) is lost, and must be set again.
- Bad: the guard compares wire strings, so a server that reformats its
  colors orphans existing overrides.
