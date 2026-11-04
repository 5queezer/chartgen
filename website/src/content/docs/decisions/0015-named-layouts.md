---
title: "0015 — Named chart layouts"
description: "Store named snapshots of symbol, timeframe, chart type, scale mode, indicators and drawings under one localStorage key, applied through the existing stores."
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
- `localStorage` is small (about 5 MB) and drawings already have caps
  (200 KB per symbol, 100 symbols).
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

**Move drawings into the layout and drop the per-symbol keys.** Layouts
would own their drawings cleanly, but it changes the drawing format and
its LRU/size logic, and drawings would no longer be shared between layouts
on the same symbol.

**Snapshot every symbol's drawings.** Simple to reason about, but 50
layouts times 100 symbols times 200 KB cannot fit in storage.

## Decision Outcome

Chosen: snapshots under `chartgen.layouts`, applied through the existing
setters. Drawings stay in their per-symbol keys, and a layout keeps a copy
for the symbol it was saved on plus the symbols it touched earlier, at most
5 (the newest are kept), each already capped at 200 KB. Applying a layout
writes those copies back to the per-symbol keys, so a layout owns the
drawings of the symbols it lists; other symbols are left alone. The
unsaved-changes comparison covers the current symbol's drawings only, so a
drawing on another symbol never marks the layout dirty.

Other rules:

- At most 50 layouts; names are 1 to 64 characters and unique
  case-insensitively (a clash gets ` (2)`, ` (3)`...).
- A fresh install, or a file that is missing or entirely unusable, becomes
  one layout named `Unnamed` built from the current state. Malformed items
  are dropped; a stale `active` falls back to the first item.
- Leaving a layout with unsaved changes asks Save / Don't save / Cancel.
- Export and import use `{ version: 1, name, state }` and the same
  validation as storage.

## Consequences

**Positive.** No change to existing persisted formats; layouts are
purely additive and removing the feature leaves the old keys valid.
Corrupt layout data costs only the saved copies, never the live chart.

**Negative.** State is duplicated (live stores and snapshot), so the dirty
check must compare normalised forms. Applying a layout overwrites the
per-symbol drawings of the symbols it lists, including drawings made
elsewhere since it was saved. The dot ignores drawing edits on symbols
other than the current one. Snapshots of a symbol the layout never visited
are not kept, so drawings made there are not part of the layout until it
is saved on that symbol.

## More Information

- Related: [ADR-0005](./0005-client-indicator-state/),
  [ADR-0013](./0013-indicator-visibility-and-instances/).
