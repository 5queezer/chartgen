---
title: "0057 — The e2e suite runs in duration-balanced CI shards"
description: "CI builds the debug chartgen binary once and runs the Playwright suite on six runners, each with whole spec files assigned by measured duration and its own servers; an aggregate job keeps the e2e check name."
sidebar:
  order: 57
---

**Status:** Accepted
**Date:** 2026-11-09
**Amends:** [ADR-0008](/chartgen/decisions/0008-e2e-playwright/) (its CI job: one
`e2e` job running the whole suite)

## Context and Problem Statement

The `e2e` job ran all 760 tests on one runner with one Playwright worker and took
about 46 minutes (master run 42508737009: the test step alone 2755 s). Setup was
not the cost: the cached debug build took 15 s and the servers were up 30 s into
the step. The tests were: 45.1 minutes of test time, spread over 65 spec files
from 1 s to 7.2 minutes each.

One worker is a constraint of the suite, not of CI: chartgen's OAuth store is
process-global, so tests sharing one server must not run concurrently
(`fullyParallel: false`, `workers: 1`). Each set of servers can serve only one
test at a time, but nothing stops several runners from each having their own.

How do we cut the wall time of a PR's e2e run without dropping, skipping or
weakening any test?

## Decision Drivers

- Coverage stays exactly as it is: every test runs, with the same assertions,
  timeouts and retries.
- The single worker per set of servers stays.
- A spec file's tests stay together, in order, on one set of servers: some share
  state across tests.
- The check name `e2e` stays.
- `npm run test:e2e` locally does not change.

## Considered Options

1. **More Playwright workers on one runner.** Needs one set of servers per
   worker and four projects' worth of servers each; a 4-core runner would be
   CPU-bound with that many chartgen, Vite and Chromium processes.
2. **Playwright's `--shard=i/n` on n runners.** With `fullyParallel` off it keeps
   files whole, but balances shards by test count. Seconds per test differ
   tenfold between files (signin-workspace: 32 tests, 435 s; drawing-modifiers:
   77 tests, 331 s), so test count is a poor stand-in for time.
3. **n runners, whole files assigned by measured duration.** A committed table of
   seconds per spec file and a greedy longest-first assignment.
4. **Removing fixed waits in the specs.** The heavy files wait on purpose: the
   700 ms after each click keeps two clicks out of klinecharts' 500 ms double-click
   window, and the sign-in specs wait out sync intervals, back-offs and slow answers
   to show what happens, or does not, within them. Not a way to make a large cut
   without changing what they test.

## Decision Outcome

Chosen: **option 3, six shards**.

- `e2e-build` builds the debug binary once (`cargo build --bin chartgen`, the
  Swatinem cache under the shared key `e2e`, which the single job used) and
  uploads it.
- `e2e-shard` is a matrix of six, `fail-fast: false`. Each downloads the binary
  and runs `npx playwright test` on the files `scripts/e2e-shard.mjs <i> <n>`
  prints. `CHARTGEN_BIN` makes `playwright.config.ts` start that binary instead
  of `cargo run --`, and `oauth-stale-client.spec.ts`, which starts a chartgen of
  its own, does the same; without it nothing changes locally. Each shard starts
  all of the config's servers, so every project works as before.
- `scripts/e2e-shard.mjs` lists every `*.spec.ts` under `web/tests/e2e`, takes
  each file's seconds from `scripts/e2e-shard-durations.json` (a file not in the
  table counts as 20 s, so a new spec runs before it is measured), and assigns
  files longest first to the least-loaded shard. The order is deterministic, so
  every shard computes the same split. A file's seconds are its wall time in a
  shard's log, from the previous result to its last, so they include its hooks
  (a `beforeAll` that builds the app); a test's own duration leaves those out.
  With the durations of the first green sharded run the six shards get 470–473 s
  each; the longest file alone is 448 s, so a seventh shard would barely help.
- Shards write blob reports. The `e2e` job needs all shards, fails unless all
  passed, and on failure merges the blobs into one HTML report (artifact
  `playwright-report`, as before). Test results are uploaded per shard
  (`playwright-test-results-<i>`).

Refresh the table when the split drifts, from every shard's job log of one run
(see the [Testing guide](/chartgen/guides/testing/#how-ci-runs-it)). Retries are
left out, so a flake does not count against its file.

### Consequences

- Good: a PR's e2e wall time drops from about 46 minutes to the longest shard plus
  per-runner setup. On the implementing PR (run 42630402422) the e2e jobs took
  10.1 minutes from the first shard's start to the end of `e2e` (shards 7.7–9.6
  minutes), and the whole CI run 12.4 minutes instead of 49.4 on master
  (run 42508737009).
- Good: a failing shard does not stop the others, and the merged report still
  shows every failure in one place.
- Bad: total runner minutes rise, since each of six runners pays checkout,
  `npm ci`, the Chromium install and server start-up: 54 minutes on that run
  against 46.5 for the single job. The Chromium install's `--with-deps` apt step
  also runs six times, and one slow mirror once cost a shard 3 minutes.
- Bad: the duration table can go stale. A stale table only costs balance, never
  coverage: every spec file is assigned to exactly one shard regardless.
- Neutral: the floor is the longest spec file (signin-workspace, about 7 minutes).
  Cutting below it means splitting that file.

## More Information

- [ADR-0008 — End-to-end tests with Playwright + CI integration](/chartgen/decisions/0008-e2e-playwright/)
- [Testing guide](/chartgen/guides/testing/#how-ci-runs-it)
