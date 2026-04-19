#!/usr/bin/env node
// Convert chartgen's tools/list JSON-RPC result into TypeScript input
// interfaces, one per tool, named `${PascalToolName}Input` (e.g.
// generate_chart -> GenerateChartInput).
//
// Input:  the JSON dumped by `cargo run --example gen_mcp_types`. Path is
//         the first CLI arg; falls back to $CHARTGEN_TOOLS_JSON; defaults
//         to /tmp/chartgen-tools.json.
// Output: web/src/types/generated-input.ts, relative to repo root.
//
// Deterministic: tools are sorted by name before emission so the committed
// file is stable for the drift CI check.

import { readFile, writeFile, mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { compile } from "json-schema-to-typescript";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(SCRIPT_DIR, "..", "..");
const OUTPUT_PATH = resolve(REPO_ROOT, "web/src/types/generated-input.ts");

const HEADER = `// AUTO-GENERATED FROM chartgen tools/list — DO NOT EDIT.
// Regenerate with \`./scripts/gen-mcp-types.sh\`.
//
// Source: Rust \`chartgen::mcp_schema::tools_list_result()\` via
// \`examples/gen_mcp_types.rs\`.
// Drift is enforced by CI (\`mcp-types-drift\` job).

/* eslint-disable */
`;

function toPascalCase(name) {
  return name
    .split(/[_\-\s]+/)
    .map((part) => (part.length === 0 ? "" : part[0].toUpperCase() + part.slice(1)))
    .join("");
}

async function main() {
  const inputPath =
    process.argv[2] ??
    process.env.CHARTGEN_TOOLS_JSON ??
    "/tmp/chartgen-tools.json";

  const raw = await readFile(inputPath, "utf8");
  const parsed = JSON.parse(raw);

  const tools = parsed?.result?.tools;
  if (!Array.isArray(tools)) {
    throw new Error(
      `Expected JSON-RPC result.tools array at ${inputPath}; got ${typeof tools}`,
    );
  }

  // Deterministic order.
  const sorted = [...tools].sort((a, b) => a.name.localeCompare(b.name));

  const parts = [HEADER];
  for (const tool of sorted) {
    if (!tool?.name) {
      throw new Error(`Tool without name in ${inputPath}`);
    }
    const interfaceName = `${toPascalCase(tool.name)}Input`;
    const schema = {
      ...tool.inputSchema,
      title: interfaceName,
    };
    const ts = await compile(schema, interfaceName, {
      bannerComment: "",
      additionalProperties: false,
      declareExternallyReferenced: true,
      style: { singleQuote: false, semi: true },
    });
    parts.push(ts.trim() + "\n");
  }

  const out = parts.join("\n");
  await mkdir(dirname(OUTPUT_PATH), { recursive: true });
  await writeFile(OUTPUT_PATH, out, "utf8");
  console.log(`wrote ${OUTPUT_PATH} (${sorted.length} tools)`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
