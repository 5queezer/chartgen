---
title: "0080 — Server alerts on futures data: funding"
description: "An indicator alert can watch a futures-data indicator, funding first, at the closes of its interval. The series is the one charts show, read off the alerts' lock on a blocking thread through the same cache and budget, from the last 50 bars. A crossing compares with the value the alert read at its previous close. A symbol with no futures market is refused when the alert is made."
sidebar:
  order: 80
---

**Status:** Accepted
**Date:** 2026-11-24
**Amends:** [ADR-0067](/chartgen/decisions/0067-indicator-alerts-on-the-server/) (indicator alerts
may watch an indicator that fetches futures data; its "What it does not do yet" named funding at
settlement)

## Context and Problem Statement

An agent client is woken by server alerts
([ADR-0061](/chartgen/decisions/0061-alerts-that-wake-the-agent/)). Its playbook checks the
funding rate against a threshold, a sign of a crowded side, and today it can only poll for it:
indicator alerts take ten indicators computed from the bars alone
([ADR-0067](/chartgen/decisions/0067-indicator-alerts-on-the-server/)), and `funding` fetches
Binance's funding history besides the bars. An open-interest change over N bars is the next
condition of the same kind.

Three things make a futures series different from RSI at a close:

1. **It is a request.** Reading it can take seconds (a 20 s deadline), while an indicator alert is
   evaluated under the alerts' lock, on the task that receives the feed.
2. **It is not a bar series.** Funding settles every 8 hours (on most symbols), and the chart
   gives each bar the settlement nearest its open. Recomputed at a close, the bar before can
   already show the settlement that just came, so the line "one bar back" does not say what was
   known one close ago.
3. **A symbol may have none.** A spot pair with no perpetual has no funding, and an alert on it
   would never fire.

How does a server alert watch a funding threshold?

## Considered Options

When and from what it reads:

- a. **An indicator alert at the closes of its interval, on the series charts show (chosen).**
  The rate is the one in the Funding panel and in a wake-up's `context.funding_pct`
  ([ADR-0072](/chartgen/decisions/0072-wake-up-context-and-near-stop/)), read through the
  series cache and budget charts use, shared by every alert and chart on a symbol.
- b. **A source of its own per symbol, read at each settlement** (what ADR-0067 sketched). The
  alert fires at the settlement itself, a little sooner. It needs a schedule per symbol (most
  settle every 8 h, some every 4 h or 1 h, and that changes), a second reader beside the
  series cache, and a condition model that is neither a price nor a close.
- c. **The predicted rate, live** (`premiumIndex`, the rate the next settlement would have now).
  It moves every few seconds, so `greater` fires on noise, and it is not the number the agent
  reads anywhere else in chartgen.

Where the request runs:

- i. **Under the lock**, as bar-only indicators are: simple, and one slow request stops every
  alert on the server.
- ii. **On a blocking thread with no lock held, then evaluated (chosen)**, as the bars of an
  indicator alert are read already.

What a crossing compares:

- x. **The series at the bar that closed and the bar before**, as ADR-0067 does. With the
  nearest settlement on each bar, both bars show the new rate at the close where it first
  appears, and a crossing never fires.
- y. **The value read now against the value this alert read at its previous close (chosen)**,
  as a price alert compares with its previous price.

## Decision Outcome

Chosen: **a**, **ii** and **y**, in `src/server_alerts/indicator.rs` and `src/server_alerts.rs`.

```json
{ "symbol": "BTCUSDT", "interval": "1h", "evaluate_on": "close",
  "indicator": "funding", "condition": "crossing_up", "level": 0.05,
  "trigger": "once", "notify_agent": true }
```

- **The indicator.** `funding` (or `funding_rate`, stored as `funding`) joins the allowlist
  as a futures-data indicator (`FUTURES`). Its line is `Funding`, the series the chart draws as
  bars, in percent, so `level: 0.05` is 0.05%. `compare: "close"` is refused: a close is not a
  rate. The rest is as for an indicator alert: `evaluate_on: close`, 1h or longer, the same
  limits, the same `indicator` object in the list, the webhook and the wake-up.
- **At a close.** The alerts on futures data are taken out of the evaluation under the lock.
  For each, a blocking task reads the series over the last 50 closed bars (`FUTURES_BARS`),
  with the symbol and interval, and then takes the lock and evaluates the alert with that
  reading. A read that comes back after a later close's is dropped. A failed read skips the
  close and is logged, and the alert keeps the value it read before: funding moves only at a
  settlement, so its crossing shows at one close, and the next read still finds it, a close
  late, if the rate has not gone back across the level meanwhile. A read still out when the
  alert is resumed or redefined is dropped.
- **A crossing** compares the value now with the value this alert read at its previous close,
  kept in memory as a price alert's baseline is, both against the level as it is now. So, as
  for a price alert and unlike a bar-only indicator alert, the first close after creating the
  alert, a restart, a resume, a new definition or a gap in the feed never fires a crossing. `greater` and
  `less` can fire on it.
- **When it is made**, the server reads the symbol's last 50 bars and the series once. If the
  series has no value at the last close, the alert is refused with the reason the chart's panel
  gives, e.g. `Cannot watch funding on BTCFDUSD: … (no futures market)`.

### Cost

One short read per symbol and close: 50 bars of 1h span about two days, one page of funding
history. It goes through the series cache, so alerts and charts on one symbol share it, and
through the Binance futures gate and the funding budget. The 1h minimum and the limit of 16
watched pairs bound it as they bound every indicator alert.

### Consequences

- An agent can be woken when funding crosses a threshold, with the number it sees in its
  wake-up context.
- A settlement is read at the first close after it is published: within an hour on 1h. On 1d,
  the value is the settlement nearest the day's open, as on the chart, a day old; the docs say
  to use 1h.
- An open-interest change over N bars can use the same path: a futures-data indicator, read off
  the lock at a close. It waits for the change over N bars in the indicators' answer (ADR-0082,
  in review), so that the alert and the tool share one definition.
- **After a rollback**, a build from before this loads a stored `funding` alert and computes it
  at each close without the symbol: that sends no request and finds no `Funding` line, so the
  alert never fires there and its `value` stays `null`. Once this build runs again, it reads
  as before.
