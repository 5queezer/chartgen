---
title: "0018 — Pine v5 semantics for indicator math"
description: "Compute every built-in indicator with TradingView Pine Script v5 semantics from one helper module, and drop the ta crate."
sidebar:
  order: 18
---

**Status:** Accepted
**Date:** 2026-11-05

## Context and Problem Statement

chartgen is used next to TradingView, and users compare the numbers. The
original indicators were built on the [`ta`](https://crates.io/crates/ta)
crate (0.5) and on small local helpers. Their seeding and smoothing differ
from Pine Script v5, so values visibly disagree with TradingView:

- `ta` EMAs seed with the first value and output from bar 0. `ta.ema` in
  Pine seeds with the SMA of the first `length` values and is `na` before.
- `ta`'s RSI smooths gains and losses with an EMA (alpha `2/(n+1)`) seeded
  with 0.1. Pine's `ta.rsi` uses Wilder's RMA (alpha `1/n`). The gap lasts
  for hundreds of bars.
- `ta`'s ATR is an EMA of the true range, and Keltner, Supertrend and the
  slow stochastic build on those EMAs. Pine uses `ta.rma` for ATR and
  `ta.sma` for the stochastic.
- `ta`'s SMA, Bollinger Bands, CCI and ROC emit partial-window values from
  bar 0. Its CCI takes the mean deviation of the close instead of hlc3.
- The local HMA rounded `sqrt(len)`, where `ta.hma` floors it. A local
  stdev dropped NaNs. The ADX seeded its DX average one bar late.

The TradingView-parity indicators added later (`trend.rs`,
`oscillators.rs`) already use `src/indicators/pine.rs`, a shared module of
Pine semantics. The built-ins did not, so the codebase had two
conventions.

## Decision Drivers

- Values should equal TradingView's for the same parameters and history.
- One implementation of each primitive (`ema`, `rma`, `stdev`, `atr`, ...)
  so a fix lands everywhere.
- Indicator output shape (lines, labels, colors, hlines, `y_range`) and the
  MCP surface stay unchanged.
- Fewer dependencies where the crate adds nothing.

## Considered Options

1. **Keep `ta` and patch around it** (pre-seed, mask warmup). Its RSI,
   ATR and CCI formulas differ in kind, not only in seeding, so most
   indicators would still need their own code.
2. **Port every built-in onto `pine.rs` and drop `ta` (chosen).**
3. **Another TA crate.** None we looked at documents Pine-compatible
   seeding and `na` handling, and it would still be a second convention
   next to `pine.rs`.

## Decision Outcome

Chosen option 2. Pine Script v5, as described in the Pine reference
manual (including its "the same in Pine" reference implementations), is
the specification. `src/indicators/pine.rs` is the single source of
those semantics. `indicator::ema`, `sma`, `highest`, `lowest` and
`stoch` remain as names for existing callers but delegate to it.

**NaN rule (Pine `na`).** Warmup values are NaN (serialized as `null`),
never 0 and never a partial window. Non-finite input counts as NaN.

- The recursive smoothers (`ta.ema`, `ta.rma`) seed with the SMA of the
  first `n` valid values. When an input is NaN, that bar's output is NaN,
  the state is kept, and the recursion continues on the next valid bar.
- Windowed helpers (`sma`, `sum`, `wma`, `stdev`, `dev`, `highest`,
  `lowest`, ...) are NaN while any NaN is inside the window.
- Division by zero is NaN. For example, a flat Williams %R or stochastic
  is NaN, not 0. `ta.rsi` is the exception: as in Pine, it returns 100
  when the average loss is 0 and 0 when the average gain is 0.
- Where a script relies on Pine's `nz()` or on comparisons with `na`
  being false, the port does the same. Examples: the Schaff Trend Cycle
  in Cipher B, the Kalman filter state in KVF, and bar 0 of `ta.mfi`.

**The `ta` crate is dropped.** No indicator uses it any more, so it is
removed from `Cargo.toml`.

**Composites.** Ports of community scripts (Cipher B, WaveTrend,
RSI/MFI/Stoch, Kalman Volume Filter) use the same helpers for their
`ta.ema` / `ta.sma` / `ta.rsi` / `ta.hma` / `ta.stdev` calls. Their own
defaults and chartgen-specific signal filters, such as Cipher B's
`strict` dot mode, are unchanged.

Remaining known differences are listed on the
[indicator reference](/chartgen/reference/indicators/#values-match-tradingview).
They are defaults that differ from TradingView's, VWAP and pivot
anchoring, Ichimoku's future projection, intrabar-based profiles, and
long warmups.

### Consequences

- Good: built-in values equal TradingView's when both start from the same
  bar with the same parameters. With enough history, the start-up effect
  of the recursive smoothers decays, so a `warmup` in series mode
  reproduces TradingView's visible values.
- Good: one module and one set of unit tests, with hand-computed Pine
  values, cover every primitive. The `ta` dependency is gone.
- Bad, **values change for existing users**. Every ported indicator
  returns different numbers, most visibly early in a series:
  - Warmup bars are now `null`.
  - RSI, ATR, ADX and Supertrend differ for a long time, because RMA
    replaces EMA.
  - The Stochastic %K/%D are SMA-smoothed.
  - The historical-volatility annualization is `sqrt(365)` (TV) instead
    of `sqrt(252)`.
  - Ichimoku's leading spans are drawn 25 bars later.
  - Parabolic SAR and Supertrend start the way `ta.sar` and
    `ta.supertrend` do.
  - Alerts and signals that read these values, such as RSI thresholds or
    Cipher B dots, can fire on different bars. Saved alert thresholds
    may need review.
- Bad: short histories show less. An indicator needs a full window before
  its first value. KVF, for example, needs about 280 bars.
