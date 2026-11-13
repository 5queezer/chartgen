---
title: "0067 — Server alerts on an indicator's line, at bar closes"
description: "A server alert can watch a built-in indicator's line instead of the price, at the closes of bars of 1h or longer: the line against a level, or the close against the line. Each watched pair keeps its last 999 closed bars, read once over REST and kept up from the feed. Ten line indicators are allowed, with their params checked by name and stored whole; scripts and indicators that fetch other data are not."
sidebar:
  order: 67
---

**Status:** Accepted
**Date:** 2026-11-12
**Amends:** [ADR-0042](/chartgen/decisions/0042-server-side-alerts/) (indicator alerts may now run
on the server, at bar closes; a watched pair can hold a window of bars) and
[ADR-0061](/chartgen/decisions/0061-alerts-that-wake-the-agent/) (a wake-up's `alert` carries the
indicator, and its `level` is the one at the fire)

## Context and Problem Statement

An agent trading on paper is woken by server alerts
([ADR-0061](/chartgen/decisions/0061-alerts-that-wake-the-agent/)). Server alerts watch the price
only ([ADR-0042](/chartgen/decisions/0042-server-side-alerts/)), and indicator alerts run in a
browser tab with the chart open ([ADR-0034](/chartgen/decisions/0034-drawing-and-indicator-alerts/)).
Most of what invalidates or triggers the agent's setups is an indicator at a bar close:

1. RSI 14 crossing a threshold at the close of a 4h or 1d bar (under 25, over 75).
2. Funding crossing a threshold, at settlement.
3. Open interest changing by more than some percent over N bars.
4. A close beyond a drawing.
5. The close crossing an EMA (21, 50, 200) on 4h or 1d.

All on 1h, 4h and 1d, nothing shorter. How does the server compute an indicator at a close,
on a 3.8 GB ARM board shared with everything else, without polling Binance or letting strangers
spend its CPU?

## Decision Drivers

- The value at a close must be the value a chart of it shows.
- No REST traffic that grows with the number of alerts or with time (ADR-0042).
- Bounded CPU and memory per alert and in total, whoever creates them. Client ids are free.
- Nothing a stranger supplies runs as code on every close.
- A restart neither fires a close twice nor loses the next one.
- A build from before this must not misread an indicator alert as a price alert.
- Price alerts and their payloads unchanged, byte for byte.

## Considered Options

Where the bars come from:

1. **Join a chart group** (`subscribe_chart`'s window,
   [ADR-0021](/chartgen/decisions/0021-realtime-chart-push/)). It recomputes and serializes a
   whole series tail on every tick, for subscribers; an alert would pay that per tick to read one
   number per close.
2. **Read the last bars over REST at every close.** Simple, always right, and one request per
   watched pair per close, forever.
3. **A window per watched pair (chosen).** The pair's last 1000 closed bars, read once over
   REST when an indicator alert needs them, then kept up from the closes of the feed the alerts
   already watch.

How to say "the close crosses EMA 50", whose level moves:

- a. **`compare`** (chosen): `level` (the default) compares the line with `level`; `close`
  compares the bar's close with the line, and takes no `level`.
- b. **`level_indicator`**: an indicator object that replaces the number `level`, against the
  price.

Which indicators:

- i. **Any built-in.** Volume profiles, chart patterns and the indicators that fetch funding or
  open interest are not lines, not linear in the bars, or not bars at all.
- ii. **An allowlist of line indicators computed from the bars alone (chosen).**
- iii. **User scripts too.** Code a stranger wrote, run at every close with nobody watching.

## Decision Outcome

Chosen: **3**, **a** and **ii**, in `src/server_alerts/indicator.rs` and `src/server_alerts.rs`.

### The alert

```json
{ "symbol": "BTCUSDT", "interval": "4h", "evaluate_on": "close",
  "indicator": "rsi", "params": { "length": 14 },
  "condition": "crossing_down", "level": 25, "trigger": "every", "notify_agent": true }

{ "symbol": "ETHUSDT", "interval": "1d", "evaluate_on": "close",
  "indicator": "ma", "params": { "type": "ema", "length": 50 }, "compare": "close",
  "condition": "crossing_up", "trigger": "once" }
```

