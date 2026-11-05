---
title: "0023 — Bar replay"
description: "Bar replay cuts the loaded series at a replay clock on the client and asks the server to recompute only the indicators that repaint, with end_time at the clock, throttled while playing."
sidebar:
  order: 23
---

**Status:** Accepted
**Date:** 2026-11-05

## Context and Problem Statement

TradingView's Bar Replay rewinds a chart to a past bar and plays it forward:
future bars are hidden, and indicators, overlays and drawings show what they
showed at that moment. The web chart needs the same: pick a start bar (click,
or a date), step, play at 1, 3 or 10 bars per second, and jump back to real
time. Indicator maths stays on the server
([ADR-0003](./0003-web-frontend-stack/)), so the question is where "the
chart as of bar N" comes from.

## Decision Drivers

- No indicator maths in the browser.
- Indicators must not show the future. Most outputs are causal: the value at
  bar `i` only uses bars up to `i`. Some repaint: volume and time profiles
  (`volume_profile`/VPVR, `session_vp`, `hvn_lvn`, `naked_poc`, `tpo`)
  aggregate the whole window, ZigZag's last leg and the linear regression
  channel move with each new bar, and Ichimoku's lagging span plots the close
  26 bars ahead.
- Stepping and playing (up to 10 bars per second) must stay smooth: no full
  relayout and no round trip per step.
- Replay must not disturb the live machinery: no polls or pushes while
  replaying, paging still works, and price alerts never fire on replayed
  prices.
- No wire or Rust change.

## Considered Options

**Server recompute per step** (`generate_chart` with `end_time` at the
cursor on every step). Exact for every indicator, but a round trip per bar:
10 bars per second is 10 full requests per second, and each step waits for
the network.

**Client slice only.** Cut the loaded series at the cursor: bars, every
per-bar channel, and signals and divergence lines ending at or before it.
Free and exact for causal indicators, wrong for repainting ones (a profile of
the whole window, a ZigZag leg drawn toward a pivot that has not happened).

**Client slice, server recompute for repainting indicators only (chosen).**

## Decision Outcome

Chosen: the client slice, with a throttled server recompute of the
repainting indicators.

