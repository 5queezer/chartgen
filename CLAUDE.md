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

## graphify

This project has a graphify knowledge graph at graphify-out/.

Rules:
- Before answering architecture or codebase questions, read graphify-out/GRAPH_REPORT.md for god nodes and community structure
- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- After modifying code files in this session, run `python3 -c "from graphify.watch import _rebuild_code; from pathlib import Path; _rebuild_code(Path('.'))"` to keep the graph current
