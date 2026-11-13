---
title: "0069 — An alert at a bar's close"
description: "A server alert with condition bar_close fires at every close of its interval, 1h or longer, whatever the price, with no level and no indicator. It reuses the close path of indicator alerts and its fired_bar, so a close fires it once, also across a restart, and a close nobody saw is not fired late."
sidebar:
  order: 69
---

**Status:** Proposed
**Date:** 2026-11-13
**Amends:** [ADR-0042](/chartgen/decisions/0042-server-side-alerts/) (a server alert can fire at a
bar's close, whatever the price)

## Context and Problem Statement

The trading agent (Hermes) wants to be woken "at the 4h close" to look at its charts, whatever
the price. A server alert watches the price against a level
([ADR-0042](/chartgen/decisions/0042-server-side-alerts/)) or, since
[ADR-0067](/chartgen/decisions/0067-indicator-alerts-on-the-server/), an indicator's line at a
close. Neither says "every close": the nearest is an indicator alert whose condition always holds,
which costs an indicator computation over 999 bars per close to say nothing.

This was first built on the unreviewed track-record branch (its ADR-0064, never merged), which
the assessment of that branch split up. This is that third part, rebuilt on the alerts as
ADR-0067 left them.

## Decision Drivers

- An agent can be woken at a close with no browser open and no level to invent.
- Nothing new to keep or read: no bars, no REST, no state beyond what a close alert has.
- It cannot flood the owner's deliveries or event log: one alert on 1m closes would fire 1440
  times a day.
- A restart neither fires a close twice nor makes up one it did not see.
- Price and indicator alerts, their stored rows and their payloads unchanged.

## Considered Options

1. **`condition: "bar_close"`** on `create_server_alert`.
2. **A timer object of its own** (a new tool, "wake me at these times").
3. **Leave it to an indicator alert** with a condition that always holds.

## Decision Outcome

Chosen: **1**.

- **What it is.** An alert like any other: the same record, limits, `trigger` (`once`, or every
  close), webhook, push, `notify_agent` and `setup_id`. It fires when a bar of its `interval`
  closes, whatever the price.
- **What it takes.** An `interval` of **1h or longer**, with a Binance kline stream, as an
  indicator alert. `evaluate_on` is `close`, and `live` is refused. `level`, `indicator`,
  `params`, `line` and `compare` are refused: it is a time, not a reading.
- **Its kind is fixed.** `update_server_alert` changes its trigger, note and delivery, but no
  alert changes into `bar_close` or out of it.
- **Stored with no level.** Like an indicator alert that compares the close, the row has no
  top-level `level` (`Option`, skipped when `None`). A list, the webhook body and the wake-up
  show `"level": null`, `"condition": "bar_close"`, and `price` as the bar's close.
- **A close fires it once.** It shares the `fired_bar` of indicator alerts: the open time of the
  bar it last fired for, stored with the fire. A close at or before it does not fire, so a bar
  the feed sends again after a restart is skipped. A bar that closed before the alert was made
  does not fire it either.
- **A close nobody saw is not fired late.** After a restart or an outage it fires at the next
  close the server sees, with that bar's time. Firing for a missed close would need a price
  nobody read, or a history read on the delivery path.
- **On the close path that exists.** It is evaluated where the closes of indicator alerts are,
  with the fire rules of every alert (`fires` holds for it at every close), and needs none of
  their bar window.

### Why not the others

- **2, a timer object,** would be a second kind of thing with its own tools, limits, storage and
  listing, for what an alert already does: a schedule tied to a symbol's bars, delivered the
  way alerts are.
- **3, an always-true indicator alert,** is a hack that costs an indicator computation per close,
  and reads as a condition on a line when it is none.

### Consequences

- Good: an agent can be woken at every 1h, 4h or 1d close with no browser, and the setup it is
  about can ride along (`setup_id`).
- Good: no new storage, state or REST traffic; the fired-once rule is the one indicator alerts
  have.
- Good: the 1h minimum bounds it to 24 fires a day per alert, 480 for a client's 20, so it cannot
  push an owner's other deliveries out the way 1m closes would.
- Bad: a close missed while the server is down is not reported. An agent that needs every close
  needs its own schedule as well.
- Neutral: a build from before this cannot read `bar_close` and skips the alert at start
  ("skipping an unreadable alert"), running the others and leaving its row alone.
- Neutral: the web app does not create these. It lists one made by an agent ("4h bar close")
  and can pause, resume and delete it, and does not open the price dialog for it.

### Confirmation

`src/server_alerts/tests.rs`:
- it fires at each close and at nothing else (not on ticks, not twice for one bar), and its
  webhook body has `level: null`, `condition: bar_close`, the close as `price` and no
  `indicator`;
- every refusal: a level, an indicator, params, line or compare, `evaluate_on: live`, no
  interval, an interval under 1h, and a change into or out of the kind;
- it counts in the owner's alert limit, and its stored row has no `level`;
- after a restart it does not fire again for the bar it fired for, fires nothing for the closes
  it missed, and fires at the next close it sees.

`web/src/stores/serverAlerts.test.ts`: its text, its parse, and that the dialog does not edit it.
