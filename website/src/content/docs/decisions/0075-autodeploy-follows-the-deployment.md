---
title: "0075 — The autodeploy follows a deployment until the platform finished it"
description: "The deploy script keeps the deployment the container platform named for a deploy and reads it on the next ticks. A finished one is recorded as running; a failed one is deployed again, at most three times in all, a tick apart; then the script gives up loudly. Rollback goes to the image that ran before the one that runs now."
sidebar:
  order: 75
---

**Status:** Accepted
*Edited 2026-11-15 to remove deployment details.*
**Date:** 2026-11-15
**Supersedes in part:** [ADR-0060](/chartgen/decisions/0060-image-built-in-ci/) (what the script
records after a deploy, and where `rollback` goes: an image counts as having run once the container
platform finished deploying it, no longer once it accepted the request; and the polling of the platform that
ADR-0060 left out is now done. The rest of ADR-0060 stands)

## Context and Problem Statement

The timer once deployed a commit that the platform accepted, then failed at the image
pull: containerd on the host could not commit a layer ("failed commit on ref ... ingest"), a
passing fault, as a pull by hand a minute later worked. The script had already recorded the commit
as deployed and the head as dealt with, so no tick tried again. Production ran the image before it
until an operator deployed by hand.

ADR-0060 knew this: "the timer still reports that a deploy was triggered, not that it is healthy
... Confirming a deployment would mean polling the platform, which this decision leaves out". The cost
of leaving it out was a release that silently did not go out.

## Decision Drivers

- A deploy that failed for a passing reason goes out without anyone acting.
- One that keeps failing stops, and says so where an operator looks: the unit fails, and the
  watchdog, which compares the image production runs with the one the script triggered last, keeps
  alarming.
- `rollback` goes to an image that ran, not to one that was only triggered.
- The lock, the commands by hand and the timer's mark of `master` stay as they are.

## Considered Options

1. **The next ticks read the deployment the platform named** (chosen). The deploy call already answers
   with its uuid; `GET /api/v1/deployments/<uuid>` gives `queued`, `in_progress`, `finished`,
   `failed` or `cancelled-by-user`.
2. Wait in the same run until it ends. Simpler state, but a tick would hold the lock for minutes,
   and a command by hand waits at most two of them.
3. Compare the image the container runs with the one triggered (`docker ps`). That is what the
   watchdog does. The script runs with `DynamicUser` and no access to Docker, and the platform's own
   answer says why a deploy failed.

## Decision Outcome

After a deploy, by the timer or by hand, the script keeps the deploy it follows in its state
directory, in `pending`: the commit, the deployment the platform named, the attempt, and when the first
attempt was made. The platform names a deployment whenever it queued one, also one already queued for
the same image; an answer that names none means nothing was queued, and the script counts it as
not triggered. An attempt is written to `pending` before it is made, so the attempts stay bounded
when the state directory cannot be written.

Every tick reads that deployment first, and while one is followed it deploys nothing else:

- **`finished`:** the image is recorded as running (`running-sha`), and the one that ran before it
  as the one before (`previous-sha`). The tick then goes on as usual and may deploy a newer head.
- **`failed`:** it is deployed again, with the same commit, at the next tick, up to
  `DEPLOY_ATTEMPTS` (3) attempts in all. An attempt the platform refused counts too. After the last one
  the script logs an `ERROR` that names what runs and the command to put the commit out, stops
  following it, and the tick exits 1. It does not deploy that commit again; the next commit on
  `master` deploys as usual.
- **`cancelled-by-user`:** someone meant it. It is not deployed again.
- **`queued`, `in_progress`, or no answer:** the tick waits. One followed for longer than
  `DEPLOY_WAIT_SECS` (45 minutes) is given up with an `ERROR`, as above.

`deployed-sha` keeps its meaning: what the script triggered last, or was told runs. The watchdog
reads it and needs no change: after a give-up it still differs from the image that runs.

A command by hand first reads the deploy followed so far: one that finished is recorded, so that
what ran before it is known; one that failed or was cancelled is replaced; **one still queued or in
progress is not replaced**, because the platform would still run it after the command's own, and the
command deploys nothing and says why. **`rollback` deploys the image that ran before the one that
runs now**, and refuses while the script does not know what runs. `running <sha>` records what
runs, ends any following, and on a state from an older script drops its `previous-sha`, which
names an image triggered, not one known to have run.

### Consequences

- Good: a deploy that failed for a passing reason goes out at the next tick, without an operator.
- Good: `rollback` never goes to an image that only failed.
- Good: a deploy that keeps failing ends in a failed unit and an `ERROR` line, and the watchdog's
  alarm stays.
- Bad: a newer head waits while a deploy is followed: up to three attempts a tick apart, or 45
  minutes at most when the platform holds one queued.
- Bad: "finished" is the platform's word. On this application the platform's health check is off and the
  image has none, so it means the container started, not that it answers.
- Bad: a deploy or rollback by hand waits for a deploy under way to end, or for the operator to
  cancel it in the platform.
- Neutral: two more state files, `running-sha` and `pending`. An older script's state has no
  `running-sha`; `running <sha>` once after the upgrade gives `rollback` its starting point.
