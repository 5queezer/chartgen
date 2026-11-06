---
title: "0046 — Pattern detection: candlesticks, chart patterns, auto fib"
description: "Three indicators detect patterns on the server from closed bars, deciding each pattern from what was known then. New series channels carry labels, whole-window patterns and levels, and events an alert can wait for."
sidebar:
  order: 46
---

**Status:** Accepted
**Date:** 2026-11-06
**Amends:** [ADR-0034](/chartgen/decisions/0034-drawing-and-indicator-alerts/) (indicator alerts get a detection as a source), [ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/) (channels re-detected as a whole on every tail)

## Context and Problem Statement

TradingView has candlestick pattern recognition, auto chart patterns and an auto
fib retracement. TradingView publishes neither the whole list of patterns nor its
fib defaults, so this is no claim of equivalence. All three look at many bars and
at what comes after them, which raises four questions the existing channels do not
answer:

- **Repainting.** A pattern tool that redraws history looks better than it is. What
  may change after it was drawn, and when is a pattern really known?
- **The wire.** A live tail is two bars. `signals` have no text and the web chart
  does not draw them. `divlines` before a tail's window are taken as final, but a
  pattern's status changes far from its end, and a fib plot moves on every bar when
  the swing does.
- **Alerts.** Indicator alerts read a plot's newest value against a level. A
  detection has no level, and a value of 1 on an event bar is missed when two
  patterns follow each other. A price plot on the candle axis would wreck its scale.
- **Noise.** What does a detector find where there is nothing to find?

## Decision Drivers

- Pure detection in Rust, unit-tested on hand-built bars (ADR-0003).
- No silent repaint: state what can change, and make the rest impossible.
- A tail costs no O(bars) work and needs no reload to show a new pattern.
- Reuse ADR-0034's alert rule, and the plot alerts where a plot exists.

## Considered Options

1. **Detect in one pass over all bars.** Simple. It has hindsight: measured on a
   random walk (see below), breakouts counted before their last pivot was known, a
   moved pivot rewrote older patterns, and newer overlaps evicted older ones, so
   confirmations looked right 59% of the time and 45% of them vanished later.
2. **Keep detections server-side per chart.** Exact history, but needs per-chart
   state and storage, and a reload loses it.
3. **Walk through time on every request, with new channels (chosen).** Stateless
   and repeatable: the same closed bars always give the same patterns.

## Decision Outcome

Chosen: option 3.

### Detection

- **`candlestick_patterns`**: 50 patterns in `src/indicators/candle_patterns.rs`,
  with the shape conventions of TradingView's published Pine library (body against
  an EMA 14 of bodies, a 5% doji body). `pattern` picks all, one side, or one
  pattern. `trend` is the prior trend for patterns that need one, read on the bar
  before the pattern: close against SMA 50, also SMA 50 against SMA 200, or none. A
  hit on a closed bar only reads bars up to it, so it never changes.
- **`chart_patterns`**: 18 kinds in `src/indicators/chart_patterns.rs` on ZigZag
  pivots of the closed bars of the last 600. Deviation is in ATR(10), read at each
  pivot's own bar, so later volatility does not move older pivots and one default
  fits every timeframe. This differs from the `zigzag` indicator's percent.
- **`auto_fib`**: levels over the last two pivots; a close past the start of that
  swing breaks it, and the swing then runs from the newest pivot to the extreme
  since.
- **Elliott waves are left out.** An honest count needs a degree and alternative
  counts. One rule check over pivots would label noise with confidence.

### What can change

| | On the forming bar | After it closed |
|---|---|---|
| Candlestick label | Can change or vanish; outlined and marked | Never |
| Chart pattern | Nothing | Forming ones can reshape or vanish as swings confirm. In the newer half of the 600 bars, confirmed ones keep their confirmation, and can still fail. In the older half, confirmed ones can appear and disappear as the window slides |
| Fib levels | Nothing | Redrawn on a newer confirmed swing or a broken start. Older bars show the new levels too |

