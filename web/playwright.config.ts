// Playwright config for chartgen's browser E2E suite.
//
// Implements ADR-0008. Two webServer entries spin up chartgen (release
// build) and the Vite dev server as fixtures so the suite is fully
// self-contained — `npx playwright test` from `web/` is all that's
// needed locally and in CI.
//
// Single worker because the chartgen OAuth store is process-global and
// multiple tests racing through the PKCE dance would collide on state.

import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  fullyParallel: false,
  workers: 1,
  // Retry once in CI to smooth over first-run OAuth redirect timing.
  retries: process.env.CI ? 1 : 0,
  reporter: [["html", { open: "never" }], ["list"]],
  use: {
    baseURL: "http://localhost:5173",
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
      command: "cargo run --release -- --serve",
      cwd: "..",
      port: 9315,
      reuseExistingServer: !process.env.CI,
      // Release build from cold can chew ~3 minutes on a slow CI
      // runner. Cached Swatinem builds drop this to seconds.
      timeout: 180_000,
      stdout: "pipe",
      stderr: "pipe",
    },
    {
      command: "npm run dev",
      cwd: ".",
      port: 5173,
      reuseExistingServer: !process.env.CI,
      timeout: 60_000,
      stdout: "pipe",
      stderr: "pipe",
    },
  ],
});
