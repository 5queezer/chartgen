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

- Windowed helpers (`sma`, `sum`, `wma`, `stdev`, `dev`, `highest`,
  `lowest`, ...) ignore NaN values, as the Pine reference documents for
  `ta.sma`, `ta.highest`, `ta.lowest`, `ta.stdev`, `math.sum` and
  `ta.mfi`. They compute over the last `n` valid values and are NaN only
  until `n` valid values have been seen. A NaN bar therefore still gets
  the value of the previous `n` valid inputs, so a zero-range stochastic,
  a Cipher B bar with `high == low` or a WaveTrend bar with `d == 0`
  leaves no gap.
- The recursive smoothers (`ta.ema`, `ta.rma`) seed with the SMA of the
  first `n` valid values. On a NaN input they keep their state and repeat
  the previous value (no gap). **This behaviour is unverified against a
  live TradingView chart**; the Pine reference only says these functions
  ignore `na`.
- Running sums are re-summed exactly every `n` updates and are exactly 0
  when every value in the window is 0. Zero tests on sums use a relative
  epsilon (`|x| <= 1e-12 * scale`), so `ta.mfi` and CMF do not turn float
  residue (seen as 1.65e-8 at BTC scale) into a value.
- Division by zero is NaN. For example, a flat Williams %R or stochastic
  is NaN, not 0. `ta.rsi` is the exception: as in Pine, it returns 100
  when the average loss is 0 and 0 when the average gain is 0.
- Where a script relies on Pine's `nz()` or on comparisons with `na`
  being false, the port does the same. Examples: the Schaff Trend Cycle
  in Cipher B, the Kalman filter state in KVF, bar 0 of `ta.mfi`, and the
  previous bands in `ta.supertrend`.

**Timeframe and calendar.** HV annualization, VWAP anchors and pivot
periods need the chart timeframe. It comes from `data.interval`, parsed
in any letter case and multiple (`1D`, `1W`, `2w`; a capital `M` means
months), else from the spacing of the bar times. An unknown timeframe
stays unknown: HV is NaN, it is not treated as intraday. Calendar
periods are UTC days, ISO weeks, months and years of the bar times.

**Defaults are TradingView's.** Defaults are part of the MCP contract, so
these changes are listed explicitly:

| Indicator | Parameter | Before | Now (TradingView) |
|---|---|---|---|
| `keltner` | `multiplier` | 1.5 | 2.0 |
| `keltner` | `atr_length` (new) | ATR length = `period` | 10 |
| `stoch` | `k_smooth` | 3 | 1 |
| `roc` | `period` | 14 | 9 |
| `histvol` | `period` | 20 | 10 |
| `vwap_bands` | `std_dev` | 2.0 | 1.0 |
| `vwap`, `vwap_bands` | `anchor` (new) | none (from the first bar) | `session` |
| `ichimoku` | `displacement` (new) | not shifted | 26 |

`wavetrend` keeps the VuManChu Cipher B lengths 9 / 12 / 3, because it is
the Cipher B component; LazyBear's standalone script uses 10 / 21 / 4.

**The `ta` crate is dropped.** No indicator uses it any more, so it is
removed from `Cargo.toml`.

**Composites.** Ports of community scripts (Cipher B, WaveTrend,
RSI/MFI/Stoch, Kalman Volume Filter) use the same helpers for their
`ta.ema` / `ta.sma` / `ta.rsi` / `ta.hma` / `ta.stdev` calls. Their own
defaults and chartgen-specific signal filters, such as Cipher B's
`strict` dot mode, are unchanged.

Remaining known differences are listed on the
[indicator reference](/chartgen/reference/indicators/#values-match-tradingview).
They are UTC calendar periods instead of exchange sessions, pivots built
from the loaded bars, the cloud projected past the last bar,
intrabar-based profiles, and long warmups.

### Consequences

- Good: built-in values equal TradingView's when both start from the same
  bar with the same parameters. With enough history, the start-up effect
  of the recursive smoothers decays, so a `warmup` in series mode
  reproduces TradingView's visible values.
- Good: one module and one set of unit tests, with hand-computed Pine
  values, cover every primitive. The `ta` dependency is gone.
- Bad, **values change for existing users**. Every ported indicator
  returns different numbers, most visibly early in a series:
  - Warmup bars are now `null`. A NaN inside a series no longer creates
    a gap.
  - RSI, ATR, ADX and Supertrend differ for a long time, because RMA
    replaces EMA.
  - The Stochastic %K/%D are SMA-smoothed.
  - The historical-volatility annualization is `sqrt(365 / per)` (TV)
    instead of `sqrt(252)`.
  - Ichimoku's leading spans are drawn 25 bars later, and it has a new
    "Lagging Span" line.
  - VWAP resets every session by default, and pivots come from the
    previous day / week / month / year instead of the previous bar.
  - The new defaults above.
  - Alerts and signals that read these values, such as RSI thresholds or
    Cipher B dots, can fire on different bars. Saved alert thresholds
    may need review.
- Bad, output shape: Parabolic SAR has no dot on bar 0 (`ta.sar` is `na`
  there), and Supertrend draws nothing on bar 0 or during the ATR
  warmup. KVF draws no dots until its 200-bar normalisation has data,
  where it used to draw 0-valued placeholder dots. Ichimoku has one more
  line.
- Bad: short histories show less. An indicator needs `length` valid
  values before its first value. KVF, for example, needs about 280 bars.