- **Cursor.** The cursor is a replay clock: the close time of the cursor
  bar. A frame holds every bar closed by then. A history page shifts indices
  but not the clock; a symbol, timeframe or indicator change reloads and the
  new data is cut at the same clock (a 1h cursor on 15m bars ends with the
  last 15m bar closed by the 1h bar's close). If the new data starts after
  the clock, history is paged back to it first. While no bar is closed by the
  clock the frame is empty (never a later bar), and the replay bar says "No
  data before the replay start", or why paging failed.
- **Slice** (`web/src/lib/replay.ts`, `sliceSeries`). Bars and every per-bar
  array (lines, histograms, fill edges) are cut at the cursor; signals on or
  before it are kept; a divergence line is kept only if it ends on or before
  the cursor (it is known once its end bar has printed, so it is dropped, not
  clipped). Some indicators place a marker before the bar that confirms it:
  Cipher B centers its fractal dots and divergence ends two bars back
  (`center = i - 2`), the Kalman volume filter marks a cross on the bar
  before. Their markers wait that many bars (`x + lag` must be on or before
  the cursor). A moving average (`ma`, `vwma`, `hma`) with a negative
  `offset` plots later bars' values on its newest bars, so its last
  |offset| values are blank. These lags are listed per indicator in
  `replayLags`. hlines, hbars and `y_range` pass through. Indices are those of the
  full series, so the result has the shape of a live series and the chart's
  geometry code ([ADR-0010](./0010-overlay-geometry-rendering/)) needs no
  change.
- **Recompute** (`replayFrame`). For each repainting indicator the window of
  the last 500 bars before the cursor is taken from a recompute:
  `generate_chart(format=series, indicators=<only the repainting ones>,
  bars=500, warmup=300, end_time=<clock × 1000>)`. `end_time` is exclusive on
  the bar open, so exactly the bars closed by the clock are included. The
  server already recomputes windowed profiles over the visible bars when
  `warmup` is set ([ADR-0009](./0009-live-tail-polling-and-history-paging/)).
  It runs at most once per 500 ms while the cursor moves (leading and
  trailing), and again on pause, one at a time: a recompute is never aborted
  for a newer cursor (while playing, each would be outrun by the next step
  and none would land); a call meanwhile waits and runs once the one in
  flight settles. A result is used only for the same request and only if it
  ends at or before the clock, even when the cursor has moved on since. In the window, bars it
  does not cover yet (none has landed, or the cursor has moved on) have no
  value: a repainting value from the full series would show the future.
  Before the window the full series' values stay; a full-series line that
  crosses into the window (ZigZag's leg into it) is clipped at the window
  start, where the recompute's lines take over. hbars and hlines of these
  indicators come only from the recompute.
- **Chart updates.** Frames carry the live feed's `SeriesUpdate` contract: a
  step is a `tail` from the previous cursor (from the start of the recompute
  window when something repaints), so the chart appends with `updateData`
  and the view keeps following; a history page under an unchanged cursor is
  `older`, prepended through klinecharts' load callback; anything else (a new
  start before the cursor, new data) is a `reset`. Frame versions are
  numbered in their own range, far below the live feed's, so the chart never
  takes one for the successor of the other.
- **Live feed.** Entering replay suspends the live feed. The feed and the
  compare feed get a `doc` whose `visibilityState` reads `hidden` while
  replaying, so neither restarts polling or a push subscription on a
  visibility change or after a reload; paging older history still works.
  Entering also ends a push subscription
  ([ADR-0021](./0021-realtime-chart-push/)), so no pushed tick is applied.
  Exiting resets the feed with its full series (paged history included),
  which subscribes again (or polls) and catches up any gap.
- **Alerts.** Price alerts are fed from the live feed's snapshots only
  ([ADR-0020](./0020-client-side-price-alerts/)), never from frames. While
  replaying, the chart symbol is evaluated from the `get_quotes` poll like any
  other symbol, so its alerts still see real prices. The feed reset on exit
  holds the close of the replay start, older than those prices, so it is not
  taken as the alerts' baseline (it would fire an "every" crossing again).
- **Input.** While picking a start, a layer over the chart takes the mouse:
  the pick click never reaches klinecharts, so an armed drawing tool gets no
  point, and drags and the wheel are passed on as scroll and zoom. Entering
  that mode cancels an armed tool. `Esc` cancels an armed tool first and
  exits replay only when none is armed.

### Consequences

**Positive.** Steps cost no request; causal indicators are exact for free.
Repainting indicators show the server's own values at the cursor. Paging,
drawings (anchored by time), compare lines and timeframe changes keep
working. No wire or Rust change.

**Negative.** While playing, repainting indicators lag the cursor by up to
the throttle plus one round trip, and bars after the last recompute are blank
meanwhile. Only the last 500 bars before the cursor are recomputed; further
back the full series' values stay, which is what ZigZag and Ichimoku showed
anyway once those bars were 500 bars old. Marker lags are listed by hand:
an indicator added later that marks a bar before confirming it needs an
entry in `replayLags`, or its markers show early. Drawings anchored after the
cursor are drawn past the last bar, where klinecharts places future points.
A timeframe change to coarser bars shows the bar holding the clock only once
it closes, so the frame can end up to one coarse bar earlier.

## More Information

- Related: [ADR-0003](./0003-web-frontend-stack/),
  [ADR-0009](./0009-live-tail-polling-and-history-paging/),
  [ADR-0010](./0010-overlay-geometry-rendering/),
  [ADR-0020](./0020-client-side-price-alerts/).
- Code: `web/src/lib/replay.ts` (slice, frame, update rules, throttle),
  `web/src/services/replay.ts` (state, recompute, playback),
  `web/src/components/ReplayBar.tsx` (control bar, cut line, shortcuts).
