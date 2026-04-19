---
title: "0003 — Web frontend stack: SolidJS + TanStack Query + Kobalte + Tailwind v4"
description: "Adopt a bundled SolidJS-based stack for the chartgen browser frontend."
sidebar:
  order: 3
---

**Status:** Accepted
**Date:** 2026-04-19

## Context and Problem Statement

The v0 browser frontend scaffolded in PR #66 used Vite plus vanilla
TypeScript plus `@modelcontextprotocol/sdk` plus klinecharts. A single
screen — ticker, timeframe, chart — enough to prove that the MCP contract
works from a browser and that OAuth 2.1 PKCE authenticates a human user
against chartgen. For that narrow scope it was the right call. Reactive
frameworks would have been overhead on top of a one-screen demo.

The next set of features pushes well past one screen: real-time streaming
candles from the trading engine, an alerts UI with server-sent updates,
order entry, backtests, a saved-charts gallery. Vanilla TypeScript
accumulates imperative DOM-update code quickly once more than two or three
state variables interact, and hand-rolled reactivity does not compose —
every new feature re-implements the same subscribe/diff/update loop. We
need a framework before the glue code eats the feature pace.

## Decision Drivers

- Runtime performance for streaming data. Candle updates arrive at
  seconds-or-faster cadence per symbol; chart redraws and indicator panels
  need to re-render without jank.
- Bundle size. This is a dashboard a trader may leave open for hours on a
  secondary monitor; startup and memory footprint matter.
- Accessible-by-default UI primitives (dialogs, selects, tooltips,
  comboboxes) — rebuilding these from scratch with correct keyboard and
  screen-reader behaviour is a distraction from the trading UX.
- Small API surface for asynchronous server state — caching,
  stale-while-revalidate, retries — because every screen fetches from
  chartgen's MCP tools.
- A styling system that matches the dense, utility-driven aesthetic of a
  trading dashboard without imposing a visual design system.

## Considered Options

**React.** The largest ecosystem and the biggest library selection. That
cuts both ways: lots of templates and examples, but also the largest
runtime overhead and a reconciliation model that was not designed for
high-frequency updates. Fine-grained reactivity can be bolted on with
signals libraries, but at that point we are paying React's cost and then
working around it. Ecosystem advantage is real, but the runtime-perf and
data-streaming-fit axes both favour Solid for this workload.

**Svelte 5.** Runes are conceptually comparable to Solid's signals, and
the compiled output is comparably small. The trading-UI library situation
is weaker — fewer chart-library examples, a smaller pool of accessible
component primitives than Kobalte — so the combined stack is less
obviously a fit. Close second, but no compelling advantage over Solid.

**Vue 3.** Perfectly capable, large ecosystem, composition API is
pleasant. But nothing it offers lines up with this workload in a way the
other options do not, and its trading-UI ecosystem is comparable to
Svelte's — reasonable, not distinguished. No decisive tiebreaker.

**SolidJS + TanStack Query (Solid adapter) + Kobalte + Tailwind v4.**
Chosen — see below.

**Vanilla TypeScript (status quo).** Works for v0. Does not scale past two
or three screens without devolving into bespoke reactivity glue. Rejected
as the long-term target.

## Decision Outcome

Chosen option: **SolidJS** for the framework, **`@tanstack/solid-query`**
for server state, **Kobalte** for accessible UI primitives, **Tailwind v4**
for styling, and **klinecharts** retained from v0 for charting. Each piece
is a reasonable default on its own; the combination optimises for the
streaming-data, dense-UI profile that this app specifically has.

*SolidJS* — fine-grained reactivity compiles away the virtual-DOM layer
entirely. Signals compose naturally with WebSocket feeds: each price tick
updates only the DOM nodes that depend on it, not a component subtree.
Runtime is around 7 KB; the compiler output is close to what we would
write by hand for a specific update. This matches the streaming profile
better than any framework whose update model was designed for mostly-
static component trees.

*TanStack Query (Solid adapter)* — caching, deduplication, retries,
stale-while-revalidate, 401 retry on token refresh, devtools. Cache keys
like `['generate_chart', ticker, timeframe, indicators]` make
interval-switching instant after the first fetch, which is the difference
between "this feels like a trading terminal" and "this feels like a web
page".

*Kobalte* — WAI-ARIA-correct headless primitives: Dialog, Select,
Combobox, Tooltip, DropdownMenu, Slider. Solid-native. No visual design
system imposed; we style with Tailwind on top. Replaces the pile of
bespoke, maybe-accessible components we would otherwise write.

*Tailwind v4* — CSS-first config (no `postcss.config.js` dance), faster
build than v3, OKLCH color space for perceptually-uniform dark-mode
palettes. Dense utility classes match the trading-dashboard visual
language where almost every element is measured in single-pixel
adjustments.

## Consequences

**Positive.** Small runtime, streaming-friendly reactivity, less glue
code per screen, accessible primitives by default. The bundle stays small
enough that we keep the "opens instantly on a laptop" feel past v0. The
stack composes well: Solid signals + TanStack Query + Kobalte primitives
all speak the same reactivity language.

**Negative.** Smaller community than React. Fewer templates and examples
for trading-specific UI patterns (though TradingView's lightweight-charts
and klinecharts both have Solid integrations). Hiring is slightly
narrower; mitigated by Solid's JSX surface being close enough to React's
that the ramp-up is measured in days, not weeks.

We must port PR #66's OAuth PKCE logic to the new app scaffolding,
preserving the CSRF `state` check that was fixed during review. That port
is not free, but it is also not large — the PKCE flow is transport-
and framework-agnostic.

**Neutral.** Theme scope for v0.1 is dark-only. A light-mode toggle is
deferred until the dark theme is fully dialled in — Tailwind v4's OKLCH
palettes make the later addition straightforward but not free.

Dev server uses Vite's proxy to reach chartgen locally, not direct
browser-to-chartgen with CORS. This removes one cross-cutting concern
from local development; the production deployment continues to run the
frontend and backend on the same origin.

## More Information

- [SolidJS docs](https://www.solidjs.com/)
- [TanStack Query — Solid adapter](https://tanstack.com/query/latest/docs/framework/solid/overview)
- [Kobalte](https://kobalte.dev/)
- [Tailwind v4](https://tailwindcss.com/)
- [klinecharts](https://klinecharts.com/)
