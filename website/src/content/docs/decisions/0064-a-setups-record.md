---
title: "0064 — A setup states what it was made with, once, and the record is read back as counts"
description: "A setup can state features, a probability and a playbook version when it is made. They cannot change afterwards, and the setup's append-only log is what holds them. get_track_record counts closed setups, and their fixed exit variants, and judges nothing."
sidebar:
  order: 64
---

**Status:** Proposed
**Date:** 2026-11-11
**Amends:** [ADR-0059](/chartgen/decisions/0059-setups-paper-scored/) (a setup holds three
stated fields that no tool changes after it is made, and a tool counts closed setups)

## Context and Problem Statement

An agent stores setups, the server scores them on paper
([ADR-0059](/chartgen/decisions/0059-setups-paper-scored/)), and the server can wake the agent
([ADR-0061](/chartgen/decisions/0061-alerts-that-wake-the-agent/)). One thing was missing for an
agent that wants to get better at this.

It could not say, at the time, **what it saw and how sure it was**, in a form that can be
counted later. A thesis is text. And whatever it says must not be changeable once the outcome
is known, or the record proves nothing.

## Decision Drivers

- A record that can be rewritten after the outcome is no record.
- The server must not learn, or pretend to know, what the agent's fields mean.
- Counting must not turn into claims. A few setups, which overlap in time and share causes,
  support very little.
- Nothing unbounded, and no new feed, table or limit where an existing one does.
- A replay server must answer the same on every run.
- A build from before this must still start on a store that has these things.

## Considered Options

For what a setup states:

1. **Three optional fields on `create_setup`, fixed from then on.**
2. The same fields, revisable with a reason, as the thesis is.
3. A free JSON blob.

For reading the record back:

1. **One tool that counts, with filters and one grouping.**
2. The agent lists its setups and counts by itself.
3. The server computes a score, a calibration figure or a significance per group.

## Decision Outcome

Option 1 in each.

**Scope.** This decision was first written with two more parts, which land in changes of their
own: an alert that fires at the close of a bar (`bar_close`), which will be recorded with that
change, and the event log, which is in the amendment to
[ADR-0061](/chartgen/decisions/0061-alerts-that-wake-the-agent/).

### What a setup states

`create_setup` takes three optional fields:

| Field | What it holds |
|---|---|
| `features` | A flat object of at most 16 entries. Names are 1 to 32 characters of `a-z`, `0-9` and `_`. Values are a number, `true` or `false`, or a string of at most 32 characters. |
| `probability` | A number from 0 to 1: the agent's own estimate that the setup reaches its first target. |
| `playbook_version` | A string of at most 32 characters: a label for the rules the agent followed. |

**The server gives them no meaning.** It stores them, shows them, and counts by them.

**They are fixed when the setup is made.** `revise_setup` and `cancel_setup` refuse them by
name. The first entry of the setup's log, the one of kind `created`, holds them under
`changes`, and the store's triggers keep a log entry from being changed or replaced. **The
log is what counts:** at start the server takes the three fields from that first entry,
whatever the setup's record row says.

A setup made without them is stored and answered exactly as before. The fields are not
written when not stated, so no stored row and no answer of such a setup changes by a byte.
There is no migration.

### The record read back

`get_track_record` counts the caller's **closed** setups:

- Filters: `symbol`, `direction`, `decision`, `playbook_version`, and `from` and `to` on the
  time the setup was made.
- One grouping, `group_by`: `playbook_version`, `feature` (with `feature`, the name of one),
  `probability` in tenths, `symbol` or `direction`. A setup that stated none is in the group
  whose key is null.
- Per group and in all: `count`, `won`, `invalidated`, `expired`, `cancelled`, `with_result`,
  `total_r`, `mean_r`, `entered`, `mean_r_entered`, `gaps`, and under `probability` how many
  stated one, its mean, how many of those won, and that share; and the same three over the
  setups that were entered (`stated_entered`, `won_entered`, `observed_entered`).
- **Two means, because there are two honest readings.** A setup that was cancelled, expired
  or invalidated before its entry was reached has a result of 0 R
  ([ADR-0059](/chartgen/decisions/0059-setups-paper-scored/)). `mean_r` is over every result
  and so holds those zeros: what following every setup gave. `entered` counts the setups
  whose entry was reached and `mean_r_entered` is the mean over those: what the trades
  gave. The observed share under `probability` is over all that stated one, so it mixes the
  chance of a fill into a probability that is about reaching the first target once in. The
  share over the entered ones leaves that out; both are given, as the two means are.
