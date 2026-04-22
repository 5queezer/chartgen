---
title: Python-authored indicators
description: Proposal to let chartgen users register sandboxed, Starlark-based indicators alongside the Rust builtins.
---

**Status:** proposed
**Authors:** christian.pojoni@gmail.com (via brainstorming session, 2026-04-21)
**Relates to:** future ADRs — (a) Starlark as script runtime, (b) per-user indicator registry + MCP surface.

## Understanding summary

- **What:** A way for chartgen users to register their own Python-shaped indicators via MCP, run them sandboxed alongside the existing 38 Rust builtins, and fork builtin composites (starting with Cipher B) into editable form.
- **Why:** The Rust registry is closed — adding or tweaking an indicator needs a repo edit + rebuild. Complex composites like Cipher B encode opinionated logic every user wants to tweak differently. A Pinescript-flavored authoring surface in a familiar syntax opens that up without compromising the Rust core.
- **Who:** Authenticated users of the hosted chartgen MCP server. Per-user namespace scoped by the existing OAuth identity.
- **Non-goals:**
  - Not replacing the Rust indicators; they stay as fast defaults.
  - No numpy/pandas/scipy inside scripts.
  - No network or file I/O from scripts.
  - No per-bar streaming/live evaluation.
  - No multi-language support — one scripting language only.
  - No visual/block editor in scope.
  - No cross-user script sharing / marketplace in v1.

## Assumptions

1. Per-script hard timeout **1000 ms**, memory cap **~64 MB**, target for typical scripts **<50 ms** on 5000 bars.
2. Script metadata (params schema, description, category) declared in the script itself via a module-level `PARAMS = [...]` list and `META = {...}` dict.
3. Reference implementations only for **composite builtins** users actually want to fork (Cipher B first; WaveTrend, Ichimoku, RSI combo likely next). Simple indicators (SMA, RSI) stay Rust-only and are exposed via `ta.*` instead.
4. Script errors (syntax at register, runtime at compute) surface as clear MCP errors; the rest of the chart still renders.
5. Each forkable builtin has one canonical refimpl `.star` file maintained in the chartgen repo, parity-tested against Rust in CI (fail-closed on drift).
6. Custom indicators appear in `list_indicators` for the authenticated user alongside builtins, and in the web picker.

## Decision log

| # | Decision | Alternatives | Why |
|---|---|---|---|
| 1 | User-authored extensions (fork model for composites) | Replace all Rust; coexist same-footing; DSL-only | Preserves Rust perf; unlocks composite tweaking; zero disruption to existing callers |
| 2 | MCP-hosted, sandboxed, multi-user | Dev-time only; local single-user; hybrid | Matches the hosted-chartgen product reality |
| 3 | Pinescript-shape authoring (OHLCV + curated `ta.*`, no I/O) | numpy subset; full Python + I/O; implicit per-bar | Compatible with sandboxing; smallest attack surface; fits Cipher B |
| 4 | `register_indicator` with per-user persistence | Inline-per-call; Gist URL; hybrid | Ties cleanly to OAuth; reuse without heavy payloads |
| 5 | Fork model (user script shadows builtin for that user) | User always wins; separate namespace; prefixed | Delivers "Cipher B editable" without a forced rename |
| 6 | `chart` object passed into `compute(ohlcv, params, chart)` | Pinescript globals; declarative dict return; hybrid | Python-idiomatic, testable, explicit dep |
| 7 | Starlark runtime via `starlark-rust` | RustPython; subprocess CPython + nsjail; WASM/Pyodide | Sandbox is a language property not a policy; fastest; cleanest embed; smallest maintenance tax |
| 8 | Module-level `PARAMS = [...]` for schema | `def params()`; decorator | Least magic; easy static inspection |
| 9 | Python loops allowed as escape hatch for missing `ta.*` | Block; punt | Sandbox handles it; 1 s timeout caps cost |
| 10 | SQLite `user_indicators` keyed on `(user_id, name)` | In-memory; JSON-per-user | Survives restarts; trivial overhead; scales |
| 11 | Starlark step-counter for timeout | Worker thread + timer; OS signal | Deterministic; no race with mutable state |
| 12 | Extend `indicators::by_name(name, ctx)` | Parallel dispatch fn; global registry; name parsing | Preserves the single dispatch seam the whole stack uses |

