import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { dark, light } from "../lib/theme";

const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");
const paths = (source: string) => [...source.matchAll(/ d="([^"]+)"/g)].map((m) => m[1]);

describe("wordmark", () => {
  const app = paths(read("./Wordmark.tsx"));

  it("is two paths, chart and gen", () => {
    expect(app).toHaveLength(2);
  });

  it.each([
    ["dark", dark],
    ["light", light],
  ] as const)("the docs site's %s logo has the app's outlines and colours", (theme, palette) => {
    const svg = read(`../../../website/src/assets/logo-${theme}.svg`);
    expect(paths(svg)).toEqual(app);
    expect([...svg.matchAll(/fill="([^"]+)"/g)].map((m) => m[1])).toEqual([palette.fg, palette.brand]);
  });
});
