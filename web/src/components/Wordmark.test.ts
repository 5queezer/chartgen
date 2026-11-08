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

  it("the server's sign-in pages have the app's outlines in the dark colours", () => {
    const rust = read("../../../src/server/signin.rs");
    const svg = rust.match(/const WORDMARK: &str = r##"(.*)"##;/)![1]!;
    expect(paths(svg)).toEqual(app);
    expect([...svg.matchAll(/fill="([^"]+)"/g)].map((m) => m[1])).toEqual([dark.fg, dark.brand]);
  });

  it("the favicons are the g of gen, the same files on the app and the docs site", () => {
    // "gen" starts with the g's two contours, so every mark's path is a prefix of it.
    for (const file of ["favicon.svg", "icons/icon.svg"]) {
      const [g] = paths(read(`../../public/${file}`));
      expect(app[1]!.startsWith(g!), file).toBe(true);
      expect(g!.match(/M/g)).toHaveLength(2);
    }
    // The server answers /favicon.* itself, ahead of the built app.
    for (const file of ["favicon.svg", "favicon.ico"]) {
      const web = readFileSync(new URL(`../../public/${file}`, import.meta.url));
      for (const copy of [`../../../website/public/${file}`, `../../../src/server/${file}`]) {
        expect(readFileSync(new URL(copy, import.meta.url)).equals(web), copy).toBe(true);
      }
    }
  });
});
