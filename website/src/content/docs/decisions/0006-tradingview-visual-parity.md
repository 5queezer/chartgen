---
title: "0006 — TradingView visual parity for the web frontend"
description: "Adopt TradingView's visual language (Inter, #131722 palette, per-pane headers, last-price badge) as the default look for the chartgen browser frontend."
sidebar:
  order: 6
---

**Status:** Accepted (bundled with implementation)
**Date:** 2026-04-19

## Context and Problem Statement

The SolidJS frontend landed in PR #71 (ADR-0003) is functional but
visually generic: system-default sans-serif font, an OKLCH neutral
palette borrowed from Tailwind v4, and the stock klinecharts dark
preset. It works. It does not look like a trading tool. Product
direction for chartgen is "quality trading tool, TradingView parity" —
the frontend's resting-state visual design should read as instantly
familiar to someone who spends their day in TradingView, not as a
generic SaaS dashboard.

The question is what visual baseline to adopt. Doing nothing ships a
frontend whose look actively works against the product. Inventing a
custom design system takes design capacity we do not have and iterates
more slowly than copying a known-good reference. The bar is: can a
trader open chartgen, see a chart, and immediately know where to look?

## Decision Drivers

- **Familiarity for traders.** The target user spends hours per day
  inside TradingView. A visual language that is close to theirs has
  zero learning cost at the chart layer.
- **Signal density matching data density.** Trading dashboards are
  dense on purpose — OHLCV, volume, several indicator lines, signal
  markers, all at once. Minimalist / modern-SaaS aesthetics hide this
  density behind whitespace, which is the wrong tradeoff for this
  audience.
- **Dark-theme-first.** Traders run dashboards on second monitors for
  long sessions; reduced-glare palettes are table stakes. ADR-0003
  already committed to dark-only for v0.1.
- **No design capacity.** We are not going to design a custom visual
  system in this PR. Anything we adopt has to be mostly-ready to wear.

## Considered Options

- **Custom chartgen design system.** Build our own palette, type
  scale, component spec. Would give us brand differentiation and long-
  term control. Rejected: no design capacity; iteration would stall
  everything else; for a trading tool, brand differentiation is less
  valuable than being immediately legible to the audience.
- **shadcn/ui or similar React-tradition component library.** Visually
  excellent, well-documented, large community. Rejected: we already
  chose Kobalte for accessible primitives (ADR-0003) and have no need
  for another component layer. A generic "modern SaaS" visual from
  shadcn would also conflict with the signal-density driver.
- **Minimalist / modern-SaaS aesthetic** (generous whitespace, soft
  pastel accent, thin typography). Rejected: wrong signal density for
  trader users.
- **TradingView visual parity.** Adopt TV's dark-theme baseline as our
  own: Inter sans-serif, `#131722` background family, their candle
  palette, per-pane header rows showing indicator values, last-price
  axis highlight. Chosen — see below.

## Decision Outcome

Chosen: **TradingView's dark-theme visual language** as the chartgen
frontend baseline. Specifically:

- **Typography.** Inter variable font (weights 400/500/600) loaded
  from Google Fonts via an `<link>` in `index.html`, exposed as
  `--font-sans` in `globals.css`, and passed to klinecharts via the
  `family` property on `candle.priceMark.*`, `xAxis.tickText`,
  `yAxis.tickText`, and the candle/crosshair tooltip text styles.
- **Palette.** CSS custom properties match TradingView's dark theme:
  `--color-bg: #131722`, `--color-surface: #1e222d`,
  `--color-border: #2a2e39`, `--color-fg: #d1d4dc`,
  `--color-fg-muted: #787b86`, `--color-accent: #2962ff` (TV blue),
  `--color-bull: #26a69a`, `--color-bear: #ef5350`. The same literals
  are mirrored into the klinecharts `TV_STYLES` object in
  `web/src/components/Chart.tsx` because canvas rendering cannot
  resolve CSS variables.
- **Per-pane indicator header rows.** A new `PaneHeader.tsx` Solid
  component overlays the top-left of each pane with the indicator
  name, parameter tuple, and the last non-null value per line, colored
  to match the series stroke. The overlay is absolutely positioned
  relative to the chart host; the built-in canvas-drawn indicator
  tooltip is suppressed (`indicator.tooltip.showRule: 'none'`) so the
  two do not stack. Hover behaviour still uses klinecharts's crosshair
  tooltip, which is styled to match.
- **Last-price axis badge + horizontal line.** klinecharts has a
  native `candle.priceMark.last` feature — we enable it with a dashed
  1px line in `--color-fg-muted`, and a filled right-axis badge that
  flips between `--color-bull`, `--color-bear`, and `--color-accent`
  based on the last tick direction. No custom overlay needed.
- **Compact y-axis formatting.** `setCustomApi({ formatBigNumber })`
  collapses 43_750 → `43.75K`, 1_250_000 → `1.25M`,
  1_250_000_000 → `1.25B`. Values under 10 000 pass through
  unchanged.
- **Crosshair.** 1px dashed line in `--color-fg-muted`, with badge
  readouts on both axes using `--color-surface` as the background and
  Inter at 11 px.

**Out of scope for this decision / PR.** Visual parity for VPVR
horizontal bars, Bollinger/Ichimoku fill regions, and divergence lines
is gated by the backend being able to serialize those shapes into
`SeriesPayload` (channels `hbars`, `fills`, `divlines`). That is a
separate ADR / PR (ADR-0007, "backend channel serialization for
overlay geometry"). This ADR covers only the visual foundation.
Drawing tools, multi-chart layouts, watchlists, light theme, and a
custom theme editor are not part of visual parity and are not
addressed here.

## Consequences

**Positive.**
- Instantly-familiar look for anyone who has used TradingView.
- Signal density in the UI now matches the information density of the
  underlying data — the frontend stops visually working against the
  product direction.
- All visual tokens live in two places (CSS custom properties and the
  mirrored `TV_STYLES` object), so rebranding or theming later is a
  local edit rather than a refactor.

**Negative.**
- Tracking TradingView's visual choices is a moving target. If they
  change their palette, our "parity" claim quietly drifts; we will
  need to revisit periodically.
- Zero visual differentiation from TradingView. Acceptable for a tool
  (the brand story is the data and the trading logic, not the pixels)
  but explicit.
- Two copies of the palette (CSS variables + JS literals) must stay in
  sync. Mitigated by colocation and a one-line comment in
  `Chart.tsx`.

**Neutral.**
- Inter is served from Google Fonts CDN rather than bundled via
  `@fontsource/inter`. The font is already cached for many visitors
  from other sites, the `<link>` preconnect is a one-line cost, and
  the ~30 KB WOFF2 does not ship through our build. If CDN
  availability becomes a concern we can swap to the `@fontsource`
  package without touching the rest of the stack.
- `indicator.tooltip.showRule` is set to `'none'`. If a future PR
  wants a hover tooltip for a specific indicator, it will be a
  per-indicator override rather than a global enable.

## More Information

- [TradingView dark theme reference](https://www.tradingview.com/)
- [klinecharts `setStyles` / `Styles` interface](https://klinecharts.com/)
- ADR-0003 — Web frontend stack (SolidJS + TanStack Query + Kobalte
  + Tailwind v4)
- ADR-0007 (forthcoming) — backend channel serialization for overlay
  geometry (VPVR, fills, divergence lines)
