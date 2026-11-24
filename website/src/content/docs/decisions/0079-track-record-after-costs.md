---
title: "0079 — The record says a result's sign after costs, groups by tag prefix and skip reason, and asks for a playbook version once one is used"
description: "A setup's status stays as it is: won means the first target was reached. Beside it, net says whether the result after costs was a gain, a loss or even, and get_track_record counts net_gain and net_loss. get_track_record groups by tag prefixes and by a new stated field, skip_reason. An owner whose newest setup states a playbook_version is refused a setup without one."
sidebar:
  order: 79
---

**Status:** Accepted
**Date:** 2026-11-24
**Amends:** [ADR-0064](/chartgen/decisions/0064-a-setups-record/) (a fourth stated field,
two more groupings, two more counts, and a rule for `playbook_version`). The statuses of
[ADR-0059](/chartgen/decisions/0059-setups-paper-scored/) stay as they are.

## Context and Problem Statement

An agent client that reviews its own track record reported four things that led it to wrong
conclusions or that it could not ask.

1. **`won` with a negative result.** A setup is `won` when a 1m bar reaches its first target.
   Its `result_r` is the move from the fill to the exit in R, less the assumed fee and slippage
   on both sides ([ADR-0059](/chartgen/decisions/0059-setups-paper-scored/)). When the target
   is nearer to the fill than the round-trip costs, the setup is `won` and its result is
   negative. The client counted such setups as wins. This happens most often on a skip. A
   skip's bracket is set around the close of the last bar of its interval, and the skip fills
   at the open of the next 1m bar. If the price has moved toward the target in between, the
   fill can land almost on the target. A take can do the same when its target sits just
   beyond a wide entry zone.
2. **No grouping by tag.** Tags are the client's own labels, for example `trigger:alert-…` and
   `trigger:scan-…` for what made it look. `list_setups` filters by tag, but `get_track_record`
   cannot count per tag, so the client cannot compare its triggers.
3. **No field for why a skip was skipped.** The client writes it into `features.skip_reason`.
   That takes one of the 16 features an agent may state, and the client already uses all 16.
   It is also a convention that only this client knows.
4. **A missing `playbook_version`.** `playbook_version` is optional. The client forgot it on two
   setups. Those setups sit in the `null` group, and the server did not catch the mistake.

The status set and its values are stored, are part of the wire contract, and live agent
clients rely on them. Their skills cannot be updated in the same step as a deploy, so a
change must add fields and must not rename or remove any.

## Decision Drivers

- An agent reading its record must be able to tell a target hit from a gain after costs,
  without doing the arithmetic itself.
- No stored value changes meaning, and no field is renamed or removed.
- `get_track_record` keeps reporting counts and sums and judges nothing (ADR-0064).
- What a setup is made with stays fixed, and the first log entry holds it (ADR-0064).
- The server invents no value an agent did not state.

## Considered Options

### Won after costs

1. **A new status**, such as `target_hit_net_loss`, or a split of `won`.
2. **Redefine `won`** as "closed with a positive result".
3. **A derived `net` beside the status**, plus `net_gain` and `net_loss` counts in the record.
4. Documentation only.

### Grouping by tag

1. **`group_by: tag` with a list of `tag_prefixes`**: each prefix is a group.
2. `group_by: tag` with one prefix, each full tag that starts with it a group.
3. Every tag a group.

### Skip reason

1. **A stated field `skip_reason`**, on a skip only, with `group_by: skip_reason`.
2. Allow more than 16 features, or exempt `skip_reason` from the count.
3. Leave it in `features`.

### Missing playbook version

1. **Refuse a setup without `playbook_version` when the owner's newest setup states one.**
2. Make `playbook_version` required on every setup.
3. A default, such as the owner's last version, or a fixed word.
4. Leave it.

## Decision Outcome

### `net` beside the status (option 3)

Every setup answer, from `get_setup`, `list_setups` and the others, has **`net`**: `gain` when
`result_r` is above 0, `loss` below 0, `even` at 0, and `null` while there is no result. It is
derived from `result_r` when the answer is made and is not stored. `get_track_record` counts
**`net_gain`** and **`net_loss`** in the total and in each group, next to `won`.

`won` keeps its meaning: the first target was reached. It is what a stated `probability` is
compared with ("the chance that this setup reaches its first target", ADR-0064), and that
comparison would break if `won` meant something else. A new status (option 1) would reach every
reader of the status at once: list filters, the web app, the event log, and clients that branch
on `won`. Redefining `won` (option 2) would silently change stored history. Documentation alone
(option 4) leaves every reader to do the arithmetic, which is how the mistake was made.

The skip bracket's anchor, which causes most of these setups, is not changed here. That would
change how skips are scored, and it needs a decision of its own (see Consequences).

### Tag prefixes (option 1)

