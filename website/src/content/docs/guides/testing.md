---
title: Testing
description: How to run chartgen's browser E2E suite locally and in CI.
---

The `web/tests/e2e/` directory holds a Playwright suite that drives
the SolidJS frontend end-to-end against a real `chartgen --serve`
process. It implements
[ADR-0008](/chartgen/decisions/0008-e2e-playwright/) and covers the
v0.2 happy paths: cold boot through the OAuth 2.1 PKCE flow, chart
render with the default indicator set, and the indicator-picker
modal round-trip (add, persist, remove).

## Run locally

Playwright's `webServer` config launches both the chartgen backend
and the Vite dev server as managed fixtures, so the only prerequisite
is installing the toolchain once:

```bash
cd web
npm install
npx playwright install chromium
```

Then run the suite:

```bash
npm run e2e          # headless, CI-style run
npm run e2e:ui       # Playwright UI mode — pick a spec, step through
npm run e2e:debug    # open the inspector, pause on the first step
```

If you already have `cargo run -- --serve` or `npm run dev` running,
Playwright will reuse the existing server (`reuseExistingServer: true`
outside CI). The first cold run builds chartgen in release mode,
which takes a few minutes without a cache — subsequent runs are
fast.

## How CI runs it

The `e2e` job in `.github/workflows/ci.yml` gates on `build-release`
so the release binary is always cached by the time the suite starts.
The job:

1. Restores the Rust and Playwright browser caches.
2. Builds chartgen in release mode.
3. `npm ci` in `web/`.
4. `npx playwright install --with-deps chromium`.
5. `npx playwright test`.

On failure the job uploads `playwright-report/` and `test-results/`
as artifacts. Open the HTML report locally with
`npx playwright show-report playwright-report`.

## Add a new spec

Specs live in `web/tests/e2e/`. Use the shared fixture in
`fixtures/app.ts` when you want a pre-booted app state:

```ts
import { test, expect, expectChartRendered } from "./fixtures/app";

test.describe("my feature", () => {
  test("happy path", async ({ bootedApp: page }) => {
    // page is already through OAuth, with the initial chart loaded.
    await page.getByRole("button", { name: "My Feature" }).click();
    await expectChartRendered(page);
  });
});
```

Skip the fixture when you want to observe the boot itself — see
`smoke.spec.ts` for how to run the OAuth redirect from scratch.

The suite runs with `workers: 1` because chartgen's OAuth store is
process-global and two tests racing through PKCE will collide on
state. Don't set `fullyParallel: true`.

## Scope caveats

The v0.2 suite is Chromium-only and targets desktop viewports. Visual
regression, multi-browser coverage, mobile viewports, order/alert
flows, and accessibility audits are deferred — see
[ADR-0008](/chartgen/decisions/0008-e2e-playwright/) for the full
list and why each was left out.
