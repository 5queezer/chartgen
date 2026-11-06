---
title: Contributing
description: How to develop, keep docs in sync, and pass CI.
---

## Local workflow

```bash
# Rust side
cargo fmt            # pre-commit hook enforces --check
cargo clippy -- -D warnings
cargo test

# Docs side
cd website
npm ci
npm run dev          # live preview at http://localhost:4321/chartgen
npm run build        # what GitHub Pages runs
```

The pre-commit hook enforces `cargo fmt`. Clippy is gate-keeped in CI with
`-D warnings`. Never push with `--no-verify`.

## Source layout

```
src/
├── main.rs              CLI and mode dispatch
├── lib.rs               library root
├── data.rs              Bar, OhlcvData
├── interval.rs          interval parsing (1m, 7m, 2W, ...)
├── indicator.rs         Indicator trait, PanelResult, Line/Fill/HBar
├── indicator_state.rs   streaming indicator state for the engine
├── indicators/          registry (mod.rs), built-ins, Pine v5 helpers (ADR-0018)
├── scripting/           Starlark runtime for scripted indicators
├── fetch.rs             Yahoo Finance and Binance REST
├── feed.rs              Binance WebSocket kline feed
├── binance_gate.rs      shared limits for Binance calls
├── search.rs            symbol search
├── mtf.rs               multi-timeframe aggregation
├── engine.rs            trading engine: live bars, indicators, alerts
├── trading/             alerts, orders, positions, subscriptions, persistence
├── server_alerts.rs     alerts that run on the server
├── webhook.rs           webhook delivery for server alerts
├── chart_push.rs        live chart push (subscribe_chart)
├── renderer.rs          plotters PNG rendering
├── mcp.rs               MCP JSON-RPC, shared by stdio and HTTP
├── mcp_schema.rs        tools/list schemas
└── server.rs            axum HTTP, OAuth 2.1 PKCE, SSE
web/                     SolidJS web app
website/                 this site
```

## Keeping docs in sync

`website/src/content/docs/reference/indicators.md` is **generated**. Do not
hand-edit it. When you add, remove, or reparameterize an indicator:

```bash
cargo run --example gen_docs     # rewrites indicators.md
./scripts/gen_screenshots.sh     # refreshes website/src/assets/indicators/*.png
```

CI has a `docs-drift` job that reruns `cargo run --example gen_docs` and
fails the build if the committed `indicators.md` differs from the generated
one. If a PR lands that only changes `Indicator::description()` /
`Indicator::params()`, you will see this job fail — commit the regenerated
file.

Other docs are hand-written, but must be kept in sync when you ship a
user-facing change. See the project `CLAUDE.md` for the checklist Claude
follows; human contributors should follow the same rules.

## Testing at another time of day

Sample bars are a function of the clock, so an e2e spec can pass for days and
then fail for an hour or two when the data takes a different shape, and a retry
won't help because it sees the same bars. `scripts/e2e-at-clock.sh` runs specs
against a sample server with its clock moved, to reach those hours on demand:

```bash
scripts/e2e-at-clock.sh 5h tests/e2e/scale-invert-lock.spec.ts
scripts/e2e-at-clock.sh 25d tests/e2e/smoke.spec.ts -g "cold boot"
```

The first argument is the offset (seconds, or `5h`, `25d`), and the rest go to
`playwright test`, from `web/`. It needs Linux and `gcc`: it moves the clock by
preloading a small shim into the server. Only the server's clock moves, not the
browser's. It uses `CHARTGEN_PORT` and `WEB_PORT` like the suite does, and stops if
either port is in use. Try a spread of offsets when a spec depends on the data;
this found failures that runs on the real clock did not.

## CI jobs

`.github/workflows/ci.yml`:

| Job | What it runs |
|-----|--------------|
| `check` | `cargo check --all-targets`. |
| `test` | `cargo test` (e2e + unit). |
| `clippy` | `cargo clippy -- -D warnings`. |
| `fmt` | `cargo fmt --check`. |
| `docs-drift` | Reruns `cargo run --example gen_docs` and fails if `reference/indicators.md` moved. |
| `build-release` | After the gates pass, builds the Linux x86_64 release artifact. |

Separate workflows handle GitHub Pages (`docs.yml`, Node 22 after PR #60),
Coolify auto-deploy (pull-based, a systemd timer on the host; see
[deployment](/chartgen/guides/deploy/)), and
tagged releases (`release.yml`, publishes four platform artifacts on `v*`
tags — see [deployment](/chartgen/guides/deploy/)).

## Review

After opening a PR, wait for CodeRabbit AI to post a review before merging.
If CodeRabbit stalls on "Review in progress", comment `@coderabbitai review`
to retrigger it. Never merge while its review is pending.
