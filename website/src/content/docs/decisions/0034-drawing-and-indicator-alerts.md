---
title: "0034 — Drawing and indicator alerts"
description: "Web alerts can watch a drawing (price crossing a trend line, ray, horizontal line or channel) or an indicator plot (against a value, another plot or the price), evaluated in the browser like price alerts."
sidebar:
  order: 34
---

**Status:** Accepted
**Date:** 2026-11-05
**Amends:** [ADR-0020](/chartgen/decisions/0020-client-side-price-alerts/) (alerts get a source besides the price)

## Context and Problem Statement

[ADR-0020](/chartgen/decisions/0020-client-side-price-alerts/) gave the web
app price alerts: the price against a fixed level. TradingView alerts can
also watch a drawing ("price crossing this trend line") and an indicator
("RSI crossing up 70", "MACD crossing Signal", "close crossing EMA 200").
These use the same conditions, but the level moves: a sloped line has a
different value on every bar, and an indicator has a new value on every
tick. Where does that level come from, how is a record shaped, and what
happens when the drawing or the indicator is gone?

## Decision Drivers

- Keep ADR-0020's model: evaluated in the browser on live ticks, stored in
  `localStorage`, alerts only fire while a tab is open, no Rust changes.
- Existing alert records must keep working without a migration step.
- The indicator math stays on the server (ADR-0003). The browser only reads
  values the chart already has.
- One evaluation rule for every kind of alert, so "once", "every time" and
  re-arming behave the same everywhere.

## Considered Options

1. **A `source` on the alert record, evaluated in the browser (chosen).**
   The record says what it watches. Drawing alerts read the drawing from
   its stored record, indicator alerts read the chart's live series.
2. **Server-side evaluation with the indicator engine.** The server knows
   every indicator, also for symbols that are not on the chart. It needs
   the alert engine in `--serve`, per-user storage and the SSE path ADR-0020
   left out, and the server has no drawings at all.
3. **Indicator alerts as their own list.** Separate storage and engine for
   indicator alerts. Duplicates the trigger, re-arm, delivery and UI code
   for no gain.

## Decision Outcome

Chosen: option 1.

- **Record.** An alert gets an optional `source`. Absent (every record from
  before this ADR) means a price alert, so no migration is needed. Otherwise:
  - `{kind: "drawing", drawingId, label}`: the drawing's record id on the
    alert's symbol, and its display name for the Alerts tab.
  - `{kind: "indicator", interval, instanceKey, line, name, against}`: the
    instance key of the indicator in the series payload
    ([ADR-0028](/chartgen/decisions/0028-multiple-indicator-instances/)),
    the index of the plot in its `lines`, a display name, and the right
    operand: `{kind: "value", value}`, `{kind: "plot", line, name}` (another
    plot of the same indicator) or `{kind: "price"}`. With `price` the price
    is the left operand and the plot the level: "Close crossing EMA 200".

  The symbol stays on the alert itself, not in the source. Non-price alerts
  store `price: 0`; their value lives in the source, so an indicator alert
  can compare with 0 or a negative value.
- **One rule.** Every source becomes a reading: the signed distances of the
  left operand from each level. The existing conditions are evaluated on two
  consecutive readings, which is ADR-0020's rule when there is one level.
  A parallel channel has two levels: crossing either line fires, greater
  means above both, less means below both.
- **Drawing levels.** A line's value at the current bar's time, linear in
  time/value space between its two points and extended past them (a trend
  line counts as extended both ways, as in TradingView). A ray has no value
  before its start; horizontal lines are constant. The price is the live
  close. The previous reading uses the line as it is now at the previous
  tick's bar time, so moving a drawing over the price does not fire it.
  Drawing alerts work for any symbol: the drawing is read from its stored
  record, and symbols off the chart are fed by the quotes poll at the time
  of the quote.
- **Drawing ids.** Drawing records gain an `id` (the klinecharts overlay id),
  kept across reloads and undo. Older records get one the next time their
  symbol is loaded.
- **Deleted drawings.** When a symbol's drawings are saved without a drawing
  an alert watches, the alert is switched off with the note
  "drawing deleted", not deleted. The Alerts tab shows it as
  *inactive: drawing deleted*; it can be edited, resumed or deleted.
- **Indicator alerts** are evaluated on each live tail of the chart, with the
  indicator's values on the forming (last) bar: a crossing is detected
  between the previous and the current tick, like TradingView's "every
  tick". Once / every time work as for price alerts. They only run while the
  chart shows that symbol at that interval with that indicator. The Alerts
  tab says *inactive: open BTCUSDT 1h* (or *indicator not on the chart*) otherwise.
- **Entry points.** A drawing's right-click menu has **Add alert on
  &lt;tool&gt;** for trend line, ray, extended line, horizontal line,
  horizontal ray and parallel channel. Indicator legend rows have a bell
  button **Add alert on &lt;indicator&gt;**: an overlay (EMA) opens as "price
  crossing its first plot", an oscillator (RSI) as "its first plot crossing
  a value". The alert dialog has **Source** (the price, a drawing, a plot)
  and **Level** (a value, or a plot) selects. An existing alert keeps its
  source; condition, value, trigger and message can be edited.

### Consequences

- Indicator alerts need the chart open on their symbol and interval in a
  visible tab. An alert on RSI 1h does not run while the chart shows 4h.
- A crossing is seen between ticks on the forming bar, so a plot that
  crosses and crosses back within one poll interval is missed, as for
  price alerts. There is no "once per bar close" option yet.
- A changed indicator setting keeps the alert on the same instance key and
  plot index: an alert on RSI (14) follows the RSI to length 21. The stored
  display name keeps the old parameters.
- Lines are read in time, not bar-index, space: on markets with gaps
  (weekends) a sloped line's alert value can differ slightly from where
  the line is drawn, which uses bar indices.
- Webhooks, email and SMS stay out of scope. Server-side alerts that run
  with no tab open would supersede this ADR and ADR-0020 together.
