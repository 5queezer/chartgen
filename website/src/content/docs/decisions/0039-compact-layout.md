---
title: "0039 — Compact layout for phones and small tablets"
description: "One app with a CSS-scoped compact layout and a single breakpoint signal, instead of a parallel mobile component tree. Mobile viewports join the E2E suite, in emulation."
sidebar:
  order: 39
---

**Status:** Accepted
**Date:** 2026-11-06
**Amends:** [ADR-0008](/chartgen/decisions/0008-e2e-playwright/) (the suite now
covers mobile viewports, and the app supports them; the rest of 0008 stays in
force)

## Context and Problem Statement

The app had no mobile layout. On a 390 px phone the fixed sidebars left the
chart about 20 px wide. ADR-0008 deferred mobile viewports because the app did
not support them. How should phones and small tablets be supported without
changing the desktop layout?

## Decision Drivers

- The desktop layout must not change.
- Rotating or resizing must keep state and not reload the chart.
- Many open branches touch the same components, so few edits inside them.

## Considered Options

1. One component tree, with a CSS-scoped compact layout.
2. A parallel mobile component tree, chosen at the breakpoint.
3. A separate mobile route or app.

## Decision Outcome

Chosen option: **1**. Options 2 and 3 remount the chart at the breakpoint, which
loses its state and refetches its data, and they duplicate every panel.

- `lib/viewport.ts` holds the one breakpoint: below 768 px wide, or on a coarse
  pointer below 1024 px wide or under 500 px tall. It drives `html.compact` for
  CSS and a `compact()` signal for the little behaviour that has to branch.
- A tablet held sideways keeps the desktop layout, with larger touch targets.
- On touch, a long press belongs to the klinecharts crosshair. The chart and
  drawing context menus stay mouse-only.
- E2E covers mobile viewports through Chromium device emulation.

### Consequences

- Good: the chart stays mounted across the breakpoint, so nothing reloads.
- Good: no second tree to keep in step with the desktop one.
- Bad: the CSS reaches into components by ARIA label and test id. Renaming
  `Drawing tools`, for example, breaks the layout without a type error. The spec
  in `tests/e2e/mobile.spec.ts` catches it.
- Bad: copy price, and adding an alert or line at a tapped price, have no touch
  equivalent.
- Bad: emulation is not a real device. WebKit, a real on-screen keyboard and
  non-zero safe-area insets are not covered.
