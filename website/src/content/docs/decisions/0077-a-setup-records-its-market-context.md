---
title: "0077 — A setup records the market context it was made in"
description: "create_setup adds three server-owned features to every setup: ctx_trend_4h, ctx_trend_1d and ctx_fear_greed, read from the symbol's market context when the setup is made, and unknown where there is none. An agent's own feature may not start with ctx_. They are fixed as an agent's features are, and get_track_record groups by them as it does by any feature."
sidebar:
  order: 77
---

**Status:** Accepted
**Date:** 2026-11-15
**Amends:** [ADR-0064](/chartgen/decisions/0064-a-setups-record/) (`features` also holds three
features the server writes, and the `ctx_` prefix is the server's)
**Uses:** [ADR-0072](/chartgen/decisions/0072-wake-up-context-and-near-stop/) and
[ADR-0073](/chartgen/decisions/0073-no-context-for-a-symbol-not-trading/) (the market context,
and when a symbol has none)

## Context and Problem Statement

[ADR-0064](/chartgen/decisions/0064-a-setups-record/) lets an agent state `features` when it
makes a setup, and `get_track_record` can group closed setups by one of them. The question an
agent most often wants to ask of its record is how its setups did in each kind of market: in an
uptrend, in a downtrend, in fear or in greed. To ask it, the agent has to type the regime into
its own features on every setup.

That is inconsistent and incomplete. One setup says `trend: "up"`, the next
`trend_4h: "bullish"`, a third forgets it. Setups that do not use the same name and the same
words cannot be grouped together, and a setup made without it cannot be grouped at all. Yet the
server already reads the regime: `get_market_context`
([ADR-0072](/chartgen/decisions/0072-wake-up-context-and-near-stop/)) answers a symbol's trend
on 4h and on 1d and the Fear & Greed index, with the same code for every caller.

## Decision Drivers

- Every setup should be groupable by its market regime without the agent doing anything.
- The values must be the same words for every setup, in the vocabulary the context already
  uses, so that a group means the same thing across setups and agents.
- Making a setup must not fail or wait long because the context could not be read.
- An absence must be recorded as an absence, the same way every time, not guessed and not
  confused with an agent's own value.
- What a setup was made with stays fixed once it is made (ADR-0064), and the log holds it.
- `get_track_record` should not grow a second way of grouping.

## Considered Options

1. **Server-owned features under a reserved prefix** in the setup's existing `features`.
2. A new field on the setup, such as `context`, with its own grouping in `get_track_record`.
3. Leave it to the agent, and only document a naming convention.

## Decision Outcome

Option 1. Option 2 would add a field, a second stored copy in the log and a new kind of
grouping for what `features` and `group_by: feature` already do. Option 3 is what exists, and
is the inconsistency this decision removes.

### The features

When `create_setup` makes a setup, the server reads the market context of its symbol, as
`get_market_context` does (the same code and the same one-minute cache), and adds three
features:

| Feature | Value | Read from the context's |
|---|---|---|
| `ctx_trend_4h` | `up`, `down` or `mixed` | `trend["4h"]` |
| `ctx_trend_1d` | `up`, `down` or `mixed` | `trend["1d"]` |
| `ctx_fear_greed` | `extreme fear`, `fear`, `neutral`, `greed` or `extreme greed` | `fear_greed.label` |

The trend is the price against the 20 and 50 EMAs of that interval; the Fear & Greed band is
the label the context gives the index. The price, funding and long/short split are not
recorded: they are numbers that rarely repeat, not a regime to group by.

### When there is no context

A feature the server could not read is the string **`unknown`**. All three are written on every
setup, so a setup either holds a regime or says it did not have one, and `unknown` is a group of
its own in `get_track_record`. That is:

- all three on a **replay server**, which has no market context (and where the answer must be
  the same on every run);
- all three on a symbol that has **no context** under
  [ADR-0073](/chartgen/decisions/0073-no-context-for-a-symbol-not-trading/), such as one that
  is not trading, or when the read **failed**;
- all three when the read takes longer than **10 seconds**. The read goes on in the
  background and is held for the next caller; the setup is made without it;
- one trend when the context has too few bars for it, as on a newly listed symbol;
- `ctx_fear_greed` on a symbol that is **not on Binance**, which the context gives no Fear &
  Greed index.

`create_setup` succeeds in each case. A setup made before this decision has none of the three,
so its group key is `null`, as for any feature a setup does not hold. That keeps "made before"
apart from "made without a context".

### The prefix is the server's

A feature name of the agent's that starts with **`ctx_`** is refused, with a problem on
`features.<name>`, and the setup is not made. The server could instead overwrite the agent's
value, or let the agent's win, but either silently changes what one of them said, and a
record whose values could come from either is no record. Refusing is loud and costs an agent a
rename. The whole prefix is reserved, not only the three names, so that the server can add a
context feature later without colliding with one an agent already uses.

**The three do not count against the 16 features an agent may state.** The limit is checked on
the agent's own features alone, and the server's are added after that check, so an agent that
states all 16 keeps all 16 and the setup holds 19. The server's are bounded on their own: a
fixed set of three names, each with a value of at most 13 characters. A later context feature
would be added to that set by a decision of its own, never by taking room from the agent.

### Fixed once made

The three are part of `features`, so ADR-0064 holds for them as for an agent's: they go into
the first entry of the setup's log, which the store keeps append-only; at start the server
takes them from that entry; `revise_setup` and `cancel_setup` refuse `features`. A revision
does not read the context again. The record says what the market was when the decision was
made, not later.

### Grouping

`get_track_record` groups by any feature already, so `group_by: "feature"` with
`feature: "ctx_trend_4h"` counts closed setups per 4h trend, with no change to the tool.

### Consequences

- Good: every new setup can be grouped by the regime it was made in, in the same words, with no
  effort from the agent.
- Good: no new field, table, tool or grouping; the log and the store keep the values as they
  keep the agent's features.
- Neutral: `features` is no longer only what the agent stated. A setup made without features
  now answers with the three server features, where before it answered with none. Its answer
  and its log's first entry change accordingly; probability and playbook version are still
  absent when not stated.
- Neutral: making a setup on a live server reads the context, which is usually a cache hit
  and otherwise up to three bar requests and, on a Binance symbol, the futures series. It
  waits at most 10 seconds for them.
- Bad: an agent that already names a feature `ctx_...` must rename it.

## Validation

- A unit test reads the three features from a full context, a partial one and none, and gets
  `unknown` for each part that is missing.
- On a live test server a new setup holds both trends in the context's words, equal to what
  `get_market_context` reads for the symbol, and `unknown` for the Fear & Greed band, which the
  sample source does not have; a revision leaves them as they were.
- A setup on a replay server holds `unknown` for all three, and the pinned replay answers the
  web types are tested against show them.
- `create_setup` refuses `ctx_trend_4h` and any other `ctx_` name as a feature of the agent's.
- A setup with the 16 features an agent may state is made, and holds those 16 and the
  server's three; 17 of the agent's are still refused.
- `get_track_record` with `group_by: feature` and `feature: ctx_trend_4h` groups closed setups
  under the trend values, and the groups add up to the closed total.
