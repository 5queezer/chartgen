---
title: "0001 — Record architecture decisions"
description: "Adopt MADR 3.0 for architecture decision records, living alongside the Starlight docs."
sidebar:
  order: 1
---

**Status:** Accepted
**Date:** 2026-04-19

## Context and Problem Statement

chartgen began as a personal tool — a Rust CLI that generates trading charts
and, over time, an MCP server that Claude can talk to. It is now turning into
something closer to a product: multiple transports, a browser frontend, an
alerts pipeline, and the start of a trading engine. Choices made at this
stage — transport protocol, frontend framework, auth model, data-contract
shape, persistence layer — have long-term consequences. They constrain what
is easy and what is expensive for months or years to come.

Today none of that rationale is written down. The commit log tells us *what*
changed but rarely *why* one option was picked over another. Anyone joining
the project later — including the author six months from now — cannot
reconstruct the constraints that shaped a decision, which makes it hard to
revisit one intelligently. We keep re-litigating small choices and forgetting
large ones.

We need a lightweight, durable way to record decisions where they can be
found alongside the code.

## Decision Drivers

- Rationale must survive personnel turnover and time.
- The overhead per decision must be small enough that we actually write
  them — a heavy template will be skipped.
- Records must live in the same repository as the code, versioned with it,
  not in a separate tool that contributors will forget exists.
- The format should be recognisable to contributors who have seen ADRs
  elsewhere, so no ramp-up is required.

## Considered Options

**MADR 3.0.** A lightweight, well-specified Markdown template that covers
context, drivers, options, outcome, and consequences. It is the de facto
standard for ADRs today. Opinionated enough to guide the author, loose
enough to fit one-paragraph decisions and multi-page ones alike.

**Michael Nygard's original ADR format.** The 2011 precursor to MADR. Fine
for capturing a decision, but structurally thinner (Context, Decision,
Consequences only) and less well-suited to recording the alternatives that
were rejected. Most of the reason to write an ADR at all is to preserve the
rejected alternatives, so giving them first-class structure matters.

**`adr-tools` CLI.** A small shell toolkit that scaffolds and numbers ADRs.
Useful for large teams, but it is another dependency to install and keep
in sync across contributors. For a project this size the manual workflow is
trivial and the tool buys us nothing we cannot get from a filename
convention.

**Off-repo systems (Notion, Linear, Confluence).** Decisions end up in a
place that is invisible to contributors reading the code, and that drift
out of sync when the code changes. Rejected — ADRs belong with the source.

## Decision Outcome

Chosen option: **MADR 3.0**, because it is the lightest format that still
forces the author to write down the alternatives, and because it lives in
the repository as plain Markdown where grep and git blame already work.

ADRs live under `website/src/content/docs/decisions/` and are published as
part of the Starlight documentation site. They are named
`NNNN-kebab-title.md` with sequential numbering starting at 0001. The
status of each ADR follows the lifecycle
`proposed → accepted → deprecated | superseded by NNNN`. Once merged, an
ADR is immutable — corrections to typos aside. To revisit a decision, write
a new ADR that supersedes the old one and add a cross-link from the old one
to the new.

Enforcement is human discipline, codified in the project's `CLAUDE.md`
contributor rules. If that proves insufficient, a lightweight CI check
(e.g. "PRs touching transport/framework/auth must reference an ADR") can be
added later.

## Consequences

**Positive.** Rationale becomes durable and searchable. Onboarding gets
easier because new contributors can read the decisions before reading the
code. Design constraints are visible in pull-request reviews, where they
can be challenged with context rather than guessed at.

**Negative.** A few minutes of overhead per meaningful decision. Some
decisions will fall into an awkward middle ground — too big to be
undocumented, too small to justify an ADR — and we will occasionally
write them anyway and occasionally skip them. That's acceptable.

**Neutral.** Enforcement is social, not mechanical. If ADRs start being
ignored, we escalate to CI, not before.

## More Information

- [MADR 3.0 specification](https://adr.github.io/madr/)
- [Michael Nygard's original post on ADRs](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions)
