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
Coolify auto-deploy (`deploy.yml`, runs on `workflow_run` success), and
tagged releases (`release.yml`, publishes four platform artifacts on `v*`
tags — see [deployment](/chartgen/guides/deploy/)).

## Review

After opening a PR, wait for CodeRabbit AI to post a review before merging.
If CodeRabbit stalls on "Review in progress", comment `@coderabbitai review`
to retrigger it. Never merge while its review is pending.
