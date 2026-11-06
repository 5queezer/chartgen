---
title: "0037 — Pull-based deploy from the Coolify host"
description: "A systemd timer on the Coolify host polls GitHub and triggers the local Coolify deploy API, instead of a GitHub workflow pushing to Coolify."
sidebar:
  order: 37
---

**Status:** Accepted
**Date:** 2026-11-06

## Context and Problem Statement

`.github/workflows/deploy.yml` called an old Coolify host with `GET` after
`CI` succeeded; that host now answers 403. The current Coolify runs on a
Radxa on the LAN (`http://192.168.1.242:8000`) and GitHub-hosted runners
cannot reach it. Coolify's deploy endpoint is `POST /api/v1/deploy` and a
deploy takes 20–25 minutes. How does a green `master` reach Coolify?

The repository is public, which rules out some options on security grounds.

## Decision Drivers

* No inbound exposure of the Coolify API.
* No Coolify credentials in GitHub.
* Fork pull requests must never run code on the host.
* The host's `/home` is LUKS-encrypted and stays locked after an unattended
  reboot, so the mechanism cannot live there.

## Considered Options

* A systemd timer on the host polls GitHub and calls Coolify locally.
* A self-hosted GitHub runner on the host.
* A Cloudflare tunnel route to the Coolify API, called from the workflow.
* Coolify's own GitHub webhook / auto-deploy on push.

## Decision Outcome

Chosen option: "systemd timer polls GitHub", because it needs no inbound
path and no secret outside the host.

Every 5 minutes `scripts/autodeploy/chartgen-autodeploy.sh` reads `master`'s
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

* Good, because Coolify stays LAN-only and GitHub holds no Coolify secret.
* Good, because the script is small and tested against a stub `curl`.
* Bad, because deploys lag a green CI by up to about 5 minutes.
* Bad, because the unauthenticated GitHub API allows 60 requests per hour per
  IP; polling costs 12 per hour plus one per new commit. A rate-limit response
  is logged and retried.
* Bad, because the install is a manual step on the host, outside the repo's CI.

## Pros and Cons of the Options

### Self-hosted GitHub runner

* Good, because the workflow keeps its current shape.
* Bad, because on a public repository a fork PR can run arbitrary code on the
  host. Rejected.

### Cloudflare tunnel route

* Good, because a GitHub-hosted runner could call it.
* Bad, because it exposes the Coolify API to the internet. Rejected.

### Coolify GitHub webhook

* Good, because it needs no script.
* Bad, because it deploys on push without waiting for CI, and the host is not
  reachable from GitHub anyway. Rejected.
