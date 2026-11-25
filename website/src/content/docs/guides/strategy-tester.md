---
title: Strategy tester
description: Test a mechanical trading rule over replay fixtures in seconds with chartgen backtest, with no model in the loop. The entry is a server alert's condition, each trade is scored as a setup, and the answer is the track record of its trades.
---

`chartgen backtest` tests a rule over [replay fixtures](/chartgen/guides/replay/): at every close
of the rule's interval it reads the rule's conditions as a server alert reads them, and where all
of them hold it enters a trade that is scored as a [setup](/chartgen/guides/setups/) is, by the
same scorer, with the same costs and exit variants. There is no model in the loop, so a quarter
of five symbols runs in about a second. The decision is
[ADR-0083](/chartgen/decisions/0083-a-rule-based-strategy-tester/).

It tests only the mechanical part of a rule. A rule that is a thesis read from a chart cannot be
written down here, and a backtest number must not stand for it.

## Run it

```bash
chartgen backtest --rule examples/rules/rsi-overbought-short.json \
  --fixtures fixtures/2026-q3 --trades trades.csv
```

| Flag | Meaning |
|------|---------|
| `--rule` | The rule, a JSON file (below). Required. |
| `--fixtures` | A directory of fixtures, as `chartgen replay-fixtures` writes them. Required. |
| `--trades` | Also write every trade to this file: CSV if it ends in `.csv`, else JSON, each trade as `get_setup` shows a setup. |

The answer is JSON on standard output. Nothing is written to a setups store, and no request is
sent anywhere. It exits with status 1 on any error, such as a rule it refuses or a symbol the
fixtures do not hold.

## A rule

```json
{
  "name": "rsi-overbought-short",
  "symbols": ["BTCUSDT", "ETHUSDT"],
  "interval": "1h",
  "direction": "short",
  "when": [
    { "indicator": "rsi", "params": { "length": 14 }, "condition": "crossing_up", "level": 70 }
  ],
  "stop": { "atr": 1 },
  "targets_r": [1.5],
  "expiry": "24h"
}
```

Short when RSI(14) on the 1h closes up through 70; stop one ATR above the fill, target 1.5 R, out
after 24 hours. The example is made up and is in the repository as
`examples/rules/rsi-overbought-short.json`.