- `indicator` is one of `rsi`, `ma`, `macd`, `stoch`, `cci`, `mfi`, `williams_r`, `roc`, `atr`,
  `adx`: built-ins with lines, computed from the bars alone, about linear in them. An EMA of a
  length is `ma` with `type: "ema"`; the registry's `ema` is the fixed 8/21/50/200 stack.
- `params` are checked as a chart checks them, and **by name**: a key the indicator does not
  take is refused (`rsi has no parameter lenght; it takes length`) rather than ignored. The
  alert stores every param the indicator takes, with the defaults filled in, so the stored row,
  the list and every webhook say exactly what is computed. `offset` is refused: the alert reads
  the line at the bar that closed.
- `stoch`, `cci`, `mfi`, `williams_r` and `roc` take **at most 200** for each param on a server
  alert. They rescan a window at every bar, so they cost bars times period (see the cost below).
- `line` is a legend label, any case, defaulting to the first line. It is resolved when the alert
  is made, so a stored alert names a line that exists.
- `evaluate_on` must be `close` and the interval 1h or longer. A close of 1h is the finest Hermes
  needs, and what an alert costs is per close.
- `level` is any finite number with `compare: "level"`, and refused with `compare: "close"`.
- The indicator, its params, line and compare cannot change later; condition, level and trigger
  can, as for a price alert.

Why **a** and not **b**: one optional field covers both cases the agent asked for, keeps the
alert one series against one level, and leaves the price alert's `level` meaning what it meant.
`level_indicator` would add a second object with its own params and line to validate, and its
general case (one indicator against another) was not asked for.

### The bars

When an indicator alert runs on a pair, the server reads that pair's last 1000 bars once,
through the path a chart seeds with (`ChartHub::seed_bars`), so through the Binance gate. It keeps
the closed ones, usually 999: the forming bar is not a close. Each close the feed sends is then
added, and the window stays at most 1000. **1000 is one Binance page**, and 999 closed bars are
enough for EMA 200 (891 bars to settle) and RSI up to about 120. An indicator whose
`warmup_bars` with its params, plus the two closes a crossing reads, is more than 999 is refused
when the alert is made.

That bound is only as good as `warmup_bars`. `stoch`, `cci`, `mfi`, `williams_r` and `roc` used
to declare 0, so any period passed and a large one read nothing, forever. They now declare what
their first value reads: the period, and for `stoch` period + %K smoothing + %D smoothing - 2.
A test reads every allowed indicator at the largest params an alert accepts on a 999-bar window,
so an indicator that under-declares again fails it. Charts read these needs too: a series asked
with a non-zero `warmup` below them is raised to them (ADR-0032), which only fills their first
values; the web chart and images already read more.

- **Read again, not patched.** A close that does not follow the last bar of the window, or a
  feed that lagged, drops the window and reads it again. The close that showed the gap waits
  and is read once the bars are back, if REST already has it. Closes that arrive while the bars
  are read are added after them.
- A refused or failed read is tried again by the upkeep, each minute, and the alert's `feed`
  reads `waiting` meanwhile.
- **At most one read a minute per pair after a gap or a lag.** A feed that keeps dropping closes
  waits out the minute since the last such read that succeeded, then the upkeep reads it, so
  within two minutes. The first read of a pair does not count.
- A pair whose last indicator alert goes keeps no window.
- These reads are the only new REST traffic: one per pair when its first indicator alert
  starts, after a restart, and after a gap or a lag.

### A close

At each close of its interval, each indicator alert computes its indicator over the window up
to that bar and takes the line at that bar and at the bar before. With `compare: "level"` the
reading is line minus level; with `close`, close minus line. The browser's fire rules then
apply to those two readings as they do to two prices (ADR-0042), so `crossing_up` needs the
reading before to be under and this one on or over.

**The reading before comes from the bars, not from memory.** A price alert's previous price is
lost on a restart, on purpose, and its first price after one is only a baseline. An indicator
alert's previous reading is the close before, which the window has, so the first close after
creating it or after a restart can fire a crossing. That is the point for a daily close: a
restart must not cost a day.

**A close fires an alert once.** The alert stores the open time of the bar it last fired on
(`fired_bar`), with the fire, and a close at or before it does not fire. A feed can send the
last close again after a restart; it is read and skipped.

