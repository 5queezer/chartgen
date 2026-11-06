---
title: "Architecture Decision Records"
description: "Index of architecture decisions for chartgen — what was chosen, and why."
sidebar:
  order: 0
---

Architecture Decision Records (ADRs) capture the significant technical
decisions made while building chartgen. Each one describes the context that
forced the decision, the options considered, and the consequences of the
choice. The goal is that a contributor arriving six months from now — or the
author's future self — can reconstruct the *why*, not just the *what*.

## When to write an ADR

Write an ADR when a decision has long-term consequences and more than one
viable option: transport protocols, frameworks, auth models, data-contract
shape, persistence layers, a major dependency. Keep them short. Ship the ADR
in the same pull request as the code that implements the decision — never
after the fact.

chartgen uses [MADR 3.0](https://adr.github.io/madr/) as the ADR format.
Naming is sequential: `NNNN-kebab-title.md`. Statuses move through
`proposed → accepted → deprecated | superseded by NNNN`. ADRs are immutable
once merged; to revisit a decision, write a new ADR that supersedes the old
one and link both ways.

## Records

| # | Title | Status | Date |
|---|---|---|---|
| [0001](/chartgen/decisions/0001-record-architecture-decisions/) | Record architecture decisions | Accepted | 2026-04-19 |
| [0002](/chartgen/decisions/0002-mcp-transport-streamable-http/) | MCP transport: Streamable HTTP 2025-03-26 | Accepted | 2026-04-19 |
| [0003](/chartgen/decisions/0003-web-frontend-stack/) | Web frontend stack: SolidJS + TanStack Query + Kobalte + Tailwind v4 | Accepted | 2026-04-19 |
| [0004](/chartgen/decisions/0004-mcp-type-safety/) | Type safety: codegen from MCP schemas + Zod at boundary | Accepted | 2026-04-19 |
| [0005](/chartgen/decisions/0005-client-indicator-state/) | Client-side indicator state with localStorage persistence | Accepted | 2026-04-20 |
| [0006](/chartgen/decisions/0006-tradingview-visual-parity/) | TradingView visual parity for the web frontend | Accepted | 2026-04-19 |
| [0007](/chartgen/decisions/0007-complete-series-serialization/) | Complete series serialization: fills, hbars, divlines | Accepted | 2026-04-19 |
| [0008](/chartgen/decisions/0008-e2e-playwright/) | End-to-end tests with Playwright + CI integration | Accepted | 2026-11-04 |
| [0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/) | Live tail polling and history paging | Accepted | 2026-11-04 |
| [0010](/chartgen/decisions/0010-overlay-geometry-rendering/) | Overlay geometry rendering via klinecharts indicator draw callbacks | Accepted | 2026-11-04 |
| [0011](/chartgen/decisions/0011-watchlist-quotes/) | Watchlist quotes: batched get_quotes, rolling 24 h change for Binance | Accepted | 2026-11-04 |
| [0012](/chartgen/decisions/0012-geometry-colors-on-the-wire/) | Geometry colors on the wire | Accepted | 2026-11-04 |
| [0013](/chartgen/decisions/0013-indicator-visibility-and-instances/) | Indicator visibility, parameter editing and one instance per indicator | Accepted; superseded in part by [0028](/chartgen/decisions/0028-multiple-indicator-instances/) | 2026-11-04 |
| [0015](/chartgen/decisions/0015-named-layouts/) | Named chart layouts | Accepted | 2026-11-04 |
| [0016](/chartgen/decisions/0016-light-theme/) | Light theme with a single TypeScript token source | Accepted | 2026-11-04 |
| [0017](/chartgen/decisions/0017-indicator-plot-styles/) | Indicator plot styles | Accepted | 2026-11-05 |
| [0018](/chartgen/decisions/0018-pine-semantics-for-indicators/) | Pine v5 semantics for indicator math | Accepted | 2026-11-05 |
| [0019](/chartgen/decisions/0019-chart-settings/) | Chart settings: per-theme overrides over the palette | Accepted | 2026-11-05 |
| [0020](/chartgen/decisions/0020-client-side-price-alerts/) | Client-side price alerts | Accepted | 2026-11-05 |
| [0021](/chartgen/decisions/0021-realtime-chart-push/) | Real-time chart push over the MCP SSE stream | Accepted | 2026-11-05 |
| [0022](/chartgen/decisions/0022-compare-symbols/) | Compare symbols | Accepted | 2026-11-05 |
| [0023](/chartgen/decisions/0023-bar-replay/) | Bar replay: client slice, server recompute of repainting indicators | Accepted | 2026-11-05 |
| [0024](/chartgen/decisions/0024-starlark-script-runtime/) | Starlark as the script runtime for user indicators | Accepted | 2026-11-05 |
| [0025](/chartgen/decisions/0025-user-indicator-registry/) | Per-user indicator registry and MCP surface | Accepted | 2026-11-05 |
| [0026](/chartgen/decisions/0026-line-and-histogram-colors/) | Line and histogram colors on the wire | Accepted | 2026-11-05 |
| [0027](/chartgen/decisions/0027-serve-web-app-same-origin/) | Serve the web app from the MCP server's origin | Accepted | 2026-11-05 |
| [0028](/chartgen/decisions/0028-multiple-indicator-instances/) | Several instances of one indicator | Accepted | 2026-11-05 |
| [0029](/chartgen/decisions/0029-shareable-chart-links/) | Shareable chart links as query parameters | Accepted | 2026-11-05 |
| [0030](/chartgen/decisions/0030-custom-chart-types/) | Chart types klinecharts does not draw | Accepted | 2026-11-05 |
| [0031](/chartgen/decisions/0031-multiple-watchlists/) | Multiple named watchlists, sections and flags | Accepted | 2026-11-05 |
| [0032](/chartgen/decisions/0032-indicator-aware-warmup/) | Indicator-aware warmup | Accepted | 2026-11-05 |
| [0033](/chartgen/decisions/0033-indicator-projection-past-last-bar/) | Indicator projection past the last bar | Accepted; superseded in part by [0038](/chartgen/decisions/0038-custom-intervals/) | 2026-11-05 |
| [0034](/chartgen/decisions/0034-drawing-and-indicator-alerts/) | Drawing and indicator alerts | Accepted | 2026-11-05 |
| [0035](/chartgen/decisions/0035-styleable-geometry/) | Styleable indicator geometry colors | Accepted | 2026-11-05 |
| [0036](/chartgen/decisions/0036-own-origin-invalid-client-redirect/) | Redirect an unknown OAuth client back to the server's own web app | Accepted | 2026-11-06 |
| [0037](/chartgen/decisions/0037-pull-based-deploy/) | Pull-based deploy from the Coolify host | Accepted | 2026-11-06 |
| [0038](/chartgen/decisions/0038-custom-intervals/) | Custom intervals and session-aware future bars | Accepted | 2026-11-06 |
| [0039](/chartgen/decisions/0039-compact-layout/) | Compact layout for phones and small tablets | Accepted | 2026-11-06 |
| [0040](/chartgen/decisions/0040-layout-compare-restore/) | A layout stores the scale mode its compare symbols return to | Accepted | 2026-11-06 |
| [0041](/chartgen/decisions/0041-script-editor-in-the-web-app/) | Script editor in the web app | Accepted | 2026-11-06 |
| [0042](/chartgen/decisions/0042-server-side-alerts/) | Alerts that run on the server, with webhooks | Accepted | 2026-11-06 |
| [0043](/chartgen/decisions/0043-multi-chart-grid/) | Multi-chart grid: per-pane store instances behind an active-chart API | Accepted | 2026-11-06 |
| [0046](/chartgen/decisions/0046-pattern-detection/) | Pattern detection: candlesticks, chart patterns, auto fib | Accepted | 2026-11-06 |