## Design

### 1. Architecture & module layout

```
src/scripting/          ← new module
  mod.rs                 public API: compile(), execute(), errors
  runtime.rs             Starlark Evaluator + globals wiring
  chart_builder.rs       the `chart` object exposed to scripts
  ta.rs                  `ta.*` primitives (Rust fns bound into Starlark)
  persistence.rs         SQLite DAO for user scripts
  refimpl/               reference impls for forkable builtins
    cipher_b.star        (starts here; more added on demand)
```

**Dispatch integration.** `indicators::by_name()` (src/indicators/mod.rs) grows one branch: on miss against the builtin match, look up `(user_id, name)` in the script registry. If found, construct a `ScriptIndicator` — a thin adapter that impls the existing `Indicator` trait and delegates `compute()` to the Starlark runtime. No other code in `src/mcp.rs`, `renderer.rs`, or `examples/gen_docs.rs` needs to change — they keep talking to `dyn Indicator`.

**Compilation cache.** Parse + static-check of a Starlark script takes ~1 ms. Cache the frozen `FrozenModule` keyed by `(user_id, name, source_hash)`, in-process, LRU(1024). Source edit → new hash → recompile on next call.

**Evaluator per call.** Starlark's `Evaluator` is cheap (<100 µs) and not `Send`; build a fresh one per `compute()`. The frozen module is shared; mutable state (call stack, `chart` accumulator) is per-invocation.

**`user_id` plumbing.** The OAuth identity from MCP auth must reach `by_name()`. Current signature takes only `&str`; it grows to `by_name(name: &str, ctx: &ScriptContext)` where `ScriptContext` carries `user_id` (or is `None` for CLI, which only sees builtins).

### 2. Script-facing API

One file per indicator. Example (forked Cipher B):

```python
PARAMS = [
    {"name": "n1", "type": "int", "default": 10, "desc": "WaveTrend channel length"},
    {"name": "n2", "type": "int", "default": 21, "desc": "WaveTrend average length"},
    {"name": "ob", "type": "float", "default": 60.0, "desc": "Overbought"},
]
META = {"title": "Cipher B (fork)", "category": "panel", "overlay": False}

def compute(ohlcv, params, chart):
    hlc3 = ta.hlc3(ohlcv)
    esa = ta.ema(hlc3, params["n1"])
    de  = ta.ema(ta.abs(ta.sub(hlc3, esa)), params["n1"])
    ci  = ta.div(ta.sub(hlc3, esa), ta.mul_scalar(de, 0.015))
    wt1 = ta.ema(ci, params["n2"])
    wt2 = ta.sma(wt1, 4)

    chart.line(wt1, title="WT1", color="green")
    chart.line(wt2, title="WT2", color="red")
    chart.fill(wt1, wt2, color="auto", alpha=0.2)
    chart.hline(params["ob"], color="gray", style="dashed")
    chart.hline(-params["ob"], color="gray", style="dashed")
```

**`chart` methods** (1:1 with `PanelResult` fields): `line`, `fill`, `bars`, `dot`, `hline`, `hbar`, `divline`, `set_y_range`, `set_label`.

**`ohlcv` object**: attributes `open`, `high`, `low`, `close`, `volume`, `time` (frozen series) + helpers `hlc3`, `hl2`, `ohlc4`.

**`ta.*` starter set (~20 fns, all Rust-backed vectorized):**

- MAs: `sma`, `ema`, `wma`, `rma`
- Oscillators: `rsi`, `stoch`, `atr`, `macd`
- Rolling: `highest`, `lowest`, `stdev`, `change`, `roc`
- Series math: `add`, `sub`, `mul`, `div`, `abs`, `max`, `min` (with `_scalar` variants)
- Events: `crossover(a, b)`, `crossunder(a, b)` → bool series
- Pivots: `pivot_high`, `pivot_low`

**Ergonomic cost.** Starlark has no operator overloading on custom types. Series arithmetic must be function calls (`ta.sub(a, b)`, not `a - b`). Documented up-front.

### 3. Script lifecycle, MCP surface, persistence

**New MCP tools:**

