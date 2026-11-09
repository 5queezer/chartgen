#!/usr/bin/env node
// Splits the Playwright e2e specs into CI shards by measured duration (ADR-0057).
//
//   node scripts/e2e-shard.mjs <shard> <total>     prints shard <shard>'s spec files,
//                                                  relative to web/, one per line
//   node scripts/e2e-shard.mjs --refresh <log>...  rewrites e2e-shard-durations.json
//                                                  from the list-reporter output of a
//                                                  green run, e.g. every shard's job log
//
// Playwright's own --shard keeps whole files together (fullyParallel is off) but
// balances them by test count, and our files differ tenfold in seconds per test.
// Every spec file under web/tests/e2e is assigned, so a new one runs before it has a
// measured time: it counts as DEFAULT_SECONDS until the next refresh.

import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const SPECS = join(root, "web/tests/e2e");
const DURATIONS = join(root, "scripts/e2e-shard-durations.json");
const DEFAULT_SECONDS = 20;

if (process.argv[2] === "--refresh") {
  const seconds = {};
  const line = /\] › tests\/e2e\/(\S+?\.spec\.ts):\d+:\d+ .*\((\d+(?:\.\d+)?)(ms|s|m)\)\s*$/;
  // A GitHub log stamps every line: the time since the previous result also counts
  // a file's hooks (a beforeAll that builds the app), which a test's own duration
  // leaves out. Without stamps, the reported durations are all there is.
  const stamp = /^(?:.*\t)?(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?Z) /;
  for (const log of process.argv.slice(3)) {
    let previous;
    for (const l of readFileSync(log, "utf8").split("\n")) {
      const at = stamp.exec(l)?.[1];
      if (at && / Running \d+ tests? using /.test(l)) previous = Date.parse(at);
      const m = line.exec(l);
      if (!m) continue;
      const s =
        at && previous !== undefined
          ? (Date.parse(at) - previous) / 1000
          : Number(m[2]) * { ms: 0.001, s: 1, m: 60 }[m[3]];
      if (at) previous = Date.parse(at);
      // A retry is the cost of a flake, not of the file.
      if (!/\(retry #\d+\)/.test(l)) seconds[m[1]] = (seconds[m[1]] ?? 0) + s;
    }
  }
  if (Object.keys(seconds).length === 0) {
    console.error("no test results in those logs; the table is unchanged");
    process.exit(1);
  }
  const sorted = Object.keys(seconds).sort().map((f) => [f, Math.round(seconds[f])]);
  writeFileSync(DURATIONS, `${JSON.stringify(Object.fromEntries(sorted), null, 2)}\n`);
  process.exit(0);
}

const [shard, total] = process.argv.slice(2).map(Number);
if (!(total >= 1 && shard >= 1 && shard <= total)) {
  console.error("usage: e2e-shard.mjs <shard> <total> | --refresh <log>...");
  process.exit(2);
}
const seconds = JSON.parse(readFileSync(DURATIONS, "utf8"));
const files = readdirSync(SPECS, { recursive: true })
  .filter((f) => f.endsWith(".spec.ts"))
  .map((f) => [f, seconds[f] ?? DEFAULT_SECONDS])
  // Longest first, then by name, so every shard computes the same split.
  .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
const load = Array(total).fill(0);
const mine = [];
for (const [f, s] of files) {
  const least = load.indexOf(Math.min(...load));
  load[least] += s;
  if (least === shard - 1) mine.push(`tests/e2e/${f}`);
}
console.log(mine.join("\n"));
