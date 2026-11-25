---
title: "0087 — A skip's bracket is measured from its fill"
description: "A skip is filled at the next 1m bar's open, but its bracket was measured from the close of the interval's last closed bar, so a move between the two could leave it won with little or no R. Its levels now move to the fill by the same distances. Skips scored before keep their results and are marked bracket_anchor: signal_close; the track record can group by it."
sidebar:
  order: 87
---

**Status:** Accepted
**Date:** 2026-11-25
**Amends:** [ADR-0059](/chartgen/decisions/0059-setups-paper-scored/) (a skip's levels were fixed
when it was made, "not moved to the fill"), with the record of
[ADR-0064](/chartgen/decisions/0064-a-setups-record/) and the shadow variants of
[ADR-0066](/chartgen/decisions/0066-shadow-exit-variants/) following from the fill as before

## Context and Problem Statement

A skip is a decision not to trade, scored as if taken (ADR-0059). The server builds its bracket:
one ATR(14) of the setup's interval against it and two with it, or the least risk and twice that
where the ATR is less. `create_setup` said it is "scored as if taken at the next bar's open with
the server's bracket (1 x ATR against, 2 x ATR with)", which reads as a 1 R stop and a 2 R target
from the fill.

That is not what was scored. `skip_bracket` places the levels around the close of the last
**closed bar of the setup's interval**, and the scorer fills a skip at the open of the next
**1m** bar. ADR-0059 expected the two to differ "a little". On a `1m` skip they do; on a `4h` or
`1d` skip the close can be hours old, and price can be most of the way to either level by the
fill. With a close of 100 and an ATR of 2 (invalidation 98, target 104) and a fill at 103.9:

- the risk at the fill is 5.9 and the target is 0.1 away, so the first touch of 104 is `won` at
  0.1 / 5.9 = 0.017 R before costs, about -0.008 R after them;
- had the fill opened below 98, the skip ended `invalidated` with no fill and 0 R, though it was a
  decision about that very price.

So `won` and `result_r` disagree for a skip in a way they cannot for a take (whose fill is
clamped into the agent's own entry zone and whose target is the agent's), and a skip's record
is skewed by where price happened to be at the fill rather than by what it did afterwards.

How is a skip's bracket placed, and what happens to the skips already scored?

## Considered Options

For the bracket:

- a. **Move the levels to the fill (chosen).** At the bar that fills a skip, entry, invalidation
  and target move by the fill's distance from the close they were placed around: the same ATR
  distances, measured from the price it was taken at.
- b. **Measure from the newest 1m close when the skip is made.** Much nearer the fill, but still
  not it: a fast minute leaves the same lopsided bracket, only less often.
- c. **Keep the levels and fill a skip at the close they were placed around.** The bracket is then
  exact, but the fill is a price that was no longer to be had: a skip would be scored on a trade
  nobody could have made.
- d. **Keep both and report R in ATR instead of in the risk at the fill.** `result_r` would then
  agree with the bracket, but `won` would still be reached by a 0.1 move, and R would mean one
  thing for takes and another for skips.

For the skips already scored:

- e. **Leave their results and mark them by the rule that scored them (chosen).** A field
  `bracket_anchor` says `fill` or `signal_close`; the track record groups by it.
- f. **Rescore them**, with a re-runnable command that reads each skip's 1m bars again and
  replays the new rule.

## Decision Outcome

Chosen: **a** for the bracket and **e** for stored skips, in `src/setups.rs`.

- `skip_bracket` and `stock_skip_bracket` build the levels as before, around the interval's last
  close, so a skip shows a bracket while it waits for its first bar.
- **At the bar that fills it, a skip's levels move to the fill**: entry becomes the fill, and the
  invalidation and target move by the fill's distance from that close. The fill's `status` entry
  in the log has `entry`, `invalidation` and `targets` from and to, beside `fill`: nothing is
  changed without a log entry. Shadow results, excursions and durations follow from the moved
  levels, as for any fill.
- A skip is therefore filled at its first bar; one whose first open was past the old
  invalidation is no longer `invalidated` unfilled.
