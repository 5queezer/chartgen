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
| [0008](/chartgen/decisions/0008-e2e-playwright/) | End-to-end tests with Playwright + CI integration | Accepted | 2026-04-19 |
