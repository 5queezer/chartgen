---
title: "0060 — The production image is built in CI and pulled by the host"
description: "CI builds the linux/arm64 image on a GitHub-hosted arm64 runner after every other job passed for the commit and pushes it to GHCR under the commit SHA. The host's timer sets the Coolify application's image tag to that SHA and deploys; the host builds nothing."
sidebar:
  order: 60
---

**Status:** Accepted
**Date:** 2026-11-11
**Amends:** [ADR-0037](/chartgen/decisions/0037-pull-based-deploy/) (what the timer
triggers: a pull of an image built in CI, no longer a build on the host; and which
commit goes out: exactly the one that passed CI)

## Context and Problem Statement

Production runs on a Radxa board with 4 GB of memory, under Coolify. Since
[ADR-0037](/chartgen/decisions/0037-pull-based-deploy/) a timer on the board asks
Coolify to deploy when `master` is green, and Coolify then builds the repository's
`Dockerfile` on the board. The release profile is fat LTO with one codegen unit, so
the last crate is one large compile.

On 2026-11-11 such a build made the board unresponsive for half an hour, and it
rebooted. Everything else the board runs was down with it. A deploy already took 20
to 25 minutes of the board's whole capacity. The timer was switched off, so nothing
could be deployed at all.

Where is the image built, so that a deploy cannot take the host down, and how does
the host get it?

## Decision Drivers

- A deploy must not be able to exhaust the production host.
- What goes out is a commit that passed CI, and exactly that commit.
- As in ADR-0037: nothing inbound to the host, no Coolify credential in GitHub, and
  no code from a fork's pull request anywhere near the host or a credential.
- The binary that runs stays the one the release profile builds.
- A rollback is quick and needs no build.

## Considered Options

1. **Cap the build on the board:** limit the build's memory and CPU (a cgroup, fewer
   jobs, a swap file).
2. **A lighter release profile:** thin LTO and more codegen units, so the last step
   needs less memory.
3. **Build the image in CI** on a GitHub-hosted arm64 runner, push it to a registry,
   and let the board pull it.
4. **Cross-build elsewhere:** on the developer's machine or another host, by
   emulation or a cross toolchain, and push or copy the image.

## Decision Outcome

Chosen option: **3, build in CI.** It is the only one that takes the build off the
board altogether, and it also closes the gap ADR-0037 accepted.

### The build

A job `image` at the end of the `CI` workflow builds the unchanged `Dockerfile` for
`linux/arm64` on `ubuntu-24.04-arm`, natively, and pushes
`ghcr.io/5queezer/chartgen:<full commit sha>`. It has the labels
`org.opencontainers.image.source`, which links the package to the repository, and
`org.opencontainers.image.revision`.

**The gate is `needs`:** the job needs every other job of the workflow. An image for
a commit therefore exists only if that same run was green for that same commit.
There is no second workflow, no lookup of another run and no window between a check
and a build.

It runs on a push to `master`, and when the workflow is run by hand on a ref, which
runs the whole of CI on that ref and then builds its image. That is how a staging
image is made. A run by hand uses the workflow file of the ref it runs on, so a ref
without this job gets no image. It never runs on a pull request.

`:master` is moved only by a run on `master`, and only if its commit is still
`master`'s head when its build is done. Builds of one ref run one at a time. So of
two pushes close together the older run cannot take the tag back from the newer.

Only this job may write, and only packages (`contents: read`, `packages: write`).
It uses no action besides `actions/checkout`: logging in and building are the
runner's own `docker`. The layer cache is a tag in the same package.

### The package is public

The repository is public, and the image holds what its `Dockerfile` builds from the
repository and nothing else: no secret is given to the build. A public package
therefore shows nothing that is not already public. In return the board and Coolify
pull with no credential, and the timer can ask the registry whether an image exists
the way anyone can. A private package would put a GitHub token with `read:packages`
on the board, in Coolify and in the timer's environment, to protect nothing.

### The deploy

Still the timer of ADR-0037, still pull-based. One run:

1. Read `master`'s head. Unchanged since the last deploy: done.
2. Look up the `CI` run for it: pending waits, failed or cancelled is not deployed.
   Since the image exists only after a green run this decides nothing by itself; it
   is kept for what it tells the log.
