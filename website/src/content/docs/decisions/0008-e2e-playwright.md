---
title: "0008 — End-to-end tests with Playwright + CI integration"
description: "Playwright-driven browser E2E suite that launches chartgen + Vite as fixtures, runs in CI, and covers the v0.2 OAuth/chart/indicator-picker happy paths."
sidebar:
  order: 8
---

**Status:** Accepted
**Date:** 2026-04-19

## Context and Problem Statement

The web frontend shipped through v0.2 on manual verification alone.
Every reviewer had to boot `cargo run -- --serve`, then a Vite dev
server, then click through the OAuth flow, then load a symbol, then
click through the indicator picker — and any regression in OAuth
redirect handling, chart rendering, or the modal interaction was
caught only if a reviewer happened to exercise that exact path.

v0.1's "scaffold" did run HTTP-surface checks against chartgen, but
those don't cover the browser — the real contract the frontend has
to honor is the one an actual browser sees (PKCE redirects, SPA
routing after `?code=&state=` scrubbing, klinecharts canvas render,
Kobalte Dialog portal behavior, localStorage persistence). None of
those are testable from curl.

We need an automated end-to-end suite that:

- Runs in CI on every PR, against a release build of chartgen.
- Covers the OAuth 2.1 PKCE flow all the way through.
- Asserts the klinecharts canvas is actually rendered, not just that
  the page painted.
- Covers the indicator-picker modal add/remove/persist loop described
  in [ADR-0005](/chartgen/decisions/0005-client-indicator-state/).
- Keeps the PR surface small enough that future engineers will
  actually maintain it.

## Decision Drivers

- **Reproducibility.** `npx playwright test` from `web/` must be the
  only local command; the suite launches its own backend and frontend.
- **CI-integrable.** Existing GitHub Actions runners, no external
  services, no tunneling localhost to a cloud browser farm.
- **TypeScript-native.** The frontend is TS already; the suite lives
  next to the code it tests so the same people who write the app can
  maintain it.
- **Low-cost matrix for v0.2.** Chromium-only is enough for "does the
  OAuth flow still work"; cross-browser coverage is a later tradeoff.
- **Full control.** No cloud service fees, no account lock-in, no
  data egress surprises.

## Considered Options

**Selenium / WebDriverIO.** Mature, but Playwright's auto-waiting and
first-class TypeScript definitions are a meaningfully better fit for
a single-engineer team. Selenium in particular carries a lot of
setup-friction tax (driver binaries, Grid, Java bridges) for a
use-case where one process per spec is plenty. WebDriverIO is a
reasonable alternative but its ecosystem is smaller than
Playwright's in 2026 and its webServer-style fixture story is not
built-in.

**Cypress.** The iframe-based architecture makes multi-tab and
multi-origin flows awkward — and the OAuth redirect here technically
round-trips via `/authorize` on a separate port (9315). Cypress's
same-origin assumption has historically fought that pattern. The
tool also spent years Chrome-only; cross-browser is possible now but
not a reason to prefer it over Playwright.

**Cloud browser service (Browserbase, BrowserStack, LambdaTest).**
None of these can reach `localhost:9315` without a tunnel. Tunneling
in CI is overhead without obvious benefit; we'd still want to
self-host the chartgen process for reproducibility, at which point
the cloud browser is just latency.

**Manual-only, status quo.** The thing we're replacing. Rejected on
principle — unreviewable regressions are piling up and every PR
reviewer pays the cost.

## Decision Outcome

**Chosen: Playwright, Chromium only, webServer-managed fixtures.**

- Suite lives at `web/tests/e2e/`. Spec files: `smoke.spec.ts`
  (cold-boot happy path) and `indicator-picker.spec.ts` (ADR-0005
  modal flow). A shared fixture in `fixtures/app.ts` handles the
  cold-boot + OAuth + initial Load so specs can opt in.
- `web/playwright.config.ts` declares two `webServer` entries:
  `cargo run --release -- --serve` in the repo root and `npm run dev`
  in `web/`. `reuseExistingServer: !process.env.CI` keeps local loops
  fast. Chartgen gets a 180-second timeout because a cold release
  build is measured in minutes on CI without a cache; on a warm
  Swatinem cache it's seconds.
- `fullyParallel: false` and `workers: 1`: chartgen's OAuth store is
  process-global, and two tests racing through PKCE will collide on
  state.
- Single Chromium project — Firefox/WebKit are deferred until there's
  a concrete regression they'd catch. Desktop viewport only; mobile
  isn't supported by the app in v0.2 anyway.
- CI job `e2e` in `.github/workflows/ci.yml` gates on `build-release`,
  caches `~/.cache/ms-playwright`, runs `npx playwright test`, and
  uploads `playwright-report/` + `test-results/` as artifacts on
  failure.

Not in scope for this ADR (explicitly deferred):

- Visual regression snapshots (pixel diffs on chart rendering).
- SSE stream content assertions beyond "connection opens".
- Multi-browser matrix.
- Mobile viewports.
- Axe-core accessibility audit.
- Order / alert flows (not in the v0.2 frontend).

## Consequences

**Positive.** CI now catches frontend regressions before merge: OAuth
flow breakage, chart render failure, picker modal layout bugs, and
localStorage persistence drift. The specs double as executable
documentation of the OAuth PKCE flow — a new contributor can read
`smoke.spec.ts` and see what the happy path looks like. The chartgen
release binary runs unchanged under test, so we're testing the same
artifact that ships.

**Negative.** CI wall-clock grows by roughly 2-3 minutes (Chromium
install + release build + two specs). The test retries once on first
run to smooth OAuth redirect timing, which hides first-attempt
flakiness; we accept that in exchange for lower noise. Every future
indicator added to the default seed has to be consistent with the
smoke test's expectations about `.pane-header` count and RSI
presence.

**Neutral.** `web/package.json` gains a `@playwright/test` devDep and
three scripts (`e2e`, `e2e:ui`, `e2e:debug`). The suite is skipped
entirely in environments without a browser runtime — CI provides it
via `playwright install --with-deps chromium`; local developers who
skip that step get a clear error from Playwright.

## More Information

- [ADR-0005 — Client-side indicator state](/chartgen/decisions/0005-client-indicator-state/)
- [Testing guide](/chartgen/guides/testing/)
