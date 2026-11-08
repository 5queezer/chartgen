---
title: "0013 — Indicator visibility, parameter editing and one instance per indicator"
description: "Extend the persisted indicator set with a client-only hidden flag, edit parameters from the registry schema, and keep one instance per indicator name."
sidebar:
  order: 13
---

**Status:** Accepted; superseded in part by [ADR-0028](/chartgen/decisions/0028-multiple-indicator-instances/) and [ADR-0053](/chartgen/decisions/0053-hidden-indicators-stay-in-the-status-line/)
**Date:** 2026-11-04
**Amends:** [ADR-0005](./0005-client-indicator-state/) (stored shape only; 0005 stays in force and is not superseded)

> **Superseded in part by [ADR-0028](/chartgen/decisions/0028-multiple-indicator-instances/).**
> The one-instance-per-name rule and the deduplication by name on load no
> longer apply: entries carry an instance id, and the series payload is
> keyed by it. The hidden flag and parameter editing stay in force.
>
> **Superseded in part by [ADR-0053](/chartgen/decisions/0053-hidden-indicators-stay-in-the-status-line/).**
> Hidden entries are requested too, so they keep their pane and legend row;
> the client draws nothing for them. The flag itself stays client-only.

## Context and Problem Statement

ADR-0005 persists the active indicator set as an array of `generate_chart`
wire entries (`"rsi"` or `{ name, ...params }`). TradingView-style
indicator management needs two more things per entry: hiding an indicator
without losing its settings, and editing its parameters after adding it. We
also have to decide whether the same indicator may appear twice with
different parameters (two RSIs).

## Decision Drivers

- Existing persisted arrays must keep loading unchanged.
- Hidden state is a presentation concern; it must not reach the server or
  split the server-side cache.
- The server's `generate_chart` series response is keyed by indicator
  name (`parse_panels` / `tool_generate_chart` in `src/mcp.rs`).
- No Rust or wire-format changes in this iteration.

## Considered Options

**Optional `hidden: true` on the stored entry (chosen).** Entries stay
wire-shaped; the store adds one client-only key. `toWire()` drops hidden
entries and strips the key before the request. The chart hides the pane or
overlay and its legend immediately (it filters by the visible names), while
the next request simply no longer asks for that indicator; the server never
sees the flag. Old arrays have no `hidden` keys and load as
all-visible.

**Separate storage key for hidden names.** Keeps entries pure wire shape
but splits one logical state over two keys that can drift apart (remove in
one, stale in the other).

**Send hidden indicators and filter in the client.** Simplest, but wastes
server compute and bandwidth for panes nobody sees.

**Allow multiple instances per indicator.** Not possible today: two
requested entries with the same name collide on the same key in the
response, so the second silently overwrites the first.

## Decision Outcome

Entries store only parameters that differ from the schema defaults (the
picker adds the bare name), and the stored set is deduplicated by name on
load. Chosen: the optional `hidden` flag on stored entries, parameter editing by
entry index driven by the registry's `configurable_params` schema
(`{ name, type: integer|number|boolean|string, default, description? }`),
and **one instance per indicator name**, enforced by `addIndicator`.

## Consequences

**Positive.** Backward-compatible persistence; hide/unhide is instant for
the UI and the settings survive hiding; the settings form is generated, so
new indicator parameters need no frontend work.

**Negative.** Hiding drops the indicator from the request (so the cached
response key changes) and unhiding refetches it. Two instances of one
indicator (e.g. RSI 14 and RSI 21) are not supported until the backend
keys response entries distinctly, e.g. by an instance id echoed back in
the series payload; that needs a separate ADR with the wire change.
The schema has no `min`/`max`/enum today: integers are validated as
whole numbers of at least 1 (the server parses them as unsigned and a zero
length can panic it) and the
form honours optional `min`/`max` if the registry adds them.

## More Information

- Related: [ADR-0005](./0005-client-indicator-state/),
  [ADR-0006](./0006-tradingview-visual-parity/).
