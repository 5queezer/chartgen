---
title: "0028 — Several instances of one indicator"
description: "Indicator entries take an optional id that keys their result in the series and summary payloads, so one indicator can be on the chart several times with different parameters."
sidebar:
  order: 28
---

**Status:** Accepted
**Date:** 2026-11-05
**Supersedes in part:** [ADR-0013](/chartgen/decisions/0013-indicator-visibility-and-instances/) (the one-instance-per-name rule only; its hidden flag and parameter editing stay in force)

## Context and Problem Statement

TradingView lets a chart show the same indicator several times: RSI 14 and
RSI 21, or EMA 20 and EMA 50 as two `ma` instances, each with its own
inputs, style, visibility and pane. chartgen could not do this.
`generate_chart` keys the `indicators` object of its `series` and `summary`
payloads (and `indicator_errors`) by indicator name (`parse_panels` in
`src/mcp.rs`). Two requested entries with the same name got the same key,
so the second result silently overwrote the first. ADR-0013 therefore
allowed one instance per name in the web app and deduplicated stored sets
by name.

How does a response tell two instances of one indicator apart, and how
does the client map each result back to its instance?

## Decision Drivers

- Existing MCP clients keep working: bare names, `{ name, ...params }`
  objects and their response keys stay as they are.
- The mapping must not depend on the order of entries in the response.
- Push tails (`subscribe_chart`, ADR-0021), tail polls and history pages
  (ADR-0009) and replay recomputes (ADR-0023) must map to the same instance
  as the full load.
- Saved indicator sets and layouts (ADR-0005, ADR-0015) load unchanged.

## Considered Options

**Optional `id` on an entry, echoed as its response key (chosen).** An
object entry may carry `id` (a string of 1 to 64 characters). Its result is
keyed by the id instead of the name in `indicators` and `indicator_errors`.
Entries without an id keep their name as key. A name requested again
without an id is keyed `name#2`, `name#3`, and so on. Explicit ids are
reserved first, so an automatic key never takes one. A repeated or invalid
id is a tool error, and so is an id equal to the name of an entry without
one (`[{"name": "rsi", "id": "macd"}, "macd"]`): both would claim the
key `macd`. The id is not passed to the indicator as a parameter, and a
scripted indicator may not declare a param called `name`, `id`, `hidden` or
`style` (keys of an entry on the wire and in the web app).
Key assignment keeps a next-free counter per name, so it is linear in the
number of entries. `generate_chart` accepts at most 50 entries (the web
app's layout cap; `subscribe_chart` keeps its limit of 10); more is a tool
error.

**Return an array in request order.** This also separates instances, but it
changes the payload shape for every client and makes the client depend on
order. A response would also have to be matched to the request it answers.

**Key by a canonical name+params string** (for example `rsi(length=21)`).
No new field is needed, but two instances with the same params collide
again. The key changes on every parameter edit, which breaks the link to the
client-side style and visibility of the instance.

**Server-generated ids only (`name#n` by position).** This works for MCP
clients that do not care which result is which, but the client would map
by position. Hiding the first of two RSIs drops it from the request, so the
second becomes `rsi` and takes over the first one's style.

## Decision Outcome

Chosen: the optional `id`, with automatic `name#n` keys for repeats that
have no id.

The web app gives every active entry an **instance key** that does not
change while the entry exists. The first instance of a name is keyed by the
name itself and is stored as before (`"rsi"` or `{ name: "rsi", length: 14 }`).
Each later instance is stored with `id: "<name>#<n>"`, using the smallest
free `n` of 2 or more. `toWire` sends the id. Its value is the same as the
server's automatic key, so the server sees exactly what the client
assigned. Every consumer of the payload maps by instance key: panes and
legends, plot styles (ADR-0017), the data window, the object tree, the live
merge, and replay lags and recomputes.

- Adding an indicator that is already active adds a new instance with the
  schema defaults.
- Settings, hide and remove act on a single instance.
- Legends show each instance's params. When an indicator is active more
  than once, its legends, chips, object-tree rows, data-window sections and
  settings dialog title fill in the schema defaults and show the params
  that differ between the instances, so they read apart: `RSI (14)` and
  `RSI (21)`. Instances that differ in nothing show their first three
  params, so an indicator with many inputs (Cipher B) keeps a short label.

Loading is a transparent migration. Sets and layouts saved before this ADR
have no ids, and their names are unique (ADR-0013 deduplicated them), so
they load unchanged. A set with repeated names, a malformed id (not
`<own name>#<n>`, or longer than 64 characters) or a repeated id is repaired
on load: valid ids keep their key, and the other entries get the next free
key. Layout dirty checks compare instance ids too.

## Consequences

**Positive.** One indicator can be on the chart several times. The wire
change is additive, so existing requests and responses are byte-identical.
An MCP client that sends the same indicator twice now gets both results
(`rsi` and `rsi#2`) where one used to be lost. Push tails, polls and pages
need no change: they repeat the same indicator list, so their keys match.

**Negative.** A response key is no longer always an indicator name. A
client that looks up registry metadata by key must strip the `#n` suffix
(the web app does this with `keyName`) or keep its own map from id to name.
A request whose own id looks like an automatic key (for example `rsi#2` on
a `macd`) is accepted, and the automatic key of a later repeat skips to the
next number. Two instances with identical params look the same, as they do
in TradingView.

## More Information

- Supersedes in part: [ADR-0013](/chartgen/decisions/0013-indicator-visibility-and-instances/).
- Related: [ADR-0005](/chartgen/decisions/0005-client-indicator-state/),
  [ADR-0015](/chartgen/decisions/0015-named-layouts/),
  [ADR-0017](/chartgen/decisions/0017-indicator-plot-styles/),
  [ADR-0021](/chartgen/decisions/0021-realtime-chart-push/),
  [ADR-0023](/chartgen/decisions/0023-bar-replay/).
- Code: `instance_keys` / `parse_panels` in `src/mcp.rs`;
  `instanceKey`, `normalizeInstances` and `addIndicator` in
  `web/src/stores/indicators.ts`; `keyName` in `web/src/lib/instances.ts`.
