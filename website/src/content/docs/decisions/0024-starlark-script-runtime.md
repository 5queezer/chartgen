---
title: "0024 — Starlark as the script runtime for user indicators"
description: "User indicators run in starlark-rust 0.14, embedded in-process, with a Series value type whose operator overloads give Pine-like math, and with layered tick, time, heap, size and allocation limits."
sidebar:
  order: 24
---

**Status:** Accepted
**Date:** 2026-11-05
**Implements:** [Proposal 0001](/chartgen/proposals/0001-python-indicators/) (Phase 1, runtime half; the registry half is [ADR-0025](./0025-user-indicator-registry/))

## Context and Problem Statement

Users want to write their own indicators, the way TradingView users write
Pine Script, and to fork the built-in composites (WaveTrend, Cipher B)
instead of asking for a Rust change. The scripts come from users of the
hosted MCP server, so the runtime must be safe to run in the server
process, fast enough for 5000-bar charts, and close enough to Pine that
scripts read like Pine.

Which language and runtime should run those scripts, and what does the
script API look like?

## Decision Drivers

- Sandboxing: no file, network or process access, and no way to load
  other code. CPU time and memory must be bounded.
- Familiar syntax, ideally Python-like, with Pine-style series math
  (`close - open`, `close[1]`, `a > b` per bar).
- Embeddable in the Rust binary, without a subprocess or a WASM toolchain.
- Pine v5 semantics shared with the built-ins ([ADR-0018](./0018-pine-semantics-for-indicators/)).
- Reasonable dependency weight and an active upstream.

## Considered Options

1. **Starlark via `starlark-rust` (chosen).** A deterministic Python
   dialect designed for untrusted configuration code. There is no I/O in
   the language itself, `load()` can be turned off, and the evaluator has
   tick, heap and call-stack limits. Meta maintains it as the Buck2
   evaluator.
2. **RustPython.** Full Python, so the sandbox is a policy (removing
   modules) and not a property of the language. Larger and slower.
3. **CPython in a subprocess with nsjail or seccomp.** Strong isolation,
   but a process per evaluation, an OS-specific jail and a Python install
   on every host.
4. **WASM (Pyodide or a custom DSL compiled to WASM).** Strong isolation
   with fuel metering, but a heavy toolchain, and Pyodide is large.
5. **A custom Pine-like DSL.** Exact Pine semantics, but a parser,
   evaluator and tooling to build and maintain ourselves.

## Decision Outcome

Chosen option 1, `starlark = "0.14"` (with `starlark_syntax` for the AST,
and `allocative` and `anyhow` for its derive macros and errors).

### Script shape

A script defines a `PARAMS` list, a `META` dict and
`def compute(ohlcv, params, chart)`. `compute` draws through `chart`, whose
methods map one to one onto `PanelResult` (`line`, `fill`, `bars`, `dot`,
`hline`, `hbar`, `divline`, `set_y_range`, `set_label`). It returns nothing.
`ohlcv` exposes `open`, `high`, `low`, `close`, `volume`, `time`, `hl2`,
`hlc3`, `ohlc4` and `hlcc4` as series. The `ta` namespace binds the Pine
helpers in `src/indicators/pine.rs` (`sma`, `ema`, `wma`, `rma`, `rsi`,
`stoch`, `tr`, `atr`, `macd`, `highest`, `lowest`, `stdev`, `change`,
`roc`, `crossover`, `crossunder`, `pivot_high`, `pivot_low`, `nz`, `na`,
`fixnan`, and element-wise `add`, `sub`, `mul`, `div`, `max`, `min`,
`pow`, `abs`, `sqrt`, `log`, `exp`, with the proposal's `_scalar`
aliases). Nothing is reimplemented: the crossover and pivot helpers that
were missing were added to `pine.rs`, where the built-ins can use them too.

### Operator overloading: the proposal was wrong

Proposal 0001 says "Starlark has no operator overloading on custom types",
so series math would be `ta.sub(a, b)`. That is not true of
starlark-rust. Checked against the 0.14.2 source
(`src/values/traits.rs`), the `StarlarkValue` trait lets a custom value
implement `add`/`radd`, `mul`/`rmul`, `sub`, `div`, `percent`,
`floor_div`, `bit_and`/`bit_or`/`bit_xor`/`bit_not`, unary `minus`/`plus`,
`at` (indexing), `equals` and `compare`. There are two gaps:

- `sub`, `div` and `percent` exist only for the left operand. An `int` or
  `float` on the left rejects a custom right operand, so `1 - series` and
  `100 / series` fail.
- `compare` returns an `Ordering`, so `a < b` can only produce one bool,
  not a series.

