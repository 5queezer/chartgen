---
title: "0085 — Server alerts on open interest's change over N bars"
description: "An indicator alert can watch open interest by its change over change_bars bars, in percent, the number get_indicators answers as change.pct with the same change_bars. It is a futures-data indicator read as funding is (ADR-0080); its value as a count of contracts is not offered."
sidebar:
  order: 85
---

**Status:** Accepted
**Date:** 2026-11-25
**Amends:** [ADR-0080](/chartgen/decisions/0080-futures-data-on-server-alerts/) (a second
futures-data indicator, compared by its change instead of its value)

## Context and Problem Statement

An agent client's playbook asks whether open interest is rising or falling before a short, and
by how much: "open interest up x % over N bars". `get_indicators` answers that with its
`change_bars` argument, but only when asked, so the agent polls. Server alerts read futures data at a
close since ADR-0080, for funding, and compare a line's value with a level. Open interest's value
is a count of contracts: 2 billion on one symbol is nothing like 2 billion on another, and a
threshold on it goes stale as the market grows. Its change is what the playbook means.

How does a server alert watch open interest's change?

## Considered Options

- a. **`oi` with `change_bars`, compared by its change in percent (chosen).** The alert names the
  bars as `get_indicators` does and compares `change.pct` with `level`, read through the same
  `change_over` and `change_pct`, so a number the agent saw in a tool answer is the number the
  alert fires on.
- b. **A third `compare`, `change`, on any indicator** (what ADR-0067 sketched). General, but a
  percent change of RSI or ATR is rarely meant, and of a line that crosses 0 (MACD) it is
  undefined; nothing asks for it beyond open interest.
- c. **`oi` by its value, with the change left to the agent.** A level of contracts per symbol,
  which the agent would have to re-set as the market moves.

## Decision Outcome

Chosen: **a**, in `src/server_alerts/indicator.rs`, with `change_pct` beside `change_over` in
`src/indicator.rs` and used by `get_indicators` too.

```json
{ "symbol": "ETHUSDT", "interval": "1h", "evaluate_on": "close",
  "indicator": "oi", "change_bars": 6, "condition": "crossing_up", "level": 5,
  "trigger": "once" }
```

- **`oi`** (or `open_interest`) joins `funding` as a futures-data indicator: read off the alerts'
  lock at each close, through the series cache and the futures budget, and a crossing compares
  with the value the alert read at its previous close (ADR-0080). Its line is `OI`.
- **`change_bars`**, 1 to 199 as in `get_indicators`, is required with `oi` and refused on any
  other indicator, inside `and`, and in an update. The reading is the change from `change_bars`
  bars before the bar that closed to that bar, in percent of the earlier value; none where either
  end has no value or the earlier is 0. `level` is that percent.
- **The bar that closed must have a value.** `change_over` ends at a series' last finite value,
  which is what a tool answer wants; for an alert that would read the change up to an earlier bar
  as this close's, and could fire (and a `once` alert be spent) on a close with no open interest.
  So the alert has no reading at such a close.
- **The same number as the tool, at the close.** `get_indicators` reads up to the bar still
  forming (`fetch_ohlcv` with no end time), so its `change.pct` ends at the forming bar and moves
  within it; the alert's ends at the bar that closed. At a close they are the same definition over
  bars that end one bar apart: the tool asked right after a close reports the new forming bar,
  the alert the bar that just closed.
- **The bars read** are the last `max(50, change_bars + 2)` closed bars, so the change has its
  start.
- **When it is made**, the server reads the series once; an alert whose change has no value (a
  symbol with no futures market, or `change_bars` beyond the 30 days Binance keeps: about 185 bars
  on 4h, 31 on 1d) is refused with the reason.
- **What it shows:** the alert's, the webhook's and the wake-up's `indicator` carry `change_bars`
  (only on such an alert), and `value` is the change in percent.

### Consequences

- An agent can be woken when open interest rises or falls by a percentage over the bars it
  chooses, with the number `get_indicators` gives it.
- `change_pct` is now the one definition of a tool answer's `change.pct` and of the alert's
  reading.
- Open interest older than 30 days still cannot be read; recording it is ADR-0082's question.
- **After a rollback**, a build from before this (with ADR-0080 and ADR-0084) loads a store with an
  `oi` alert: it validates nothing at load and ignores the unknown `change_bars`. Not knowing `oi`
  as futures data, it computes it as a bar-only indicator without the symbol: no request, no `OI`
  value, so that alert never fires and its `value` stays `null`. Every other alert runs: checked
  on that build with such a store, a price alert fired and an RSI alert on the same feed read its
  bars, with no error logged. When that build stores the `oi` alert again (an edit), it drops
  `change_bars`; delete such alerts after rolling forward and make them again.
