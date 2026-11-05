// Pins what compare lines rely on (ADR-0022), without a DOM: klinecharts'
// percent axis normalises every value of a pane by the close of the first
// visible bar of the MAIN series. If a klinecharts upgrade changes this, the
// compare lines (lib/compare.ts `rebaseScale`) would be normalised wrongly,
// so this test fails and the upgrade needs the ADR revisited.

import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";

const pkg = createRequire(import.meta.url).resolve("klinecharts/package.json");
const source = readFileSync(join(dirname(pkg), "dist/index.esm.js"), "utf8");

describe("klinecharts percent axis (assumed by compare lines)", () => {
  it("takes the first visible bar from the visible range's `from`", () => {
    expect(source).toMatch(
      /getVisibleFirstData = function \(\) \{[^}]*getVisibleRange\(\)\.from;[^}]*this\._dataList\[from\]/,
    );
  });

  it("maps axis values as (v - firstVisible.close) / firstVisible.close * 100", () => {
    // Autoscale range, pixel conversion and the crosshair label all use it.
    expect(source).toMatch(
      /YAxisType\.Percentage:[^]*?var fromData = chartStore\.getVisibleFirstData\(\);[^]*?\(min - fromData\.close\) \/ fromData\.close \* 100/,
    );
    expect(source).toMatch(
      /v = \(value - fromData\.close\) \/ fromData\.close \* 100/,
    );
  });
});
