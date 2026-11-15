---
title: "0037 — Pull-based deploy from the host"
description: "A systemd timer on the production host polls GitHub and triggers the container platform's local deploy API, instead of a GitHub workflow pushing to the platform."
sidebar:
  order: 37
---

**Status:** Accepted; superseded in part by [ADR-0060](/chartgen/decisions/0060-image-built-in-ci/) (the timer deploys an image built in CI and no longer has the container platform build on the host; exactly the commit that passed CI goes out)
*Edited 2026-11-15 to remove deployment details.*
**Date:** 2026-11-06

## Context and Problem Statement

`.github/workflows/deploy.yml` called an old deploy host with `GET` after
`CI` succeeded; that host now answers 403. The container platform now runs on a
small ARM server on a private network, and GitHub-hosted runners
cannot reach it. The platform's deploy endpoint is `POST /api/v1/deploy` and a
deploy takes 20–25 minutes. How does a green `master` reach the platform?

The repository is public, which rules out some options on security grounds.

## Decision Drivers

* No inbound exposure of the platform's API.
* No platform credentials in GitHub.
* Fork pull requests must never run code on the host.
* The host's `/home` is encrypted and stays locked after an unattended
  reboot, so the mechanism cannot live there.

## Considered Options

* A systemd timer on the host polls GitHub and calls the platform locally.
* A self-hosted GitHub runner on the host.
* A tunnel route to the platform's API, called from the workflow.
* The platform's own GitHub webhook / auto-deploy on push.

## Decision Outcome

Chosen option: "systemd timer polls GitHub", because it needs no inbound
path and no secret outside the host.

Every 5 minutes a pull-based deploy script, kept outside this repository, reads `master`'s
head SHA from the unauthenticated GitHub API and exits if it equals the last
SHA it triggered. Otherwise it looks up the `CI` workflow's push run for that
SHA (the same gate the old workflow used via `workflow_run`): pending waits
for the next tick, failed or cancelled is not deployed, success POSTs the
deploy and records the SHA atomically. A failed POST keeps the old state and
retries next tick. The first run records the head without deploying, so
installing never causes a surprise deploy. The unit is a hardened oneshot
(`DynamicUser`, `ProtectSystem=strict`, `ProtectHome`) with its token in
`/etc/chartgen-autodeploy.env`.

### Consequences

* Good, because the platform stays on the private network and GitHub holds no platform secret.
* Good, because the script is small and tested against a stub `curl`.
* Bad, because deploys lag a green CI by up to about 5 minutes.
* Bad, because the unauthenticated GitHub API allows 60 requests per hour per
  IP. An idle tick makes 1 call (12 per hour); while a new head is still
  undeployed (CI pending, failed or cancelled) each tick makes 2 (24 per
  hour). A rate-limit response is logged and retried.
* Bad, because the platform's deploy endpoint takes no SHA and builds the branch
  head. A commit that lands between the CI check and the build can go out
  before its own CI on `master` has finished. Accepted: every merge passes
  pull-request CI first and the window is small, so no workaround is built.
* Bad, because the install is a manual step on the host, outside the repo's CI.

## Pros and Cons of the Options

### Self-hosted GitHub runner

* Good, because the workflow keeps its current shape.
* Bad, because on a public repository a fork PR can run arbitrary code on the
  host. Rejected.

### Tunnel route

* Good, because a GitHub-hosted runner could call it.
* Bad, because it exposes the platform's API to the internet. Rejected.

### Platform GitHub webhook

* Good, because it needs no script.
* Bad, because it deploys on push without waiting for CI, and the host is not
  reachable from GitHub anyway. Rejected.
