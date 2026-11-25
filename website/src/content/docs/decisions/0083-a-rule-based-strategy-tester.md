---
title: "0083 — A rule-based strategy tester over replay fixtures"
description: "Accepted: option 3. A playbook rule that can be written down mechanically (an indicator condition, an entry, a bracket) could be tested over a past quarter in minutes, with no model in the loop: the rule finds its signals in replay fixtures, and each signal is scored as a setup by the live scorer, with the live costs and exit variants. Of six options, the owner chose an entry rule in the shape of a server alert's conditions (AND-joined), with a bracket, scored by Record::apply_bar, run as the CLI chartgen backtest over fixtures first."
sidebar:
  order: 83
---

**Status:** Accepted
**Date:** 2026-11-24 (proposed), 2026-11-25 (accepted)
**Builds on:** [ADR-0059](/chartgen/decisions/0059-setups-paper-scored/) (the setup scorer),
[ADR-0064](/chartgen/decisions/0064-a-setups-record/) (the track record),
[ADR-0066](/chartgen/decisions/0066-shadow-exit-variants/) (the exit variants),
[ADR-0067](/chartgen/decisions/0067-indicator-alerts-on-the-server/) (indicator conditions at
bar closes), [ADR-0068](/chartgen/decisions/0068-replay-over-real-history/) (replay fixtures from
real history) and [ADR-0077](/chartgen/decisions/0077-a-setup-records-its-market-context/) (the
market context on a setup)

