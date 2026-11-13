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
| [0008](/chartgen/decisions/0008-e2e-playwright/) | End-to-end tests with Playwright + CI integration | Accepted; amended by [0039](/chartgen/decisions/0039-compact-layout/) and [0057](/chartgen/decisions/0057-e2e-shards/) | 2026-11-04 |
| [0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/) | Live tail polling and history paging | Accepted | 2026-11-04 |
| [0010](/chartgen/decisions/0010-overlay-geometry-rendering/) | Overlay geometry rendering via klinecharts indicator draw callbacks | Accepted | 2026-11-04 |
| [0011](/chartgen/decisions/0011-watchlist-quotes/) | Watchlist quotes: batched get_quotes, rolling 24 h change for Binance | Accepted | 2026-11-04 |
| [0012](/chartgen/decisions/0012-geometry-colors-on-the-wire/) | Geometry colors on the wire | Accepted | 2026-11-04 |
| [0013](/chartgen/decisions/0013-indicator-visibility-and-instances/) | Indicator visibility, parameter editing and one instance per indicator | Accepted; superseded in part by [0028](/chartgen/decisions/0028-multiple-indicator-instances/) and [0053](/chartgen/decisions/0053-hidden-indicators-stay-in-the-status-line/) | 2026-11-04 |
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
| [0027](/chartgen/decisions/0027-serve-web-app-same-origin/) | Serve the web app from the MCP server's origin | Accepted; amended by [0062](/chartgen/decisions/0062-cors-per-route-and-no-framing/) | 2026-11-05 |
| [0028](/chartgen/decisions/0028-multiple-indicator-instances/) | Several instances of one indicator | Accepted | 2026-11-05 |
| [0029](/chartgen/decisions/0029-shareable-chart-links/) | Shareable chart links as query parameters | Accepted | 2026-11-05 |
| [0030](/chartgen/decisions/0030-custom-chart-types/) | Chart types klinecharts does not draw | Accepted | 2026-11-05 |
| [0031](/chartgen/decisions/0031-multiple-watchlists/) | Multiple named watchlists, sections and flags | Accepted | 2026-11-05 |
| [0032](/chartgen/decisions/0032-indicator-aware-warmup/) | Indicator-aware warmup | Accepted | 2026-11-05 |
| [0033](/chartgen/decisions/0033-indicator-projection-past-last-bar/) | Indicator projection past the last bar | Accepted; superseded in part by [0038](/chartgen/decisions/0038-custom-intervals/) | 2026-11-05 |
| [0034](/chartgen/decisions/0034-drawing-and-indicator-alerts/) | Drawing and indicator alerts | Accepted | 2026-11-05 |
| [0035](/chartgen/decisions/0035-styleable-geometry/) | Styleable indicator geometry colors | Accepted | 2026-11-05 |
| [0036](/chartgen/decisions/0036-own-origin-invalid-client-redirect/) | Redirect an unknown OAuth client back to the server's own web app | Accepted | 2026-11-06 |
| [0037](/chartgen/decisions/0037-pull-based-deploy/) | Pull-based deploy from the Coolify host | Accepted; superseded in part by [0060](/chartgen/decisions/0060-image-built-in-ci/) | 2026-11-06 |
| [0038](/chartgen/decisions/0038-custom-intervals/) | Custom intervals and session-aware future bars | Accepted | 2026-11-06 |
| [0039](/chartgen/decisions/0039-compact-layout/) | Compact layout for phones and small tablets | Accepted | 2026-11-06 |
| [0040](/chartgen/decisions/0040-layout-compare-restore/) | A layout stores the scale mode its compare symbols return to | Accepted | 2026-11-06 |
| [0041](/chartgen/decisions/0041-script-editor-in-the-web-app/) | Script editor in the web app | Accepted | 2026-11-06 |
| [0042](/chartgen/decisions/0042-server-side-alerts/) | Alerts that run on the server, with webhooks | Accepted | 2026-11-06 |
| [0043](/chartgen/decisions/0043-multi-chart-grid/) | Multi-chart grid: per-pane store instances behind an active-chart API | Accepted | 2026-11-06 |
| [0044](/chartgen/decisions/0044-sign-in-through-github-and-google/) | Sign-in through GitHub and Google | Accepted; superseded in part by [0050](/chartgen/decisions/0050-per-user-workspace-sync/); amended by [0058](/chartgen/decisions/0058-refresh-tokens-for-headless-clients/) and [0062](/chartgen/decisions/0062-cors-per-route-and-no-framing/) and [0063](/chartgen/decisions/0063-operator-page/) | 2026-11-06 |
| [0045](/chartgen/decisions/0045-web-push-for-server-alerts/) | Browser push for server alerts | Accepted | 2026-11-06 |
| [0046](/chartgen/decisions/0046-pattern-detection/) | Pattern detection: candlesticks, chart patterns, auto fib | Accepted; amended by [0055](/chartgen/decisions/0055-provisional-swing-ends/) | 2026-11-06 |
| [0047](/chartgen/decisions/0047-price-based-charts/) | Price-based charts: Renko, Kagi, Point & Figure, Line Break, Range | Accepted | 2026-11-06 |
| [0048](/chartgen/decisions/0048-order-flow/) | Order flow from lower-timeframe bars: footprints, TPO, volume profiles | Accepted | 2026-11-06 |
| [0049](/chartgen/decisions/0049-one-kline-read-per-symbol/) | One Binance kline read per symbol and interval | Accepted | 2026-11-06 |
| [0050](/chartgen/decisions/0050-per-user-workspace-sync/) | A signed-in user's workspace, on any device | Accepted; amended by [0063](/chartgen/decisions/0063-operator-page/) | 2026-11-07 |
| [0051](/chartgen/decisions/0051-bounded-connections/) | What one connection may cost | Accepted | 2026-11-07 |
| [0052](/chartgen/decisions/0052-indicator-cost/) | What an indicator costs | Accepted | 2026-11-07 |
| [0053](/chartgen/decisions/0053-hidden-indicators-stay-in-the-status-line/) | A hidden indicator stays in the status line | Accepted | 2026-11-08 |
| [0054](/chartgen/decisions/0054-agent-drawings/) | An agent draws with the web app's tools | Accepted | 2026-11-08 |
| [0055](/chartgen/decisions/0055-provisional-swing-ends/) | Provisional swing ends in zigzag and auto fib | Accepted | 2026-11-09 |
| [0056](/chartgen/decisions/0056-png-and-summary-warm-up/) | A png or summary reads its indicators' warmup | Accepted | 2026-11-09 |
| [0057](/chartgen/decisions/0057-e2e-shards/) | The e2e suite runs in duration-balanced CI shards | Accepted | 2026-11-09 |
| [0058](/chartgen/decisions/0058-refresh-tokens-for-headless-clients/) | Refresh tokens, so a client authorized once keeps working | Accepted | 2026-11-11 |
| [0059](/chartgen/decisions/0059-setups-paper-scored/) | A setup is a stored record, scored on paper by the server | Accepted; amended by [0066](/chartgen/decisions/0066-shadow-exit-variants/) and [0068](/chartgen/decisions/0068-replay-over-real-history/) | 2026-11-11 |
| [0060](/chartgen/decisions/0060-image-built-in-ci/) | The production image is built in CI and pulled by the host | Accepted | 2026-11-11 |
| [0061](/chartgen/decisions/0061-alerts-that-wake-the-agent/) | An alert can wake the owner's agent | Accepted | 2026-11-11 |
| [0062](/chartgen/decisions/0062-cors-per-route-and-no-framing/) | Other origins get an answer on the MCP and OAuth client routes only, and the app is not framed | Accepted | 2026-11-11 |
| [0063](/chartgen/decisions/0063-operator-page/) | A read-only operator page, for a signed-in operator in the web app only | Accepted | 2026-11-11 |
| [0064](/chartgen/decisions/0064-a-setups-record/) | A setup states what it was made with, once, and the record is read back as counts | Proposed | 2026-11-11 |
| [0066](/chartgen/decisions/0066-shadow-exit-variants/) | Fixed shadow exit variants on every filled setup | Accepted | 2026-11-12 |
| [0067](/chartgen/decisions/0067-indicator-alerts-on-the-server/) | Server alerts on an indicator's line, at bar closes | Accepted | 2026-11-12 |
| [0068](/chartgen/decisions/0068-replay-over-real-history/) | Replay over real Binance history | Accepted (phase 1); phases 2 and 3 proposed | 2026-11-12 |
| [0069](/chartgen/decisions/0069-an-alert-at-a-bars-close/) | An alert at a bar's close | Accepted | 2026-11-13 |
