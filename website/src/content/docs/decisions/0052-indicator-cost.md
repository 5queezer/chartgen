---
title: "0052 — What an indicator costs"
description: "A tool call's cost counts what each indicator really draws, and the bricks a price-based chart's indicators run on, and a built call keeps only its answer, because one heavy chart that fitted the budget could reach most of the container by itself."
sidebar:
  order: 52
---

**Status:** Accepted
**Date:** 2026-11-07
**Amends:** [ADR-0051](/chartgen/decisions/0051-bounded-connections/) (how a tool call is charged; the budget of 32,000 stays)

## Context and Problem Statement

ADR-0051 holds tool calls to a budget of 32,000 units, a call costing `bars × (indicators + 1)`. The units
assume every indicator costs about the same, 0.8 to 1.5 KiB a bar. It left one attack open, with a token:
eight clients looping a chart of 627 bars and 50 of the heaviest indicators, with one answer unread and
100 half-sent bodies, got the server killed in the second round, on `candles` as on `renko` and `range`.

Measured on a release build, one call at a time on the stdio server, peak memory above an empty chart:

| Indicator | Per bar shown | Per brick built beyond those |
|---|---|---|
| `cipher_b` | 4.4 KiB | 232 B |
| `kalman_volume` | 2.5 KiB | 150 B |
| `rsi_mfi_stoch`, `ichimoku` | 1.7 KiB | 70 B |
| `wavetrend` | 1.5 KiB | 62 B |
| `candlestick_patterns` | 0.9 KiB | 189 B |
| `fear_greed` | 0.8 KiB | 122 B |
| `bbands`, `keltner`, `donchian`, `vwap_bands` | 1.2 to 1.3 KiB | 46 to 48 B |
| The other 66 | 0.15 to 1.2 KiB | up to 75 B |

Peak memory follows the answer's size, at 6 to 7 times it: the answer is built as a JSON tree, written out
and written out again inside the JSON-RPC reply. So one indicator can cost 30 times another, and:

- 50 `cipher_b` at 627 bars cost 31,977 and fitted, and peaked at 137 MiB in the server by itself. Two at once,
  or one after another on two arenas, ran out of the container.
- In `series`, a price-based chart's indicators run on every brick built, up to 20,000, though only `bars` are
  returned. The cost did not see them. On the sample source a renko box of 0.15% builds about 19,000, so 80
  bars of 20 `cipher_b` peaked at 98 MiB, charged as 21,680.

One client calling a chart of 627 bars and 50 heavy indicators one call after another, never two at once,
had 86 MiB in use after 10 calls and reached a peak of 170 MiB within 27. That fits each glibc arena keeping the
peak of the last call it built until the next trim, at most 2 s later. So what one call may build has to be small against the container, and a
looped attack is bounded by about twice that, not by the budget alone.

A cost that sees the indicators makes heavy charts dearer, and so would make the app's own grid wait. Three
unread charts of 500 bars and 20 indicators, 4 of them `cipher_b`, must all get their answers
(`test_the_apps_charts_do_not_take_turns`). At their real cost, 16,500 each, they need 49,500.

## Considered Options

1. **A per-indicator weight, a charge for bricks, and a call keeps only its answer once built.**
2. The same weights, with the budget raised to 50,000 so that the three unread charts fit.
3. A lower budget, with no weights.
4. Refusing a price-based chart after the build by bricks × series. Rejected before this: it does not close
   `candles`.

## Decision Outcome

Chosen: option 1, because it is the only one that closes every route measured and keeps the app's charts
running as before.

- **Weights.** An indicator counts as its measured share of a usual one: `cipher_b` 4, `candlestick_patterns`
  3, and `kalman_volume`, `rsi_mfi_stoch`, `ichimoku`, `wavetrend` and `fear_greed` 2, under every name the tool
  takes for them, as a string or as an object's `name`. Any other is 1. A unit is then at most 1.3 KiB a bar.
  The default list, `ema_stack`, `cipher_b` and `macd`, counts 6.
