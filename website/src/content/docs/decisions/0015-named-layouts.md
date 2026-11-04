---
title: "0015 — Named chart layouts"
description: "Store named snapshots of symbol, timeframe, chart type, scale mode and indicators under one localStorage key; drawings stay global per symbol, applied through the existing stores."
sidebar:
  order: 15
---

**Status:** Accepted
**Date:** 2026-11-04
**Amends:** [ADR-0005](./0005-client-indicator-state/) and [ADR-0013](./0013-indicator-visibility-and-instances/) (adds a key; their stored shapes are unchanged)

## Context and Problem Statement

The web frontend persists its chart state in several independent
`localStorage` places: toolbar state under `chartgen.layout.*`, the active
indicator set under `chartgen.indicators.active` (ADR-0005, ADR-0013) and
drawings per symbol under `chartgen.drawings.<SYMBOL>`. TradingView-style
use needs several named setups that can be saved, switched, renamed and
deleted, with an unsaved-changes marker. How do layouts relate to that
existing state?

## Decision Drivers

- The persisted formats of the existing stores must not change.
- Everything read from storage is validated; corrupt data must never break
  the app.
- Applying a layout must never silently destroy user work such as
  drawings made since it was saved.
- No Rust or wire changes.

## Considered Options

**Snapshots under one new key, live state stays in the existing stores
(chosen).** `chartgen.layouts` holds `{ version: 1, active, items: [{ id,
name, updatedAt, state }] }`. The existing stores remain the live state
and the single source of truth for rendering; a layout is a saved copy.
Saving snapshots the stores, switching writes the snapshot back through
their setters. The unsaved-changes dot is a comparison of the two.

**Make layouts the source of truth and derive the old keys.** One place
for state, but every existing store and its tests change, and a bug would
lose live state instead of a saved copy.

**Include drawings in the layout.** Either by moving them into the
layout or by snapshotting them per symbol. Both make applying a layout
overwrite the per-symbol drawings made since the last save, which is
silent data loss, and snapshots of every symbol cannot fit in storage
(50 layouts times 100 symbols times 200 KB). Rejected.

## Decision Outcome

Chosen: snapshots under `chartgen.layouts` of symbol, timeframe, chart
type, scale mode and the indicator set, applied through the existing
setters. **Layouts do not include drawings.** Drawings stay global per
symbol under `chartgen.drawings.<SYMBOL>`, as in TradingView with "sync
drawings to all layouts": switching layouts never reads, writes or
removes them, and they never mark a layout as changed. A leftover
`drawingsBySymbol` key in stored or imported layouts is ignored.

Other rules:

- At most 50 layouts; names are 1 to 64 characters and unique
  case-insensitively (a clash gets ` (2)`, ` (3)`...).
- A fresh install, or a file that is missing or entirely unusable, becomes
  one layout named `Unnamed` built from the current state. Malformed items
  are dropped; a stale `active` falls back to the first item.
- Leaving a layout with unsaved changes asks Save / Don't save / Cancel.
- "Create new layout" keeps the current symbol and timeframe and resets
  chart type, scale mode and indicators to their defaults.
- Export and import use `{ version: 1, name, state }`. Import refuses files
  over 256 KB and other versions, keeps at most 50 indicators, and keeps
  only number, string and boolean params (with `__proto__`, `constructor`
  and `prototype` keys dropped); stored layouts get the same sanitising.
  Import is validated before any unsaved-changes prompt.
- The open layout cannot be deleted (as in TradingView); deleting is done
  from the load list, so there is always at least one layout.
- Every change re-reads `chartgen.layouts` from storage before applying
  it, and a `storage` event refreshes the list, so a second tab does not
  clobber the first. Each tab keeps its own active layout.
- If the stored value exists but is unusable (bad JSON, another version,
  no valid items), it is copied once to `chartgen.layouts.bak` and never
  overwritten by the in-memory fallback; the first explicit change writes
  a fresh file and the `.bak` stays.
- The unsaved-changes check compares indicator entries with params equal
  to the registry default removed (the settings dialog strips them too),
  so pressing OK without edits is not a change. The hidden flag is
  compared, so hiding or showing an indicator does count as a change.
- No `beforeunload` prompt: live state is persisted per key as it changes,
  so leaving the page loses nothing. The dot only means "differs from the
  saved layout".

## Consequences

**Positive.** No change to existing persisted formats; layouts are
purely additive and removing the feature leaves the old keys valid.
Corrupt layout data costs only the saved copies, never the live chart.

**Negative.** State is duplicated (live stores and snapshot), so the dirty
check must compare normalised forms. A layout cannot carry its own
drawings: the same symbol shows the same drawings in every layout, and
per-layout drawings would need a new ADR that solves the overwrite
problem.

## More Information

- Related: [ADR-0005](./0005-client-indicator-state/),
  [ADR-0013](./0013-indicator-visibility-and-instances/).
