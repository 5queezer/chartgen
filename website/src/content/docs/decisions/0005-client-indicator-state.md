---
title: "0005 — Client-side indicator state with localStorage persistence"
description: "Persist the active indicator set per-browser in localStorage for v0.2; defer cross-device sync until auth-backed user state exists."
sidebar:
  order: 5
---

**Status:** Accepted
**Date:** 2026-04-20

## Context and Problem Statement

The v0.1 frontend hardcoded two indicators (`ema_stack` and `rsi`) into
the `generate_chart` request. Product direction toward TradingView parity
requires user-configurable indicator sets: users must be able to browse
the full registry, add indicators, remove them, and have their selection
survive a page reload. That forces a decision about *where* the active
set lives.

## Decision Drivers

- **Shareability.** Can a link represent a particular indicator set?
- **Offline availability.** The picker UI itself should work without a
  live backend connection.
- **Backend pressure.** Toggling an indicator on and off is a UI action;
  it should not roundtrip to the server.
- **Authentication scope.** chartgen already requires OAuth for MCP
  calls, but we do not yet have a notion of user identity tied to
  persistent profile data — the OAuth flow gates API access, not user
  records.
- **Time to ship.** v0.2 is a single-session feature; multi-session and
  multi-device concerns are explicitly deferred to a later tier.

## Considered Options

**Client `localStorage` (chosen).** The active indicator set is stored
under a single JSON key per-browser. Writes happen synchronously on
every mutation; reads happen once on app load. No backend involvement,
no auth integration, no migrations. The picker works offline because the
registry response is cached by TanStack Query with `staleTime: Infinity`
— the only thing that needs the network is the actual chart fetch.
Limitation: the set does not follow the user to another device or
browser.

**Server-side per-user.** Store the set in chartgen behind the OAuth
identity, round-trip on every toggle. Gains cross-device sync and an
audit trail. Costs: schema + migrations for user data we do not
currently persist, a latency hit on every checkbox click, and a
dependency on the OAuth provider returning a stable subject claim that
we commit to as our user identifier. That last commitment is a one-way
door: changing it later breaks every saved set.

**URL-encoded state.** Encode the set into the URL query string.
Excellent shareability — a URL becomes a chart preset. Mediocre
persistence (any nav that drops the query string loses it), and URL
length becomes a ceiling at ~30 indicators with parameters. This is a
strong option for a "share this chart" feature that we do not yet have.

**Memory only.** Drop the set on reload. Unacceptable UX: a casual tab
reload wipes the user's setup.

## Decision Outcome

Chosen: **client `localStorage` for v0.2.** It aligns with the product
state: one-browser, one-user, no persistent identity tied to saved data.
The TanStack Query key includes a stable-stringified encoding of the
active set so toggling refetches correctly without server cooperation.
The storage key is namespaced (`chartgen.indicators.active`) so a later
migration to server-backed state can coexist during the transition.

Cross-device sync is a Tier 2 concern in the product roadmap, paired
with IdP-delegated user identity. Revisit this ADR once that lands — at
minimum, the shape of the stored data should carry over as the
server-side payload, so the write path becomes "write to server; mirror
to localStorage as offline cache" rather than a rewrite.

## Consequences

**Positive.** Zero backend coupling; indicator toggles are instant; the
picker modal renders without any network except the one-shot registry
fetch, which is cached forever. No auth-integration surface. No schema
migrations to plan for a v0.2 ship.

**Negative.** No cross-device sync. Clearing browser data wipes the set.
No shareable chart URLs — TradingView-style "send this layout to a
friend" is a separate feature and is not enabled by this decision.

**Neutral.** Adds a small store module (`web/src/stores/indicators.ts`)
that owns the persistence contract. Adds a stable-stringify helper used
by the chart query key so semantic equality hits the cache.

## More Information

TradingView's production behaviour is the long-term target: authenticated
users get server-backed layouts; unauthenticated users get localStorage.
That is exactly the trajectory implied by deferring the server option
here — we are shipping the unauthenticated path first, with the shape
of the data chosen so a later migration to server-backed state is a
straightforward extension rather than a rewrite.

- Related: [ADR-0003](./0003-web-frontend-stack/) (frontend stack).
