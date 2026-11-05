---
title: "0031 — Multiple named watchlists, sections and flags"
description: "The watchlist is a versioned localStorage document of named lists, each an ordered mix of symbols and section dividers, plus one flag color per symbol shared by all lists."
sidebar:
  order: 31
---

**Status:** Accepted
**Date:** 2026-11-05

## Context and Problem Statement

The watchlist was a single array of symbols under
`chartgen.watchlist.symbols`
([ADR-0011](/chartgen/decisions/0011-watchlist-quotes/)). TradingView has
several named lists, sections inside a list, drag reordering and color flags,
and the [parity matrix](/chartgen/guides/tradingview-parity/) lists the gap.
The storage format has to hold all of that, keep the old list without data
loss, and still let the quotes poll stay within the `get_quotes` limit of 50
symbols.

## Decision Drivers

- No server state: the watchlist stays in the browser
  ([ADR-0005](/chartgen/decisions/0005-client-indicator-state/)).
- Existing users keep their list.
- Reordering and sections must not need ids on every symbol.
- A format that can grow (another version) without a second migration scheme.

## Considered Options

**One versioned document (chosen).** `chartgen.watchlist.v2` holds
`{ version: 2, active, lists, flags }`.

**One key per list.** Simple writes, but the list order, the active list and
the cross-list flags need extra keys that can disagree after a partial write.

**Sections as a field on each symbol.** Reordering sections would mean
rewriting every symbol, and an empty section would have nowhere to live.

## Decision Outcome

Chosen option: **one versioned document**, because every change is one
`setItem` and the loader can validate the whole thing at once.

- `lists` is an array of `{ id, name, items }`. `items` is an ordered mix of
  `{ kind: "symbol", symbol }` and
  `{ kind: "section", id, name, collapsed }`. A symbol belongs to the section
  above it; symbols before the first section belong to none. Removing a
  section removes only the divider.
- `flags` maps a symbol to one of `red`, `orange`, `yellow`, `green`, `cyan`,
  `blue` or `purple`. A flag belongs to the symbol, not to a list, so it shows
  in every list that holds the symbol. A flag is dropped when no list holds
  the symbol any more.
- `active` is stored: a list id, or the reserved id `flagged`, a virtual
  read-only view of every flagged symbol across lists. The contents of that
  view are derived from the lists and flags, never stored, so they cannot go
  out of date. It shows at most 50 symbols.
- Limits: 20 lists, 20 sections per list, 50 symbols per list (the
  `get_quotes` limit), names up to 40 characters. The last list cannot be
  deleted.
- Quotes are polled for the symbols of the active view only, so the 50-symbol
  cap applies per list and switching lists changes the poll.
- Another tab's write (the `storage` event) replaces the lists in this tab,
  which keeps its own active list while that list still exists.
- A symbol never lands hidden: when a move or a section removal puts a
  symbol under a collapsed section, that section is expanded in the same
  save. A section never moves above the first section, so the ungrouped
  symbols at the top stay ungrouped.
- Moving is by index in the flat `items` array. A section moves together with
  the symbols under it and snaps to a block boundary. Drag and drop is
  pointer-based on a grip button and Alt+ArrowUp/Down moves the focused row,
  so reordering also works without a pointer.
- Migration: when `v2` is absent or invalid, the old `symbols` key (validated
  as before, defaults when missing or corrupt) becomes the list "Watchlist"
  with no sections. The old key is left in place and the first change writes
  `v2`, which wins from then on. `chartgen.watchlist.collapsed` is unchanged.
- Loading validates everything: invalid or duplicate symbols, ids and flags
  are dropped, caps are enforced, and an unusable document falls back to the
  migration path.

### Consequences

- Good: no data loss on upgrade, one atomic write per change, and the format
  carries a version for the next change.
- Good: the Flagged view needs no storage of its own.
- Bad: lists and flags are per browser profile with no sync, like layouts and
  alerts.
- Bad: only one Flagged view; TradingView also has one list per flag color.
  Adding a color filter later needs no format change.
- Bad: a downgrade to an older build ignores `v2` and shows the old list.
- A document with a `version` above 2 is never overwritten: that session
  starts from the old list and keeps its changes in memory only.

## More Information

Implemented in `web/src/stores/watchlist.ts` (with unit tests) and
`web/src/components/Watchlist.tsx`. User guide:
[Watchlist](/chartgen/guides/web/#watchlist).