| Field | Meaning |
|-------|---------|
| `name` | The rule's name. Its trades are tagged `backtest` and `rule:<name>`. |
| `symbols` | Binance symbols the fixtures hold. Stocks are refused for now. |
| `interval` | The interval the conditions are read on, 1m or longer, built from the fixture's 1m bars as a replay builds it. |
| `direction` | `long` or `short`. |
| `when` | One to four conditions, joined by AND. Each is written as a [server alert's `and` condition](/chartgen/reference/mcp/#conditions-joined-by-and): `condition` (`crossing`, `crossing_up`, `crossing_down`, `greater`, `less`) with a price `level`, or with an `indicator` (`rsi`, `ma`, `macd`, `stoch`, `cci`, `mfi`, `williams_r`, `roc`, `atr`, `adx`), its `params`, `line` and `compare` (`level` or `close`). Funding and open interest are refused: fixtures hold neither. |
| `stop` | `{ "atr": k }`: k times the ATR(14) of the interval at the signal. Or `{ "pct": p }`: p percent of the fill price. |
| `targets_r` | One to four targets in R of the risk at the fill, nearest first. The first decides a win, as on a setup. |
| `expiry` | How long a trade may run from its signal, as an interval: `24h`, `3d`. At most 90 days. |
| `costs` | Optional: `{ "fee_bps_per_side": 5, "slippage_bps_per_side": 2 }`. The live costs when left out. |

A condition is refused or accepted exactly as `create_server_alert` refuses or accepts it as an
`and` condition, with the same messages, so a rule that runs here can be set as a live alert. A
server's indicator alerts read intervals of 1h and longer only.

## How a signal becomes a trade

- **The signal** is read at a bar's close, from closed bars only: the last 999 bars of the
  interval up to that close, as an alert reads them. A crossing compares this close with the
  one before. `greater` and `less` hold at every close where they are true, so with them a new
  trade opens at the first close after the last one ended.
- **The fill** is at the open of the first 1m bar that opens at or after the signal bar's close,
  never inside the signal bar.
- **The levels** are set from the fill price: the stop at the rule's distance, the targets in R
  of that risk, the expiry from the signal. They are checked as `create_setup` checks them; a stop
  closer than the risk floor (three round trips of costs, 42 bps on Binance) is not traded and is
  counted as `refused`, never widened.
- **The scoring** is `apply_bar` on every 1m bar from the fill, the setup scorer: its stop, target,
  expiry and tie rules, its costs, `mae_r` and `mfe_r`, and the three exit variants.
- **Overlap:** one trade per rule and symbol at a time. A signal before the 1m bar in which that
  trade ended has closed is counted as `skipped_open`. The other symbols are not affected.

## The answer

```json
{
  "backtest": true,
  "rule": { "...": "as given" },
  "rule_sha256": "…",
  "fixtures": { "BTCUSDT": { "from": "…", "to": "…", "minutes": 132480 } },
  "costs": { "fee_bps_per_side": 5.0, "slippage_bps_per_side": 2.0 },
  "closes": 4416, "before_warmup": 220, "signals": 61,
  "traded": 47, "skipped_open": 6, "refused": 8, "no_fill": 0, "open_at_end": 1,
  "summary": { "n": 46, "won": 17, "invalidated": 27, "expired": 2,
               "win_rate": 0.37, "mean_r": 0.05, "net_r": 2.3 },
  "total": { "...": "as get_track_record's total" },
  "by_symbol": [ { "key": "BTCUSDT", "...": "per symbol" } ],
  "shadows": { "...": "as get_track_record's shadows" }
}
```

(The numbers are made up.)

| Field | Meaning |
|-------|---------|
| `closes` | Closes of the interval read, over all symbols. |
| `before_warmup` | Closes where a condition, or the ATR of an `atr` stop, had no value yet. |
| `signals` | Closes where every condition held. Each is traded, skipped, refused or not filled. |
| `traded` | Signals that became a trade. |
| `skipped_open` | Signals while the rule's trade on that symbol was open. |
| `refused` | Signals whose levels a setup would refuse. |
| `no_fill` | Signals at the fixture's last close, with no 1m bar after them. |
| `open_at_end` | Trades still open when the fixture's bars run out. They have no result and are in no figure below. |
| `summary` | The closed trades: their count, the statuses of a setup, the share won, and the mean and sum of `result_r`, which is after costs. |
| `total`, `by_symbol`, `shadows` | The figures `get_track_record` gives, by the same functions, over every closed trade. |

`won` says the first target was reached; a won trade whose target is nearer than the costs is
still a loss after costs, which `net_gain` and `net_loss` in `total` count
([ADR-0079](/chartgen/decisions/0079-track-record-after-costs/)).

## Reading a result

- **Warm-up.** Fixtures start at their window's first minute, with nothing before it. A
  condition is read over the bars there are, so for the first 999 bars of the interval an
  indicator is seeded over fewer bars than a live alert's and a signal right at a threshold can
  differ from a live one. On 1h, RSI(14) has settled well within the first weeks.
- **Overfitting.** Every rule you try on a window is a test of that window. Keep a second window
  you have not tried rules on, and run the rule there once before it counts.
- **Costs.** The live costs are what a setup is scored with, not the market's. To see a rule under
  worse fills, run it again with higher `costs`.
- **Survivors.** Symbols that are liquid today are a survivor sample; the answer says which
  symbols it ran on.
- **Gaps.** A minute missing from a fixture stays missing; a trade across it is scored on the
  bars there are.
