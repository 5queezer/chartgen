// Playwright config for chartgen's browser E2E suite.
//
// Implements ADR-0008. Two webServer entries spin up chartgen and the
// Vite dev server as fixtures so the suite is fully self-contained —
// `npm run test:e2e` from `web/` is all that's needed locally and in CI.
//
// Ports are configurable so parallel checkouts/sessions don't collide:
//   CHARTGEN_PORT (default 9315) — chartgen --serve
//   WEB_PORT      (default 5173) — Vite dev server
//
// Single worker because the chartgen OAuth store is process-global and
// multiple tests racing through the PKCE dance would collide on state.

import { defineConfig, devices } from "@playwright/test";

const CHARTGEN_PORT = Number(process.env.CHARTGEN_PORT ?? 9315);
const WEB_PORT = Number(process.env.WEB_PORT ?? 5173);

export default defineConfig({
  testDir: "./tests/e2e",
  fullyParallel: false,
  workers: 1,
  // Retry once in CI to smooth over first-run OAuth redirect timing.
  retries: process.env.CI ? 1 : 0,
  reporter: [["html", { open: "never" }], ["list"]],
  use: {
    baseURL: `http://localhost:${WEB_PORT}`,
    // The theme defaults to the OS scheme; pin it so specs start dark.
    colorScheme: "dark",
    trace: "on-first-retry",
    screenshot: "only-on-failure",
    video: "retain-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  // Launch chartgen + vite as managed fixtures. `cwd` is resolved
  // relative to this file; we walk up one level to the repo root for
  // the Rust binary.
  webServer: [
    {
      // CHARTGEN_DATA_SOURCE=sample makes the server serve deterministic
      // synthetic bars without network (ignored by older servers, which
      // fall back to sample data when the fetch fails). Debug build:
      // faster to compile than release and fast enough for the suite.
      command: `cargo run -- --serve --port ${CHARTGEN_PORT}`,
      cwd: "..",
      port: CHARTGEN_PORT,
      env: { CHARTGEN_DATA_SOURCE: "sample" },
      reuseExistingServer: !process.env.CI,
      // Cold build can chew several minutes on a slow CI runner.
      // Cached Swatinem builds drop this to seconds.
      timeout: 300_000,
      stdout: "pipe",
      stderr: "pipe",
    },
    {
      command: `npm run dev -- --port ${WEB_PORT} --strictPort`,
      cwd: ".",
      port: WEB_PORT,
      env: { CHARTGEN_DEV_TARGET: `http://localhost:${CHARTGEN_PORT}` },
      reuseExistingServer: !process.env.CI,
      timeout: 60_000,
      stdout: "pipe",
      stderr: "pipe",
    },
  ],
});
