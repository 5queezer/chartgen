---
title: "0016 — Light theme with a single TypeScript token source"
description: "Add a TradingView-style light theme and keep the dark and light palettes in one TS module instead of mirroring them in CSS and canvas code."
sidebar:
  order: 16
---

**Status:** Accepted
**Date:** 2026-11-04
**Amends:** [ADR-0006](./0006-tradingview-visual-parity/) (palette storage only; 0006 stays in force and is not superseded)

## Context and Problem Statement

ADR-0006 mirrors the dark palette in two places: CSS custom properties in
`globals.css` for the DOM, and a `PALETTE` object in `Chart.tsx` because the
klinecharts canvas cannot read CSS variables. TradingView offers a light and
a dark theme; adding the second palette would double every duplicated
literal. Where should the palettes live?

## Decision Drivers

- One edit per color, not one per rendering surface.
- The canvas chart must restyle live, without reloading data.
- The DOM must be correct before JavaScript runs (no white flash for dark users).
- The choice persists, and defaults to the OS preference on first load.

## Considered Options

1. Duplicate: `[data-theme=light]` block in CSS plus a second JS palette.
2. Single TS module that sets the CSS variables at runtime.
3. Single CSS source, read back with `getComputedStyle` for the canvas.

## Decision Outcome

Chosen option: 2, with the dark values kept in `@theme` as the pre-JS default.

`web/src/lib/theme.ts` exports the `dark` and `light` palettes.
`applyTheme` writes them to CSS variables on `document.documentElement`
(overriding the `@theme` defaults) and sets `color-scheme`. `Chart.tsx`
builds its `setStyles` payload from the active palette and re-applies it
when the theme changes. The `theme` field lives in the layout store, is
validated on load, and falls back to `prefers-color-scheme`.

Because Tailwind v4 needs literal values in `@theme`, the dark palette still
appears in `globals.css`; `theme.test.ts` asserts it equals `dark` so the two
cannot drift. Server-sent colors (indicator fills, hlines, hbars) are not
themed, as in TradingView. The light palette uses TradingView's values except
`fgMuted` (`#6a6d78` instead of `#787b86`), darkened to meet WCAG AA for text.

### Consequences

- Good: adding or tuning a color is a one-file change plus one test run.
- Good: theme switches repaint the canvas through `setStyles`; no data reload.
- Bad: the dark values exist twice (TS and `@theme`), guarded by a test.
- Bad: drawings already on the chart keep the colors they were created with.