A close with too little history before it for a settled value (a new listing), or where the
line is empty, reads nothing.

### What a fire carries

The webhook payload and the wake-up keep their fields, and an indicator alert's add
`"indicator": {"name", "params", "line", "compare", "value"}` with the line's value at that
close. `level` is the alert's level, or with `compare: "close"` the line's value the close
crossed. A price alert's body has no `indicator` and is unchanged. The wake-up's `level` is now
the one stored with the fire rather than the alert's when sent; the two differ only if the
level was changed while the wake-up waited.

`list_server_alerts` shows the same `indicator` object, with `value` at the last close read,
and `level: null` with `compare: "close"`.

### Stored

The row keeps `level` inside `indicator` and none at the top. A build from before this cannot
read such a row and skips it ("skipping an unreadable alert"), rather than running an RSI level
of 25 as a price of 25. Price alerts are stored as before.

### Cost on the radxa

- **Memory.** A window is 1000 bars of about 100 bytes: 100 KB per pair, at most 1.6 MB with the
  16 pairs the server watches, and nothing for pairs with no indicator alert.
- **CPU.** Per alert and per close, one computation over the window and a copy of it, under
  the alerts' lock. Nothing runs on ticks. Measured on x86 (release build, 999 bars, one read):

  | Indicator, params | Per close |
  |---|---|
  | `rsi` 14 or 126, `ma` ema 200 or sma 997, `macd`, `atr`, `mfi` 200, `roc` 200 | 40-70 µs |
  | `adx` 14 | 93 µs |
  | `cci` 200 | 0.3 ms |
  | `williams_r` 200, `stoch` 200/200/200 | 0.7 ms |

  Most of the linear ones' time is the copy of the bars. The rescanning five grow with their
  period, which is why they stop at 200. The worst the limits allow, 200 alerts of
  `williams_r` 200 on one 1h pair, is about 0.14 s once an hour on x86; the radxa's ARM cores
  are slower, by an unmeasured factor of a few. Typical alerts, RSI and EMAs,
  cost 200 × 60 µs = 12 ms at that worst.
- **Network.** One REST page per pair as above, through the gate that charts use.

The limits of ADR-0042 are unchanged and count indicator alerts like any: 20 per client, 200
live, 16 pairs and 2 per client, 10 changes a minute. The 1h minimum, the 999-bar window, the
cap of 200 on the rescanning five and the allowlist are what bound the cost per alert.

### What it does not do yet

- **Funding at settlement.** Funding is not a bar series: it settles every 8 hours, comes from
  the futures API, and the `funding` indicator fetches it beyond the bars. It needs a source of
  its own that reads each symbol's rate once per settlement through the gate, shared by every
  alert on it, and an evaluation at settlement rather than at a close.
- **Open interest change over N bars.** Open interest also comes from the futures API (the last
  30 days, per period), so it needs the same kind of source, and "changed by more than x% over N
  bars" is not a line against a level: it would be a third `compare`, the change of the line.
- **A close beyond a drawing.** Drawings belong to the browser or to the agent's store
  ([ADR-0054](/chartgen/decisions/0054-agent-drawings/)); a line drawn through two points is a
  level that moves with time, which `compare` could take once the server can read the drawing.
- **The dialog** creates price alerts only. The Alerts tab lists indicator alerts and can pause,
  resume and delete them, but not edit them.

### Consequences

- An agent can be woken by RSI or a close across an EMA at 1h, 4h or 1d, with no browser open.
- An indicator alert's first close after a restart can fire, unlike a price alert's first price.
- A close missed while the server is down is not reported, as for a price alert. A close behind
  a gap is reported late, once the bars are read again, or not at all if REST does not yet have it.
- After a downgrade, indicator alerts do not run, and the older build keeps their rows.
- A pair with an indicator alert holds 100 KB more than one without.

### Confirmation

`src/server_alerts/tests.rs`: RSI fires at the close it crosses and not on ticks; the close
crossing an EMA compares two closes; a missing close reads the bars again and then fires on it;
a close seen again after a restart does not fire twice, and the stored row has no top-level
`level`; the webhook body carries the line and its value; bad input is refused with the reason.
`src/server_alerts/indicator.rs`: every allowed indicator has a line and computes from the bars,
and the tool schema offers exactly the allowed indicators.