| Tool | Shape | Notes |
|---|---|---|
| `register_indicator` | `(name, source, params_schema?) → {ok, errors?}` | Parses, static-checks, dry-runs against synthetic OHLCV. Upserts. |
| `fork_indicator` | `(builtin_name) → {source, params_schema}` | Returns the repo's `refimpl/{name}.star`. User edits and calls `register_indicator`. |
| `list_my_indicators` | `() → [{name, title, category, ...}]` | Per-user. Merged into `list_indicators` with `source: "user"` tag. |
| `delete_indicator` | `(name) → {ok}` | Removes from registry. |
| `update_indicator` | `(name, source) → {ok, errors?}` | Shortcut = `register_indicator` with existing name. |

`generate_chart` unchanged. Resolution order: user's scripts → Rust builtins. Implements the "fork shadows builtin for that user" semantics.

**Register-time validation pipeline:**

1. Starlark parse (syntax).
2. Static check: only allowed globals (`ta`, `ohlcv`, `params`, `chart`, builtins); no `load(...)`.
3. Presence check: `PARAMS` list, `META` dict, `compute(ohlcv, params, chart)` function.
4. Dry-run on a 100-bar synthetic OHLCV with defaults, 200 ms timeout. Catches most runtime errors cheaply.

Errors from any step return structured `{step, line, message}` to the caller. `generate_chart` on a broken script fails the indicator, not the whole chart, and surfaces the error in the response.

**SQLite schema (new migration):**

```sql
CREATE TABLE user_indicators (
  user_id      TEXT    NOT NULL,
  name         TEXT    NOT NULL,
  source       TEXT    NOT NULL,
  meta_json    TEXT    NOT NULL,   -- PARAMS + META snapshot at register
  source_hash  TEXT    NOT NULL,   -- cache key for FrozenModule
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  PRIMARY KEY (user_id, name)
);
```

**Timeout mechanism.** Starlark step-counter (instruction budget via `Evaluator::set_max_callstack_size` + a custom step hook). Deterministic; no thread-kill. Target: 1 M steps ≈ 50 ms typical.

### 4. Testing, rollout, risks

**Testing strategy:**

1. **Refimpl parity.** Each forkable builtin has an OHLCV fixture battery (trending up/down, low/high vol, sparse). The `refimpl/{name}.star` output must match Rust `{name}` output within 1e-9 on every series. CI-enforced (same pattern as `docs-drift`). If Rust changes, the `.star` must be updated in the same PR.
2. **Sandbox escape suite.** Fixed scripts attempting `load("//:stdlib")`, recursion bombs, infinite loops, huge allocations, shadowed builtins. All must produce clean errors, never a process crash.
3. **Budget guard.** Bench: pathological 5000-bar Cipher B ×100, assert p99 < 50 ms. Nightly CI.
4. **Golden integration.** End-to-end per flow: register → list → use → delete.

**Rollout:**

- **Phase 1 (MVP):** `scripting/` module, Starlark runtime, `register_indicator` + `fork_indicator` + `list_my_indicators`, Cipher B refimpl, SQLite persistence, `ta.*` starter set. Feature-flagged off by default; opt-in per-user via env var.
- **Phase 2:** `update_indicator` + `delete_indicator`, web frontend UI for script editing, more refimpls (WaveTrend, Ichimoku) on demand.
- **Phase 3 (later, not committed):** shareable scripts, registry view, version pinning.

**Risks:**

1. **`starlark-rust` upstream health.** Meta maintains; Buck2 is the production consumer. Active, but a non-trivial dep. Mitigation: pin major version, track releases.
2. **Refimpl drift.** Rust builtin evolves, `.star` lags. Mitigation: parity test in CI, fail-closed; refimpl update required in the same PR as the Rust change.
3. **User ergonomics without operator overloading.** `ta.sub(a, b)` vs `a - b`. Mitigation: document clearly; revisit with a preprocessor if users complain.
4. **Breaking signature change.** `indicators::by_name` affects CLI, `gen_docs`, integration tests. Mechanical fix, but scope it carefully.
5. **ADR obligations.** Two ADRs required at implementation time: (a) Starlark as script runtime, (b) per-user indicator registry + MCP surface.

## Exit criteria (from brainstorming)

- [x] Understanding Lock confirmed
- [x] Design approach (R1, Starlark) explicitly accepted
- [x] Major assumptions documented
- [x] Key risks acknowledged
- [x] Decision Log complete

Ready for implementation planning. The Phase 1 MVP is the first concrete deliverable; ADRs follow with the implementing PR.
