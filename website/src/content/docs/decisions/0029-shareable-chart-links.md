---
title: "0029 — Shareable chart links as query parameters"
description: "A chart link is the app URL with symbol, interval, type, ind, compare and tz query parameters, applied once on load and then removed from the address bar."
sidebar:
  order: 29
---

**Status:** Accepted
**Date:** 2026-11-05
**Amended by:** [ADR-0043](/chartgen/decisions/0043-multi-chart-grid/) (in a grid, a link carries and opens in the active chart) and [ADR-0054](/chartgen/decisions/0054-agent-drawings/) (a link may carry drawings of its symbol, `draw`)

## Context and Problem Statement

All chart state lives in `localStorage`
([ADR-0005](/chartgen/decisions/0005-client-indicator-state/),
[ADR-0015](/chartgen/decisions/0015-named-layouts/)), so there is no way to
send someone the chart you are looking at. TradingView has shareable chart
links and the [parity matrix](/chartgen/guides/tradingview-parity/) lists the
gap. The app is served from the MCP server's origin
([ADR-0027](/chartgen/decisions/0027-serve-web-app-same-origin/)), so a link
is just a URL to that origin.

The OAuth redirect also lands on `/?code=...&state=...`, so share parameters
must not collide with it and must survive the round trip.

## Decision Drivers

- No new server state or endpoints.
- Readable, hand-editable URLs.
- Tolerant of bad input and of indicators this server lacks.
- Works with the sign-in redirect and with other work on indicator instances.

## Considered Options

**Query parameters (chosen).** `?symbol=ETHUSDT&interval=4h&type=candles&ind=rsi(length:21),ema_stack&compare=BTCUSDT&tz=UTC`.

**URL hash.** `#symbol=...`. Never sent to the server and out of the way of
`?code`, but it needs a second parser path next to the OAuth query handling,
and hashes are not what other chart tools or people expect to edit.

**Server-side short links.** `/s/abc123` resolved by chartgen. Short and able
to carry drawings and styles, but it needs storage, ids, expiry and abuse
handling on a server that is otherwise stateless for the web app.

## Decision Outcome

Chosen option: **query parameters**, because they need no server change, read
well and are easy to write by hand.

- Parameters: `symbol`, `interval`, `type`, `ind`, `compare`, `tz`, all
  optional. `ind` is a comma list of names with optional `(key:value;...)`
  parameters, written only when they differ from the registry default; text
  values are percent-encoded, numbers never use exponent notation. Hidden
  indicators are left out, `hidden`, `style` and `name` are never read from a
  link, and `ind` is limited to 2 KB and 50 entries. `ind=` is an explicit
  empty list and `compare=` clears the compare symbols.
- The app reads them once, before the first render (`applyShareLink`), through
  the normal store setters, so they persist like a manual change. It then
  removes only the share parameters from the address bar with
  `history.replaceState`; there is no live URL syncing.
- Because the values are written to the stores before any sign-in redirect,
  they survive the OAuth round trip without being stashed. `code` and
  `state` are never touched.
- Invalid values fall back to the stored state and decoding never throws.
  `tz` must be one of the zones the app offers (the keywords are
  case-insensitive).
- Indicators a link brings in are decoded as raw strings and checked against
  `list_indicators` before the first chart request (the query is held until
  then). Their names are kept in `sessionStorage` so the check still happens
  after the OAuth redirect. Unknown names are removed, and parameters are typed
  by the registry spec and dropped when invalid (type, min/max, enum), so the
  default applies; a toast lists what was left out. Only names from the link are
  touched, because `list_indicators` can fall back to an empty list when script
  storage fails and a global prune would then wipe the user's own scripts. If
  the list cannot be loaded at all, the link's indicators are kept as decoded.
- "Copy link to chart" in the toolbar and the chart context menu builds the
  URL from the current state, uses `navigator.clipboard` and falls back to a
  hidden readonly input with `execCommand("copy")`, then shows a toast.
- The parsing and encoding is one module, `web/src/lib/shareLink.ts`, with unit
  tests.

### Consequences

- Good: no server work; links are readable and diffable.
- Good: unknown or malformed input degrades to the stored chart.
- Bad: links carry the chart setup only. Drawings, alerts, indicator styles
  and chart colors are not shared.
- Bad: opening a link overwrites the receiver's stored symbol, timeframe,
  chart type, indicators, compare symbols and time zone, as a manual change
  would. A named layout is the way to keep the old setup.
- Good: a repeated name in `ind` is another instance of the indicator
  ([ADR-0028](/chartgen/decisions/0028-multiple-indicator-instances/)):
  `ind=rsi,rsi(length:21)` yields `rsi` and `rsi#2`. Decoding runs the list
  through `normalizeInstances`, so ids are valid and unique, and the check
  against the registry runs per instance (by instance key).
- Bad: the first chart load waits for `list_indicators` when the link
  carries indicators.
