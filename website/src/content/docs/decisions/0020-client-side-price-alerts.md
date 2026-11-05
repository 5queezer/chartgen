---
title: "0020 — Client-side price alerts"
description: "Evaluate web price alerts in the browser on live ticks and persist them in localStorage, instead of driving the server's alert engine."
sidebar:
  order: 20
---

**Status:** Accepted
**Date:** 2026-11-05

## Context and Problem Statement

The web frontend needs TradingView-style price alerts: "price crosses / crosses
up / crosses down a level" and "price greater / less than", shown on the chart
and in a list, firing a notification, surviving a reload. The backend already
has `set_alert`, `list_alerts` and `cancel_alert` tools, but they exist only in
`--trade` mode, next to the trading engine. The web app talks to
`chartgen --serve`, which has no engine. Where are alerts evaluated and stored?

## Decision Drivers

- No Rust changes in this work, and `--serve` must stay a plain data server.
- The browser already receives live prices: the chart's tail poll
  ([ADR-0009](./0009-live-tail-polling-and-history-paging/)) and the quotes poll
  ([ADR-0011](./0011-watchlist-quotes/)).
- Same persistence model as watchlist, drawings and layouts
  ([ADR-0005](./0005-client-indicator-state/)): localStorage, validated on load.
- A good first version, with a clear path to something sturdier.

## Considered Options

**Server engine alerts via `set_alert`, with SSE push.** Alerts would fire
with no tab open and be shared across browsers. It needs the trading engine
(or a new engine in `--serve`), per-user alert storage behind OAuth, an SSE
channel the web client does not use yet (`notifications/alert_triggered` is
only logged), and Rust work to evaluate alerts against live feeds. Too large
for this change, and the server's alert model has no every-time re-arming.

**Client-side evaluation (chosen).** Alerts live in `localStorage`
(`chartgen.alerts.list`, at most 200). Every live price goes through one
function, `processPrice(symbol, price)`, which compares it with the previous
price of that symbol and fires, deactivates or re-arms.

## Decision Outcome

Chosen: client-side alerts.

- **Prices.** The chart symbol is evaluated from the live feed's last close on
  every snapshot. Every other symbol with an active alert gets its own
  `get_quotes` poll at the watchlist's 5 s cadence (the chart symbol is left
  out so a price is never counted twice). One poll, independent of the
  watchlist panel, so alerts keep working when the Watchlist tab is not shown.
- **Semantics** follow TradingView. Crossing: the previous price on one side
  and the current on the other, or on the level. Greater / less are states.
  "Every time" crossing alerts need no extra rule; greater / less alerts stay
  disarmed after firing until the price is back on the other side.
- **Delivery.** A toast, a short WebAudio beep and, if permitted, a browser
  `Notification`. Permission is requested when the first alert is created,
  never on load.
- **UI.** Alerts are overlays on the chart (draggable), a tab in a new
  Watchlist | Alerts sidebar, and a dialog opened from the header bell, `Alt+A`
  or `openAlertDialog()`.

### Consequences

- **Alerts only fire while a tab is open** and the page is visible: both polls
  pause in a hidden tab. A browser that was asleep or closed misses crossings;
  on return only the current price is seen, so a "crossing" that happened and
  reverted in the gap is lost, while "greater / less" alerts fire if still true.
- With several tabs open, one visible tab evaluates and delivers (a Web Lock,
  `chartgen-alerts`); the others mirror the list through `storage` events.
  Without Web Locks every tab evaluates and an alert can fire once per tab.
- Alerts are per browser profile; they do not sync across devices.
- Crossings are detected between ticks, so a price that spikes through a level
  between two polls is seen as a crossing only if the polled prices straddle it.
- The server alert tools stay available for `--trade` users and agents and are
  unaffected. If server-side alerts are added to `--serve` later, a new ADR can
  supersede this one, with the browser as a client of the server's alert list
  and the SSE stream.
