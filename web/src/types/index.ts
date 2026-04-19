// Barrel export. See ADR-0004.
//
// - `generated-input` is auto-generated from chartgen's tools/list
//   JSON Schemas. Regenerate with `./scripts/gen-mcp-types.sh`.
// - `responses` is hand-written Zod schemas for the response payloads
//   (no formal MCP outputSchema exists upstream yet).

export * from "./generated-input";
export * from "./responses";
