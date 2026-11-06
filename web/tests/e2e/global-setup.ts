// Boots the app once before the first test. Vite transforms each of the
// app's ~190 modules on first request and the debug chartgen build is slow
// on its first chart, so a cold boot costs several times a warm one. Here
// that cost is paid once, outside every test's timeout.

import { chromium, type FullConfig } from "@playwright/test";

import { gotoApp } from "./fixtures/app";

/** Upper bound for the warm-up; a cold machine under load can be slow. */
const WARMUP_TIMEOUT_MS = 180_000;

export default async function globalSetup(config: FullConfig): Promise<void> {
  const browser = await chromium.launch();
  // Every project has its own dev server to warm (playwright.config.ts).
  await Promise.all(
    // Not the sign-in project: gotoApp waits for a token, and there a person has to
    // choose a provider first. signin.spec.ts warms its own server.
    config.projects.filter((p) => p.name !== "chromium-signin").map(async ({ use: { baseURL } }) => {
      try {
        const page = await browser.newPage({ baseURL });
        page.setDefaultTimeout(WARMUP_TIMEOUT_MS);
        page.setDefaultNavigationTimeout(WARMUP_TIMEOUT_MS);
        await gotoApp(page);
      } catch (err) {
        // Only a warm-up: if it fails, the specs report the real problem.
        console.warn(`[global-setup] warm-up boot failed: ${String(err)}`);
      }
    }),
  );
  await browser.close();
}
