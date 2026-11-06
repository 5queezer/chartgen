---
title: "0040 — A layout stores the scale mode its compare symbols return to"
description: "A layout snapshot gains an optional compareRestore, left out of the dirty check, so applying a layout gives back the scale mode to return to when the last compare symbol is removed."
sidebar:
  order: 40
---

**Status:** Accepted
**Date:** 2026-11-06
**Amends:** [ADR-0015](/chartgen/decisions/0015-named-layouts/) (a layout's state gains an optional field) and [ADR-0022](/chartgen/decisions/0022-compare-symbols/) (a layout switch no longer drops the remembered mode)

## Context and Problem Statement

The first compare symbol switches the axis to percent and remembers the mode it
replaced; removing the last one restores it. ADR-0022 kept that memory outside
layouts, and a layout switch dropped it. So applying a layout with compare symbols
lost the mode: reverting a log layout that has a compare symbol, or coming back to
it, then removing the symbol ended on the normal scale, a mode the layout never had.

## Considered Options

- **Keep the current memory when the compare list stays the same.** No format
  change. It fixes reverting a layout, but the mode leaks between two layouts with
  the same list (one on log, one on normal), and a revert to a different list still
  loses it.
- **Keep it only when re-applying the active layout.** Fixes the leak, not the
  revert to a different list or coming back from another layout.
- **Store it in the layout (chosen).**

## Decision Outcome

Chosen: an optional `compareRestore` in a layout's state, one of the scale modes.

- Written with the rest of the state when a layout is saved, while compare symbols
  are shown. Applying a layout hands it to the compare store.
- Left out of the dirty check: the user did not set it and cannot see it, so it
  must not raise the unsaved-changes dot.
- Validated on load; an invalid value is dropped.
- Exports carry it as part of the state.

### Consequences

- Each layout returns to its own mode, however it was reached.
- The stored format stays backward compatible: `version` stays 1, and older
  readers ignore the field.
- A layout saved without the field returns to the normal scale until it is saved
  again. This is what it did before, except when reverting with an unchanged list.
