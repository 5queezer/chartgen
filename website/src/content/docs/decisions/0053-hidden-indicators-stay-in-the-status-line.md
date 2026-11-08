---
title: "0053 — A hidden indicator stays in the status line"
description: "Indicators are managed from their legend rows only. A hidden indicator is still requested, keeps its pane and its legend row, and the client draws nothing for it."
sidebar:
  order: 53
---

**Status:** Accepted
**Date:** 2026-11-08
**Supersedes in part:** [ADR-0013](/chartgen/decisions/0013-indicator-visibility-and-instances/) (hidden entries are no longer dropped from the request; the hidden flag stays client-only)

## Context and Problem Statement

The web app showed each indicator twice: as a row in the chart's status line (the pane legend,
[ADR-0006](/chartgen/decisions/0006-tradingview-visual-parity/)) and as a chip in a row under the
header. TradingView has no such row: an indicator is managed from its status line only, and a hidden
indicator stays there, dimmed, with its values blank and a crossed-out eye that shows it again. Its
pane stays, empty.

ADR-0013 dropped hidden entries from the `generate_chart` request. A hidden indicator then had no
payload, so no pane and no legend row, and only its chip or its Object Tree row could show it again.
Removing the chips means the legend row of a hidden indicator has to exist, and to know whether the indicator is
an overlay or has a pane of its own.

## Decision Drivers

- One place to manage an indicator, as in TradingView, on desktop and on touch.
- A hidden indicator keeps its place: its pane, its row, its order.
- Hiding and showing are instant and do not refetch.
- The hidden flag stays client-only: it is not sent, and is not part of the query key.

## Considered Options

1. **Request hidden indicators too, and draw nothing for them on the client.**
2. Keep them out of the request, and build a placeholder row and pane from the registry.
3. Keep the chips for hidden and failed indicators only.

## Decision Outcome

Chosen: option 1. `toWire()` keeps every entry and strips `hidden`, so hiding never changes the
request or its cache key. The chart passes the flag to each indicator's klinecharts `extendData`: a
hidden indicator's `calc` returns no values (nothing is drawn, nothing scales the axis, no last-value
label) and its `draw` paints none of its fills, levels or marks. Its legend row keeps the name and
params, dimmed, with no values; its eye button is crossed out, says *Show …*, and stays in view
without hover. A tap on a row on touch shows its controls, as before.

Option 2 needs the overlay flag and the pane layout without a payload. The registry does not say
whether a scripted indicator is an overlay, and showing it again would refetch. Option 3 keeps two
places to manage one thing.

An instance the server reports as failed has no payload either, so no row. It is listed in the error
row under the header on every layout, which only the compact layout had before.

### Consequences

- Good: no chip row; the chart starts right under the header. Hide and show cost no request.
- Good: a hidden indicator's data is at hand, so indicator alerts on it keep running, as they do on
  TradingView, rather than reading *indicator not on the chart*. Replay and the chart's alert context
  see the same request as the chart.
- Bad: a hidden indicator costs the server what a shown one does, and counts against the tool-call
  budget ([ADR-0052](/chartgen/decisions/0052-indicator-cost/)).
- Neutral: share links still leave hidden indicators out
  ([ADR-0029](/chartgen/decisions/0029-shareable-chart-links/)); layouts keep the flag.
- Neutral: a hidden Ichimoku keeps the right margin of its projection.