Proposed on 2026-11-24 with the questions at the end; the owner chose option 3 on 2026-11-25.
The answers and what was built are under [Decision Outcome](#decision-outcome). The options and
their discussion below are kept as they were proposed. The guide is
[Strategy tester](/chartgen/guides/strategy-tester/).

## Context and Problem Statement

An agent client that trades on paper through chartgen keeps playbook rules, and a rule counts
for it only after it has a handful of closed live setups of its own (five, in the client that
asked for this). At a few setups a week that is weeks per rule, and weeks more to see whether a
change to the rule helped.

[ADR-0068](/chartgen/decisions/0068-replay-over-real-history/) shortened that with a replay over
real Binance history, but it tests the agent, not the rule: its harness starts one model session
per bar close. A quarter stepped at 4h closes is 540 sessions, 4.5 to 9 hours and an estimated
$30 to $160 per run. That is right for a rule that is a thesis read from a chart. It is far too
slow and too expensive for the part of a rule that is mechanical, such as:

> RSI(14) on the 4h crosses above 70 at a bar's close: short at that close, stop above that
> bar's high, target 2R, out after 24 bars.

"At that close" is read here as "as soon as the signal bar has closed", which in 1m bars is the
open of the next minute. A rule that waits one more bar ("short at the close of the bar after
the signal") is the same rule with its entry one bar later; every option below can say either.

ADR-0068 considered a batch `backtest_rule` tool and set it aside as "a later option, not a
replacement": fast and deterministic, worth having for the mechanical part of a rule once one is
written down. This ADR is that later option. The question is how chartgen should test a rule
of that kind over history in minutes, with no model in the loop, so that its result can be read
beside a live track record without being mistaken for one.

### What exists (checked in the code at `e359ecc`)

- **The scorer is pure.** `Record::apply_bar` in `src/setups.rs` moves one setup through one
  closed 1m bar, with the fill, stop, target, expiry and tie rules of ADR-0059, the costs
  (`Costs`, 5 bps fee and 2 bps slippage per side on Binance), `mae_r`/`mfe_r`, and the three
  shadow exit variants of ADR-0066. It does no I/O; the live feed and the replay both call it.
  On a replay the catch-up wraps it in one more step (`stocks::score` in
  `src/setup_service/stocks.rs`): a stock's splits and the fixture's dividends go through
  `Record::adjust` at their time, and `Record::expire_by` ends a stock setup whose expiry has
  passed with no bar after it (a weekend, the end of the data) at its last close. For a Binance
  setup both are no-ops.
- **A market entry already exists.** A skip fills at the open of the first 1m bar it sees
  (`Decision::Skip => bar.open`). A take fills when a 1m bar reaches its entry zone, at the
  bar's open clamped into the zone.
- **The risk floor.** `check_levels` refuses a setup whose stop is closer than 3 round-trip
  costs (42 bps of the price on Binance).
- **The track record is a function.** `track_record(closed, by)` and `shadow_record(closed)`
  count any slice of records: count, won, invalidated, expired, cancelled, total and mean R,
  entered and `mean_r_entered`, gaps, the probability block, and the variants each paired with
  its canonical figures. It reports counts and sums and judges nothing (ADR-0064). The service
  around it reads at most the newest 500 closed setups.
- **Replay fixtures.** `chartgen replay-fixtures` writes 1m Binance (or Yahoo) history per
  symbol, at most 460 symbol-days a run. `Replay::bars` builds every longer interval from those
  1m bars, whole bars only, and drops a bucket that began before the fixture's first minute.
- **Indicator conditions at a close.** A server indicator alert (`src/server_alerts/indicator.rs`)
  names one of ten built-ins (`rsi`, `ma`, `macd`, `stoch`, `cci`, `mfi`, `williams_r`, `roc`,
  `atr`, `adx`), its params, a line by its legend label, `compare: level | close`, and a
  condition (`crossing`, `crossing_up`, `crossing_down`, `greater`, `less`). At each close it
  computes the indicator over the last 999 closed bars and compares now with one bar before.
  These ten are causal: each value depends only on its bar and those before it. That module is
  in the binary crate today, `pub(super)`.
- **Scripted indicators.** A Starlark runtime (ADR-0024) with a `ta` module, opt-in through
  `CHARTGEN_SCRIPTING=1`, with per-run timeouts.
- **Limits on a replay server.** 20 open setups, 500 setups and 5 open symbols per owner, as
  live; the 10-changes-a-minute limit is lifted there.

## Decision Drivers

- **No model in the loop**, and minutes per run, not hours.
- **The live scorer, indicators and costs,** so a tested rule's R means what a live setup's R
  means (the argument of ADR-0066 and ADR-0068 against scoring outside the server).
- **No look-ahead** in the signal, the fill or the indicators, provable by a test.
- **Clearly a backtest:** a result can never be read as, or mixed into, a live track record.
- **Repeatable and auditable:** the same rule on the same fixtures gives the same answer, and
  the run says what it was run on.
- **Small,** and no second language where one exists: what an alert can watch, a rule can test.
- **The live server is not put at risk** (its memory, its Binance weight, its store).

## Considered Options

1. **Do nothing: rely on the replay of ADR-0068,** and its proposed phase 2 (`advance`, wake-ups
   at events rather than every close).
2. **A model-free script against a replay server.** No chartgen code: a script on the client's
   side walks the closes, reads indicators with `as_of`, decides in code, and calls
   `create_setup` with `as_of`; the replay scores as it does today.
3. **An entry rule in the shape of a server alert's indicator condition, plus a bracket,**
   evaluated by chartgen over replay fixtures and scored by `Record::apply_bar` (a CLI first,
   `chartgen backtest`).
4. **A rule language over indicator lines:** several conditions joined by AND/OR, on several
   intervals, with bar fields, evaluated and scored as in 3.
5. **A rule as a Starlark script** on the existing scripting runtime, returning signals that are
   scored as in 3.
6. **An external backtest library** (for example vectorbt or backtesting.py in Python, or barter
   in Rust).

### The example rule in each option

**1. Replay.** The rule stays prose in the agent's playbook. The harness wakes a model session
at each 4h close, the model reads RSI with `get_indicators` and decides. 540 sessions a quarter.

**2. A script against a replay server.** The rule is code on the client's side:

```python
for t in closes("4h", window):                       # time-major: the replay clock only moves forward
    for sym in symbols:
        s = generate_chart(sym, "4h", format="series", indicators=[{"name": "rsi", "length": 14}],
                           bars=2, warmup=997, as_of=t)   # 997 + 2 = the 999 closed bars an alert reads
        if len(s.rsi) < 2 or None in s.rsi[-2:]:
            continue                                  # first closes of the window: RSI not warmed up yet
        prev, now = s.rsi[-2], s.rsi[-1]
        if prev < 70 <= now and not open_setup(sym):
            bar = s.bars[-1]
            create_setup(sym, "4h", "short", thesis="rule rsi-ob-4h v1",
                         entry={"low": bar.close, "high": bar.close * 1.001},
                         invalidation=bar.high, targets=[bar.close - 2 * (bar.high - bar.close)],
                         expires_at=t + 24 * 4 * 3600, tags=["backtest", "rule:rsi-ob-4h"], as_of=t)
```

(`get_indicators` takes indicator names with default params and returns no bar fields, so the
script reads a `series` chart; without `warmup` a two-bar series would compute RSI over two
bars, and at a window's first closes there is not even that much history.) The entry has to be a zone; "at that close" becomes a narrow zone the next 1m bar usually
reaches. The target is fixed before the fill, so where the fill lands inside the zone the reward
is not exactly 2R of the risk at the fill: one of the limits of this option.

**3. An alert condition and a bracket.**

```json
{
  "name": "rsi-ob-4h",
  "symbols": ["BTCUSDT", "ETHUSDT", "SOLUSDT"],
  "interval": "4h",
  "direction": "short",
  "when": { "indicator": "rsi", "params": { "length": 14 }, "line": "RSI",
            "compare": "level", "condition": "crossing_up", "level": 70 },
  "entry": "next_open",
  "stop": { "at": "signal_high", "buffer_atr": 0 },
  "targets_r": [2],
  "expiry_bars": 24,
  "overlap": "one_per_symbol"
}
```

`when` is exactly the indicator part of `create_server_alert`'s arguments, checked by the same
parser. `entry: next_open` fills at the open of the first 1m bar that opens at the signal bar's
close, as a skip fills today.

**4. A rule language.** The same, with `when` a tree:

```json
"when": { "all": [
  { "indicator": "rsi", "params": { "length": 14 }, "interval": "4h",
    "condition": "crossing_up", "level": 70 },
  { "indicator": "ma", "params": { "type": "ema", "length": 50 }, "interval": "1d",
    "compare": "close", "condition": "less" }
] }
```

**5. Starlark.**

```python
def signals(bars, params):
    r = ta.rsi(bars.close, 14)
    return [i for i in range(1, len(r)) if r[i - 1] < 70 and r[i] >= 70]
```

The stop, target and expiry stay JSON as in 3.

**6. An external library.** The rule, the RSI, the fills, the costs and the exits are all written
in the library's terms, over bars the library loads.

## Common to every option that scores in chartgen (2 to 5)

### How a signal becomes a scored setup

- **The signal** is read at a bar's close only, from closed bars only: the bar of the
  rule's interval whose grid end is at or before the time asked. A higher interval in a rule
  (option 4) reads its last *closed* bar, never the one forming.
- **The fill** is at the open of the first 1m bar that opens at or after the signal bar's close
  (`next_open`), as a skip's fill. A rule may instead give an entry zone, scored as a take.
- **The stop** from the signal bar's high (a short) or low (a long), optionally plus a buffer in
  ATR of the rule's interval. **The risk floor stays:** a signal whose stop would be closer than
  3 round-trip costs is not traded and is counted as `refused_min_risk`. It is not silently
  widened (the skip's `floor` bracket widens; whether a rule may opt into that is a question
  below).
- **The targets** in R from the fill, **the expiry** in bars of the interval (at most 90 days).
- **Checked at the fill, not at the signal.** The levels are set once the fill price is known,
  and checked as `create_setup` checks them: a fill at or beyond the stop (a gap through the
  signal bar's extreme), or one that leaves less risk than the floor, is not traded and counted
  as `refused_at_fill`.
- **The end of the fixture.** A stock trade whose expiry the fixture has passed is first ended
  at its last close by `expire_by`, as the replay does. A trade still open after that, when the
  fixture's 1m bars run out, has no result. It is counted as `open_at_end` and listed, and is not in the closed figures, as an open
  setup is not in a live track record. Shadow variants still open there are counted as open, as
  `shadow_record` does. A rule whose expiry is long beside the window loses trades this way, and
  the count says how many.
- **Overlap and limits:** by default one open position per rule and symbol; a signal while one is
  open is counted as `skipped_open`. The live per-owner limits apply too (20 open setups, 5
  symbols with an open setup), so a run never holds a trade a live owner could not have opened;
  a signal they turn away is counted as `skipped_limit`.
- Each signal becomes a `Record` scored over the fixture's 1m bars by the replay's own scoring
  step (`apply_bar`, with a stock's dividends through `Record::adjust` and its expiry through
  `expire_by`, as above), not by `apply_bar` alone, with the live `Costs`, so it has `result_r`, `mae_r`/`mfe_r` and the three shadow variants exactly as a live
  setup would. `ctx_` features are `unknown`, as on any replay (ADR-0077).

### How results are reported

A run's answer holds the same figures `get_track_record` gives, computed by the same
`track_record` and `shadow_record` functions over the run's records (all of them, not cut at
500), and says on its face that it is a backtest:

```json
{
  "backtest": true,
  "rule": { "...": "as given" }, "rule_sha256": "…",
  "fixtures": { "from": "2026-08-01", "to": "2026-10-31", "symbols": ["BTCUSDT", "ETHUSDT", "SOLUSDT"], "sha256": "…" },
  "costs": { "fee_bps_per_side": 5.0, "slippage_bps_per_side": 2.0 },
  "signals": 41, "traded": 32, "refused_min_risk": 3, "skipped_open": 6, "skipped_limit": 0, "refused_at_fill": 0, "before_warmup": 0, "open_at_end": 1,
  "total": { "count": 31, "won": 13, "invalidated": 15, "expired": 3, "mean_r": 0.11, "...": "as get_track_record" },
  "groups": [ { "key": "BTCUSDT", "...": "per symbol" } ],
  "by_month": [ { "key": "2026-08", "...": "per month" } ],
  "shadows": { "...": "as get_track_record" },
  "setups": [ { "symbol": "ETHUSDT", "signal_at": "…", "fill": "…", "exit": "…", "status": "won", "result_r": 1.86 } ]
}
```

(The numbers are made up.) Per-symbol and per-month groups are there so a reader sees whether
one symbol or one month carries the result. Whatever the live record gains (a figure after costs,
for one, is being designed) comes with it, since it is the same function.

**No interval and no significance by default.** ADR-0064 decided that the record reports counts
and sums and judges nothing, because setups overlap in time and share causes, and the changes
to the record being designed now keep that.
A backtest has more trades, which makes an interval tempting, but the dependence is worse, not
better: one rule on correlated symbols fires on the same moves. Whether a backtest should add a
descriptive spread figure anyway is a question for the owner.

**Never in the live record.** A run's records are not written to the setups store, not shown in
the Setups panel and not counted by `get_track_record` on the live server. If a later phase
makes them visible there, they carry `backtest: true` and the tag `backtest`, and the record
leaves them out unless asked.

### Risks

- **Look-ahead.**
  - *Indicators that repaint:* zigzag, pivots, chart patterns, auto fib and provisional swing
    ends (ADR-0055) change past values as bars arrive. They are not allowed in a rule (the ten
    alert indicators are all causal), and a new one is allowed only with a test that its value at
    every bar of a series equals its value on the series cut at that bar.
  - *Recursive indicators:* RSI and EMA depend on how much history they were seeded with. With
    999 closed bars before each close (see the warm-up gap below), the tester reads each close
    over the same 999 bars an alert reads, so a signal is the one a live alert would have fired,
    not a close relative of it.
  - *The signal bar:* nothing about a bar is used before its close, and the fill is never inside
    the signal bar. A higher interval's condition reads its last closed bar.
  - *Selection:* symbols chosen because they exist and are liquid today are a survivor sample;
    the run records which symbols it was run on, and nothing hides that.
  - Option 5 has its own: a script gets the whole series and can read bar `i + 1` when it decides
    bar `i`, to emit a signal or to hold one back. So the script is re-run on the series cut at
    every close of the window, not only at its signals, and the run is refused when any cut
    answer for that close differs from the full-series one.
  - Unlike ADR-0068, the model's memory is not a risk: there is no model.
- **Fees and slippage.** The live costs are applied, so a tested rule is comparable with live
  setups. They are not the market's costs: an entry exactly at a 4h close, in the minute
  everyone else enters, may slip more than 2 bps, and funding is not charged to a position held
  for days (live setups do not charge it either). A run could add a line at stressed costs (say
  twice the slippage) beside the live one, never instead of it.
- **Overfitting.** A rule tuned on the window it is tested on looks better than it is, and every
  parameter tried is a test. A playbook rule derived from the recent live weeks is already
  in-sample on them. The run log keeps every rule tried on a fixture set, and the guide asks for
  a second, untouched window before a rule counts. Whether chartgen should enforce that is a
  question below.
- **The warm-up gap.** A fixture holds 1m bars from its window's first minute only, and the
  replay builds every interval from them, so nothing before the window exists: RSI(14) on the 4h
  has no value for its first bars and is seeded differently from live for longer, and an EMA 200
  on the 1d needs 200 days, which the 460 symbol-day bound does not allow beside a second
  symbol. A replay already shows the agent short indicators at a window's start. Two ways out:
  - **A lead of native bars before the window** (klines of the rule's interval, one request per
    1000 bars, kept in the fixture beside the 1m bars): 999 bars before the first close, 166 days
    at 4h. Signals then match a live alert's from the window's first close. It costs a fixture
    format change and a few requests per symbol.
  - **Warm up inside the window:** no signal until the indicator's own `warmup_bars` are met
    there, counted as `before_warmup`. That costs the first days of each window (for RSI(14) on
    the 4h, a few days), and the values are read over the bars there are, fewer than an alert's
    999, so a signal near the threshold can differ from a live alert's. Waiting for the full 999
    bars inside the window is not an option: a quarter has about 540 4h bars, so it would give no
    signal at all.
- **Gaps.** A missing minute stays missing (ADR-0068); a trade across a gap is marked `gap` as a
  live one is, and counted in `gaps`.
- **Scope.** Only the mechanical part of a rule is tested. A rule that is a thesis read from a
  chart ("a breakout that holds above the range") cannot be, and its backtest number would
  wrongly stand for it. The rule a run tested is in its answer.

## Decision Outcome

Chosen option: **option 3**, an entry rule in the shape of a server alert's conditions plus a
bracket, scored by `Record::apply_bar`, run as a CLI over fixtures. It is built smaller than
proposed above: no separate signals core and no code moved into the library. The tester is a
subcommand of the binary, `src/backtest.rs`, which reads the alerts' own condition parser and
reading (`server_alerts::clauses`, now `pub(crate)`) and the replay's own loader and bar
builder (`replay.rs`) in place.

```bash
chartgen backtest --rule rule.json --fixtures <dir> [--trades trades.csv]
```

### The questions, answered

1. **Which option?** Option 3. Option 2 was not run first.
2. **Where does it run?** A CLI on the machine that holds the fixtures, with no server, no store
   and no Binance request. A tool on a replay server or on the live server is a later step, with
   its own decision.
3. **How much can a rule say?** What an alert can say now: up to four conditions joined by AND
   (an alert's own condition and its three `and` clauses, [ADR-0084](/chartgen/decisions/0084-conditions-joined-by-and/)),
   each a price level or one of the ten bar-only indicators of ADR-0067, all on the rule's one
   interval. Each is parsed by the parser an alert's `and` clause goes through, so a rule's
   condition is accepted, refused and read exactly as one. Funding and open interest are refused
   (the fixtures hold neither), and so are drawings. `bar_close` is refused, as in a clause.
4. **Entry and bracket.** In at the open of the first 1m bar that opens at or after the signal
   bar's close. The levels are set from that fill price, not from the signal close, so a target
   of 1.5 R is 1.5 R of the risk at the fill: the stop is `atr` times the ATR(14) of the rule's
   interval at the signal (the skip bracket's ATR), or `pct` percent of the fill; the targets are
   R multiples of that risk, nearest first, at most four; the expiry is a duration from the
   signal (`24h`, `3d`), at most 90 days. The trade is a take whose entry zone is the fill price,
   so it fills in that bar, and is scored by `apply_bar` exactly as a setup. Levels that
   `create_setup` would refuse, in practice a stop closer than the risk floor, and levels with a
   price at or below 0 (`Levels::positive`), are not traded and are counted as `refused`; they
   are never widened. The stop at the signal bar's extreme is
   not in this first version. **Overlap:** one open trade per rule and symbol. A signal before
   the 1m bar in which that trade ended has closed is counted as `skipped_open`. A trade whose
   exit variants are still open does not hold the symbol. The per-owner limits of a server
   (open setups, symbols with an open setup) are not applied: a run holds at most one trade per
   symbol it names.
5. **Reporting:** the live record's figures only: `track_record` (in all and by symbol) and
   `shadow_record` over the closed trades, all of them, plus a `summary` (n, won, invalidated,
   expired, win rate, mean R, net R) that restates `total`. No interval or spread figure.
6. **Warm-up:** inside the window, for now. This is not the recommendation: the fixtures that
   exist hold no lead of native bars, and the format change is its own piece of work. A close
   where a condition (or the ATR of an `atr` stop) has no value yet is counted as
   `before_warmup`. After that, a condition is read over the closed bars the fixture has, up to
   999, so in a window's first 999 bars an indicator is seeded over fewer bars than a live
   alert's, and a signal near its threshold can differ from a live alert's there. A lead of
   native bars in the fixture remains the way to exact parity from the first close.
7. **Costs:** the live costs by default (5 bps fee and 2 bps slippage per side on Binance). A
   rule may set its own `costs`, for a stressed run beside the live one; the answer says which
   costs it used. A stressed line is a second run, not a second figure in one answer.
8. **Overfitting and what counts:** chartgen does not enforce a second window; the guide asks
   for one. A run writes nothing to a setups store and its answer says `"backtest": true`, so its
   trades cannot enter a live track record; whether an agent client counts them toward its own
   rules is the client's rule.

### What was not built, and why

- **Stock fixtures** are refused: a stock's session grid, splits and dividends (`stocks::score`)
  are not wired in yet.
- **The `skipped_limit`, `refused_at_fill` and `refused_min_risk` counts** of the proposal are
  one count, `refused`, since the levels are checked once, at the fill; and per-owner limits are
  not applied (above).
- **`groups` by month** and a fixture hash are not in the answer; the answer gives each
  symbol's first and last minute and its count of minutes instead, and the rule's SHA-256.

### As proposed

Why this one:

- It is the smallest server change that is model-free and scored by the live scorer, with the
  costs and exit variants a live setup has.
- A rule's signal is the alert a live agent client could already set, so a tested rule can go
  live as an alert unchanged, and a live alert can be backtested unchanged.
- The rule language grows with the alert conditions (the new conditions and AND joins being
  designed beside this ADR), not as a second language beside them. Option 4 becomes the
  alerts' growth rather than its own project.
- The signals input lets a script (option 2) or an external library (option 6) hand signals to
  the live scorer later, which removes their main weakness.

**What could be done before a decision:** option 2 needs no chartgen code and works on a replay
server today. If the owner wants a first answer on one rule this week, a script of about a day
gives it, within the replay's limits (500 setups and 5 open symbols per owner: one OAuth client
per run). It is a good way to learn whether a tester is worth building at all.

### Cost (estimate)

| Option | chartgen work | Per run | Notes |
|---|---|---|---|
| 1. Replay only | none (phase 2 `advance`: about 2–3 days) | 4.5–9 h, ~$30–160 per quarter at 4h | tests the agent, not the rule |
| 2. Script on replay | none | minutes (one chart call per symbol per close) | rule lives outside chartgen; replay limits per owner |
| 3. Alert condition + bracket | about 3–5 days: core, front end, CLI, warm-up, tests, guide | seconds to a minute per symbol-quarter | single condition, one interval, until alert conditions grow |
| 4. Rule language | 3 plus about 3–5 days: grammar, validation, several intervals, docs | as 3 | best done as the alerts' own AND joins |
| 5. Starlark | 3 plus about 3–5 days: signal API, cut re-runs, limits | slower; the script re-run on the series cut at every close | needs scripting on, runs code a client wrote over long history |
| 6. External library | none in chartgen, its own project outside | fast | not comparable with live R |

The scoring cost is small: a symbol-quarter is about 130,000 1m bars, and each trade walks only
its own bars until it is out. The evaluation of an alert condition at every 4h close over 999
bars is about 540 × 999 steps per symbol-quarter.

### Consequences

If option 3 is chosen:

- Good, because a mechanical rule gets dozens of scored trades over a quarter in about a minute,
  scored and costed as live setups are, with the exit variants beside them.
- Good, because rule and alert are one shape: what is tested is what can be watched live.
- Good, because a run is a rule, a fixture hash and a JSON answer, repeatable byte for byte.
- Bad, because only what an alert can express can be tested until the alert conditions grow:
  one condition, one interval, ten indicators. A rule with an OI or funding condition cannot be
  tested, since fixtures hold neither (ADR-0068 phase 3).
- Bad, because a fast tester invites trying many variants on one window; the overfitting guard is
  advice and a log unless the owner wants it enforced.
- Bad, because the warm-up fix changes the fixture format (a lead of native-interval bars), or
  the tester gives up the first days of each window and exact parity with a live alert.
- Neutral: the server alert's condition parser moves from the binary into the library, with its
  tests.

## Pros and Cons of the Options

### 1. Do nothing: replay only

- Good: no new code; tests the agent as it trades, theses and all.
- Bad: a model session per close, hours and dollars per run; the model's memory is a look-ahead
  nothing removes. Phase 2 cuts the sessions but not the model.

### 2. A script against a replay server

- Good: no chartgen code; the live scorer, indicators and costs; available today.
- Bad: the rule is code in a client's script that nobody reading the record sees; the entry must
  be a zone (no market entry); the replay's per-owner limits bound a run; one process is one
  clock, so runs do not go in parallel on one server; and its answer is a replay track record,
  labelled `replay`, not `backtest`.

### 3. An alert condition and a bracket

- Good: smallest model-free change in chartgen; one shape for alerts and rules; exact parity with
  a live alert's signal; a scoring core others can feed.
- Bad: one condition on one interval until alerts grow; no drawing-based or OI/funding rules.

### 4. A rule language over indicator lines

- Good: says most mechanical rules (AND, several intervals, bar fields).
- Bad: a grammar, its validation and its docs are a project of their own, and it would be a second
  way to say what an alert says unless it is the alerts' own extension.

### 5. A Starlark rule

- Good: says anything; the runtime and `ta` module exist.
- Bad: code a client wrote, run over months of bars; look-ahead is the script's to avoid and
  chartgen can only catch it by re-running the script on the series cut at every close; scripting is opt-in and
  off by default.

### 6. An external backtest library

- Good: mature tooling, fast, nothing to build in chartgen.
- Bad: its indicators, fills, tie rules and costs are not chartgen's, so its R is not comparable
  with live `result_r` and nobody reading a setup can check it (the reason ADR-0066 and ADR-0068
  set aside scoring outside the server). Feeding its signals to option 3's core removes that.

## Questions for the owner

As proposed; each is answered under [Decision Outcome](#decision-outcome).

1. **Which option?** The recommendation is 3. Or run option 2 first, with no code, and decide
   after one rule's result?
2. **Where does it run?** A CLI on a workstation over fixtures (recommended first), a tool on a
   replay server, or a tool on the live server so an agent client can test a rule itself (which
   needs 1m history read within the live server's Binance budget and memory)?
3. **How much can a rule say?** One alert condition on one interval for a start, or wait for AND
   and several intervals in alert conditions?
4. **Entry and bracket defaults:** a market entry at the next 1m open; a stop at the signal bar's
   extreme; below the risk floor refuse (recommended) or widen as a skip's `floor` bracket does;
   target in R; expiry in bars; one open position per rule and symbol?
5. **Reporting:** the live record's figures only (recommended), or also a spread figure such as a
   bootstrap interval on mean R, which ADR-0064 declined for the live record?
6. **Warm-up:** a lead of native-interval bars in the fixture (exact parity with a live alert, a
   format change; recommended), or a warm-up inside the window (no format change, the first days
   lost, signals near a threshold may differ from live)?
7. **Costs:** live costs only, or also a stressed-cost line beside them?
8. **Overfitting and what counts:** should chartgen require a second, untouched window before a
   rule's result is shown as confirmed, and may an agent client count backtest trades toward the
   live setups its rules need? (The second is the client's rule, not chartgen's; chartgen only
   keeps the two apart.)

## Confirmation

The PR that implements this shows, in tests of `src/backtest.rs` on small fixtures whose outcome
is worked out by hand:

- A run of the same rule on the same fixtures twice gives byte-identical answers.
- Fixtures cut after a signal give the same signals up to it, and a fill is never inside its
  signal bar.
- A stop under the risk floor is refused and counted; a signal while the trade is open is
  counted as `skipped_open`; a trade still open when the fixture ends is counted as
  `open_at_end` and is in no closed figure.
- Each trade's fill, exit, status and `result_r` after costs, by hand, and the report's
  `total`, `by_symbol` and `shadows` from `track_record` and `shadow_record` over the trades.
- A rule's conditions are refused and read by the parser and reading of an alert's `and`
  clause, `server_alerts::clauses`, and are read over at most the 999 closed bars an alert reads.
- Nothing is written to a setups store: the CLI opens none.
