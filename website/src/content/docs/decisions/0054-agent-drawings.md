---
title: "0054 — An agent draws with the web app's tools"
description: "generate_chart takes drawings in the web app's own tools and record shape, anchored to real bars and checked all together; inspect_chart and list_drawing_tools are read-only helpers, and a share link's draw= opens the result as normal drawings."
sidebar:
  order: 54
---

**Status:** Accepted
**Date:** 2026-11-08
**Amends:** [ADR-0029](/chartgen/decisions/0029-shareable-chart-links/) (a link may carry drawings of its symbol, `draw`)

## Context and Problem Statement

An AI agent using chartgen's MCP could render indicators but not annotate a chart: no trendline,
support zone, fib or trade idea. The web app has 18 drawing tools with a stored record shape
(`DrawingRecord`, points as `{timestamp, value}`), and the server had no drawing model at all.

What does an agent need to draw on a chart and know that it drew what it meant? Language models
miscalculate coordinates, guess syntax that is not documented, and read images loosely. A design that
lets them place points in prices and pixels they compute themselves fails quietly.

## Decision Drivers

- One model for a person and an agent: the agent uses the web app's tools and records, nothing
  agent-only, so what it draws a person can open and edit.
- Schema from code: the tool list, point counts and style fields come from one table that the MCP
  serves and the web app is tested against.
- Anchors and read-only inspection instead of computed coordinates.
- Verify and roll back: a call draws all of its drawings or none, and says exactly what it drew.
- A bridge to the web app: the result opens as normal drawings.

## Considered Options

1. **`drawings` on `generate_chart`, anchored to bars, checked all together; `inspect_chart`,
   `list_drawing_tools` and a `draw` share parameter.**
2. A drawing session: `create_drawing`, `update_drawing`, `delete_drawing` on a stored chart, and
   `render_chart` to see it.
3. Draw what is valid and report the rest (partial success).
4. An agent-only vocabulary (`support_zone`, `trendline_between_lows`) translated to the web app's
   tools on the server.
5. Points in bar indices or pixels.

## Decision Outcome

Chosen: option 1.

- **Contract.** `generate_chart` takes `drawings`, at most 50: `{tool, points, style?, text?}`. `tool`
  is one of the web app's `DRAWING_NAMES`; each takes exactly its points. A point is
  `{time, price}` or an anchor `{time, snap}` with `snap` one of `open`, `high`, `low`, `close`. A time is
  unix seconds (below 1e11 in magnitude), unix ms, or ISO 8601 (UTC unless it has an offset). It
  resolves to the bar containing it, or past the last bar to that bar's grid, rounded to a bar, as the
  web app places future points. `style` takes the fields the web app's style bar sets on that tool
  (`color`, `width` 1-4, `dash`, `fill` on the filled tools, `fontSize` on text) with the same color
  forms; `text` is cgText's only, at most 200 characters.
- **All or nothing.** The shapes are checked before any bars are read, the times once they are in. Any
  problem fails the call with a tool error listing each bad drawing by index, with a reason and, where
  useful, a hint ("time … is before the first bar of the chart, …; load more history or pick a later
  time"). Nothing is drawn by half.
- **What was drawn.** On success the answer adds a JSON text item: each drawing with its resolved points
  (ISO time, timestamp in ms, price, bar index, snap), `warnings` for what the image cuts off (a price
  outside the visible price range, a point beyond the right edge), and `open_url`.
- **The PNG draws all 18 tools** with the web app's dark-theme colors and labels: fib levels with
  prices, filled zones with alpha, measure labels, long/short boxes with target, stop and ratio. The
  image extends past the last bar for future points, up to half the bars shown.
- **`inspect_chart`** is read-only: first and last bar, bar count, highest high and lowest low, last
  close, the visible price range, and the newest 12 swing highs and lows (zigzag pivots with
  `swing_depth` bars on each side, deviation 0) with time, price and the `snap` that anchors to them.
  It costs `bars × 2` of the call budget (at least 1000), bars capped at 1000
  ([ADR-0052](/chartgen/decisions/0052-indicator-cost/)). Drawings add nothing to a chart's cost: at
  most 50, they are small against its bars and image.
- **`list_drawing_tools`** serves `drawings::TOOLS`: name, label, what each point is, the style fields
  and whether it takes text. `web/src/generated/drawing-tools.json` is a copy, kept equal by a Rust
  test, and the web app's tests check its names and fills against `DRAWING_NAMES` and `FILLED`, and
  each tool's klinecharts template against the point counts.
- **`draw` share parameter.** `tool(t:p;t:p;key:value)` items joined by commas: each point is unix
  seconds (decimals allowed) and a price, then `color`, `fill`, `width`, `dash`, `fontSize` and cgText's
  `text`, written as `ind` writes values. At most 50 items and 8192 characters. Decoding never throws:
  an item with an unknown tool, the wrong point count or a malformed point is dropped, and style fields
  pass the stored drawings' checks. The server builds `open_url` from `CHARTGEN_BASE_URL` with `symbol`,
  `interval`, `type` (candles or Heikin Ashi only), `ind` when the call named indicators, and `draw`;
  without a base URL it leaves `open_url` out.
- **Merge, not replace.** A link's drawings join the symbol's stored ones as the user's own. One with
  the same tool, points and text as a stored drawing is that drawing and is skipped, so a link opened
  twice adds nothing, and none go past the 500 a symbol keeps. The chart then loads them as usual:
  editable, lockable, deletable, with undo for every change after the load. Copy link adds the symbol's
  visible drawings when they fit.
- **Prompt.** `prompts/list` and `prompts/get` serve `draw-on-chart`, the workflow (inspect, anchor,
  draw, check the image and warnings, share `open_url`) with three worked examples; `initialize`
  announces `prompts`.

Option 2 needs server-side drawing state per user and chart, ids, and a second round trip to see
anything, for what one call does. An agent that wants to change a drawing sends the list again.
Option 3 leaves the agent to work out which of its drawings exist, and an image that is partly what
it meant is easy to misread as right. Option 4 is the agent-only vocabulary the first driver rules out:
a person could not edit `support_zone`, and two names for one rectangle drift. Option 5 makes the
agent compute indices or pixels, which change with every timeframe, page and image size.

### Consequences

- Good: an agent draws what a person draws, checks it, and hands over a link to editable drawings.
- Good: one table; a tool added to the web app fails the Rust test until the server has it.
- Bad: the PNG approximates the web app (plotters fonts, its own dash pattern, a dark theme only), and
  the simple price label's price sits inside the plot, not on the price axis.
- Bad: times round to bars. A point between two bars lands on the earlier one, as in the web app, so
  `open_url` drawings match the PNG on the same timeframe; on another timeframe they keep their times.
- Bad: `open_url` carries no price-based style (`type` with `tp`): those bars have irregular times, and a
  link opens such drawings on candles.
- Neutral: the import is not an undo step: undo history starts after the load, as for any stored drawing.
