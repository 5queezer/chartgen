---
title: "0055 — Provisional swing ends in zigzag and auto fib"
description: "The newest ZigZag leg and the auto fib swing follow price past their last confirmed pivot to the newest extreme, keep the deviation threshold, and say they are provisional: a dashed leg, a forming fib."
sidebar:
  order: 55
---

**Status:** Accepted
**Date:** 2026-11-09
**Amends:** [ADR-0046](/chartgen/decisions/0046-pattern-detection/) (when `auto_fib` redraws its swing)

## Context and Problem Statement

A pivot with `depth` bars on each side is known `depth` bars after its extreme.
QA on BTCUSDT 4h (issues #130 and #134) found two ways the newest bars were
misread:

- `zigzag` drew its last leg from the last confirmed pivot (a low at 74888) to
  the highest high since (76250, +1.8%). That is under its own 5% deviation, so it
  was not a swing. A newer low at 73510, already below the pivot, was not drawn at
  all, and nothing said that the leg was not confirmed.
- `auto_fib` kept a down swing 81196 → 74888 `confirmed` while the price traded
  under its 0 level at 73510. Only a close past the start (level 1) redrew it.

## Decision Drivers

- No lookahead: confirmed pivots never move because of later bars, apart from the
  newest one, which may move on to a more extreme pivot of its kind.
- The chart should show where price is now, not stop at the last confirmed pivot.
- A reader, human or agent, must be able to tell confirmed from provisional.
- No new response fields. `divlines[].dashed` and `patterns[].status` exist.

## Considered Options

1. **Keep the old legs and document them.** The zigzag leg stays under the
   threshold and the fib stays `confirmed` below its 0 level.
2. **Mark the fib as broken (`failed`) and keep its levels.** The levels then
   describe a swing that is over.
3. **Re-anchor to the newest extreme and say it is provisional.** This is how
   ZigZag-based fib tools work: the newest leg follows price until a pivot confirms
   it. TradingView's built-in source was not checked.

## Decision Outcome

Chosen: **option 3.**

- **`zigzag`** reads the last `depth` bars too, each against the bars after it so
  far, with the same swing test and the same deviation. The pivots this adds, or
  the newest confirmed pivot moved to a more extreme bar of its kind, end
  **dashed** legs. Legs between confirmed pivots stay solid. There is no leg to a
  move under the deviation.
- **`auto_fib`** walks the bars after its two newest pivots in order. A high above
  an up swing's end, or a low under a down swing's end, moves the end (level 0) to
  that extreme. A close past the start breaks the swing as before: it then runs
  from its end to the extreme since. Either makes the swing provisional, with
  status `forming` instead of `confirmed`.
- **Unset fib slots after the last set one have no plot.** The default 8th slot
  no longer sends an all-null `Level 8`. Slots before a set one keep their plot,
  so plot indices, which alerts and styles refer to, stay the same.
- `inspect_chart`'s swings stay confirmed pivots only.

### Consequences

- Good: the newest leg and the 0 level follow the newest extreme, and the output
  says so: `dashed: true`, `status: "forming"`.
- Good: tests hold the no-lookahead property: a prefix's confirmed pivots, but the
  newest, are pivots of the longer series, and a fib swing changes only through
  the bar just added.
- Bad: the dashed leg and a forming fib move with every new extreme and can vanish
  when a pivot confirms elsewhere. Bar replay already recomputes `zigzag`
  (ADR-0023). Fib alerts already drop a reading whose levels moved (ADR-0046), so a
  moved 0 level is no crossing.
- Bad: a chart whose last pivot is followed by only small moves ends at that pivot,
  with no leg into the newest bars.

## More Information

- [ADR-0023 — Bar replay](/chartgen/decisions/0023-bar-replay/)
- [ADR-0046 — Pattern detection](/chartgen/decisions/0046-pattern-detection/)
- `zigzag_trailing_leg_follows_a_newer_extreme`, `zigzag_confirmed_pivots_do_not_look_ahead`
  in `src/indicators/trend.rs`; `price_past_the_end_moves_the_end`,
  `the_swing_reads_no_later_bar` in `src/indicators/auto_fib.rs`
