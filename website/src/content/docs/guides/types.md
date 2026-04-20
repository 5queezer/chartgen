---
title: TypeScript Types for MCP
description: How chartgen keeps client TypeScript types in sync with its MCP tool contract.
---

chartgen's MCP surface has two type tracks on the client:

1. **Tool inputs** — generated from each tool's `inputSchema` in
   `tools/list`. The Rust side owns the schemas; TypeScript interfaces
   are emitted from them.
2. **Tool response payloads** — hand-written Zod schemas, because MCP
   does not yet require a formal `outputSchema` and chartgen's response
   shapes live in Rust serializer functions
   (`panel_result_to_series_json`, `panel_result_to_json`,
   `tool_get_indicators`, `tool_list_indicators`).

Both tracks land in `web/src/types/`.

See
[ADR-0004 — codegen from MCP schemas + Zod at the boundary](/chartgen/decisions/0004-mcp-type-safety/)
for the decision record.

## Generated inputs

Source of truth: `chartgen::mcp_schema::tools_list_result()` in
`src/mcp_schema.rs`. Regenerate after any edit to that module (or to
`handle_tools_list` in `src/mcp.rs`):

```sh
./scripts/gen-mcp-types.sh
```

The script runs `cargo run --example gen_mcp_types`, then feeds the
`tools/list` JSON through `json-schema-to-typescript`, writing
`web/src/types/generated-input.ts`. Each tool gets a
`${PascalToolName}Input` interface, e.g. `GenerateChartInput`,
`ListIndicatorsInput`, `GetIndicatorsInput`.

CI runs the same script and diffs against the committed file — the
`mcp-types-drift` job fails when they disagree.

## Hand-written response schemas

`web/src/types/responses.ts` exports Zod schemas for every response
shape the frontend consumes:

- `SeriesPayloadSchema` — `generate_chart` with `format=series`.
- `SummaryPayloadSchema` — `generate_chart` with `format=summary` (also
  embedded in `format=both`).
- `ListIndicatorsPayloadSchema` — `list_indicators`.
- `GetIndicatorsPayloadSchema` — `get_indicators`.

Each schema is paired with a `z.infer<>` TS type. Where the Rust side
emits `null` for NaN via `finite_or_null`, the Zod schema uses
`z.number().nullable()`.

These schemas are part of the wire contract: change them in the same PR
as any change to a response-shaping Rust function. The project-wide
`CLAUDE.md` enforces this.

## Validation at the boundary

Parse incoming MCP responses with Zod before handing them to UI code:

```ts
import { SeriesPayloadSchema } from "./types";

const payload = SeriesPayloadSchema.parse(rawJson);
// payload is now a typed SeriesPayload — safe to destructure.
```

Zod raises a precise error identifying the exact field that disagreed —
far easier to diagnose than a generic `undefined` deep inside a
component tree.

## Related

- [ADR-0004 — codegen + Zod](/chartgen/decisions/0004-mcp-type-safety/)
- [MCP reference](/chartgen/reference/mcp/)
- [`json-schema-to-typescript`](https://github.com/bcherny/json-schema-to-typescript)
- [Zod](https://zod.dev/)
