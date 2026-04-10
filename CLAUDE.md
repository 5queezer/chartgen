## Workflow

- Never push directly to `master`. Create a feature branch and open a PR.
- Branch naming: `feat/<topic>`, `fix/<topic>`, `chore/<topic>`
- `cargo fmt` is enforced by a pre-commit hook and a Claude settings.json hook.
- Run `cargo clippy -- -D warnings` before pushing.

## graphify

This project has a graphify knowledge graph at graphify-out/.

Rules:
- Before answering architecture or codebase questions, read graphify-out/GRAPH_REPORT.md for god nodes and community structure
- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- After modifying code files in this session, run `python3 -c "from graphify.watch import _rebuild_code; from pathlib import Path; _rebuild_code(Path('.'))"` to keep the graph current
