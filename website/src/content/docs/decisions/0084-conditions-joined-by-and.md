---
title: "0084 — Server alert conditions joined by AND"
description: "A server alert can take up to three more conditions, joined by AND: it fires only at a close where its own condition fires and each of them holds. Each is a price level or a bar-only indicator, read from the feed's window of closed bars at that close and the one before, so nothing about them is kept in memory."
sidebar:
  order: 84
---

**Status:** Accepted
**Date:** 2026-11-24
**Amends:** [ADR-0042](/chartgen/decisions/0042-server-side-alerts/) (an alert can join conditions)
and [ADR-0067](/chartgen/decisions/0067-indicator-alerts-on-the-server/) (an alert with
conditions keeps a window of bars, as an indicator alert does)

## Context and Problem Statement

An agent client's playbook checks combinations: a break of a level while RSI is not yet
stretched, or a close over EMA 50 while the price is above a range. Each server alert watches
one thing, so the agent makes several and works out the combination itself when woken, or
polls. A wake-up for half a condition costs it a turn.

How does one alert say "this, and that, at the same close"?

## Considered Options

The shape:

- a. **The alert's own condition, plus `and`: a list of clauses that must hold (chosen).** An
  alert without `and` is exactly what it was; the clauses are an addition beside it, and the fire
  rules (`once`, `every`, re-arming, the rate limit) stay those of its own condition.
- b. **A tree of conditions** (`all`, `any`, nested). More general; nothing asks for OR, and a
  tree changes what every alert is, on the wire and in the store.
- c. **Several alerts linked by a group id**, firing when all in the group fired at one close.
  Every member would need the others' state, and deleting one breaks the rest.

When a clause holds:

- i. **At the same close, from the feed's bars (chosen).** A clause reads the bar that closed and
  the bar before from the window an indicator alert already keeps: `greater` and `less` are
  states, a crossing holds when it crossed between those two closes. The same after a restart.
- ii. **Within N bars of each other.** "RSI crossed under 30 in the last 3 bars, and the price
  now breaks the level" is a real pattern, but it needs a window per clause and memory of when
  each held. Not asked for yet; a clause can be given a window later without changing (i).

## Decision Outcome

Chosen: **a** and **i**, in `src/server_alerts/clauses.rs` and `src/server_alerts.rs`.

```json
{ "symbol": "BTCUSDT", "interval": "4h", "evaluate_on": "close",
  "condition": "crossing_up", "level": 98000, "trigger": "once",
  "and": [
    { "indicator": "rsi", "condition": "less", "level": 70 },
    { "indicator": "ma", "params": { "type": "ema", "length": 50 }, "compare": "close", "condition": "greater" }
  ] }
```

- **A clause** takes `condition` (not `bar_close`) and either a price `level` (greater than 0,
  the close against it) or `indicator` with `params`, `line`, `compare` and `level`, checked as an
  indicator alert's are ([ADR-0067](/chartgen/decisions/0067-indicator-alerts-on-the-server/)).
  At most 3 clauses.
- **Where it applies:** the alert reads closes (`evaluate_on: close`, or `condition: bar_close`,
  which then fires at every close where every clause holds), on an interval of 1h or longer. Its
  feed keeps the window of closed bars, as for an indicator alert, and a close is evaluated once
  the window holds it.
- **The fire.** At a close where the alert's own condition fires, the clauses are read; if one
  does not hold, the fire is held back as if the condition had not been met (`step`'s
  `allow_fire`). A crossing held back is not kept for a later close. A clause that cannot be read
  (no bars yet, too little history) does not hold.
- **What it shows.** The alert lists `and` with each clause and what it read at the last close
  (`price`, or `indicator.value`); the webhook payload and the wake-up carry the same, at the
  close that fired. `and` cannot change.
- **Not as a clause:** a futures-data indicator such as funding
  ([ADR-0080](/chartgen/decisions/0080-futures-data-on-server-alerts/)) is refused: it is read with
  the symbol, off the alerts' lock, and a clause is read from the bars alone, so it would never
  hold. Nor a drawing of a setup
  ([ADR-0081](/chartgen/decisions/0081-alerts-on-a-setups-drawing/)). Either can be the alert's
  own condition with `and` beside it. A price or drawing alert with `and` reads a close only once
  the feed's bars hold it, so a close that comes while they are read again cannot move its
  baseline first.

### Cost

An indicator clause costs what an indicator alert costs, at each close of the alert's interval;
a price clause, nothing. The 1h minimum, the 999-bar window, the three-clause cap and the limits
of ADR-0042 bound it.

### Consequences

- An agent is woken once, for the combination it wants, with what each part read.
- An alert with `and` cannot be edited in the web app's dialog, which shows only one condition;
  the Alerts tab names it "… and 2 more".
- **After a rollback**, a build from before this ignores `and`: such an alert fires on its own
  condition alone, as a plain alert. When that build stores it again, the clauses are gone. The
  deploy guide's rollback notes say to pause such alerts first.
