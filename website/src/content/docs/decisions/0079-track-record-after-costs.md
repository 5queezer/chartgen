---
title: "0079 — The record says a result's sign after costs, groups by tag prefix and skip reason, and counts a setup without a playbook version as unversioned"
description: "A setup's status stays as it is: won means the first target was reached. Beside it, net says whether the result after costs was a gain, a loss or even, and get_track_record counts net_gain and net_loss. get_track_record groups by tag prefixes and by a new stated field, skip_reason. A setup without a playbook_version is made as before, grouped and filtered as unversioned, and create_setup warns when the owner's newest setup stated one."
sidebar:
  order: 79
---

**Status:** Accepted
**Date:** 2026-11-24
**Amends:** [ADR-0064](/chartgen/decisions/0064-a-setups-record/) (a fourth stated field,
two more groupings, two more counts, and the group of a setup without `playbook_version`). The statuses of
[ADR-0059](/chartgen/decisions/0059-setups-paper-scored/) stay as they are.

## Context and Problem Statement

An agent client that reviews its own track record reported four things that led it to wrong
conclusions or that it could not ask.

1. **`won` with a negative result.** A setup is `won` when a 1m bar reaches its first target.
   Its `result_r` is the move from the fill to the exit in R, less the assumed fee and slippage
   on both sides ([ADR-0059](/chartgen/decisions/0059-setups-paper-scored/)). When the target
   is nearer to the fill than the round-trip costs, the setup is `won` and its result is
   negative. The client counted such setups as wins.
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

1. Refuse a setup without `playbook_version` when the owner's newest setup states one.
2. Make `playbook_version` required on every setup.
3. A default stored on the setup, such as the owner's last version.
4. **Make the setup as before, warn in the answer when the owner's newest setup stated a
   version, and count it under an explicit `unversioned` key.**
5. Leave it.

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

### Tag prefixes (option 1)

`get_track_record` takes `group_by: "tag"` with **`tag_prefixes`**, a list of 1 to 10 strings.
Each prefix is a group, and its key is the prefix. A setup is in the group of each prefix that
one of its tags starts with, and counts once in that group even when several of its tags match.
A setup with no matching tag is in the `null` group. Matching is by start only, so a prefix
that is a whole tag also takes the longer tags that start with it: `trigger:scan` takes
`trigger:scan-4h`.

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

### A missing `playbook_version` (option 4)

A setup without `playbook_version` is made as before, and nothing is stored in its place.
When the owner's newest setup states a version, the `create_setup` answer carries
**`warnings`**: `["playbook_version missing; this setup is counted as unversioned"]`. An owner
that never stated a version gets no warning, and other owners' setups do not count.

`get_track_record` counts every setup without a version under the key **`unversioned`** with
`group_by: playbook_version`, in place of `null`, and the filter `playbook_version:
"unversioned"` selects them. A setup made before this decision without a version is counted
the same way. `create_setup` refuses `unversioned` as a stated version, so the word always
means "made without one". No stored setup states it.

Refusing the setup (option 1) would catch the mistake at the moment it can still be fixed. A
live client whose call is refused may not send it again, though, and then loses the setup,
which is worse than a setup without a version. It would also change what the client gets back
in a way that cannot be adjusted on the client in the same step. Requiring the field on every
setup (option 2) would break every client that does not use playbook versions, which the field
was made optional for. A default stored on the setup (option 3) is a value the agent did not
state. The owner's last version is wrong right after a bump, which is when a mistake is most
likely. Leaving it (option 5) is what the client reported: the mistake goes unnoticed and the
group reads `null`, which looks like "not grouped" rather than "made without a version".

### Consequences

- Good: an agent reads a gain or a loss after costs from `net` and from `net_gain` and
  `net_loss`, without changing what `won` and the stored status mean.
- Good: triggers or any other tag families can be compared in one call, and skips can be
  counted by why they were skipped, without using a feature slot.
- Good: an owner that uses playbook versions is told at once when a setup has none, and the
  setup is still made. The record names the group `unversioned` instead of `null`.
- Neutral: every setup answer is about 13 bytes longer (`"net":"gain"`). The record's counts
  are two fields longer.
- Neutral: with `group_by: tag`, group counts can add up to more than `total`. The tool
  description and the reference say so.
- Neutral: a client that read the `null` key of `group_by: playbook_version` now reads
  `unversioned`. The other groupings keep `null`.
- Bad: a warning can be ignored, so a setup can still be made without a version by mistake.
  The record shows how many were.

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
- An owner whose newest setup states `v1` gets a setup without a version made, with the
  warning in the answer. The next setup without one, after that one, gets no warning. An owner
  that never stated one, and another owner, are not warned.
- In the record test the setup without a version is in the group `unversioned`, and the filter
  `playbook_version: unversioned` counts it.
- The pinned replay answers that the web types are tested against include `net`,
  `skip_reason`, `net_gain`, `net_loss` and the two new groupings.