3. Ask the registry for `ghcr.io/5queezer/chartgen:<sha>`. Not there yet, refused,
   rate-limited or unreachable: nothing is deployed, and the next tick asks again.
4. Set the Coolify application's image name and tag to that commit, then call the
   deploy API. Either failing leaves the state as it was; the next tick does both
   again.
5. Record the commit, and the one deployed before it.

The Coolify application is of the kind Docker Image. It pulls and starts; it does
not build.

**Rollback** is the previous commit's tag: `chartgen-autodeploy.sh rollback`
deploys the image recorded in step 5, and `deploy <sha>` any published one. Neither
changes the timer's state, so the timer does not put the bad commit back, and the
next commit on `master` deploys as usual.

### Consequences

- Good: the board builds nothing. A deploy is a pull and a restart.
- Good: **what goes out is exactly the commit that passed CI.** Coolify's deploy
  endpoint took no SHA and built the branch head, so a commit landing between the
  check and the build could go out before its own CI; ADR-0037 accepted that. An
  image tag is a commit.
- Good: a rollback takes as long as a pull. Every image stays under its SHA.
- Good: the image can be used without Coolify:
  `docker pull ghcr.io/5queezer/chartgen:master` on an arm64 host.
- Bad: **every job of CI now gates production,** the macOS build and the Playwright
  shards included. A flaky shard delays a deploy until someone re-runs the failed
  jobs, which then also builds the image. Before, the timer waited for the same
  workflow to conclude, so this is the same gate with one more job behind it.
- Bad: a failed image build makes `CI` red on `master` although the code is fine.
  Nothing would have been deployed either way, and re-running the failed job
  repeats only the build.
- Bad: **anyone with write access to the repository can publish an image of any ref**
  by running the workflow on it, under that ref's commit SHA. They cannot move
  `:master` from another ref, and the timer deploys only `master`'s head. Write
  access already allows pushing to `master`.
- Bad: the image is `linux/arm64` only. An x86 host builds the `Dockerfile` itself or
  uses the release binaries, as before.
- Bad: images accumulate in the registry, one per commit of `master`. Nothing
  removes them yet.
- Bad: moving an existing Coolify application to an image is a manual step with a
  way to lose the data: Coolify does not offer to convert an application that
  builds from a Dockerfile, and a new application starts on a new, empty volume.
  The [steps](https://github.com/5queezer/chartgen/blob/master/scripts/autodeploy/README.md)
  begin with a copy of the volume.
- Bad: the timer still reports that a deploy was triggered, not that it is healthy.
- Neutral: the CI run of a push to `master` takes longer by the image build. Pull
  requests are not affected.
- Neutral: the unauthenticated GitHub API budget of ADR-0037 is unchanged: the
  registry is asked on another host.

## Pros and Cons of the Options

### 1. Cap the build on the board

- Good: no new moving part.
- Bad: the build still needs the memory. Capped below what the last compile wants,
  it is killed or swaps for hours; capped above, the board is where it was.
- Bad: production still shares a machine with a compiler for the length of a build.

### 2. A lighter release profile

- Good: one line, and it helps every build.
- Bad: it changes the binary everyone runs, slower and larger, to suit one host.
- Bad: it lowers the peak and does not remove it. The next dependency moves it up
  again, and the board finds out in production.

### 3. Build in CI

- Good and bad: as under Consequences.

### 4. Cross-build elsewhere

- Good: no dependence on GitHub's runners.
- Bad: on the developer's machine it is a manual step, tied to one person and one
  machine, and nothing gates it on CI.
- Bad: emulated arm64 builds of this project are slow, and a cross toolchain with
  fontconfig and freetype is a second build to maintain beside the `Dockerfile`.
- Bad: another always-on build host is a second machine to run and to secure.

A self-hosted runner on the board stays rejected for the reason in ADR-0037: on a
public repository a fork's pull request could run code on the host.

## Validation

`scripts/autodeploy/test.sh` runs the timer script against a stub that plays GitHub,
the registry and Coolify: an image not yet published, the registry's refusals and
rate limits, a refused tag change, a failed deploy and its retry, `master` moving
during a run, and both commands by hand. `actionlint` checks the workflow.

Not validated when this was written: the workflow itself, which runs only on GitHub;
the time and memory of the arm64 build; the pull on the board.
