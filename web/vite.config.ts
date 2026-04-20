import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";

// Dev proxy targets the local chartgen HTTP server (default port 9315).
// This keeps OAuth + MCP on the same origin as the Vite dev server so
// localStorage + CORS are non-issues during development.
const CHARTGEN_DEV_TARGET = "http://localhost:9315";

export default defineConfig({
  plugins: [solid(), tailwindcss()],
  server: {
    port: 5173,
    proxy: {
      "/mcp": { target: CHARTGEN_DEV_TARGET, changeOrigin: true },
      "/sse": { target: CHARTGEN_DEV_TARGET, changeOrigin: true, ws: false },
      "/register": { target: CHARTGEN_DEV_TARGET, changeOrigin: true },
      "/authorize": { target: CHARTGEN_DEV_TARGET, changeOrigin: true },
      "/token": { target: CHARTGEN_DEV_TARGET, changeOrigin: true },
      "/.well-known": { target: CHARTGEN_DEV_TARGET, changeOrigin: true },
      "/oauth": { target: CHARTGEN_DEV_TARGET, changeOrigin: true },
    },
  },
  build: {
    target: "es2022",
    sourcemap: true,
  },
});