`detect` walks in the order things became known. A pivot is known `depth` bars after
its extreme, a pattern appears then, and only closes from then on can confirm it. A
pattern still open when its last pivot moves is dropped; a confirmed or failed one
stays. A newer overlapping pattern appears only if the older one failed by then, or
is still open with fewer pivots. Tests assert that adding bars to a series never changes
a past confirmation.

A live chart's series does not grow, though: its window slides, and the first pivots depend
on where it starts. So the older half of the window gains and loses confirmed patterns as
bars arrive. `examples/pattern_stats.rs` measures it, sliding one bar at a time over 11,200
closes of a random walk:

| | Newest bars | Rest of the newer half | Older half |
|---|---|---|---|
| Appeared | 81 | 1 | 271 |
| Disappeared before leaving the window | | 1 | 360 |

The late appearances were 200 to 599 bars old, nearly all over 400. The newer half holds, and
a test keeps it so; the older half does not, and this is not hidden from the user. Anchoring
the window would not fix it without per-chart state (option 2), and a tail and a full load
would then need the same anchor.

### Wire

Series entries get four optional channels; `format=summary` has compact versions.

- `labels`: text, tooltip, bias, side, and `tentative` on the forming bar. They belong
  to their bar, and merge like `signals`.
- `patterns`: strokes, a label, status, bias, tooltip, target and invalidation.
- `levels`: `lines[line]` is flat at `y` from bar `x` on. The plot stays a normal plot
  (color, width, legend, Data Window, alerts). On a new swing the client rewrites it
  on all bars and moves the merge's `from` back.
- `events`: what an alert can wait for, and the bar time of the newest occurrence.

`patterns`, `levels` and `events` are complete on every response. An index before a
response's first bar is negative, and `t` still names the bar. A tail replaces ours;
an older page's are ignored. These indicators raise `warmup` to their 600-bar lookback
(ADR-0032), so a two-bar tail reads the same window as the full load. Replay takes them
from its recompute only, and clears fib plots on older bars, so it never shows the future.

Labeled `dots` are emitted for detections on the newest closed bar only: the MCP
`indicator_signal` alert fires on any labeled dot it is handed, so older ones would
fire at once.

### Alerts

An indicator alert's right operand can be `{kind: "event", key}`. Its reading is the
newest occurrence's bar time. A new detection is a crossing under ADR-0034's one rule, so
triggers work as ever, the first reading is only a baseline, and patterns on two bars
in a row both fire. Only closed-bar detections count.

A newer time alone is not a new detection. As the window slides, the newest occurrence can
fall back to an older bar or to none, and an old bar can appear for the first time. So it must
be newer than any seen, and no older than the bar of the previous reading, and the value kept
never falls. Bars missed while a tab slept still count, being newer than that.
`tests/fixtures/pattern_event_readings.json` records what the real indicator reports as its
window slides, and the alert tests feed it to the real store.

A fib level is a plot, so ADR-0034's
*Close crossing 0.618* works as is, except that a level jumping over the price with a
new swing is no crossing: a reading from another swing is dropped.

### Consequences

- Good: `examples/pattern_stats.rs` measures it. On a random walk chart pattern
  confirmations are right 48% of the time after 10 bars, which is chance.
- Good: the same run says what users should know: on pure noise the detectors still
  find about 10 chart patterns and 238 candlestick labels per 1000 bars. A label says
  a shape occurred, not that it predicts anything.
- Bad: with one of these on the chart, a tail fetches 602 bars instead of 302, and a
  full load 1100, which is two Binance requests.
- Bad: confirmed patterns come and go in the older half of the window, as measured above.
- Bad: gaps are rare on 24/7 markets, where a bar opens at the last close, so patterns that
  need one (stars, doji star, tri-star, kicking, abandoned baby, windows, piercing, dark cloud,
  on neck, tasuki gaps) almost never fire there, and practically never on Heikin Ashi bars,
  whose open is computed.
- Bad: a daily bar counts as forming for 24 hours from its open, also after its exchange
  closed: a closed session's label stays marked tentative until then. A month ends on the
  calendar.
- Bad: editing a fib level's value renames its plot, so a color set on it resets (ADR-0017
  keys overrides by label).
- Pattern alerts run in the browser like all indicator alerts: not on the server
  (ADR-0042), and only while the chart shows that symbol, interval and settings.
