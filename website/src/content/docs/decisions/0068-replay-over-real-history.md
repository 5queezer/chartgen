---
title: "0068 — Replay over real Binance history"
description: "A replay server (ADR-0059) can run on real Binance 1m history for a chosen symbol set and date range: a fixture builder writes the window once, a private replay instance answers from it, and a harness on the agent's side wakes the agent at bar closes with as_of. Setups score on the 1m bars between wake-ups as they do live. Server-side wake-ups and funding history are later phases."
sidebar:
  order: 68
---

**Status:** Accepted (phase 1); phases 2 and 3 proposed; amended by [ADR-0070](/chartgen/decisions/0070-stock-setups/) (stock symbols in fixtures and in the builder)
**Date:** 2026-11-12
**Amends:** [ADR-0059](/chartgen/decisions/0059-setups-paper-scored/) (a replay's fixtures may be
real history written by a fixture builder; the engine is unchanged)

## Context and Problem Statement

The trading agent (Hermes) keeps a playbook. A rule earns a place in it after at least five closed
setups. Live, at a few setups a week, that takes weeks per rule, and the exit variants of
[ADR-0066](/chartgen/decisions/0066-shadow-exit-variants/) need the same closed setups before they
say anything. Hermes ranks backtesting as its most wanted missing feature.

ADR-0059 already has a replay server. `CHARTGEN_REPLAY_FIXTURES=<dir>` loads 1m bars per symbol,
every price tool takes a required `as_of`, and one clock per process only moves forward:

- **The cut.** `Replay::bars` keeps a 1m bar only if it closed at or before `as_of`, and builds
  every other interval from those, whole bars only.
- **Quotes.** `get_quotes` is answered from the same 1m bars.
- **Setups.** Each setup call first moves the caller's setups through the 1m bars closed by
  `as_of`, with the live scorer (`Record::apply_bar`).
- **Refused tools.** Footprint, TPO, symbol search, subscriptions, every alert tool, the trading
  tools and the indicator registry are refused. So are indicators flagged `reads_beyond_bars`:
  funding, open interest, long/short ratio and fear & greed.
- **Isolation.** Live caches, the `--trade` engine cache and the database are never reached. The
  store is in memory. A replay refuses to start with sign-in, server alerts or `--trade`.
- **Tests.** The look-ahead proof compares answers from two fixtures that differ only after
  `as_of`, over every registered indicator and chart style.

What it lacks is real data: its fixtures are hand-made. The question is the smallest change that
lets Hermes run its playbook over real history in hours, without letting anything after `as_of`
reach it.

## Decision Drivers

- **No look-ahead,** through chartgen or around it. That includes the agent's other tools and what
  the model itself remembers.
- **The same scorer, indicators and costs as live,** so a replayed track record is comparable with
  a live one (the argument of ADR-0066 against scoring outside the server).
- **The agent is tested, not a formula.** Hermes's rules are written as a thesis read from charts,
  not as code.
- **Small:** reuse the ADR-0059 engine and tests. Little new server code.
- **The live server and the radxa (3.8 GB, home IP behind the Binance gate) are not put at risk.**
- **A run can be repeated and audited afterwards.**

## Considered Options

1. **A replay instance over real history:** a fixture builder, a private replay process, and a
   harness that wakes the agent at bar closes.
2. **A batch `backtest_rule` tool with a rule language:** the server walks history and opens a
   setup wherever a rule matches.
3. **A local script:** download the bars and compute signals and outcomes in Python, outside
   chartgen.

## Decision Outcome

Chosen: **1**, in three phases. Phase 1 is accepted and implemented; phases 2 and 3 are still
proposed.

### Phase 1: real fixtures, a private instance, a harness