- **Where the moved bracket would have a price at or below 0, the skip keeps the close's
  bracket** and is marked `signal_close`. That is a long filled within one risk (one ATR, or the
  floor) of 0, whose invalidation would be at or below 0, or a short filled within two risks of
  0, whose target would be; a long's target and a short's invalidation are above the fill. A
  `voided` log entry at that bar says so, with `bracket_anchor` from `fill` to `signal_close`. Such
  levels could not be stored: a row with a price at or below 0 is not read back at startup, so the
  setup would vanish at the next restart, and no bar reaches a target at or below 0. Keeping the
  bracket the skip was shown with, marked as the old rule's, is the one outcome with valid prices
  that does not invent a third bracket. It is what the old rule did, including ending a long
  `invalidated` unfilled when its open was past the close's stop. For the same reason the bracket
  built when a skip is made is refused when any of its prices, not only its invalidation, would be
  at or below 0 (a short's target from a close within two risks of 0).
- A skip that wins is about 2 R before costs, one invalidated about -1 R. The exceptions are the
  ones every setup has: a gap through the stop exits at the open, worse than -1 R, and **on a stock
  an open past the target exits at that open** (ADR-0070), so a stock skip can win well over 2 R
  (a fill at 100, stop 98, target 104 and an open at 120 is 10 R before costs); a dividend counts
  too. On Binance a win exits at the target. The least-risk floor is still judged at the close the
  skip was made at; at a fill some way from it the floor is a few bps more or less of that price.
- **`bracket_anchor`** is stored on every skip made from now on as `fill`, and written into the
  first entry of its log with what the agent stated. A skip stored without it, made before this
  rule, reads as `signal_close` and is scored by the old rule to its end: the rule a setup is
  scored by is fixed when it is made, so a restart, or a skip still waiting for its first bar at
  the deploy, scores it as it was scored before. A take has no `bracket_anchor`.
- `get_track_record` takes `group_by: "bracket_anchor"`: skips by the rule that scored them, takes
  in the `null` group. `list_setups` takes `bracket_anchor` in `fields`.

Takes are unchanged: their fill is the open clamped into the agent's entry zone, and their levels
are the agent's, so `won` already means the agent's target from a price inside its zone.

### Why stored skips are marked, not rescored

- **The log cannot be rewritten.** Each closed skip's log has its fill and its close as `status`
  entries, and database triggers refuse an update or a delete of a log entry (ADR-0059). A
  rescore could only add entries after the close, leaving a log that says `won` at one price and
  then something else, or would have to get round the triggers that make the record worth
  having.
- **Not every skip can be rescored.** Rescoring replays 1m bars from the fill to the exit with the
  new levels. Binance serves old 1m bars; Yahoo serves a stock's 1m bars for 30 days (ADR-0070),
  so a stock skip older than that could not be rescored, and the record would mix the rules anyway,
  without saying which is which.
- **It is a rewrite of production data** for a result that is still true of the rule it was
  scored by. With the mark, nothing stored changes, and the two rules are counted apart where it
  matters: in the track record, by `group_by: "bracket_anchor"`, or by `from` set to the time this
  rule was deployed.

So no command is run on a deploy, and none is shipped.

### Rollback

A build from before this one reads a row with `bracket_anchor` and ignores the field, and it
rewrites a setup's row whenever the setup moves: a fill, a close, a shadow variant's bar. The row
it writes has no `bracket_anchor`. Read by this build again, such a skip would be `signal_close`,
whichever rule scored it. This was checked: the older `Record` has no such field and no
`deny_unknown_fields`, so serde drops it on the read and the rewrite leaves it out.

So at startup, as for the other fields a row can lose (ADR-0064, ADR-0070), `restate` takes
`bracket_anchor` from the first entry of the log, which no build can rewrite, when the row has
none. Then it checks which rule filled it. A fill under the fill rule sets the entry to the fill
price, the same number, and splits divide both alike. So a skip marked `fill` whose fill is not at
its entry was filled by the older build, by the old rule, and becomes `signal_close`. The fill's
`status` entry would say the same, since only a fill under the fill rule has level changes in it,
but it is not read at startup, and the entry and the fill already say it.

- Filled before the rollback and rewritten during it (still open, or its shadows still scored):
  `fill`, as scored.
- Still waiting at the rollback and filled during it: the old rule scored it, and it is
  `signal_close`.
- Still waiting at the re-upgrade: `fill`, and scored by the fill rule from its first bar.

Nothing is left for an operator to do after a rollback and re-upgrade. A skip made **during** the
rollback has no `bracket_anchor` anywhere and is `signal_close`, as it was scored.

### Consequences

- Good: a skip's `won` is a 2 R move from the price it was taken at, and its `result_r` agrees;
  the tool description says what is scored.
- Good: skips on longer intervals are no longer skewed by where price went before the fill.
- Good: additive only. `bracket_anchor` is a new field, and a new `group_by` value; no status, field
  or default changes for a client.
- Bad: until older skips leave the window a client looks at, an ungrouped track record mixes the
  two rules. Grouping by `bracket_anchor` separates them.
- Bad: a skip's levels before and after its fill differ. The log says by how much.
- Neutral: a build from before this one, rolled back to, scores a skip still waiting for its
  first bar by the old rule. After the re-upgrade each skip is marked by the rule that filled it
  (see Rollback).
- Neutral: the first log entry of a new skip has `bracket_anchor`, beside the stated fields.

## Confirmation

`src/setups/tests.rs` scores a long skip made at a close of 100 (ATR 2) whose first 1m bar opens
at 103.9: it is not `won` at 104, its levels are 101.9 and 107.9 after the fill, and it wins near
2 R at 107.9. A skip that opens past its old stop is filled, and stopped near -1 R. A stored skip
without `bracket_anchor` reads as `signal_close` and is still `won` at a loss after costs by the
old rule. A row rewritten without the field gets it back from its log, as `fill` when filled by
the fill rule or still waiting, and as `signal_close` when filled by the old rule. A long filled at
1.5 and a short filled at 3 from a close of 100 keep the close's bracket, marked `signal_close`.
`src/setup_service/tests.rs` groups a track record by `bracket_anchor`, and restarts the service on
a long and a short skip filled near 0: both are read back as they were.