- Probability groups are tenths, `floor(p × 10) / 10`, with a probability of 1 in the `0.9`
  group: ten groups of the same width, and no group of one value that a careful agent never
  states.
- **The exit variants**
  ([ADR-0066](/chartgen/decisions/0066-shadow-exit-variants/)), beside the total and not per
  group: per variant, how many of its positions closed, the total and mean R of those with a
  result, and how many are still open. A closed setup's variant can stay open for a while, so
  open ones are counted apart and not in the means. Only a filled setup has variants, so they
  are over fewer setups than the total, which is said wherever they are.
- **Each variant is paired with the canonical result over the same setups**
  (`canonical_total_r`, `canonical_mean_r`). The open variants are not a random part: an open
  v1 is a runner after T1, a winner, so v1's closed set leans to losers until those end, while
  the total holds every filled setup. A variant beside the total would read worse than it is;
  beside its own pair it compares like with like. The tool's description says so.
- A number a feature holds is grouped by its value: `1` and `1.0` are one group, written as
  `1.0`.

**It reports counts and sums and judges nothing.** No significance, no ranking, no score. A
mean is over the results there are: a setup without a result is counted and is not given one.

Bounds: the newest 500 closed setups are read, which is also how many an owner may keep. A
grouping that gives more than 50 groups is refused and not cut, since a part of a grouping
reads as the whole.

**It works on a replay server,** answered as of the call's `as_of` like every setup tool
there. The record of a replay is what an evaluation reads its result from, and it is the
same on every run.

### Consequences

- Good: what the agent believed when it made a setup cannot be changed once the outcome is
  known, and the copy that proves it is the one the store protects.
- Good: no migration, no new table, no new limit. Setups without the fields are untouched.
- Bad: the server cannot check that a feature means the same thing from one setup to the
  next. Keeping that stable is the agent's job, and `playbook_version` is its tool for it.
- Bad: counts invite conclusions. The tool's description and the guide say plainly that a
  small group says little and that setups are not independent trials.
- Bad: **if the first log entries cannot be read at start, the three fields are taken
  from the record rows,** with the line `[setups] cannot read the setups' first log
  entries` in the log. The rows are the copy an older build may have rewritten, so in that
  one case a setup can show no fields, or a row's. It was chosen over showing none: a
  store that answers the read of the rows and not this one is a store in trouble, and
  hiding the fields would silently move every setup into the group that stated nothing.
- Bad: **a build from before this, on a store that has these things:**
  - It reads a setup that states the three fields and does not show them. **When it writes
    that setup, after a revision or a status change, the record row loses them.** The log's
    first entry keeps them, and this build reads them from there at start, so they are back
    after a roll forward.
  - It does not know `get_track_record`.

## Pros and Cons of the Options

### Fields that can be revised with a reason

- Good: a typo can be corrected.
- Bad: so can a probability, after the result. The log would show it, but every reader of the
  record would have to check the log of every setup. Cancelling and making a new setup costs
  little and keeps the record simple.

### A free JSON blob

- Good: nothing to decide about shape.
- Bad: nothing to group by, and no bound on its size or depth other than the record's.

### The agent counts by itself

- Good: no new tool.
- Bad: 500 setups with their theses through the agent's context to get a handful of numbers,
  on every question.

### A score or a significance from the server

- Good: one number to look at.
- Bad: it would be wrong. Setups overlap in time, share a market and share causes, so they are
  not independent trials, and the server cannot know how they depend on each other.

## Validation

Unit tests check: the three fields stored, shown and in the first log entry; that the store
refuses to rewrite that entry; that they survive a restart and a status change; that a setup
without them shows none; that `revise_setup` and `cancel_setup` refuse them; their bounds;
that a record row without them, or with other values, is read with the log's values at start.
For the record: totals and each grouping against results read from the setups themselves,
the filters, the refusals, a setup without a result, another owner, the 50-group bound, and a
replay run twice; the setups that were entered apart from those that never traded, in the
means and in the observed share; and each exit variant's positions, closed and still open.

`tests/fixtures/setup_responses.json`, the answers of the setup tools pinned before this
decision, is unchanged, and the test that compares against it passes. A second file,
`track_record_responses.json`, pins a setup that states the fields and the record, and the
web app's types are tested against it, strictly: a field the server sends that the types do
not have fails the test.

When this was first written, with the event log and `bar_close`, three older builds were run
on a store that build wrote: `master` and both parts of the wake-up work, and each behaved as
described under Consequences. That was not run again for this part alone, which adds no table
and no migration.

Not validated: sign-in on, and a store with 500 closed setups.
