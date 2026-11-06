---
title: "0043 — Multi-chart grid: per-pane store instances behind an active-chart API"
description: "A layout can show 1, 2 or 4 charts. Each pane has its own store instances; the module-level store exports read the active pane, so the header and panels follow it unchanged."
sidebar:
  order: 43
---

**Status:** Accepted
**Date:** 2026-11-06
**Amends:** [ADR-0021](/chartgen/decisions/0021-realtime-chart-push/) (the push cap per client address), [ADR-0015](/chartgen/decisions/0015-named-layouts/) (a layout's state gains optional `grid` and `panes`), [ADR-0029](/chartgen/decisions/0029-shareable-chart-links/) (a link carries the active chart) and [ADR-0034](/chartgen/decisions/0034-drawing-and-indicator-alerts/) (chart-bound alerts read the active chart)

## Context and Problem Statement

The web app assumed one chart: symbol, interval, scale, compare symbols and
indicators lived in module-level signals, and about thirty components read them.
Show several charts at once, each with its own state, with one active chart the
header, panels and shortcuts act on, without changing anything for a user who
keeps one chart: same storage keys, stored layouts and share links.

Most readers of that state are the header and the panels, which should follow the
active chart anyway. Only a few things belong to one chart: the query, live feed,
replay, compare feed, drawing controller, price axis controls and the chart itself.

## Considered Options

- **Pass a store through props or context to every reader.** Explicit, but every
  component and helper that reads a store changes, for a result where most of them
  mean "the active chart".
- **Resolve the pane from the Solid owner in each store call.** No call site
  changes, but a call outside an owner (a klinecharts callback, a timer) silently
  reads the active chart, whichever it is: the failure the feature must not have.
- **An iframe per chart.** Full isolation for free, but one OAuth session per
  frame, shared `localStorage` keys anyway, and four push clients.
- **Swap one chart's state in and out on activation.** Cannot keep four charts live.
- **Per-pane store instances, with the module-level exports reading the active
  pane (chosen).**

## Decision Outcome

**Stores.** `stores/layout`, `indicators` and `compare` hold one instance per pane
(`chartStoreOf(pane)`, `indicatorStoreOf`, `compareStoreOf`). Their existing exports
stay and read the instance of `activePane()`, so the toolbar, chips, picker,
watchlist, panels and shortcuts follow the active chart without a change.

**Panes.** `createChartPane(pane)` holds what belongs to one chart and only touches
that pane's instances. Components inside a chart that can run while another chart
is active (`Chart`, `PriceAxisControls`) get the pane from a context with `usePane()`.
`ChartContextMenu` and `chartActions` keep the active-chart exports: a press
activates its chart in the capture phase, before anything it leads to runs.
A key press has no such step, so a legend's Settings and Add alert activate their chart
themselves. Focus does not activate: `keepKeyboardFocus` moves focus to the first chart
when a dialog closes.

**Storage.** Pane 0 keeps the keys the single-chart app always used
(`chartgen.layout.*`, `chartgen.indicators.active`). Others use
`chartgen.pane.<n>.*`. `chartgen.layout.grid` and `chartgen.layout.activePane` are
written when the grid or the active chart changes, and the grid on a layout switch;
absent, they mean one chart.

**Layouts.** A layout keeps its first chart where it always did. `grid` and `panes`
are optional and only present with more than one chart, and part of the dirty
check. No `grid` is one chart, and an older build reads a grid layout as its first
chart. The time zone stays one per layout. Reducing the grid removes the last
charts; `window.confirm` asks first if one holds anything the active layout could
not bring back. Growing it restores the saved chart of that slot, else copies the
active chart's symbol, interval, type and indicators.

**Global by design.** Theme, chart settings, watchlists, recents and the time zone.

**Drawings** stay per symbol (ADR-0015), because alerts read them by symbol. Two
charts on one symbol would overwrite each other's saves, so a save tells the other
charts, which reload them and drop their undo history. Tools, undo and selection are
each chart's own: one `DrawingToolbar` per pane, portalled into a shared rail, shows
the active chart's. Two charts on a symbol therefore show the same drawings; that is
a documented exception to "changing one chart cannot change another".

**Share links** carry the active chart only and open in the active chart.

**Live data.** One `onChartTick` handler hands each tail to every feed, which ignores
other subscription ids. Each chart subscribes on its own. The server allows
4 subscriptions per token and 10 subscribe calls per minute (ADR-0021); past those a
chart polls, and tries push again every 5 minutes. A grid reaches these limits in
ordinary use, so a refusal cannot last until the chart's next load. "Not pushable"
stays permanent for that load.

**Push cap per client address: 8 raised to 32, the 64 total kept.** chartgen trusts
no forwarding header, so behind a proxy or tunnel every user shares one address. At 8,
two 4-chart grids would have refused push to everyone else, users without a grid
included. 32 fits eight grids. Reading `CF-Connecting-IP` behind an opt-in would give
each user their own cap, but a wrong setting lets a client forge its address and
escape the cap, so it is left out. The server total still bounds the load.

**Alerts.** Only the active chart feeds the alert engine. Other charts' symbols go
on the quotes poll, like any symbol not on the chart. Indicator and sloped-drawing
alerts therefore only run while the active chart shows their symbol and interval;
switching the active chart restarts the crossing baselines from its last close.

**Compact layout.** The compact shell ([ADR-0039](/chartgen/decisions/0039-compact-layout/)) shows only the active chart. The grid and the other
charts' state are untouched, so nothing is dirty and they come back when there is room.

### Consequences

- Good: one chart behaves as before, on the same keys. Existing e2e specs pass unmodified.
- Good: a chart cannot reach another chart's state through its own effects. Charts on
  one request share a cache entry, so a chart only adopts a new result it asked for.
- Bad: a module-level export read from code running for a non-active chart is a
  silent bug. Pane code must use the pane's instances; this rule is by review only.
- Bad: after a rollback, or in a tab still on an older build, any layout action strips
  `grid` and `panes` from every layout: that parser returns known fields only and writes
  the file back. Grids are lost, nothing is misread, and shipped builds cannot be fixed.
  Future parsers should keep unknown fields.
- Bad: chart-bound alerts on a non-active chart do not run. Evaluating several chart
  contexts would need `chartContext` and its readers to become lists.
- Bad: one chart's registered indicator names differ per pane
  (`chartgen_p<n>_<name>`), since geometry is keyed by name in klinecharts' global registry.
- Bad: a cell is short enough to hit a klinecharts race: a pane resized while a redraw
  of its canvas is pending keeps its old backing store, blank or stretched. In a grid,
  new sub-panes start at a fitted height and later changes apply one pane per frame.
- Bad: the Data Window reads the active chart, so hovering another chart shows nothing there.

Not in this version: symbol or crosshair sync across charts, more than 4 charts,
resizable splitters, and a grid in share links.