- **Bricks.** A price-based chart costs as if it showed 1200 bars more: 20,000 bricks at up to 75 bytes each
  per unit of weight, measured, come to what 1200 bars shown do. This replaces ADR-0051's charge of 20,000 for
  the source window, up to the whole budget: building the bricks themselves measured 1.3 MiB. A price-based
  call is now refused when it does not fit, as on any other style, rather than taking the whole budget.
- **A built call keeps only its answer.** Once its answer is built, a call gives back all of its share but the
  answer's length, at a unit a KiB, and holds that until the answer has gone out. What it built to get there
  has been freed by then. An answer of unknown length keeps the whole share. So three unread charts of
  16,500 hold about 1,900 each, and the budget does not need to grow.

Option 2 was built and measured: 50 `bbands` at 980 bars, a light chart it admits, peaked at 243 MiB in the
looped attack, against a limit of 256. Option 3 would need a budget of about 9,000 to hold 50 `cipher_b` to
the 40 MiB that a full budget of light indicators builds, and the three charts above cost 10,500 each without
weights, so all three would be refused.

### Measured

`worst5.py`: 8 clients looping the call, one answer unread (or 12), 100 half-sent bodies of 250 KiB, two rounds
of 70 s on one process. `worst3.py`: 5000 bars and 1000 of warmup with the 5 heaviest by answer size, 14 clients
looping, 110 half-sent bodies. Release builds under `systemd-run --scope -p MemoryMax=256M`, sample source,
peak in MiB. Before, the peak is the last one read before the kill.

| Call | Before | After |
|---|---|---|
| `candles`, 627 bars, 50 heaviest | Killed in round 2, 250 | Refused, 46 |
| `renko`, default box, 627 × 50 heaviest | Killed in round 2, 242 | Refused, 54 |
| `range`, default box, 627 × 50 heaviest | Killed in round 2, 233 | Refused, 45 |
| `renko`, 0.15%, 627 × 50 heaviest | Killed in round 1, 188 | Refused, 44 |
| `range`, 0.15% | Refused by the style: too many bars | The same |
| `worst3.py` | 119 | Refused, 48 |
| The most that fits: `candles`, 627 × 20 heaviest | Not run | 126; 135 with 12 unread |
| `candles`, 627 × 12 `cipher_b` | Not run | 148; 148 with 12 unread |
| `candles`, 627 × 50 `bbands`, the most per unit | Not run | 176; 168 with 12 unread |
| `renko` and `range`, default box, 627 × 6 heaviest | Not run | 82, 84 |
| `renko`, 0.15%, 80 × 10 heaviest | Not run | 95 |
| `renko`, 0.15%, 80 × 24 `naked_poc`, the most per brick | Not run | 113 |

All of these were alive after both rounds. With option 2, 627 × 32 heaviest reached 205 and 627 × 19 `cipher_b`
201.

### Consequences

- Good: every call measured, at the most the budget admits, stays under 180 MiB of 256 through the looped attack.
- Good: the app's own charts run as before. A chart of 500 bars and 10 indicators costs 5,500 on `candles` and
  18,700 on a price-based style, against 25,500 before, and either gives back all but its answer once built.
- Bad: a tool user asking for heavy indicators gets fewer: 627 bars take 12 `cipher_b`, not 50, and 500 bars of
  a price-based chart take 17 indicators, not 50. The refusal says why.
- Bad: the weights are measured on default parameters, at 627 bars, on x86_64. An indicator whose parameters
  add series, or a new indicator heavier than 1.3 KiB a bar, is undercharged until it is weighed. A user's
  scripted indicator counts 1, whatever it draws.
- Bad: answers held unread now hold their size, not their whole cost, so more of them fit at once. With twelve of the
  largest held, the peak was 168 MiB. Their bytes are charged, and hyper's and the kernel's buffers are ADR-0051's to
  bound, as before.
- Neutral: warmup bars are not charged. They add up to 1000 bars of per-brick cost, measured at 230 B a bar for
  `cipher_b`.
