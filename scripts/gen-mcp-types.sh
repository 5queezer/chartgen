#!/usr/bin/env bash
# Regenerate `web/src/types/generated-input.ts` from chartgen's MCP tool
# inputSchemas. See ADR-0004 and `website/src/content/docs/guides/types.md`.
#
# Steps:
#   1. Dump the canonical `tools/list` JSON via the Rust example.
#   2. Invoke the Node codegen, which writes the TS interfaces.
#
# CI (`mcp-types-drift`) runs this and fails if the tree changes.

set -euo pipefail
cd "$(dirname "$0")/.."

TOOLS_JSON="${TOOLS_JSON:-/tmp/chartgen-tools.json}"

echo "[gen-mcp-types] dumping tools/list -> ${TOOLS_JSON}"
cargo run --quiet --example gen_mcp_types > "${TOOLS_JSON}"

cd scripts/gen-mcp-types
echo "[gen-mcp-types] installing node deps"
npm install --silent --no-audit --no-fund --no-progress
echo "[gen-mcp-types] running generator"
node generate.mjs "${TOOLS_JSON}"