Both are closed with `AstModule::replace_binary_operators`, a public
starlark-rust API that rewrites a binary operator into a call. Before
compiling, the runtime rewrites `+ - * / % << < <= > >= == !=` into calls
to native `_op_*` functions. These do series math in either operand order
and return a bool series for comparisons. For plain values they use the
built-in operator, with the guards below. Augmented assignments
(`x += y`) are rewritten in the source to `x = x + (y)` first, keeping
line numbers, so they go through the same functions. `and`, `or` and
`not` are not rewritten: they keep short-circuit semantics on plain
values, and `&`, `|` and `~` are the element-wise logic for bool series.
`series[k]` is Pine's history operator: the series `k` bars ago.

The `ta.*` function forms stay, for the proposal's examples and for
people who prefer them.

What remains: a series is always truthy, so `if close > open:` does not
branch per bar. Per-bar logic uses bool series with `chart.dot(cond, ...)`
or a loop over `series.list()`.

### Sandbox

Several limits each cover one cause. None of them is a complete memory
sandbox on its own.

| Limit | Mechanism | Value |
|---|---|---|
| No load or I/O | `Dialect { enable_load: false }`; the standard globals have no I/O | — |
| CPU, deterministic | `Evaluator::set_max_tick_count` (calls + loop iterations) | 20 M ticks |
| CPU, wall clock | `set_check_cancelled` (every 1000 ticks) plus a deadline check in every native `ta.*` / series call | 1000 ms per `compute`, 200 ms at register |
| Recursion | `set_max_callstack_size` | 64 frames |
| Starlark heap | `set_max_heap_size` (checked every 1000 ticks) | 64 MiB |
| Series data | exact per-evaluation counter on series allocation (series data lives outside the Starlark heap, so the heap limit does not see it) | 32 Mi values (256 MiB) |
| One-step growth | the `_op_*` guards: `+` and `*` on strings, lists and tuples, `%` formatting, `range`, `zip`, `str` and `repr` refuse results over 20 000 elements; ints are 64-bit with overflow errors; `<<` is capped | 20 000 |
| Growth methods | static check rejects `.join`, `.format`, `.replace`, `.extend` and `getattr` | — |
| Shadowing | static check rejects top-level definitions that shadow a global (`ta`, `len`, the `_op_*` functions, ...) | — |
| Inputs | source size, bars per request (5000 + 1000 warmup), plot objects per chart | 64 KiB, 6000, 64 / 10 000 |
| Panics | `catch_unwind` around each evaluation | — |

Why the size guards: the heap and tick checks run only every 1000 ticks,
and one Starlark step can allocate a lot. For example, `[0] * 10**9` is a
single 8 GB allocation, and a few `s = s + s` doublings reach gigabytes
between two checks. Rust aborts the process when an allocation fails
(`set_alloc_error_hook` is unstable), so the bound must apply before the
allocation, not after it.

The register-time pipeline is: parse → static check → evaluate the top
level and presence check → dry run of `compute` on 100 synthetic bars
within 200 ms. Each failure is a structured `{step, line, message}`
(ADR-0025 covers how it reaches the caller).

### Consequences

- Good: Pine-like scripts (`(ap - esa) / (0.015 * d)`, `wt1[1] < wt2[1]`)
  without a custom language. The WaveTrend reference implementation is
  25 lines and matches the Rust built-in within 1e-9 on six fixtures.
- Good: the sandbox comes from the language. There is nothing to strip,
  and an unknown global is a compile error.
- Good: series math runs as native vectorized code; only explicit
  per-bar loops run in the interpreter.
- Bad: rewriting operators makes every `+` or `<` a native call (about
  50–100 ns instead of about 10 ns) and one tick each. That is negligible
  for vectorized scripts and noticeable for long per-bar loops.
- Bad: the dependency is heavy. starlark-rust adds about 115 crates
  (including `rustyline` and `lsp-types`, which are not optional) and
  build time.
- Bad: **memory limits are best effort.** Upstream says starlark-rust
  "should not be considered secure against truly malicious code". The
  guards cover the known amplification paths, but a determined user can
  still make the server allocate up to roughly a gigabyte between two
  heap checks, for example by copying lists of 20 000 elements in a
  loop. A hostile multi-tenant deployment should run chartgen under an OS
  memory limit (cgroup `MemoryMax`, a container limit) as the backstop.
  The feature is off by default for this reason.
- Bad: pivot tie-breaking (`ta.pivot_high` / `pivot_low`) is unverified
  against a live TradingView chart.

## Links

- [Proposal 0001](/chartgen/proposals/0001-python-indicators/)
- [ADR-0025: user indicator registry and MCP surface](./0025-user-indicator-registry/)
- [ADR-0018: Pine v5 semantics](./0018-pine-semantics-for-indicators/)
- [Guide: scripted indicators](/chartgen/guides/scripted-indicators/)
- starlark-rust: <https://github.com/facebook/starlark-rust>
