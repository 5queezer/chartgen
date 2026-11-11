---
title: "0064 — A setup states what it was made with, once, and the record is read back as counts"
description: "A setup can state features, a probability and a playbook version when it is made. They cannot change afterwards, and the setup's append-only log is what holds them. get_track_record counts closed setups and judges nothing. A server alert can fire at the close of a bar."
sidebar:
  order: 64
---

**Status:** Accepted
**Date:** 2026-11-11
**Amends:** [ADR-0059](/chartgen/decisions/0059-setups-paper-scored/) (a setup holds three
stated fields that no tool changes after it is made, and a tool counts closed setups) and
[ADR-0042](/chartgen/decisions/0042-server-side-alerts/) (an alert can fire at the close of a
bar, with no level)

## Context and Problem Statement

An agent stores setups, the server scores them on paper
([ADR-0059](/chartgen/decisions/0059-setups-paper-scored/)), and the server can wake the agent
([ADR-0061](/chartgen/decisions/0061-alerts-that-wake-the-agent/)). Two things were missing for
an agent that wants to get better at this.

It could not say, at the time, **what it saw and how sure it was**, in a form that can be
counted later. A thesis is text. And whatever it says must not be changeable once the outcome
is known, or the record proves nothing.

And it could only be woken by a price. A common request is "wake me when this 4h bar has
closed", whatever the price is then.

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

For the alert:

1. **A new condition on the existing alert, `bar_close`.**
2. A new kind of object with its own tools and limits.

## Decision Outcome

Option 1 in each.

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
  stated one, its mean, how many of those won, and that share.
- **Two means, because there are two honest readings.** A setup that was cancelled or expired
  before its entry was reached has a result of 0 R
  ([ADR-0059](/chartgen/decisions/0059-setups-paper-scored/)). `mean_r` is over every result
  and so holds those zeros: what following every setup gave. `entered` counts the setups
  whose entry was reached and `mean_r_entered` is the mean over those: what the trades
  gave. The observed share under `probability` is over all that stated one.
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

### An alert at the close of a bar

`condition: "bar_close"` on `create_server_alert` fires when a bar of the alert's `interval`
closes, whatever the price. It is an alert like any other: the same record, the same limits
for alerts and for watched pairs, the same `trigger` (`once`, or `every` close), and the
same `webhook_url`, `push`, `notify_agent` and `setup_id`.

- It needs an `interval` and takes no `level`. It is always evaluated at the close.
- An alert cannot change into this kind or out of it.
- In answers, in the webhook body and in the event log its `level` is null.
- **A close the server did not see is not fired afterwards.** After a restart or an outage
  the alert fires at the next close the server sees, with that bar's time. A `once` alert
  that has not fired stays armed until then.
- It never fires twice for one bar, also when the feed sends a bar again after a restart,
  and never for a bar that closed before the alert was made.
- A replay server has no alerts, so none of this exists there.

### What was left out: a switch for status wake-ups

A setup's status changes wake the agent when the setup is one it took, with no setting on
the setup ([ADR-0061](/chartgen/decisions/0061-alerts-that-wake-the-agent/)). A per-setup
switch (also for a skip, or not for a take) was considered again here and left out. It would
be one more field on the record, and unlike the three above it would have to be changeable,
which is a second kind of field to explain. Nobody has needed it yet. Every status change is
in the event log either way.

### Consequences

- Good: what the agent believed when it made a setup cannot be changed once the outcome is
  known, and the copy that proves it is the one the store protects.
- Good: no migration, no new table, no new limit. Setups without the fields are untouched.
- Good: an alert at a close reuses the feed and the limits alerts already have.
- Bad: the server cannot check that a feature means the same thing from one setup to the
  next. Keeping that stable is the agent's job, and `playbook_version` is its tool for it.
- Bad: counts invite conclusions. The tool's description and the guide say plainly that a
  small group says little and that setups are not independent trials.
- Bad: a wake-up "at the close" can be missed for good while the server is down. The agent
  needs its own schedule besides.
- Bad: **if the first log entries cannot be read at start, the three fields are taken
  from the record rows,** with the line `[setups] cannot read the setups' first log
  entries` in the log. The rows are the copy an older build may have rewritten, so in that
  one case a setup can show no fields, or a row's. It was chosen over showing none: a
  store that answers the read of the rows and not this one is a store in trouble, and
  hiding the fields would silently move every setup into the group that stated nothing.
- Bad: **a build from before this, on a store that has these things:**
  - It starts. It skips each `bar_close` alert with the line
    `[alerts] skipping an unreadable alert`, runs the others, and leaves the skipped
    alert's row alone. After a roll forward the alert is back.
  - It reads a setup that states the three fields and does not show them. **When it writes
    that setup, after a revision or a status change, the record row loses them.** The log's
    first entry keeps them, and this build reads them from there at start, so they are back
    after a roll forward.
  - It does not know `get_track_record`.
- Neutral: the stored alert record gains one optional field, the bar a `bar_close` alert last
  fired for. A `bar_close` alert stores a level of 0, which is never shown.
- Neutral: `level` is no longer required by `create_server_alert`'s schema, because one
  condition takes none. The server still refuses a price alert without it.

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

### A new kind of object for timed wake-ups

- Good: no `level: null` on an alert.
- Bad: a second set of tools, limits, deliveries and restart rules for something that is an
  alert in every respect but its condition.

## Validation

Unit tests check: the three fields stored, shown and in the first log entry; that the store
refuses to rewrite that entry; that they survive a restart and a status change; that a setup
without them shows none; that `revise_setup` and `cancel_setup` refuse them; their bounds;
that a record row without them, or with other values, is read with the log's values at start.
For the record: totals and each grouping against results read from the setups themselves,
the filters, the refusals, a setup without a result, another owner, the 50-group bound, and a
replay run twice. For the alert: that it fires at each close and at nothing else, takes no
level and keeps its kind, counts in the owner's alert limit, and after a restart fires at the
next close it sees and not twice for one bar.

`tests/fixtures/setup_responses.json`, the answers of the setup tools pinned before this
decision, is unchanged, and the test that compares against it passes. A second file,
`track_record_responses.json`, pins a setup that states the fields and the record, and the
web app's types are tested against it. One browser test puts an agent's `bar_close` alert
into the web app's list: it is shown without a price, and the dialog that edits a level does
not open for it.

Three older builds were run on a store this build wrote: `master` and both parts of the
wake-up work. Each behaved as described under Consequences.

Not validated when this was written: a real feed across a reconnect for `bar_close`, sign-in
on, and a store with 500 closed setups.
