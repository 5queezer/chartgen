// Playwright config for chartgen's browser E2E suite.
//
// Implements ADR-0008. Two webServer entries spin up chartgen and the
// Vite dev server as fixtures so the suite is fully self-contained —
// `npm run test:e2e` from `web/` is all that's needed locally and in CI.
//
// Ports are configurable so parallel checkouts/sessions don't collide:
//   CHARTGEN_PORT (default 9315) — chartgen --serve
//   WEB_PORT      (default 5173) — Vite dev server
// The `scripting` project (ADR-0041) has a second pair 1000 ports up, with
// CHARTGEN_SCRIPTING=1 and a store of its own, because whether the server lists
// the scripting tools is fixed at startup. Every other spec keeps scripting off.
//
// Servers already listening on those ports are reused only with
// PW_REUSE_SERVER=1. Otherwise Playwright fails fast on a busy port, so a
// leftover server from another checkout is never tested by mistake.
//
// Single worker because the chartgen OAuth store is process-global and
// multiple tests racing through the PKCE dance would collide on state.

import { tmpdir } from "node:os";
import { join } from "node:path";

import { defineConfig, devices } from "@playwright/test";

const CHARTGEN_PORT = Number(process.env.CHARTGEN_PORT ?? 9315);
const WEB_PORT = Number(process.env.WEB_PORT ?? 5173);
const REUSE_SERVER = !process.env.CI && process.env.PW_REUSE_SERVER === "1";
// Every test registers an OAuth client and the server persists them: start
// each run with an empty store of its own, never the user's real one. With
// PW_REUSE_SERVER=1 this does not apply, because neither the command nor its env
// is used: a reused server keeps its own store, which on a dev server is the
// real ~/.chartgen/chartgen.db, and collects the test clients there.
const CHARTGEN_DB = join(tmpdir(), `chartgen-e2e-${CHARTGEN_PORT}.db`);
const SCRIPTING_CHARTGEN_PORT = CHARTGEN_PORT + 1000;
const SCRIPTING_WEB_PORT = WEB_PORT + 1000;
const SCRIPTING_DB = join(tmpdir(), `chartgen-e2e-${SCRIPTING_CHARTGEN_PORT}.db`);
// Specs that need the scripting server.
const SCRIPTING_SPECS = /script-editor-on\.spec\.ts$/;

export default defineConfig({
  testDir: "./tests/e2e",
  // Boots the app once so the first test doesn't pay for cold servers.
  globalSetup: "./tests/e2e/global-setup.ts",
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
      testIgnore: SCRIPTING_SPECS,
      use: { ...devices["Desktop Chrome"] },
    },
    {
      name: "scripting",
      testMatch: SCRIPTING_SPECS,
      use: { ...devices["Desktop Chrome"], baseURL: `http://localhost:${SCRIPTING_WEB_PORT}` },
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
      command: `rm -f "${CHARTGEN_DB}" && cargo run -- --serve --port ${CHARTGEN_PORT}`,
      cwd: "..",
      port: CHARTGEN_PORT,
      // Off even if the developer's shell turns scripting on.
      env: { CHARTGEN_DATA_SOURCE: "sample", CHARTGEN_DB, CHARTGEN_SCRIPTING: "0" },
      reuseExistingServer: REUSE_SERVER,
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
      reuseExistingServer: REUSE_SERVER,
      timeout: 60_000,
      stdout: "pipe",
      stderr: "pipe",
    },
    {
      command: `rm -f "${SCRIPTING_DB}" && cargo run -- --serve --port ${SCRIPTING_CHARTGEN_PORT}`,
      cwd: "..",
      port: SCRIPTING_CHARTGEN_PORT,
      env: { CHARTGEN_DATA_SOURCE: "sample", CHARTGEN_DB: SCRIPTING_DB, CHARTGEN_SCRIPTING: "1" },
      reuseExistingServer: REUSE_SERVER,
      timeout: 300_000,
      stdout: "pipe",
      stderr: "pipe",
    },
    {
      // Its own dep cache: two dev servers must not optimize into one directory.
      command: `npm run dev -- --port ${SCRIPTING_WEB_PORT} --strictPort`,
      cwd: ".",
      port: SCRIPTING_WEB_PORT,
      env: {
        CHARTGEN_DEV_TARGET: `http://localhost:${SCRIPTING_CHARTGEN_PORT}`,
        VITE_CACHE_DIR: "node_modules/.vite-scripting",
      },
      reuseExistingServer: REUSE_SERVER,
      timeout: 60_000,
      stdout: "pipe",
      stderr: "pipe",
    },
  ],
});
