import { defineConfig } from "vitest/config";

export default defineConfig({
  // klinecharts' ES build, as the app gets it: Node would load the CommonJS one,
  // whose named exports it cannot see (drawingTools.test.ts registers overlays).
  resolve: { alias: [{ find: /^klinecharts$/, replacement: "klinecharts/dist/index.esm.js" }] },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
