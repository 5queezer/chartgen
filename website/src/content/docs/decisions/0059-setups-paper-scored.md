---
title: "0059 — A setup is a stored record, scored on paper by the server"
description: "An agent can store a trading setup: a thesis, drawings and levels, owned by the user. The server moves its status from closed 1m bars of the shared Binance feed and scores it in R. Changes go to an append-only log. A replay server answers as of a time from fixture bars, with setups in a store of its own."
sidebar:
  order: 59
---

**Status:** Accepted; amended by [ADR-0068](/chartgen/decisions/0068-replay-over-real-history/) (a replay's fixtures may be real Binance history, written by `chartgen replay-fixtures`)
**Date:** 2026-11-11
**Amends:** [ADR-0054](/chartgen/decisions/0054-agent-drawings/) (a setup stores drawings on the
server) and [ADR-0042](/chartgen/decisions/0042-server-side-alerts/) (its limit of 16 watched pairs
counts the feeds of setups too)

## Context and Problem Statement

An agent can draw on a chart ([ADR-0054](/chartgen/decisions/0054-agent-drawings/)) and set a price
alert ([ADR-0042](/chartgen/decisions/0042-server-side-alerts/)), but it cannot record what it
expects to happen. Nothing ties a thesis to its levels, and nothing checks afterwards whether the
agent was right. A person cannot follow the agent's reasoning over time, and the agent's judgment
has no track record.

ADR-0054 rejected stored drawings (its option 2): a drawing session needed "server-side drawing
state per user and chart, ids, and a second round trip to see anything, for what one call does".
That holds for annotating one image. A setup is different, because it has to outlive the call: the
server scores it against prices that do not exist yet. The drawing model does not change. What
changes is that one kind of record now keeps drawings.

How is a setup stored so that it can be proven wrong, the agent cannot change its outcome, and an
evaluation can be repeated with the same answers?

## Decision Drivers

- A setup must be falsifiable when it is created: it names where it is wrong and when it runs out.
- The server decides the outcome. The agent cannot set, edit or erase one.
- One drawing model and one validator, those of ADR-0054.
- No exchange load that grows with the number of setups, as for alerts (ADR-0042).
- A setup belongs to the user ([ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/)).
- A replayed scenario gives the same answers on every run, and no bar after the asked time is used.
- Anyone with an account may sign in (ADR-0044), so every limit has to hold against a stranger.
- Room for an alert linked to a setup and for decision features, without building them.

## Considered Options

1. **A setup record that holds its drawings, in tables of its own, scored on the shared 1m feed.**
2. Stored drawings as a resource of their own (ADR-0054's option 2), with a setup naming drawing ids.
3. No stored setup: the agent keeps its own and asks a stateless tool to score one.
4. A setup as a bundle of server alerts.

## Decision Outcome

Chosen: option 1. The record, its checks and its scoring are in `src/setups.rs`, the store, tools
and feed in `src/setup_service.rs`, the replay in `src/replay.rs`. On only with `CHARTGEN_SETUPS=1`.
No order is ever sent.

### The record

`symbol`, `interval`, `direction` (`long` or `short`), `decision` (`take` or `skip`), `thesis`
(plain text, at most 2000 characters), `drawings` (at most 50, ADR-0054's shape, checked by
`drawings::parse` and `drawings::resolve` on the bars of the setup's symbol and interval, stored
with their resolved points), `entry` (`low`, `high`), `invalidation`, `targets` (1 to 4 prices,
nearest first), `expires_at` and `status`.

**Falsifiable by construction.** A setup with no invalidation or no expiry is refused. The
invalidation must be below the entry zone of a long and above that of a short, every target beyond
the zone on the other side, and the expiry ahead, at most 90 days. The call is checked as a whole:
any problem refuses it, each one is named with its field and a hint, and nothing is stored. A
missing or unreadable field does not hide what is wrong with the others: every check the readable
fields allow is made in the same answer, and a relation between two fields is judged only when
both could be read. A field
the server sets (`status`, `result_r`, `fill`, `exit`, `costs`) is refused by name.

**Least risk.** The invalidation must be at least 3 round-trip costs from the entry:
`3 × 2 × (fee + slippage)` of the price, 42 bps with the assumption below. A stop a hair from the
entry would turn any move into many R. A skip's bracket, which the server builds, is held to the
same floor (below).

### Status

`watching -> triggered -> won | invalidated | expired`, and `cancelled` by the agent with a reason.
Only closed 1m bars of the symbol move it, in time order, from the first bar that opens at or
after the setup was created. A bar seen before, as the feed sends after a reconnect, moves nothing.

- While `watching`, a bar that overlaps the entry zone fills the setup: at the bar's open if that
  lies in the zone, else at the zone's edge nearest the open.
- While `triggered`, a bar that reaches the invalidation ends it as `invalidated`, at the
  invalidation, or at the bar's open if it opened beyond it. Otherwise a bar that reaches the first
  target ends it as `won`, at the target. The other targets are kept for the chart.
- A bar that opens at or after `expires_at` ends it as `expired`, at that bar's open if filled.

**A bar does not say in which order it traded, so an ambiguous bar counts the worse outcome for
the agent.** A bar that reaches the invalidation and the target is the invalidation. A bar that
reaches the entry zone and the invalidation while the setup is watching is a fill and a stop, a
loss. The bar that filled a setup can stop it out and cannot win it. Two cases end unfilled, at
0 R, because the entry was never there to take: a bar that opens beyond the invalidation, and a bar
that reaches the invalidation without reaching the zone.

### The paper score

`R = (exit - fill) / |fill - invalidation|`, negated for a short, less
`(fill + exit) × (fee + slippage) / |fill - invalidation|`. The assumption is a 5 bps fee and 2 bps
slippage per side, so 14 bps for the round trip. It is one pair of constants, copied onto the setup
when it is created, and the agent cannot set it. A setup that was never filled scores 0.

**Skip.** A decision not to trade is stored with `decision: "skip"` and scored as if taken, so
skipped events are not lost. It takes no levels from the agent. The server's bracket is the same
for every skip: in at the next bar's open, invalidation 1 × ATR(14) from the last close, one
target 2 × ATR(14) from it, on the closed bars of the setup's interval. **Where 1 ATR is less than
the least risk above, the invalidation is that far and the target twice that.** On a 1m chart one
ATR can be 5 bps against 14 bps of costs: such a skip would lose whatever the market did, and every
skip would read as right. The record says which rule set its levels, in `bracket`: `agent` for a
setup with the agent's levels, `atr` or `floor` for a skip, so a track record can tell them apart.
The bracket is measured from the last close when the skip is stored, and the fill is the next
bar's open, so the risk at the fill can be a little under or over the bracket's: 35 bps against a
42 bps floor in one review run. The levels are not moved to the fill: they are fixed when the
skip is made, as every setup's are, and R is measured on the risk at the fill.
A skip has an expiry after 24 of
those bars, or after the 90 days every setup is bound by if that is sooner (on a weekly chart 24
bars would be 168 days). A skip cannot be cancelled and its bracket cannot be revised, so its
expiry is what frees its place.

### Changes and the log

Nothing is edited in place. The log is a table of its own that is only ever inserted into, written
in one transaction with the record. The database enforces it: triggers abort an `UPDATE`, a
`DELETE` and a replacing `INSERT` on that table, so the server's own code is not the only thing
keeping the log whole. Dropping the table, or editing the file by other means, is still possible
for whoever holds the file. An entry holds its number, the time, who made it (`agent` or
`server`), its kind, the reason, and what changed as `from` and `to` per field. The server's status
moves are entries too, so the log is the whole history.

- While `watching`, the agent may revise the thesis, drawings, levels and expiry. Once filled, only
  the thesis and drawings: moving a stop or a target then would improve a score after the fact.
- **A change counts from the first bar that opens at or after it.** The bar forming when it was
  sent still meets the setup as it was. Without this an agent watching live ticks could cancel, or
  move a level, inside the bar that fills and stops it, and the loss would never exist. Levels not
  yet in effect are shown under `pending`. If the setup is filled or closed first, the log says the
  change did not take effect.
- Cancelling a watching setup ends it at 0 R. Cancelling a filled one closes it at the next bar's
  open and scores it, so a cancel cannot remove a loss.

### Ownership

The token's user, or with sign-in off its OAuth client, as for server alerts
(`owner_for_token`). A setup of another owner reads the same as one that does not exist.
`CHARTGEN_CLAIM_OWNER` does not collect setups: one owned by a client stays with that client.
While setups are on, a client that owns one is not evicted
([ADR-0036](/chartgen/decisions/0036-own-origin-invalid-client-redirect/)), as an owner of scripts
or alerts is not: its setups would go on being scored and counted with nobody able to read them.

### The feed, and what happens when the server was away

Binance symbols only. A setup reads its symbol's 1m feed through `ChartHub::watch_feed`, the one
WebSocket every chart and alert of that symbol shares. No polling is added.

A setup that is behind, after a restart, a deploy or a dropped feed, first gets the bars it missed:
one read of at most 1000 1m bars per symbol, through the Binance gate, when the next closed bar
arrives. Where that read fails or does not reach far enough, the log gets a `gap` entry naming the
stretch that was not scored, and scoring goes on after it. The setup's `gap` field is then true,
in `get_setup` and in `list_setups`, so a result with a gap is marked as such where it is read
rather than presented as complete.

### Limits

| What | Limit | Past it |
|---|---|---|
| Open setups (watching or triggered) | 20 per owner, 200 on the server | refused |
| Setups, closed ones included | 500 per owner, 20,000 on the server | refused; nothing old is dropped |
| Symbols with open setups | 5 per owner (`CHARTGEN_SETUP_SYMBOLS_PER_OWNER`) | refused |
| Watched pairs, alerts and setups together | 16 on the server (ADR-0042) | refused |
| Revisions by the agent | 50 per setup | refused |
| Creates, revisions and cancels | 10 per owner per minute | refused |
| One setup | 32 KiB | refused |
| One log entry | 64 KiB | refused |
| Setups and their logs | 4 MiB per owner, 256 MiB on the server | refused |

- **Bytes bound the store, not counts.** An entry holds `from` and `to`, so one revision of 50
  drawings is tens of kilobytes. Whatever the counts allow, one owner holds at most 4 MiB.
- **Closed setups are kept.** They are the track record, so no setup is deleted for its owner's
  inactivity, as ADR-0042 deletes alerts. An open one ends by its own expiry.
- **Pairs.** A setup watches `(symbol, 1m)`, the pair a live alert on that symbol uses, so an alert
  and a setup on one symbol are one pair and one WebSocket. Setups do not count against ADR-0042's
  2 pairs per owner: an agent watches many symbols, and each skip holds one until it resolves. They
  have their own limit of 5 symbols per owner. The server's 16 counts both, in both directions:
  `check_feed_room` of the alerts now counts the pairs setups hold. At the 16, with an owner under
  its own limit, a new setup or alert on a new symbol is refused with the limit named, and existing
  ones keep their feeds. The hub's own limits (40 new feeds a minute, 96 connected) apply unchanged.
- **Setups do not need `CHARTGEN_SERVER_ALERTS=1`.** They hold their own leases on the hub, and
  requiring alerts would open alert creation on a server that only wants setups. They follow the
  same allowlist, `CHARTGEN_SERVER_ALERT_CLIENTS`: where it names accounts, only those may create
  setups. Unset or `*`, anyone who can get a token may. The list is read at startup. From the
  restart that reads a list without an owner, that owner's setups hold no feed and are not scored,
  as ADR-0042 stops its alerts; they stay readable. Back on the list they are behind like any
  setup the server was away from: the missed bars are read if in reach, else named as a gap.
  **An unlisted owner's setup still expires.** An expiry is a time, and without bars nothing else
  would end it, so it would count against the server's 200 open setups for ever. Past its
  `expires_at` the server marks it `expired` with `gap` set. Never filled, it scores 0. Filled, it
  closes with **no result and no exit**: no bar was read, and a price is not invented. This is
  the one case of a closed setup whose `result_r` is null.

### Tools

`create_setup`, `revise_setup`, `get_setup`, `list_setups` and `cancel_setup`, listed only where
setups run, on the HTTP transport. Each returns the setup; all but the list include its log.

### Replay

`CHARTGEN_REPLAY_FIXTURES=<dir>` starts a server that answers only from the 1m bars in that
directory, as of the time each call names. It is for an evaluation that can be run again.

- **Who can select it.** The variable is read at startup, so no request can turn it on. A replay
  refuses to start with `--trade`, `--mcp`, without `--serve`, with `CHARTGEN_SERVER_ALERTS=1`,
  `CHARTGEN_SETUPS=1` or a sign-in provider, so a replay process is never also a live one.
- **`as_of`.** The setup tools and `generate_chart`, `inspect_chart`, `get_indicators` and
  `get_quotes` take `as_of` and require it. On any other server it is refused.
- **The cut.** One place reads bars: a 1m bar counts only if it **closed** at or before `as_of`,
  and every other interval is built from those, whole bars only. Nothing downstream can look
  ahead, because nothing later reaches it. `end_time` cannot be the cut: it is exclusive on a bar's
  open time, so it returns the bar that contains it whole, with a high and low from after it. It
  keeps its meaning for paging. `CHARTGEN_DATA_SOURCE=sample` cannot hold a chosen scenario, and
  its forming bar moves with the wall clock.
- **Fails closed.** A read with no time gets nothing, not the newest bar. Inside a call "now" is
  `as_of` for every chart tool and every indicator: the server and the library read one clock, so
  a fixture may be dated in the past or the future. Indicators that read more than the bars they are given (funding
  rate, open interest, long/short ratio, fear and greed) are refused, by a flag on the indicator,
  not a list. `get_footprint`, `get_tpo`, `search_symbols`, the subscriptions and the alert tools
  are refused: they read what 1m fixtures do not hold.
- **One clock per process.** Every `as_of` call, from any tool and any caller, moves the server's
  one clock, and none may name an earlier time. An agent cannot read the chart at a later time and
  then act at an earlier one. The clock is not per owner: registration is open on a replay and
  every client is a new owner, so a second client would be a way round it. One process is one
  run; parallel runs are separate processes.
- **Quotes.** `last` is the close of the newest 1m bar closed by `as_of`. The 24-hour fields come
  from the 1m bars of the 24 hours before it: `prev_close` is the open of the first, `high`, `low`
  and `volume` are over them all. With less than 24 hours of fixture they cover what there is.
- **Isolation.** A replay server opens no database file. Its setups, and the OAuth clients that
  register with it, live in memory for that process, so it cannot touch or migrate `chartgen.db`.
- **Same answers every run.** The clock is `as_of`, never the wall clock. Ids count per owner
  (`setup-000001`). Each call first moves the caller's open setups through the bars closed by
  `as_of`, oldest setup first, with the function the live feed uses. Lists are sorted. The
  per-minute limit, which runs on the wall clock, is off.

### Why not the others

Option 2 brings back a general drawing store that nobody asked for, and two lifecycles to keep in
step. Option 3 lets the agent choose what is scored and when, so the track record would be the
agent's own account of itself. Option 4 does not fit: an alert has one level, keeps its baseline in
memory and loses it on a restart on purpose, and it fires where a setup has to resolve.

### Consequences

- Good: an agent's decisions get a track record it cannot edit.
- Good: stored drawings are the web app's, so a person can open them.
- Good: an evaluation scenario can be run again and compared answer by answer.
- Bad: a status moves up to a minute after the price did, because only closed 1m bars count.
- Bad: an ambiguous bar is scored as the worse outcome, which understates a setup that in fact
  reached its target first.
- Bad: the server now stores drawings, which ADR-0054 avoided.
- Bad: each open setup's record is written once a minute, to keep the bar it is scored through
  across a restart: up to 200 rows a minute at the server's limit, each at most 32 KiB. Fine at
  these sizes; a column of its own would avoid rewriting the record.
- Bad: a setup scored across a gap is incomplete. The log says so; nothing fills it in later.
- Bad: **with setups on and the allowlist unset, strangers can hold the server's 16 watched
  pairs.** Four accounts with 5 symbols each are enough, and with sign-in off client ids are free,
  so one party can do it. That refuses the operator's own agent a new symbol and, because alerts
  count the pairs of setups, new alerts on new symbols too. As ADR-0042 says for alerts: **on a
  public server, list the accounts you trust in `CHARTGEN_SERVER_ALERT_CLIENTS`.** The 16 is a
  constant; an operator cannot raise it without building the server.
- Bad: a setup created with sign-in off belongs to an OAuth client and is out of reach once
  sign-in is turned on, as ADR-0044 describes for unclaimed rows.
- Neutral: only the first target decides `won`.
- Neutral: a later step can add an alert linked to a setup, decision features, a probability and a
  playbook version as optional fields of the record and new kinds of log entry. None of it is built.

### Testing

The record's rules are unit tests on bar sequences (`src/setups/tests.rs`), one named test for
each ambiguous-bar case. `src/setup_service/tests.rs` covers the tools, ownership, every limit, a
restart, missed bars and a gap, a stored row that cannot be read, and the hub's feed through the
real hub. For the replay it runs one scenario twice and compares every answer byte for byte, and
compares answers from two fixtures that differ only after `as_of`, over every registered indicator
and chart style, with a control that a difference before `as_of` does change them. The same tests
check that no call reached for the upstream HTTP client. They count uses of that client on the
calling thread; they do not watch sockets. `tests/replay_process.rs` starts a replay server as a
process with a home of its own and fails if any file appears there. No test of setups depends on
the time of day: bars come from the test or are placed against the setup's own levels, and
`CHARTGEN_TEST_CLOCK_SHIFT_MS` runs the suite at another time to show it.