**Fixture builder.** A `chartgen replay-fixtures --symbols BTCUSDT,ETHUSDT --from 2026-08-01 --to
2026-10-31 --out <dir>` subcommand. It reads 1m klines over Binance REST through the existing
fetch layer and gate (1000 bars per request), and writes the ADR-0059 fixture format, one file per
symbol. `Replay::fixture_bars` validates them on load as it does today.
- As built: `--to` is the last day, included, and must have ended (UTC), so a fixture holds closed
  bars only. A run may write at most **460 symbol-days** (5 symbols over a 92-day quarter, about
  66 MB in the replay's memory, see Sizes); a larger one is refused before any request.
- It pauses 100 ms before each request: at most 1200 of the 6000 weight a minute Binance allows
  the IP, so a live server on the same address keeps the rest.
- A 429, a 418 or the gate's refusal reads the same page again after the gate's wait
  (`Retry-After`, or the rest of the minute), at most 5 times, and never after a wait over 5
  minutes. Then the run stops and says how many seconds to wait before running again: a rerun at
  once would meet the same limit, and requests that keep coming get the home IP banned (418),
  which the live server shares. Any other failed read stops the run at once.
- Each file is written through a `.part` file and records its window (`from`, `to`,
  `shift_days`), which the replay ignores. A rerun keeps a symbol whose file has the same window
  and reads only the others, so an interrupted run costs at most one symbol's requests again.
- One window per directory: a replay loads every `*.json` of its directory as one run, so the
  builder refuses an `--out` holding a `*.json` of another window, or of none recorded, before
  any request. It may be a run someone means to audit.
- Cost: 90 days of one symbol is 129,600 bars, so 130 requests at weight 2: well under a minute's
  budget.
- It runs where the replay runs (the PC by default), not on the live server, so the live gate
  never sees it.
- Alternative: Binance's archive (data.binance.vision, monthly zips of 1m klines) costs no API
  weight. It would need its own reader (and its timestamps may be in µs from 2025 on: check), so
  REST through the fetch layer is the smaller change.
- **Gaps.** Binance has minutes with no trade on small symbols and exchange outages. The builder
  must not invent bars. A missing minute stays missing, and the builder reports gaps over 5
  minutes, also at the window's edges.
- **Gaps in `Replay::bars` (checked, and pinned by a test).** A bucket is built from the minutes
  it has, dated at its grid start, and kept once its grid end has passed `as_of`: a gap of hours
  inside a 4h bucket gives a 4h bar from the minutes before and after it. A bucket with no minute
  at all has no bar, as Binance's own klines of an outage have none. One edge case: a bucket that
  began before the fixture's first minute is dropped, so if a window's first minutes are missing,
  its first bar of each longer interval is dropped too.
- **Optional time shift.** `--shift-days N` moves every timestamp by N days, so the agent cannot
  recognise the dates. Fixtures may already be dated in the future (ADR-0059). Prices still betray
  the regime (see Consequences). The shift must be a multiple of 7, or weekends would land on
  weekdays and every weekly bar would hold other days; `--shift-any` allows it anyway. Any shift
  changes the monthly bars, and multi-day bars (`2D`, `3D`) unless it is a multiple of their
  length too.

**Instance.** The unchanged replay server, started with `CHARTGEN_REPLAY_FIXTURES=<dir>`:
- **Sizes.** In memory a 1m `Bar` is about 100 bytes (five f64 plus a String date). 90 days × 5
  symbols is about 65 MB; 1 year × 10 symbols is about 500 MB, too much beside the live services on
  the radxa. A fixture is about 56 bytes per bar of JSON (measured on BTCUSDT: 81 kB a day,
  ≈ 7.5 MB per symbol-quarter).
- **Where.**
  - Run it on the PC. Windows of up to ~90 days × 5 symbols also fit on the radxa as a second,
    unrouted container.
  - One process is one run (one clock). Parallel runs are separate processes on separate ports.
- **Private by network.**
  - A replay refuses a sign-in provider, and with sign-in off `/authorize` issues a code to any
    registered client without a person (checked in `oauth_authorize`).
  - Anyone who can reach it can register and move the one clock to the end of the window, which
    spoils the run. So it is never routed through Cloudflare, and is reachable on the Tailscale
    address (PC) or an internal Docker network (radxa) only.
  - `--serve` listens on every interface (`0.0.0.0`) unless `CHARTGEN_BIND` names one address.
    A bare process on the PC sets it to the Tailscale address. In a container it stays unset,
    and the privacy comes from the port being published on the Tailscale address only, or from
    an internal network with no published port. A replay server prints its address at start and
    warns when it is neither loopback nor in Tailscale's ranges.
  - Hermes's MCP client can do the OAuth flow headlessly, since no person is needed.
- **Performance.** `Replay::bars` scans every 1m bar from the window start on each call (O(window)).
  At 90 days that is ~130k bars per chart call, a few ms. A start bound from `limit` × interval can
  come later if longer windows are used.

**Harness (Hermes's side, not chartgen).** A script that, for each close of the chosen interval
(e.g. every 4h close) in the window:
1. builds the same text a live wake-up would carry, with `as_of` = that close;
2. starts a Hermes decision session whose **only** MCP servers are the replay instance and the
   replay calendar (below);
3. tells it to pass that `as_of` on every call.

Setups need no wake-up to be scored: each call's catch-up moves them through every 1m bar since
the last call, with the live scorer and the shadow variants of ADR-0066. A fill or stop between two
closes is found at the next wake-up, and was scored at the right minute. At the end the harness
reads `list_setups` (whose answer says `replay: true`) and `get_setup` for each, and writes the run (fixtures' hash, window, model, prompt
version, every setup with its log) to a JSONL file.

**Wake-ups at bar closes come from the harness, not the server.** This is the smallest working
loop. A setup's own events (filled, stopped, won) are not wake-ups in phase 1; the agent sees them
at the next close, as it would after a delayed live wake-up.

### Look-ahead: what the agent can reach at `as_of`

| Surface | In replay | Why |
|---|---|---|
| `generate_chart`, `inspect_chart`, `get_indicators` | allowed | bars cut at `as_of`; every interval built from closed 1m bars; tested by the differ-after-`as_of` fixtures over every indicator and chart style |
| bars-only indicators incl. `chart_patterns`, `auto_fib`, candle patterns | allowed | they only see the cut bars; covered by the same test |
| `get_quotes` | allowed | last 1m close and the 24 h before `as_of` |
| setup tools | allowed | require `as_of`, catch up to it with the live scorer; `expires_at` and the skip bracket use `as_of` |
| `list_indicators`, `list_drawing_tools` | allowed | no prices |
| funding, open interest, long/short ratio, fear & greed (indicators) | **refused** | not in 1m klines. Funding history is available back to listing (`/fapi/v1/fundingRate` with `startTime`), so it can become a fixture in phase 3. Open interest and long/short history only go back 30 days (`/futures/data/*Hist`), so they could only cover windows that were recent when the fixture was built. Fear & greed has a daily history. |
| `get_footprint`, `get_tpo` | **refused** | need trades, not 1m bars |
| `search_symbols` | **refused** | today's listings, including symbols that did not exist at `as_of` |
| subscriptions, browser alerts, server alerts, notifications, push | **refused** | wall-clock and live feeds; replay alerts are phase 2 |
| trading tools (`place_order`, …) | **refused** | `--trade` cannot run with a replay |
| scripted-indicator registry | **refused** | phase 3 at most, for scripts that read bars only |
| **the agent's other MCP servers** (web search, news, the macro calendar) | **must not be attached** | they answer for today. The harness gives a replay session the replay instance only. |
| macro calendar | **replaced** | `macro_blackout` covers the current week only and fails closed, so in a replay it would block every setup. The harness supplies a calendar fixture of past US releases (FOMC, CPI, PPI, NFP, claims, GDP, PCE, retail sales) for the window, answered at `as_of`. |
| **the model's own memory** | **cannot be cut** | a model trained past the window may know what BTC did next. Prefer windows after the model's training cutoff; the time shift hides dates but not price levels. This is recorded with every run. |

### Phase 2 (later, own ADR): wake-ups in replay

A replay-only `advance` tool: "move the clock to the first of: a setup event of mine, an alert of
mine, a bar close of interval X, or time T; return the wake-up there." The server walks the 1m bars
from its clock with the live evaluators: `Record::apply_bar`, and the price and indicator alert
rules of ADR-0042 and ADR-0067, run over the replay's bars instead of a feed. It returns the body
`agent_body` builds today. Server alerts would then be allowed in a replay, held in memory, with no
webhook or push. This cuts the number of agent sessions per window from "every close" to "every
event", but it is real server code (an alert evaluator over replay bars, a new tool, tests).

### Phase 3 (later): more series

Funding rate history as fixtures (refused until then), bars-only scripted indicators, and a start
bound in `Replay::bars` for year-long windows.

### Why not the others

- **Option 2 (`backtest_rule`)** tests a formula, not the agent. Hermes's rules are a thesis read
  from charts ("retest of the range high with RSI cooling"), and a rule language able to say that
  is a project of its own: grammar, validation, an ADR, and a second way to open setups. It is
  fast and deterministic, and worth having for the mechanical part of a rule once one is written
  down. It can reuse the scorer as phase 2 would, so it stays open as a later option, not a
  replacement.
- **Option 3 (a local script)** repeats the scorer, the costs and the Pine-semantics indicators
  outside chartgen, where they drift. Its R would not be comparable with live `result_r`, and
  nobody reading a setup could check it. It is the cheapest way to sanity-check a mechanical
  signal, nothing more.

### Cost (estimate)

- **chartgen phase 1:**
  - The fixture builder subcommand: ~150–250 lines with tests, reusing `fetch` paging and the
    gate.
  - A gap report.
  - Docs, this ADR, and a test that a built fixture loads and replays.
  - Deploy: a second process with its own env, no DB, private address.
  - About 1–2 days.
- **Hermes harness:** the loop, a session per close with a reduced MCP set, a calendar fixture,
  and the run log. About 1 day, depending on how Hermes's gateway starts a session from a script
  (not checked here).
- **Running it:** dominated by model calls.
  - 90 days stepped at 4h closes is 540 sessions. At ~30–60 s each that is 4.5–9 h of wall time.
  - At an assumed $0.05–0.30 per session it costs ~$30–160 per run. Both figures depend on
    Hermes's model and prompt size, which I did not measure.
  - Stepping at 1h is 4× that. Phase 2's event wake-ups would cut it.
  - Five closed setups per rule needs a window where the rule's conditions occur at least ~5
    times; a quarter of 4h bars across 3–5 symbols is a reasonable first window.
- **Fixture download:** seconds per symbol-quarter. Disk: ~9 MB per symbol-quarter.

### Consequences

- Good, because a playbook rule can collect its five closed setups in an afternoon, scored by the
  live scorer with the exit variants, and comparable with live results.
- Good, because chartgen's change is small and outside the request path. The look-ahead guarantee
  is ADR-0059's, already tested.
- Good, because a run is a file of fixtures plus a JSONL log: it can be replayed and audited.
- Bad, because the model's memory of the market is a look-ahead chartgen cannot cut. Windows
  after the model's cutoff, and the time shift, reduce it; nothing removes it.
- Bad, because rules that depend on funding, open interest or the order book cannot be tested
  before phase 3, or at all for open-interest windows older than 30 days.
- Bad, because a replay instance reachable by others can be spoiled by moving its clock, so it
  must stay off public routes.
- Neutral: replay setups live in the replay process's memory. The harness must export them before
  the process ends.

### Confirmation

- The builder's output loads with `Replay::load` and passes the existing replay tests unchanged.
- A new test builds a fixture from REST pages in Binance's format (no network) and checks the bar
  count, order, gap report and time shift, that a rerun reads nothing, that a directory holding
  another window is refused, and that a 429 reads the same page again while a long ban ends the
  run (`src/replay_fixtures.rs`). Another pins how `Replay::bars` builds buckets across a gap of
  hours (`src/replay.rs`).
- On the harness's side, not chartgen's:
  - A run of a fixed harness script against a fixed fixture twice gives byte-identical tool
    answers (the agent's choices may differ; the server's answers to the same calls do not).
  - The harness's session config is checked to contain the replay MCP server only.
