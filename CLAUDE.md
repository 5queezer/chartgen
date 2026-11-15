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

Traps that have cost time before. Where a check now catches one, the note names the check; the rest stay prose, with their reason.

### Build and test

- CI runs both feature sets: `clippy --all-targets` with default features, and the `no-default-features` job (build, `clippy --all-targets`, `--test e2e`). A `#[cfg(test)]` helper used only from a `scripting`-gated module needs the same gate. Run those commands before a PR to save a CI round.
- A debug build without `CHARTGEN_DB` opens no store (`persistence::global`, test `test_debug_build_without_chartgen_db_opens_no_default_store`): it would migrate `~/.chartgen/chartgen.db`. Set `CHARTGEN_DB` to a file under `target/` for every server, example or binary you run by hand.
- Production web files must not import from outside `web/`: CI's `docker-web` job builds with only `web/` in its context and fails. Tests may.
- The server's `/favicon.svg` and `/favicon.ico` (`src/server/favicon.*`) must match `web/public/favicon.*`: `Wordmark.test.ts` fails otherwise.
- `scripts/checks/agent-checks.sh` (CI job `agent-checks`, self-test `scripts/checks/test.sh`): a `TODO`, `FIXME` or `HACK` comment needs an issue (`#123`) or a link.

### Flaky and clock-dependent tests

- Sample data (`CHARTGEN_DATA_SOURCE=sample`) is a function of the clock. The forming bar wobbles with wall-clock seconds, and the bars on screen change with the hour, so an e2e spec can pass for days and fail for an hour. Reproduce it with `scripts/e2e-at-clock.sh <offset> <spec>`. Don't assert pixel-exact positions or color shares that the forming bar or the last-price line can move. (Prose: no check can tell a stable assertion from a lucky one.)
- In Playwright, take press points from `chart.getSize(...)` or a bounding box: `agent-checks.sh press-points` refuses literal pixels in `mouse.click`/`dblclick` and in a `mouse.move` before `mouse.down`. Mark a deliberate exception with `e2e-fixed-point: <why>`.
- Playwright file filters are regexes over the full path. `npx playwright test mobile` matches every spec when the worktree path contains "mobile", so pass exact paths (`tests/e2e/mobile.spec.ts`). (Prose: it is Playwright's own CLI.)
- Sample prices are three sinusoids plus noise, so hit rates or backtests on them are inflated and say nothing about real markets. (Prose: a judgment, not a rule.)

### Production and safety

- Nothing builds on the production host. The arm64 image comes from the `image` job in `ci.yml` (GHCR), only for green master commits. Deployment runbooks and the agent client's operating notes are kept outside this repository. (Prose: it is about operating the host.)
- MCP clients keep their old tool list after a deploy that adds or changes tools. chartgen sends no `notifications/tools/list_changed`, so a client must reconnect. (Prose: the client decides.)
- A replay server (`CHARTGEN_REPLAY_FIXTURES`) must not be reachable from other networks: sign-in is off, and any client can move its one clock. Set `CHARTGEN_BIND` to loopback or a VPN overlay address in the CGNAT range 100.64.0.0/10. On any other address it starts with a `[replay] WARNING` but does not refuse, because ADR-0068 allows a container that publishes the port on a private address only.
- Webhooks go only to public `https` addresses on port 443, enforced in `src/webhook.rs` and its tests, because the URL comes from strangers and the server sits on a private network. Don't loosen this.
- `CHARTGEN_SERVER_ALERT_CLIENTS=*` opens alerts, webhooks and setups to every client. A server with a public `CHARTGEN_BASE_URL` refuses to start with it (`open_clients_refused` in `src/server.rs`); production lists the trusted account ids.
