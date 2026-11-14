## Workflow

- Never push directly to `master`. Create a feature branch and open a PR.
- Branch naming: `feat/<topic>`, `fix/<topic>`, `chore/<topic>`
- `cargo fmt` is enforced by a pre-commit hook and a Claude settings.json hook.
- Run `cargo clippy -- -D warnings` before pushing.
- After pushing a PR or force-pushing a rebase, wait for CodeRabbit AI to complete its review before merging. If CodeRabbit is rate-limited or stuck on "Review in progress", comment `@coderabbitai review` on the PR to request a new review. Never merge while CodeRabbit review is pending.

## Documentation

The Starlight site at `website/src/content/docs/` is user-facing — update it in the same PR as any user-facing change (CLI, MCP tool, indicator, HTTP/OAuth, persistence format, env var, deployment surface).

- `reference/indicators.md` is generated. Run `cargo run --example gen_docs`;
  the `docs-drift` CI job fails if it drifts. Refresh screenshots with
  `scripts/gen_screenshots.sh`.
- New page → add it to the sidebar in `website/astro.config.mjs`.
- Before pushing: `cargo run --example gen_docs` and `npm run build` in `website/`.

## Architecture Decision Records

Decisions with multiple viable options and long-term consequences (transport protocol, framework, auth model, data-contract shape, persistence layer, major dependency) must be recorded as ADRs under `website/src/content/docs/decisions/` using MADR 3.0 format. Write the ADR in the SAME PR as the code that implements the decision — never after the fact.

- Naming: `NNNN-kebab-title.md` with sequential numbering
- Status lifecycle: `proposed → accepted → deprecated | superseded by NNNN`
- ADRs are immutable once merged. To revisit a decision, write a new ADR that supersedes the old one; link both ways.
- Reference the ADR from the PR description (e.g. "Implements ADR-0002") and, where relevant, from code comments.

## MCP Type Safety

- Tool input types in `web/src/types/generated-input.ts` are generated from chartgen's `tools/list` schemas. Regenerate with `./scripts/gen-mcp-types.sh` after any change to `handle_tools_list` in `src/mcp.rs`. CI's `mcp-types-drift` job fails if the file is stale.
- Tool response types in `web/src/types/responses.ts` are hand-written Zod schemas — no formal `outputSchema` exists upstream in MCP yet. They must be updated in the same PR as any change to a response-shaping Rust function (`panel_result_to_*_json`, `tool_get_indicators`, etc.). Treat them as part of the wire contract.
- Validate MCP responses at the boundary with Zod `.parse()` — do not cast untrusted input.

## graphify

This project has a graphify knowledge graph at graphify-out/. It is a
local development artifact — not committed to git.

Rules:
- Before answering architecture or codebase questions, read graphify-out/GRAPH_REPORT.md for god nodes and community structure
- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- After modifying code files in this session, run `python3 -c "from graphify.watch import _rebuild_code; from pathlib import Path; _rebuild_code(Path('.'))"` to keep the local graph current. Do NOT commit the resulting changes to `graphify-out/` — it is gitignored.

## Agent notes

Traps that have cost time before, each with its reason.

### Build and test

- Before a PR, run what `ci.yml` runs, not only `cargo test`: also `cargo build --no-default-features`, `cargo clippy --all-targets --no-default-features -- -D warnings` and `cargo test --no-default-features --test e2e`. Feature gates must agree: a `#[cfg(test)]` helper used only from a `scripting`-gated module needs the same feature gate, or the no-default-features job fails.
- Set `CHARTGEN_DB` to a throwaway file for every server, example or binary you run by hand. Without it the process opens and migrates `~/.chartgen/chartgen.db`. Unit tests can't open it (only `main` opens the store on demand), and `tests/e2e.rs` sets its own.
- Production web files must not import from outside `web/`. The Docker image builds the web app with only `web/` in its context. Tests may, since `tsconfig.build.json` leaves them out. CI's `docker-web` job catches this.
- The server answers `/favicon.svg` and `/favicon.ico` itself from `src/server/favicon.*`, ahead of the web app. Change those together with `web/public/favicon.*`; `Wordmark.test.ts` checks that they match.

### Flaky and clock-dependent tests

- Sample data (`CHARTGEN_DATA_SOURCE=sample`) is a function of the clock. The forming bar wobbles with wall-clock seconds, and the bars on screen change with the hour, so an e2e spec can pass for days and fail for an hour. Reproduce it with `scripts/e2e-at-clock.sh <offset> <spec>`. Don't assert pixel-exact positions or color shares that the forming bar or the last-price line can move.
- In Playwright, take press points from `chart.getSize(...)`, never hard-coded pixels. A layout change moves a legend row or pane separator under a fixed point, and a press there does not pan.
- Playwright file filters are regexes over the full path. `npx playwright test mobile` matches every spec when the worktree path contains "mobile", so pass exact paths (`tests/e2e/mobile.spec.ts`).
- `tests/e2e.rs` gives a spawned server 5 s to listen (`start_server`). On a loaded machine a test can fail on that wait and pass on rerun. This has been seen once, in `test_streams_leave_room_for_everything_else`.
- Sample prices are three sinusoids plus noise, so hit rates or backtests on them are inflated and say nothing about real markets.

### Production and safety

- Nothing builds on the production host. The arm64 image comes from the `image` job in `ci.yml` (GHCR), only for green master commits, and `scripts/autodeploy/` points Coolify at it. Its log line "triggered" means Coolify accepted the deploy, not that it ran, so check the deployment in Coolify.
- MCP clients keep their old tool list after a deploy that adds or changes tools. chartgen sends no `notifications/tools/list_changed`, so a client must reconnect. For Hermes, turn `mcp_servers.chartgen.enabled` off and on again.
- A replay server (`CHARTGEN_REPLAY_FIXTURES`) must never be reachable from other networks. Sign-in is off, and any client can register and move its one forward-only clock. Set `CHARTGEN_BIND` to loopback or the Tailscale address; unset, it listens on every interface.
- Webhooks go only to public `https` addresses on port 443 (`src/webhook.rs`), because the URL comes from strangers and the server sits on a home network. A receiver on the same LAN or host needs a public hostname. Don't loosen this.
- `CHARTGEN_SERVER_ALERT_CLIENTS=*` opens alerts and webhooks to every client. In production it lists the trusted account ids.
