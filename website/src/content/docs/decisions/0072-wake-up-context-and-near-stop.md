---
title: "0072 — A wake-up carries the market around it, and a take near its stop wakes the agent"
description: "Every wake-up carries a short market context for its symbol: price, trend on 4h and 1d, funding, the long/short split and Fear & Greed, made from chartgen's own reads and held for a minute. get_market_context answers the same object. A filled take that goes 0.8 R against it logs setup.near_stop once, which wakes the agent while the take is open."
sidebar:
  order: 72
---

**Status:** Accepted
**Date:** 2026-11-14
**Amends:** [ADR-0061](/chartgen/decisions/0061-alerts-that-wake-the-agent/) (a wake-up's body gains
`context`; a third kind of event, `setup.near_stop`), [ADR-0059](/chartgen/decisions/0059-setups-paper-scored/)
(a take's record and log say when it went near its stop)

## Context and Problem Statement

Hermes, the agent these wake-ups are for, spends four or five tool calls at the start of each woken
session reading the same context: the price, the trend on a higher interval or two, funding, the
long/short ratio and the Fear & Greed index. It records most of them on every setup as features
(`htf_trend`, `funding`, `ls_long`, `fg`). The person approved putting them into the wake-up.

Hermes also said it does not notice when a take fills. That wake-up exists: a status the server
gives a take wakes the agent (ADR-0061). The only take event since the log began was sent during a
network outage of the receiver's host and failed its five attempts. What does not exist is a warning
before the stop: a take could go to 0.9 R against it and Hermes would hear of it only at the stop.
The only way round was an alert of its own, set by hand, on every take.

## Decision Drivers

- The wake-up contract stays backward compatible: fields are added, none renamed or moved. A
  receiver that ignores what it does not know keeps working.
- Short and cheap. No new data source, and no slow external call in a wake-up's way.
- What the context says is what chartgen's tools say: the same reads and indicators.
- A warning comes once. A position that hovers near its stop must not wake the agent every minute.
- The event log stays the one record of what could wake the agent (ADR-0061).

## Considered Options

For the context:

1. **In the wake-up's body, made when it is sent** (chosen), held per symbol for a minute.
2. Stored with the event when it is logged. The event would then hold what the market was when the
   event happened, but a body made an hour later would say the old price, and an event would no longer
   be a few numbers and words.
3. Left to the agent, with a tool that answers it in one call. Saves calls, not a round trip. Offered
   as well (`get_market_context`), for wake-ups that do not come from chartgen.

For the warning:

1. **An event of its own, `setup.near_stop`, from a log entry the scorer writes** (chosen).
2. A field on `setup.status`. The status does not change at that bar, and a reader that keys on
   `change.to` would misread it.
3. An alert the server makes on every take. A second object for the agent to keep in step, deleted
   or not, and one that fires on a price where the setup has its own measure, R.

## Decision Outcome

### The context

An object, `context`, for one symbol:

| Field | What it is | Source |
|---|---|---|
| `symbol`, `at` | the symbol, and when this was read (ms) | |
| `price` | the last price: the close of the forming 4h bar | `fetch_ohlcv`, 150 bars of 4h |
| `trend` | `{"4h": …, "1d": …}`, each `up`, `down` or `mixed` | the same bars, and 150 of 1d |
| `funding_pct` | the latest funding rate, in percent | the `funding_rate` indicator |
| `long_short` | `{long, ratio}`: the share of accounts that are long, and long/short | the `long_short` indicator (Binance's global account ratio, 4h period) |
| `fear_greed` | `{value, label}`, today's index and its band | the `fear_greed` indicator (alternative.me) |
| `line` | all of it in one line | |

**Trend.** Of the bars as read, the forming one included, so its close is the current price:
`up` when the price is above the 20-bar EMA and that EMA is above the 50-bar EMA, `down` when both
are the other way round, `mixed` otherwise. The EMA is chartgen's (Pine `ta.ema`, SMA-seeded); 150
bars let the 50-bar EMA settle. It is a description of the chart, not a signal.

**Bands of the index:** below 25 `extreme fear`, below 45 `fear`, below 55 `neutral`, below 75
`greed`, else `extreme greed`, the bands the Fear & Greed panel colours by.

**What is left out.** Funding, long/short and the index are for Binance symbols only; a stock has its
price and trends. Any part that could not be read is absent, never guessed. **A macro blackout is
not in it:** chartgen has no macro calendar. That lives in Hermes's macro-calendar tool, which stays
where it is.

**Cost.** A context is held per symbol for **one minute**, the index for ten. A cold read is two
kline requests plus the funding and long/short series, which the indicators already cache and pace
through the Binance gate.

**In a wake-up.** The body is made when it is sent (ADR-0061), and `context` is made with it. It
comes **after `change` or `alert` and `note`, before `setup`**, so the thesis stays last. A wake-up
waits **at most 5 seconds** for it; a read that takes longer goes on in the background and is held
for the next attempt, and this attempt goes without `context`. It is absent too when the symbol's
bars cannot be read. The symbol is the fired alert's, else the setup's. The event in the log does not
hold it: an event says what happened, and the context, like the setup, is shown as it is when sent.

**`get_market_context`** answers the same object for 1 to 10 symbols, in the order asked, with
`{symbol, error}` for one that cannot be read. It is an ordinary MCP tool, on the same authenticated
path as every other, so a script that already holds a token (the 100eyes triage) can call it. A
replay server refuses it: the fixtures hold no funding or index.

### Near the stop

**A filled take is near its stop at the first 1m bar that takes it 0.8 R or more against the
position and leaves it open.** R and the excursion are the ones `mae_r` already has: the distance
from the fill price to the invalidation, before costs, read from the bar's low (long) or high
(short) as the scorer reads them. Concretely, the first bar whose excursion sets a new `mae_r` at or
below −0.8 while the setup stays `triggered`.

- **Once per setup.** The record keeps `near_stop_at`, that bar's open time, and is never told again,
  however often the price comes back and goes out.
- **A bar that stops it out says so with its status**, not also with a warning. The bar that fills it
  can be the one, and then logs the fill and the warning together.
- **Takes only.** A skip is not warned about, as no change of a skip wakes anyone.
- The scorer writes it as an entry of the setup's log, kind `near_stop`, by the server, with `mae_r`
  from/to and `near_stop_at`. That entry becomes an event of the owner's log, **`setup.near_stop`**,
  with `what: {mae_r, gap}` and the time of the bar, in the same transaction, as a status is.
- **It wakes the agent if the take is still `triggered` when it is written.** After the server was
  away, several bars are written at once; if one of them stopped the take, the status says more than
  the warning before it, which is then logged and not sent.
- Its wake-up has `"type": "setup.near_stop"` and `change: {mae_r, gap}`, then `context`, then the
  setup, which now shows `near_stop_at`.

0.8 R is the threshold Hermes asked for: near enough that it is rarely noise, with room left to act
before the stop.

### Consequences

- Good: a woken agent starts with the context it otherwise reads first, in the body it already gets.
  A receiver that renders a template can show it with one placeholder, `{context.line}`.
- Good: a take that goes wrong is seen before the stop, without an alert to keep in step with it.
- Good: nothing is renamed or moved; a receiver that ignores new fields and kinds is unaffected.
- Bad: a wake-up can be up to 5 seconds later than before when the context is not held. Wake-ups are
  minutes apart; their schedule and slots are unchanged.
- Bad: the context of a wake-up sent on a retry can differ from the first attempt's, as its setup can.
- Neutral: `setup.near_stop` events count toward the owner's 1000 kept events and the wake-up limits
  like any other.

## Validation

- Unit tests: the trend's three cases and its refusal with too few bars, the index bands, the line,
  the tool's argument checks; the near-stop rule (once, not on the stop bar, on the fill bar, not on a
  skip, stored and read back).
- A service test: a take filled and taken to −0.83 R logs `setup.near_stop` once at the bar's time and
  wakes the agent with `context` between `change` and `setup`; the same bars with a stop in one write
  log the warning and send only the statuses.
- On a server with the sample source, `get_market_context` answers every part but the index.
