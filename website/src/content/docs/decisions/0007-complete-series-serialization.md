---
title: "0007 — Complete series serialization: fills, hbars, divlines"
description: "Emit all PanelResult channels in format=series so TradingView-parity rendering is unblocked on the wire."
sidebar:
  order: 7
---

**Status:** Accepted
**Date:** 2026-04-19

## Context and Problem Statement

`src/mcp.rs::panel_result_to_series_json` — the serializer behind
`generate_chart` under `format=series` — covered only 4 of the 7 channels
that a `PanelResult` can carry. `lines`, `bars` (as `histogram`), `dots`
(as `signals`), and `hlines` were emitted; `fills`, `hbars`, and
`divlines` were silently dropped on the wire.

Those three channels are not decorative detail. They are the only way to
render a whole class of chartgen's 33 indicators:

- `fills` — shaded regions between two y-series: Bollinger Bands, Keltner
  Channels, Ichimoku cloud, VWAP bands.
- `hbars` — horizontal bars at price levels: VPVR (Volume Profile
  Visible Range), Session Volume Profile, HVN/LVN nodes, Naked POC, TPO.
- `divlines` — two-point diagonal segments: WaveTrend divergences,
  RSI divergences emitted by Cipher B.

Without them, a JSON-consuming client has to either fall back to the PNG
output or skip those indicators entirely. That blocks TradingView-parity
rendering on the wire, which is the whole point of `format=series`.

## Decision Drivers

- `format=series` is documented as the "full per-bar time-series" shape
  for chartgen. Dropping channels contradicts that contract.
- TV-parity rendering for BBands-style bands, VPVR-style overlays, and
  divergence markers requires these channels; they cannot be recomputed
  client-side without duplicating indicator logic.
- The change must be additive — existing consumers reading `lines`,
  `signals`, and so on must not see their payload shape shift.

## Considered Options

**Keep dropping `fills`, `hbars`, `divlines`.** Zero work, but locks the
frontend out of VPVR, band-fill regions, and divergence lines for every
indicator that emits them. Rejected — this is precisely what the PR is
meant to fix.

**Add a separate `format=full` that includes these channels, leave
`series` alone.** Splits the API surface for no real benefit. `series` is
already described as the full per-bar view; a second "really full" format
is a worse contract, not a better one. Clients end up having to know the
difference, and documentation has to explain why there are two. Rejected.

**Ship the raw `bars[]` and have the client recompute the missing
channels from price data.** Wastes work on both sides: the server already
has an authoritative indicator engine, and duplicating that engine in the
client is expensive and drifts the moment an indicator changes. Rejected.

**Emit the three channels in the existing `series` payload, additively.**
Matches the documented "full per-bar time-series" contract, keeps the API
surface flat, and preserves backward compatibility for current consumers
because the new keys are only present when the indicator emits them.
Chosen — see below.

## Decision Outcome

Chosen option: **emit `fills`, `hbars`, and `divlines` alongside the
existing channels in `panel_result_to_series_json`.** The shape is
additive and mirrors the existing conventions:

- `fills`: array of `{ y1: [...], y2: [...] }`. Each array is 1:1 with
  `bars[]`; pre-warmup values are `null` via the existing
  `finite_or_null` helper. Colors chosen server-side are dropped — the
  client themes per indicator on its side.
- `hbars`: array of `{ y, height, width, offset, left }`. Field names
  preserve the exact semantics of `chartgen::indicator::HBar` — `y` and
  `height` in price space, `width` and `offset` as 0.0–1.0 fractions of
  chart width, and the boolean `left` that picks the anchor edge. Color
  dropped.
- `divlines`: array of `{ x1, t1, y1, x2, t2, y2, dashed }`. Each
  endpoint carries both `x` (bar index) and `t` (unix seconds, or `null`
  for synthetic sample data), mirroring the signal pattern. Color
  dropped.

All three channels are omitted from the per-indicator object when the
indicator emits none, matching the convention already used for `lines`,
`histogram`, `signals`, and `hlines`. The summary serializer
(`panel_result_to_json`) is unchanged; summary keeps its last-value
semantics.

## Consequences

**Positive.** Volume-profile indicators (VPVR, Session VP, HVN/LVN,
Naked POC, TPO) are now fully renderable from the JSON payload. Band
fills for BBands/Keltner/Ichimoku/VWAP land on the wire. Divergence
segments from WaveTrend and Cipher B come through as data rather than
being flattened to signals only. API consumers reach parity with the
image output for every indicator chartgen ships.

**Negative.** Wire size grows for indicators that emit many `hbars` —
VPVR with 32 bins adds roughly 1 KB of JSON per indicator. Three new Zod
schemas (`SeriesFillSchema`, `SeriesHBarSchema`, `SeriesDivLineSchema`)
have to be kept in sync with the Rust serializer, same as the existing
per-response schemas (see [ADR-0004](/chartgen/decisions/0004-mcp-type-safety/)).

**Neutral.** Non-breaking for current consumers because the new keys are
conditional on non-empty channels. Frontend rendering for the three new
channels is out of scope for this change and will land in a follow-up
PR.

## More Information

- [ADR-0004 — Type safety: codegen + Zod at boundary](/chartgen/decisions/0004-mcp-type-safety/)
- [`reference/mcp.md`](/chartgen/reference/mcp/) — series payload shape documentation
