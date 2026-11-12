---
title: "0066 — Fixed shadow exit variants on every filled setup"
description: "Every filled setup is also scored with three exit variants the server defines (half out at T1 with a break-even runner, break-even at +1R, a time stop), on the same 1m bars and by the same ambiguity rules, as shadow_results. The canonical result_r is unchanged. A variant that outlives the setup's close keeps its feed until it is out or the expiry. Per-setup exit rules are deferred."
sidebar:
  order: 66
---

**Status:** Accepted
**Date:** 2026-11-12
**Amends:** [ADR-0059](/chartgen/decisions/0059-setups-paper-scored/) (a closed setup can still be
scored, for its shadow results only, and holds its symbol's feed while it is)

## Context and Problem Statement

A setup is scored all out at its first target (ADR-0059). The agent that keeps setups wants to test
exit-management ideas from trading books, such as taking half at the first target or moving the
stop to break-even, but a setup knows one exit. Letting the agent pick an exit per setup would
make it commit to one idea before there is data, and would make a track record mix rules.

How can a setup give comparison data for a few standard exits, after about ten setups, without
changing what its result means?

## Decision Drivers

- `result_r` keeps its meaning. A track record built so far stays comparable.
- The agent cannot choose or change a variant after the fact: a variant picked once the outcome is
  known would be a better-looking score, not data.
- A variant reads the bars exactly as the canonical scorer does, so a difference between them is
  the exit rule and nothing else.
- Old records load as they are. No migration.
- A closed setup does not hold a place among the open ones.

## Considered Options

1. Fixed variants the server defines, scored on every filled setup, as shadow results beside the
   canonical one.
2. Exit rules per setup, chosen by the agent at create time, replacing the canonical exit.
3. Leave it to the agent: it reads 1m bars itself and computes the variants.

## Decision Outcome

Chosen: **1**. Option 2 is deferred: it needs the comparison data that option 1 produces first,
and it changes what `result_r` means per setup. Option 3 repeats the scorer outside the server,
where the ambiguity rules would drift from it, and cannot be checked by anyone reading the record.

Every setup that fills, a take or a skip with its bracket, gets `shadow_results`: a map from variant
id to `{status, result_r, stop, exits, exited_at, time_stop_at}`. It is null before the fill and on
a record filled before this change (`#[serde(default)]`; the field is simply absent in old rows).

### The variants

Below, R is measured from the fill in units of the risk at the fill, before costs, as `mfe_r` is
(ADR-0059). **Break-even after costs** is the price at which an exit scores exactly 0 R with the
setup's assumed costs on both sides: `fill × (1 + s·c/2) / (1 − s·c/2)`, with `c` the round-trip
cost as a fraction (14 bps) and `s` +1 for a long, −1 for a short.

- **`v1_half_t1_be`.** Half out at T1. From the next bar on, the other half's stop is break-even
  after costs. That half runs to T2, or with one target only to that stop or the expiry. T2 may be
  reached in the bar that took T1, since the price passed T1 to get there. `result_r` is
  `0.5 · R(T1) + 0.5 · R(second exit)`, each part net of its own costs.
- **`v2_be_at_1r`.** All out at T1, no partial. Once a bar after the fill bar reaches +1R (its high
  for a long, its low for a short), the stop is break-even after costs from the next bar on.
- **`v3_time_stop`.** The canonical setup, except that if neither T1 nor the stop has been reached
  by the close of the Nth bar of the setup's interval after the bar holding the fill, it is out at
  that close when the close is below +0.5R. N is 12 for intervals shorter than 4h and 6 from 4h
  on. The bars are those of `Interval::grid_start`/`grid_end`, the grid every chart here uses:
  minutes and hours from the epoch, days from epoch day 1, weeks from Monday, months by the
  calendar. The judged close is the close of the 1m bar that ends that bar, or of the first 1m bar
  after it when that one was not seen. It is judged once: at or above +0.5R the position is kept,
  and from then on it is the canonical setup. `time_stop_at` holds the judged close time until
  then and is null after.

A variant's status is one of `open`, `won` (its last target), `stopped` (the invalidation),
`breakeven` (a stop moved to break-even), `timed_out`, `expired` and `cancelled`. `stop` is the
stop in force from the next bar; `exits` lists each part with its price, fraction and 1m bar.

### The ambiguity rules

A variant follows the canonical scorer exactly:

- The fill bar can stop it, at the invalidation, and cannot win it. It cannot move a stop either:
  the fill bar counts only against the position, so its favourable side does not reach T1 or +1R.
- A bar that reaches a stop and a target counts the stop.
- A bar that opens past a stop exits at its open. A target is exited at the target's price.
- **A stop moved during a bar takes effect from the next bar.** A bar that takes T1, or reaches
  +1R, and falls back to the fill in the same bar does not stop the moved part out.
- A cancel closes every open variant at the open of the bar it takes effect in, as it closes the
  setup. The expiry closes every open variant at the open of the first bar at or past it. A setup
  that ran out while its owner was off the operator's list closes its open variants as `expired`
  with no exit and no result, as it does itself.

### Scoring after the close

The canonical record stops at its close; a variant may not. v1's other half is the common case: the
setup is out at T1 and the half runs on. v2 and v3 close at or before the setup by construction,
but nothing relies on that.

The smallest way: one predicate, `Record::is_scoring()`, true while the setup is open **or any of
its shadows is**. `apply_bar` takes bars while it is true; on a closed setup it advances only the
shadows and `scored_through`, and adds nothing to the log. The service uses the same predicate
where it decided which setups get bars: the feeds it watches, the setups a bar moves, the catch-up
read of missed bars, the replay, and the expiry of setups whose owner is off the list. Nothing new
is stored besides `shadow_results`: the record's `scored_through` is the one cursor for both. When
the last shadow closes, the setup stops scoring and its feed is let go at the next sync.

- **Limits.** Open setups and symbols per owner count `status`, so a closed setup with an open
  shadow counts toward neither. The feed it keeps is still watched, so it counts toward the
  server's watched pairs (ADR-0042), as it must: that limit is on what the server does. It is held
  at most until the setup's expiry.
- **Gaps.** A gap while the setup is open sets `gap`, as before. A gap after the close, while only
  shadows were being scored, goes to the log only: the setup's own result covers all of its bars,
  and `gap` keeps saying that.

### Consequences

- Good, because after a handful of setups the agent has, per setup, what each exit would have done
  on the same path, with the same costs and the same tie rules, and its track record is unchanged.
- Good, because old rows load unchanged, and nothing is migrated.
- Bad, because a closed setup can hold a feed, and so one of the server's watched pairs, until its
  expiry when v1's other half never reaches T2 or its stop.
- Bad, because `scored_through` moves after the close while a shadow is open, which a reader may not
  expect of a closed setup.
- Neutral: the variants are fixed. A later variant (a Chandelier trailing stop, say) is a new field
  of `shadow_results` with `#[serde(default)]`, null on setups filled before it.

### Testing

`src/setups/tests.rs` walks each variant on a long and on its mirrored short: v1 with T2, v1 stopped
at break-even from the bar after T1, v1 with one target to the expiry, v2 moving its stop at +1R and
stopped at break-even, v2 winning, v3 timing out at its 12th 1h bar and its 6th 4h bar, v3 kept at
+0.5R and judged once, the fill-bar rule, a stop and a target in one bar, gaps through the
invalidation and through a moved stop, a cancel, an unscored expiry, and a record from before. A
service test in `src/setup_service/tests.rs` keeps v1's other half scored after the setup won,
across a restart, with its feed held and no open-setup or symbol limit taken.
