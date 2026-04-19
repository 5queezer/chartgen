---
title: "0004 — Type safety: codegen from MCP schemas + Zod at boundary"
description: "Generate TypeScript types from chartgen's MCP schemas and validate responses at runtime with Zod."
sidebar:
  order: 4
---

**Status:** Accepted
**Date:** 2026-04-19

## Context and Problem Statement

PR #66 hand-wrote TypeScript interfaces in `web/src/types.ts` for the
`format=series` MCP response payload, based on reading the Rust serializer
at `src/mcp.rs::panel_result_to_series_json`. This is a classic
hand-translated-contract situation: the TypeScript type parallels the wire
format, and nothing verifies the match at build time. The moment the Rust
side adds, renames, or restructures a field, the TypeScript type drifts
silently. The bug surfaces at runtime, usually inside a component several
frames away from the code that actually parsed the payload.

The CodeRabbit comment on `histogram` vs `histograms` during PR #66
review was a symptom of exactly this class of drift — a field was named
one way on the wire and a different way in the TypeScript interface, and
nothing above the visual review caught it. The next time it will be a
silent `undefined` in production instead of a review comment in a pull
request.

chartgen already publishes tool schemas (`inputSchema` on each tool via
`tools/list`). The response shapes for some tools — notably
`format=series` — are described in the tool's human-readable description
rather than in a formal schema, so the contract is there but not always
machine-readable. Both gaps need closing, because the client treats the
wire format as ground truth either way.

## Decision Drivers

- Types on the client must track the server contract automatically, not
  by manual edits.
- Drift must fail the build, not fail in production.
- Validation must happen at the boundary — where JSON becomes an object —
  not deep inside components that assume the object is already well-formed.
- The solution must not require a second source of truth that drifts out
  of sync with the Rust code.

## Considered Options

**Hand-written TypeScript types (status quo).** Zero tooling, zero
automation, zero safety. Every server change is a manual client change,
and the compiler cannot tell us we forgot one. This is what got us the
`histogram` / `histograms` bug. Rejected.

**`ts-rs` derive on Rust types.** Generates TypeScript from Rust struct
definitions via a derive macro. Fine for REST APIs where the Rust struct
is the contract. For chartgen the contract is the MCP JSON-RPC envelope
plus the tool's `inputSchema`, not the internal Rust types — those are
serializer-side conveniences that don't always line up with the wire
shape. Using `ts-rs` would mean maintaining a second representation of
the contract inside Rust (dedicated DTO structs decorated with `ts-rs`
derives) and keeping it in sync with the MCP serializers. That is exactly
the kind of parallel-truth problem we are trying to eliminate. Rejected.

**tRPC.** End-to-end type safety for TypeScript-to-TypeScript. Not
applicable — the backend is Rust, and the contract is MCP's JSON-RPC
envelope, not tRPC's.

**Codegen from MCP schemas plus Zod at the boundary.** Chartgen's own
`tools/list` output is the canonical contract; we generate TypeScript
types from it and validate incoming responses against Zod schemas at the
point where JSON becomes an object. Chosen — see below.

## Decision Outcome

Chosen option: **codegen from MCP schemas plus Zod at the boundary**,
because the MCP schema is already the ground truth and generating types
from it makes the wire format and the compiler agree without a second
representation of the contract.

The mechanism: a new `cargo run --example gen_mcp_types` target emits the
current `tools/list` response, feeds it through
`json-schema-to-typescript`, and writes `web/src/types/generated.ts`. A
CI job — modelled on the existing `docs-drift` job that guards
`reference/indicators.md` — fails when the generated output differs from
what is committed, so forgetting to regenerate is caught automatically.
At runtime, the client validates every MCP response with a Zod schema
before the value is handed to UI code. On a mismatch, the Zod error
identifies the exact field and type that disagreed, which is far better
than the generic "Cannot read property 'x' of undefined" that happens
three components down.

For response shapes not covered by a formal schema — `format=series` is
the current example, described in the tool description text — we add a
documented Zod schema file alongside the generated types, updated in the
same PR as the serializer. This is the one piece of parallel truth that
survives, and it is explicit and local rather than scattered through
components.

## Consequences

**Positive.** Drift-proof at build time, thanks to the CI check. Runtime
errors are caught at the boundary, where they are cheap to diagnose,
rather than deep in a component tree. IDE autocomplete reflects the
actual server contract. Review effort drops — contributors stop needing
to eyeball every field name change against the consumer code.

**Negative.** The generation step must be remembered by contributors
touching the MCP surface; the CI check mitigates this, but a first-time
contributor's local workflow will hit it once before learning. Zod
schemas must be maintained manually for the response shapes that are not
in a formal `inputSchema` yet — notably `format=series`. This is
mitigated by keeping those schemas small and colocated with the code
that consumes them, and by a longer-term push to describe those shapes
in formal MCP `outputSchema` fields once the spec stabilises that
surface.

**Neutral.** Adds `json-schema-to-typescript` and `zod` as frontend
dependencies. Both are well-maintained, widely used, and small enough
that the bundle impact is negligible — Zod is tree-shakeable per-schema.

## More Information

- [Zod](https://zod.dev/)
- [`json-schema-to-typescript`](https://github.com/bcherny/json-schema-to-typescript)
- [MCP specification — tool schemas](https://spec.modelcontextprotocol.io/)