`get_track_record` takes `group_by: "tag"` with **`tag_prefixes`**, a list of 1 to 10 strings.
Each prefix is a group, and its key is the prefix. A setup is in the group of each prefix that
one of its tags starts with, and counts once in that group even when several of its tags match.
A setup with no matching tag is in the `null` group. A whole tag is a prefix of itself, so
listing tags groups by exact tags.

**Groups can overlap**: a setup tagged `trigger:alert-x` and `trigger:scan-y` is in both
groups, so the groups need not add up to `total`. The tool says so. The other groupings give
each setup one key. Tags do not work that way: a setup can carry several tags, and the client's
setups do. One prefix with each full tag as a group (option 2) would split variants that belong
together, for example `trigger:scan-4h` and `trigger:scan-1h`, and would take a call per
prefix. Every tag as a group (option 3) would exceed the 50-group cap on any real set of tags.
Duplicate prefixes are dropped, and the answer echoes `tag_prefixes` as they were sent.

### `skip_reason` (option 1)

`create_setup` takes **`skip_reason`**, a string of 1 to 32 characters. It is a label, such as
`correlation-limit`, for counting; the explanation in words stays in the thesis. It is refused
on a take. It is a stated field like the other three: written to the first log entry, read back
from there at start, and refused by `revise_setup`. `get_track_record` groups by it with
`group_by: "skip_reason"`, and a setup without one is in the `null` group. It does not count
against the 16 features.

Raising the feature limit (option 2) would fix the room but not the convention, and every
client would invent its own name. Leaving it in `features` (option 3) is the state the client
reported. Setups that already have `features.skip_reason` keep it there, and
`group_by: feature, feature: skip_reason` still reads them. The server does not copy those
values into the new field, because what a setup was made with stays as it was made.

### `playbook_version` once used (option 1)

When the owner's newest setup states a `playbook_version`, `create_setup` refuses a setup
without one. The problem is on `playbook_version`, the hint names the newest version, and
nothing is stored. An agent that follows no playbook for a setup states a word such as `none`.
An owner that never stated a version is not affected, and other owners' setups do not count.

Requiring it on every setup (option 2) would break every client that does not use playbook
versions, which the field was made optional for. A default (option 3) is a value the agent did
not state. The owner's last version is wrong right after a bump, which is when a mistake is most
likely, and a fixed word looks like a real version in the record. Leaving it (option 4) is what
the client reported. The refusal catches the mistake at the one moment it can still be fixed,
and only for an owner who uses the field. Setups that already have no version stay in the
`null` group.

### Consequences

- Good: an agent reads a gain or a loss after costs from `net` and from `net_gain` and
  `net_loss`, without changing what `won` and the stored status mean.
- Good: triggers or any other tag families can be compared in one call, and skips can be
  counted by why they were skipped, without using a feature slot.
- Good: an owner that uses playbook versions cannot make a setup without one by accident.
- Neutral: every setup answer is about 13 bytes longer (`"net":"gain"`). The record's counts
  are two fields longer.
- Neutral: with `group_by: tag`, group counts can add up to more than `total`. The tool
  description and the reference say so.
- Bad: an owner whose newest setup states a version now gets an error where a setup used to be
  stored without one. The agent has to send the call again with a version. This is the point
  of the rule, and it is still a change of behavior for a live client.
- Open: a skip's bracket is set around the last close of its interval, but the skip fills at
  the next 1m open. When the price has moved in between, the distances in R from the fill are
  not the 1 ATR and 2 ATR the tool describes, and a skip can be `won` from almost no move.
  Anchoring the bracket on the fill would change how skips are scored, and needs its own
  decision.

## Validation

- A take whose first target is 0.01 % above its fill closes `won` with a negative `result_r`,
  its `net` is `loss`, and the record counts it in `won` and in `net_loss`. An open setup's
  `net` is `null`.
- In the record test, two setups that reached a target 5 % away and one that was stopped count
  as two `net_gain` and one `net_loss`.
- Three skips with `skip_reason` and tags group by `skip_reason` (`null` and the label) and by
  tag prefix. A skip with tags under two prefixes is in both groups, and the groups add up to
  more than `total`. `skip_reason` is refused on a take, above 32 characters, and by
  `revise_setup`. It is in the first log entry.
- `get_track_record` refuses `group_by: tag` without `tag_prefixes`, `tag_prefixes` without
  `group_by: tag`, an empty list, an empty prefix and a string in place of a list.
- An owner whose newest setup states `v1` is refused a setup without a version, and the problem
  names `v1`. `none` is accepted, and a missing version is then refused again. An owner that
  never stated one, and another owner, are not refused.
- The pinned replay answers that the web types are tested against include `net`,
  `skip_reason`, `net_gain`, `net_loss` and the two new groupings.
