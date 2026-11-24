---
title: "0081 — Server alerts on a drawing of a setup"
description: "A server alert can compare each close with a drawing of one of the owner's setups instead of a level: a trend line, ray or extended line along its slope, a horizontal line, or a rectangle's top or bottom. The drawing's points are copied when the alert is made. A crossing compares each close with the drawing's price at its own bar."
sidebar:
  order: 81
---

**Status:** Accepted
**Date:** 2026-11-24
**Amends:** [ADR-0067](/chartgen/decisions/0067-indicator-alerts-on-the-server/) (its "What it
does not do yet" named a close beyond a drawing)

## Context and Problem Statement

An agent client stores setups with drawings that show its thesis
([ADR-0054](/chartgen/decisions/0054-agent-drawings/),
[ADR-0059](/chartgen/decisions/0059-setups-paper-scored/)): a trend line the trade rests on, a
range it waits to break. Its playbook checks "a close beyond the line" and "a close out of the
range", and server alerts only take a fixed level
([ADR-0042](/chartgen/decisions/0042-server-side-alerts/)). A horizontal level it can copy by
hand; a sloping line it cannot, because the price it is compared with changes every bar. So it
polls, or skips the check. The browser's drawing alerts
([ADR-0034](/chartgen/decisions/0034-drawing-and-indicator-alerts/)) need an open tab and the
browser's own drawings.

How does a server alert watch a drawing?

## Considered Options

Where the drawing comes from:

- a. **A drawing of one of the owner's setups, named by setup and index (chosen).** The setup's
  drawings are already checked and placed on bars, and the agent made them for this trade.
- b. **The points given in the alert**, as `generate_chart` takes them. More general, but the
  agent would copy what it already stored, and the alert would need the same checks and bars.
- c. Both. Nothing asks for b yet.

When the setup's drawings change:

- i. **Copy the points when the alert is made (chosen).** The alert says what it watches, in its
  list and in its fires, whatever happens to the setup later. A revision, which replaces the
  whole list, cannot shift an index onto another drawing.
- ii. **Follow the setup.** A revised line moves the alert with it, but an index can then point
  at another drawing, a closed setup's drawings would still have to be kept for it, and a fire
  could not say which line it was.

What a crossing compares:

- x. **Each close with the drawing's price at its own bar (chosen).** For a rising line, the
  reading before is the last close's distance from where the line was then.
- y. The two closes with the line's price now. On a sloping line this reads a crossing that did
  not happen, or misses one.

## Decision Outcome

Chosen: **a**, **i** and **x**, in `src/server_alerts/drawing.rs` and `src/server_alerts.rs`.

```json
{ "symbol": "BTCUSDT", "interval": "4h", "evaluate_on": "close",
  "drawing": { "setup_id": "0b6f6f0e-…", "index": 0 },
  "condition": "crossing_down", "trigger": "once" }
```

- **Which drawings.** Lines through two points: a trend line (`segment`), read along its slope
  past its end (the slope is the thesis, and most setups draw the line to the last bar), and not
  before its first point; a ray from its origin; an extended line both ways. Horizontal lines
  and price labels at their price, a horizontal ray from its point on. A rectangle from its left
  edge on, by its top for `crossing_up` and `greater` and by its bottom for `crossing_down` and
  `less`; `crossing` is refused on it, since "either side" would be two levels. Other tools
  (text, fib levels, channels, positions) are refused, as are a vertical line and a ray that
  runs left.
- **The price at a close** is the drawing's at the open time of the bar that closed, as the chart
  draws the candle against the line.
- **Checks when it is made:** the setup is the owner's and on the alert's symbol, the index
  names a drawing, `evaluate_on` is `close`, and no `level` or `indicator` is given. The setup
  may be open or closed; the drawing is only geometry. `drawing.setup_id` is apart from
  `setup_id`, which still names the setup a wake-up carries.
- **What it shows.** The alert, its webhook payload and its wake-up carry
  `drawing: {setup_id, index, tool, points, level}`, `level` being the drawing's price at the
  last close read, or at the close that fired. The fire's `level` is that price too, as with
  `compare: close` ([ADR-0067](/chartgen/decisions/0067-indicator-alerts-on-the-server/)).
  `drawing` cannot change; `condition` and `trigger` can.
- **The reading before** is kept in memory, as a price alert's is: the first close after
  creating the alert or a restart never fires a crossing.

### Consequences

- An agent can be woken by a close through its own trend line or out of its own range, on any
  interval with a kline stream, with no browser open. It costs no more than a price alert at
  closes: no bars are kept and nothing is fetched.
- A revised setup does not move its alerts. The agent deletes and makes them again.
- The web app lists such an alert as "close crossing down trend line (121.00)" and does not offer
  the price dialog for it.
- **After a rollback**, a build from before this reads a stored drawing alert as a price alert
  with no level: it never fires and shows `level: null`. When that build stores the alert again
  (an edit), the drawing is gone from it.
